//! 池内单个账号条目及其治理状态机（冷却 / 熔断 / 实测成本账本）。
//!
//! 移植自参考实现 `internal/pool/entry.go`、`cooldown.go`、`transition.go`。
//!
//! 关键设计约束（**领域正交**，改动时不得破坏）：
//! - **冷却域**（`until` / `cool_kind` / `reason` / `soft_streak` / `model_cooldowns`）
//!   与**熔断域**（`fails` / `retry_count` / `breaker_until`）互不干预：
//!   禁用/复活会清空冷却域但**不动**熔断域；成功回调会清空熔断域但也清冷却域的一部分。
//! - `fails` **不持久化**（进程内连续失败计数，重启即清零），`retry_count` 持久化
//!   （它是退避指数，重启后不应重新从 30 分钟开始）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::timeutil;

/// 冷却类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoolKind {
    /// 硬冷却：余额不足（402）——只等「次日 04:00」解冻，期间不参与选号。
    Hard,
    /// 软冷却：限流（429）/ 404 / WAF ——按退避时长解冻。
    Soft,
}

/// 模型级独立冷却条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCooldown {
    /// 解冻时刻（毫秒）。
    pub until_ms: i64,
    /// 上游声明的重置时刻（毫秒）；无声明为 0。
    pub reset_at_ms: i64,
    /// 原因标识。
    pub reason: String,
    /// 该模型被判「封禁」的累计次数（`11102` 专用，退避指数来源）。
    ///
    /// `6004`（限流）恒为 `0` —— 两者的退避语义**不同**：
    /// 限流看上游给的墙钟（封顶 `soft_rate_max`），封禁看「这是第几次被罚」并逐次加倍。
    /// 混为一谈的后果是封禁账号的模型会**过早重试**，反复撞同一堵墙、白白消耗额度。
    #[serde(default)]
    pub hits: i32,
}

/// 实测单位成本账本条目（每 1000 token 消耗的积分）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    /// 每 1000 token 成本（EMA 平滑）。
    pub cost_per_1k: f64,
    /// 最近观测时刻（毫秒）。
    pub last_seen_ms: i64,
    /// 观测次数。
    pub samples: u64,
}

/// 成本分层结果（硬过滤用；数值越小越优先）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostTier {
    /// 实测免费（观测到 `cost_per_1k <= 0`）。
    Free,
    /// 无观测或观测已过期——保留学习机会。
    Unknown,
    /// 实测收费。
    Paid,
}

impl CostTier {
    /// 排序权重（越小越优先）。
    pub fn rank(self) -> u8 {
        match self {
            CostTier::Free => 0,
            CostTier::Unknown => 1,
            CostTier::Paid => 2,
        }
    }
}

/// 池内账号条目。
#[derive(Debug, Clone)]
pub struct PoolEntry {
    /// 账号 uid（池键）。
    pub uid: String,
    /// 归属 region（`None` 表示未知，不参与 realm 过滤）。
    pub realm: Option<crate::pool::RealmTag>,
    /// 昵称（仅展示）。
    pub nickname: String,

    /// 可用积分总量。
    pub credits: i64,
    /// 其中「快过期」的子集（四因子之一）。
    pub credits_expiring: i64,
    /// 最近一次余额刷新时刻（毫秒；`0` = 从未取过）。
    ///
    /// 由后台余额刷新循环维护（见 [`crate::credits_refresh`]）：`0` 表示从未取过，
    /// 必须刷新；否则超过 `credits_refresh_interval_ms` 才刷新。
    pub credits_refreshed_ms: i64,
    /// 快过期积分的到期时刻（毫秒；`None` = 无到期时间 / 未知）。
    ///
    /// 与 [`Self::credits_refreshed_ms`] 同批写入（`crate::credits_refresh` 刷新循环），
    /// 供 `earliest_expiry` 策略在池缓存新鲜时**零网络**择优（`crate::account_strategy`）。
    pub credits_expire_at_ms: Option<i64>,

    /// 终身成功次数（展示用）。
    pub success_count: i64,
    /// 终身错误次数（展示用）。
    pub err_total: i64,
    /// 成功率 EMA（α=0.1）。
    pub success_ema: f64,
    /// 错误率 EMA（α=0.1）。
    pub error_ema: f64,
    /// 最近成功时刻（毫秒；0 = 无记录）。
    pub last_success_ms: i64,
    /// 最近失败时刻（毫秒；0 = 无记录）。
    pub last_err_ms: i64,

    /// 账号级冷却截止（毫秒；0 = 无冷却）。
    pub until_ms: i64,
    /// 冷却类别。
    pub cool_kind: Option<CoolKind>,
    /// 冷却原因（展示用）。
    pub reason: String,
    /// 连续软冷却次数（退避指数来源）。
    pub soft_streak: i32,
    /// 模型级独立冷却表。
    pub model_cooldowns: HashMap<String, ModelCooldown>,

    /// 是否已禁用（终态，需人工/刷新成功复活）。
    pub disabled: bool,
    /// 是否已停用（账号库中的 `deactivated` 字段，持久化）。
    /// 停用的账号不会在网关池中被选中，但仍在账号库中保留。
    pub deactivated: bool,
    /// 连续会话失效计数（12153 三振）。
    pub session_dead_fails: i32,

    /// 熔断截止（毫秒；0 = 无熔断）。
    pub breaker_until_ms: i64,
    /// 连续失败计数（**不持久化**）。
    pub fails: i32,
    /// 已熔断次数（退避指数；持久化）。
    pub retry_count: i32,

    /// 最近被选中时刻（毫秒；0 = 从未）。
    pub last_used_ms: i64,
    /// 选中单调序号（LRU 权威依据）。
    pub used_seq: u64,
    /// 当前在途请求数。
    pub in_flight: i64,

    /// 实测成本账本（**不持久化**，重启重学）。
    pub model_cost: HashMap<String, ModelCost>,
}

impl PoolEntry {
    /// 新建条目。
    pub fn new(uid: impl Into<String>) -> Self {
        Self {
            uid: uid.into(),
            realm: None,
            nickname: String::new(),
            credits: 0,
            credits_expiring: 0,
            credits_refreshed_ms: 0,
            credits_expire_at_ms: None,
            success_count: 0,
            err_total: 0,
            success_ema: 0.0,
            error_ema: 0.0,
            last_success_ms: 0,
            last_err_ms: 0,
            until_ms: 0,
            cool_kind: None,
            reason: String::new(),
            soft_streak: 0,
            model_cooldowns: HashMap::new(),
            disabled: false,
            deactivated: false,
            session_dead_fails: 0,
            breaker_until_ms: 0,
            fails: 0,
            retry_count: 0,
            last_used_ms: 0,
            used_seq: 0,
            in_flight: 0,
            model_cost: HashMap::new(),
        }
    }

    /// 账号级是否可用（不看模型）。
    pub fn healthy(&self, now_ms: i64) -> bool {
        !self.disabled && now_ms >= self.until_ms && now_ms >= self.breaker_until_ms
    }

    /// 账号级 + 指定模型是否可用。
    pub fn healthy_for_model(&self, now_ms: i64, model: &str) -> bool {
        if !self.healthy(now_ms) {
            return false;
        }
        if model.is_empty() {
            return true;
        }
        match self.model_cooldowns.get(model) {
            Some(cooldown) => now_ms >= cooldown.until_ms,
            None => true,
        }
    }

    /// 请求维度的健康判定：带模型名时同时看模型级冷却。
    pub fn healthy_for_request(&self, now_ms: i64, model: &str) -> bool {
        if model.is_empty() {
            self.healthy(now_ms)
        } else {
            self.healthy_for_model(now_ms, model)
        }
    }

    /// 惰性清理已过期的模型级冷却；返回是否有条目被移除。
    pub fn prune_model_cooldowns(&mut self, now_ms: i64) -> bool {
        let before = self.model_cooldowns.len();
        self.model_cooldowns
            .retain(|_, cooldown| now_ms < cooldown.until_ms);
        before != self.model_cooldowns.len()
    }

    /// 清空冷却域（**不动**熔断域）。
    ///
    /// 禁用与复活共用这一清理，语义是「账号级的限制都解除，但熔断历史保留」。
    pub fn clear_cooling(&mut self) {
        self.until_ms = 0;
        self.cool_kind = None;
        self.reason.clear();
        self.soft_streak = 0;
        self.session_dead_fails = 0;
        self.model_cooldowns.clear();
    }

    /// 成功回调：清空熔断域与软冷却退避。
    ///
    /// **不清模型级冷却**——该模型刚失败过，让它自然过期更安全。
    pub fn note_success(&mut self, now_ms: i64) {
        self.success_count += 1;
        self.last_success_ms = now_ms;
        self.success_ema = ema_step(self.success_ema, 1.0);
        self.error_ema = ema_step(self.error_ema, 0.0);
        self.fails = 0;
        self.retry_count = 0;
        self.breaker_until_ms = 0;
        self.soft_streak = 0;
        self.session_dead_fails = 0;
    }

    /// 失败回调：累计成功率 EMA，并推进熔断计数。
    pub fn note_error(&mut self, now_ms: i64) {
        self.err_total += 1;
        self.last_err_ms = now_ms;
        self.success_ema = ema_step(self.success_ema, 0.0);
        self.error_ema = ema_step(self.error_ema, 1.0);
        self.fails += 1;
    }

    /// 推进熔断：达到阈值则按 `retry_count` 做指数退避并**清零 fails、递增 retry_count**。
    ///
    /// 退避序列（threshold=3、base=30m、max=6h）：30m → 1h → 2h → 6h → 6h…
    /// 返回是否触发熔断。
    pub fn maybe_trip_breaker(
        &mut self,
        now_ms: i64,
        threshold: i32,
        base_ms: i64,
        max_ms: i64,
    ) -> bool {
        if threshold <= 0 || self.fails < threshold {
            return false;
        }
        let mut duration = base_ms;
        for _ in 0..self.retry_count {
            duration = duration.saturating_mul(2);
            if duration >= max_ms {
                duration = max_ms;
                break;
            }
        }
        self.fails = 0;
        self.retry_count = self.retry_count.saturating_add(1);
        self.breaker_until_ms = now_ms.saturating_add(duration);
        true
    }

    /// 软冷却（有上游重置墙钟）：`min(reset_at, now + soft_rate_max)`；
    /// 墙钟已过则退化为 1ms 的瞬时冷却（让下一次选号立刻可重试）。
    pub fn cooldown_soft_until(&mut self, now_ms: i64, reset_at_ms: i64, max_ms: i64, reason: &str) {
        let cap = now_ms.saturating_add(if max_ms > 0 { max_ms } else { DEFAULT_SOFT_RATE_MAX_MS });
        let until = if reset_at_ms > cap {
            cap
        } else if reset_at_ms > now_ms {
            reset_at_ms
        } else {
            now_ms.saturating_add(1)
        };
        self.until_ms = until;
        self.cool_kind = Some(CoolKind::Soft);
        self.reason = reason.to_string();
        self.model_cooldowns.clear();
    }

    /// 软冷却（无上游墙钟）：按连续次数做**有界指数退避**。
    ///
    /// 已在有效软冷却中时**兜底探测不翻倍、不延长**——否则持续探测会把冷却推向封顶，
    /// 反而让账号长期不可用。返回是否实际更新了冷却。
    pub fn cooldown_soft_backoff(&mut self, now_ms: i64, base_ms: i64, max_ms: i64, reason: &str) -> bool {
        if self.cool_kind == Some(CoolKind::Soft) && now_ms < self.until_ms {
            return false;
        }
        let next_streak = self.soft_streak.saturating_add(1);
        let duration = soft_backoff_duration(base_ms, next_streak, max_ms);
        self.soft_streak = next_streak;
        self.until_ms = now_ms.saturating_add(duration);
        self.cool_kind = Some(CoolKind::Soft);
        self.reason = reason.to_string();
        self.model_cooldowns.clear();
        true
    }

    /// 固定时长冷却（404 / WAF 抖动），不参与退避计数，**不动** `soft_streak`。
    pub fn cooldown_fixed(&mut self, now_ms: i64, duration_ms: i64, reason: &str) {
        self.until_ms = now_ms.saturating_add(duration_ms);
        self.cool_kind = Some(CoolKind::Soft);
        self.reason = reason.to_string();
        self.model_cooldowns.clear();
    }

    /// 硬冷却至次日 04:00（CST），用于 402 余额不足。
    pub fn cooldown_until_tomorrow_4am(&mut self, now_ms: i64, reason: &str) {
        self.until_ms = timeutil::next_day_4am(now_ms);
        self.cool_kind = Some(CoolKind::Hard);
        self.reason = reason.to_string();
        self.model_cooldowns.clear();
    }

    /// 模型级独立冷却：**不写账号级 `until`**，因此切其它模型立即可用。
    pub fn cooldown_model(&mut self, now_ms: i64, model: &str, reset_at_ms: i64, max_ms: i64, reason: &str) {
        if model.is_empty() {
            return;
        }
        let cap = now_ms.saturating_add(if max_ms > 0 { max_ms } else { DEFAULT_SOFT_RATE_MAX_MS });
        let until = if reset_at_ms > cap {
            cap
        } else if reset_at_ms > now_ms {
            reset_at_ms
        } else {
            now_ms.saturating_add(1)
        };
        self.model_cooldowns.insert(
            model.to_string(),
            ModelCooldown {
                until_ms: until,
                reset_at_ms,
                reason: reason.to_string(),
                hits: 0,
            },
        );
    }

    /// 模型封禁退避基准（6h）。
    const MODEL_BLOCK_BASE_MS: i64 = 6 * 60 * 60 * 1000;

    /// 模型封禁退避封顶（24h）。
    const MODEL_BLOCK_MAX_MS: i64 = 24 * 60 * 60 * 1000;

    /// 模型封禁退避时长：`6h × 2^(hits-1)`，封顶 24h。
    ///
    /// 序列：hits=1 → 6h，2 → 12h，3 → 24h，4 及以上 → 24h。
    fn model_block_ttl_ms(hits: i32) -> i64 {
        if hits <= 0 {
            return Self::MODEL_BLOCK_BASE_MS;
        }
        let mut ttl = Self::MODEL_BLOCK_BASE_MS;
        for _ in 1..hits {
            ttl = ttl.saturating_mul(2);
            if ttl >= Self::MODEL_BLOCK_MAX_MS {
                return Self::MODEL_BLOCK_MAX_MS;
            }
        }
        ttl
    }

    /// 模型**封禁**退避（`11102`）：按「被罚次数」逐次加倍，返回本次用的时长。
    ///
    /// 与 [`Self::cooldown_model`]（限流：看上游墙钟、封顶 `soft_rate_max`）**语义不同**，
    /// 所以是独立方法而不是加个参数——混用会让被封禁的模型**过早重试**，反复撞同一堵墙、
    /// 白白消耗额度，而「逐次加倍」的设计意图正是让反复违规者被罚得越来越久。
    ///
    /// `hits` 随 [`ModelCooldown`] 一起**持久化**，因此重启不会让退避从头开始。
    /// 仍**不写账号级 `until`**：切到其它模型立即可用。
    pub fn block_model_backoff(&mut self, now_ms: i64, model: &str, reason: &str) -> i64 {
        if model.is_empty() {
            return 0;
        }
        let hits = self
            .model_cooldowns
            .get(model)
            .map(|cooldown| cooldown.hits)
            .unwrap_or(0)
            .saturating_add(1);
        let ttl = Self::model_block_ttl_ms(hits);
        self.model_cooldowns.insert(
            model.to_string(),
            ModelCooldown {
                until_ms: now_ms.saturating_add(ttl),
                reset_at_ms: 0,
                reason: reason.to_string(),
                hits,
            },
        );
        ttl
    }

    /// 会话失效计数 +1；达到阈值则禁用。返回是否**本次**触发了禁用。
    pub fn note_session_dead(&mut self, threshold: i32, reason: &str) -> bool {
        self.session_dead_fails = self.session_dead_fails.saturating_add(1);
        if threshold > 0 && self.session_dead_fails >= threshold {
            self.disable(reason);
            return true;
        }
        false
    }

    /// 禁用并清空冷却域（熔断域保留）。
    ///
    /// 注意顺序：**必须先清冷却域再写 reason**。`clear_cooling` 会一并清空 `reason`
    /// （冷却原因与禁用原因共用该字段），若颠倒顺序，禁用原因会被自己抹掉，
    /// `/status` 的 `disabled_reason` 将恒为空串。
    pub fn disable(&mut self, reason: &str) {
        self.clear_cooling();
        self.disabled = true;
        self.reason = reason.to_string();
    }

    /// 复活（刷新成功/人工解除）：解除禁用并清空冷却域。
    pub fn revive(&mut self) {
        self.disabled = false;
        self.clear_cooling();
    }

    /// 冷却剩余毫秒（无冷却为 0）。
    pub fn cool_remaining_ms(&self, now_ms: i64) -> i64 {
        (self.until_ms - now_ms).max(0)
    }

    /// 解冻时刻（取账号级冷却与熔断的较早者；都未定时返回 0）。
    ///
    /// 全冷却兜底选号用它挑「最早会解冻」的账号。
    pub fn expiry_ms(&self) -> i64 {
        match (self.until_ms, self.breaker_until_ms) {
            (0, 0) => 0,
            (0, breaker) => breaker,
            (until, 0) => until,
            (until, breaker) => until.min(breaker),
        }
    }

    /// 实测成本分层。
    pub fn cost_tier(&self, now_ms: i64, model: &str, ttl_ms: i64) -> CostTier {
        if model.is_empty() {
            return CostTier::Unknown;
        }
        match self.model_cost.get(model) {
            Some(cost) if now_ms.saturating_sub(cost.last_seen_ms) <= ttl_ms => {
                if cost.cost_per_1k <= 0.0 {
                    CostTier::Free
                } else {
                    CostTier::Paid
                }
            }
            _ => CostTier::Unknown,
        }
    }

    /// 当前有效单价（无观测/过期返回 0）。
    pub fn cost_per_1k(&self, now_ms: i64, model: &str, ttl_ms: i64) -> f64 {
        match self.model_cost.get(model) {
            Some(cost) if now_ms.saturating_sub(cost.last_seen_ms) <= ttl_ms => cost.cost_per_1k,
            _ => 0.0,
        }
    }

    /// 记录一次实测消耗：`per1k = credit / tokens * 1000`，EMA α=0.3。
    ///
    /// 不记录的情形：`tokens <= 0`、`model` 为空。`credit` 为负按 0 计。
    /// 返回是否写入了账本。
    pub fn record_cost(&mut self, now_ms: i64, model: &str, credit: f64, tokens: i64) -> bool {
        if model.is_empty() || tokens <= 0 {
            return false;
        }
        let per_1k = if credit < 0.0 { 0.0 } else { credit } / tokens as f64 * 1000.0;
        match self.model_cost.get_mut(model) {
            Some(entry) => {
                entry.cost_per_1k = entry.cost_per_1k * (1.0 - COST_EMA_ALPHA) + per_1k * COST_EMA_ALPHA;
                entry.last_seen_ms = now_ms;
                entry.samples += 1;
            }
            None => {
                self.model_cost.insert(
                    model.to_string(),
                    ModelCost {
                        cost_per_1k: per_1k,
                        last_seen_ms: now_ms,
                        samples: 1,
                    },
                );
            }
        }
        true
    }

    /// 按实测消耗扣减余额（四舍五入，扣穿钳 0），并同步压缩快过期子集。
    pub fn deduct_credits(&mut self, credit: f64) {
        if credit <= 0.0 {
            return;
        }
        let delta = (credit + 0.5).floor() as i64;
        self.credits = (self.credits - delta).max(0);
        self.credits_expiring = self.credits_expiring.min(self.credits).max(0);
    }
}

/// 成功率 EMA 平滑系数。
const SUCCESS_EMA_ALPHA: f64 = 0.1;
/// 成本 EMA 平滑系数。
const COST_EMA_ALPHA: f64 = 0.3;
/// 软冷却封顶缺省值（2h）。
pub const DEFAULT_SOFT_RATE_MAX_MS: i64 = 2 * 60 * 60 * 1000;
/// 软冷却连续次数偏移上限（防 2^n 溢出）。
const SOFT_STREAK_SHIFT_MAX: u32 = 16;

fn ema_step(previous: f64, sample: f64) -> f64 {
    previous * (1.0 - SUCCESS_EMA_ALPHA) + sample * SUCCESS_EMA_ALPHA
}

/// 软冷却退避时长：`streak <= 1` 原样；否则左移 `min(streak-1, 16)` 位，超封顶取封顶。
pub fn soft_backoff_duration(base_ms: i64, streak: i32, max_ms: i64) -> i64 {
    let cap = if max_ms > 0 { max_ms } else { DEFAULT_SOFT_RATE_MAX_MS };
    if base_ms <= 0 {
        return cap;
    }
    if streak <= 1 {
        return base_ms.min(cap);
    }
    let shift = ((streak - 1) as u32).min(SOFT_STREAK_SHIFT_MAX);
    match base_ms.checked_shl(shift) {
        Some(value) if value > 0 && value < cap => value,
        _ => cap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> PoolEntry {
        PoolEntry::new("u1")
    }

    #[test]
    fn healthy_respects_disable_and_both_deadlines() {
        let mut item = entry();
        assert!(item.healthy(1000));

        item.until_ms = 2000;
        assert!(!item.healthy(1000));
        assert!(item.healthy(2000), "边界：等于截止时刻即解冻");

        item.until_ms = 0;
        item.breaker_until_ms = 3000;
        assert!(!item.healthy(2999));
        assert!(item.healthy(3000));

        item.breaker_until_ms = 0;
        item.disabled = true;
        assert!(!item.healthy(999_999), "禁用是终态，与时间无关");
    }

    #[test]
    fn model_cooldown_is_orthogonal_to_account_cooldown() {
        let mut item = entry();
        item.cooldown_model(1000, "glm-5.2", 0, DEFAULT_SOFT_RATE_MAX_MS, "6004 model rate limit");

        assert_eq!(item.until_ms, 0, "模型级冷却不得写账号级 until");
        assert!(!item.healthy_for_model(1000, "glm-5.2"));
        assert!(item.healthy_for_model(1000, "deepseek-v4-flash"), "切模型立即可用");
        assert!(item.healthy(1000), "账号维度仍健康");
        // 不带模型名时只看账号维度
        assert!(item.healthy_for_request(1000, ""));
        // 到期后自动解冻
        assert!(item.healthy_for_model(1000 + DEFAULT_SOFT_RATE_MAX_MS, "glm-5.2"));
    }

    #[test]
    fn prune_model_cooldowns_removes_expired_only() {
        let mut item = entry();
        // reset_at 未声明（0）时冷却退化为"瞬时"（now+1ms），并不按 max 计时——
        // 要让条目真正存活一段时间，必须给出**未来**的 reset_at 时刻。
        item.cooldown_model(1000, "a", 0, 500, "x");
        item.cooldown_model(1000, "b", 3000, 10_000, "y");
        assert_eq!(item.model_cooldowns["a"].until_ms, 1001, "未声明墙钟 → 瞬时冷却");
        assert_eq!(item.model_cooldowns["b"].until_ms, 3000, "未来墙钟 → 直接采用");

        assert!(item.prune_model_cooldowns(2000));
        assert!(!item.model_cooldowns.contains_key("a"), "已过期应被清理");
        assert!(item.model_cooldowns.contains_key("b"), "未过期应保留");
        assert!(!item.prune_model_cooldowns(2000), "无变化时返回 false");
    }

    #[test]
    fn soft_backoff_doubles_then_caps() {
        let max = 6 * 60 * 60 * 1000;
        let base = 600_000;
        assert_eq!(soft_backoff_duration(base, 1, max), base);
        assert_eq!(soft_backoff_duration(base, 2, max), base * 2);
        assert_eq!(soft_backoff_duration(base, 3, max), base * 4);
        assert_eq!(soft_backoff_duration(base, 20, max), max, "必须封顶");
    }

    #[test]
    fn soft_backoff_does_not_extend_while_already_cooling() {
        let mut item = entry();
        assert!(item.cooldown_soft_backoff(1000, 600_000, DEFAULT_SOFT_RATE_MAX_MS, "429"));
        let until = item.until_ms;
        assert_eq!(item.soft_streak, 1);

        // 冷却中再次探测 → 不翻倍、不延长
        assert!(!item.cooldown_soft_backoff(2000, 600_000, DEFAULT_SOFT_RATE_MAX_MS, "429"));
        assert_eq!(item.until_ms, until);
        assert_eq!(item.soft_streak, 1);

        // 冷却结束后再次触发 → 退避翻倍
        assert!(item.cooldown_soft_backoff(until + 1, 600_000, DEFAULT_SOFT_RATE_MAX_MS, "429"));
        assert_eq!(item.soft_streak, 2);
        assert_eq!(item.until_ms, until + 1 + 1_200_000);
    }

    #[test]
    fn soft_until_clamps_to_cap_and_handles_past_wallclock() {
        let mut item = entry();
        let cap = 2 * 60 * 60 * 1000;
        // 上游给的墙钟远超封顶 → 钳到 now+cap
        item.cooldown_soft_until(1000, 999_999_999, cap, "429 rate limit");
        assert_eq!(item.until_ms, 1000 + cap);

        // 墙钟在未来但未超封顶 → 直接用墙钟
        item.cooldown_soft_until(1000, 5000, cap, "429 rate limit");
        assert_eq!(item.until_ms, 5000);

        // 墙钟已过 → 瞬时冷却（1ms），下一次选号立刻可试
        item.cooldown_soft_until(10_000, 5_000, cap, "429 rate limit");
        assert_eq!(item.until_ms, 10_001);
    }

    #[test]
    fn hard_cooldown_clears_model_cooldowns() {
        let mut item = entry();
        item.cooldown_model(1000, "m", 0, 1000, "x");
        item.cooldown_until_tomorrow_4am(1000, "余额不足");
        assert_eq!(item.cool_kind, Some(CoolKind::Hard));
        assert!(item.model_cooldowns.is_empty(), "硬冷却清空模型级表");
        assert!(item.until_ms > 1000);
    }

    #[test]
    fn model_block_backoff_escalates_then_caps_at_24h() {
        let mut item = entry();
        let hour = 60 * 60 * 1000;

        // 序列：6h → 12h → 24h → 封顶 24h（与 6004 的「看上游墙钟、封顶 2h」完全不同）
        let expected = [6 * hour, 12 * hour, 24 * hour, 24 * hour];
        for (index, ttl) in expected.iter().enumerate() {
            let returned = item.block_model_backoff(1000, "m", "11102 model blocked");
            assert_eq!(returned, *ttl, "第 {} 次封禁退避时长不符", index + 1);
            let cooldown = &item.model_cooldowns["m"];
            assert_eq!(cooldown.until_ms, 1000 + ttl, "until 应为 now + ttl");
            assert_eq!(cooldown.hits, (index + 1) as i32, "hits 应逐次累计");
            assert_eq!(
                cooldown.reset_at_ms, 0,
                "封禁没有上游墙钟，reset_at 必须留空"
            );
        }
        assert_eq!(item.until_ms, 0, "模型级退避不得写账号级 until");
    }

    #[test]
    fn model_block_makes_only_that_model_unhealthy() {
        let mut item = entry();
        item.block_model_backoff(1000, "glm-5.2", "11102 model blocked");
        let until = 1000 + 6 * 60 * 60 * 1000;
        assert!(!item.healthy_for_model(until - 1, "glm-5.2"), "封禁期内不可用");
        assert!(item.healthy_for_model(until, "glm-5.2"), "到期即解禁");
        assert!(
            item.healthy_for_model(1000, "deepseek-v4-flash"),
            "其它模型不受影响"
        );
    }

    #[test]
    fn model_block_with_empty_model_is_a_noop() {
        let mut item = entry();
        assert_eq!(item.block_model_backoff(1000, "", "11102"), 0);
        assert!(item.model_cooldowns.is_empty(), "无模型名不应产生条目");
    }

    #[test]
    fn rate_limit_and_block_share_the_map_but_use_different_ttl_rules() {
        let mut item = entry();
        // 先限流（hits 恒 0、按墙钟/封顶 soft_rate_max）
        item.cooldown_model(1000, "m", 0, DEFAULT_SOFT_RATE_MAX_MS, "6004 model rate limit");
        assert_eq!(item.model_cooldowns["m"].hits, 0, "限流不参与封禁计数");
        assert_eq!(
            item.model_cooldowns["m"].until_ms, 1001,
            "限流走「未声明墙钟 → 瞬时冷却」语义"
        );

        // 再封禁：同一模型共用条目，但封禁计数从 1 起、时长 6h
        let ttl = item.block_model_backoff(2000, "m", "11102 model blocked");
        assert_eq!(ttl, 6 * 60 * 60 * 1000, "封禁必须用 6h 基准而非 2h 封顶");
        assert_eq!(item.model_cooldowns["m"].hits, 1);
        assert!(item.model_cooldowns["m"].reason.starts_with("11102"));
    }

    #[test]
    fn fixed_cooldown_does_not_touch_soft_streak() {
        let mut item = entry();
        item.cooldown_soft_backoff(1000, 600_000, DEFAULT_SOFT_RATE_MAX_MS, "429");
        let streak = item.soft_streak;
        item.cooldown_fixed(1000, 60_000, "upstream 404");
        assert_eq!(item.until_ms, 61_000);
        assert_eq!(item.soft_streak, streak, "404 浅冷却不参与退避计数");
    }

    #[test]
    fn breaker_sequence_is_thirty_minutes_then_doubling_to_cap() {
        let max = 6 * 60 * 60 * 1000;
        let base = 30 * 60 * 1000;
        let mut item = entry();

        // 退避规则是「每次翻倍直到封顶」，因此序列为 30m → 1h → 2h → 4h → 6h（封顶）。
        let expected_sequence = [base, base * 2, base * 4, base * 8, max];
        for (index, expected) in expected_sequence.iter().enumerate() {
            for _ in 0..3 {
                item.note_error(1000);
            }
            assert!(item.maybe_trip_breaker(1000, 3, base, max));
            assert_eq!(
                item.breaker_until_ms - 1000,
                *expected,
                "第 {} 次熔断退避时长不符",
                index + 1
            );
        }
        // 封顶后恒定 6h
        for _ in 0..3 {
            item.note_error(1000);
        }
        item.maybe_trip_breaker(1000, 3, base, max);
        assert_eq!(item.breaker_until_ms - 1000, max, "封顶后不得继续放大");
    }

    #[test]
    fn breaker_requires_threshold_and_does_not_trip_below_it() {
        let mut item = entry();
        item.note_error(1000);
        item.note_error(1000);
        assert!(!item.maybe_trip_breaker(1000, 3, 1000, 10_000));
        assert_eq!(item.fails, 2, "未达阈值不得清零");
        assert_eq!(item.breaker_until_ms, 0);
    }

    #[test]
    fn note_success_clears_breaker_domain_but_keeps_model_cooldowns() {
        let mut item = entry();
        for _ in 0..3 {
            item.note_error(1000);
        }
        item.maybe_trip_breaker(1000, 3, 1000, 10_000);
        // 顺序要紧：账号级软冷却会清空模型级表，因此模型冷却必须最后写入。
        item.cooldown_soft_backoff(1000, 1000, 10_000, "429");
        item.cooldown_model(1000, "m", 50_000, 100_000, "6004");

        item.note_success(2000);
        assert_eq!(item.fails, 0);
        assert_eq!(item.retry_count, 0, "成功必须重置退避指数");
        assert_eq!(item.breaker_until_ms, 0);
        assert_eq!(item.soft_streak, 0);
        assert!(
            item.model_cooldowns.contains_key("m"),
            "成功不清模型级冷却（该模型刚失败过，让它自然过期）"
        );
        assert_eq!(item.success_count, 1);
        assert!(item.success_ema > 0.0);
    }

    #[test]
    fn session_dead_disables_only_after_threshold() {
        let mut item = entry();
        assert!(!item.note_session_dead(3, "12153 session dead"));
        assert!(!item.note_session_dead(3, "12153 session dead"));
        assert!(!item.disabled);
        assert!(item.note_session_dead(3, "12153 session dead"));
        assert!(item.disabled, "连续 3 次才禁用");
        assert_eq!(item.until_ms, 0, "禁用清空冷却域");
    }

    #[test]
    fn disable_and_revive_do_not_touch_breaker_domain() {
        let mut item = entry();
        for _ in 0..3 {
            item.note_error(1000);
        }
        item.maybe_trip_breaker(1000, 3, 1000, 10_000);
        let breaker = item.breaker_until_ms;
        let retry = item.retry_count;

        item.disable("banned");
        assert!(item.disabled);
        assert_eq!(item.breaker_until_ms, breaker, "禁用不得动熔断域");
        assert_eq!(item.retry_count, retry);

        item.revive();
        assert!(!item.disabled);
        assert_eq!(item.breaker_until_ms, breaker, "复活不得动熔断域");
    }

    #[test]
    fn disable_keeps_its_reason_after_clearing_cooling() {
        // 回归：`disable` 曾先写 reason 再清冷却域，而清冷却域会一并清空 reason
        // （两者共用同一字段），导致 `/status` 的 disabled_reason 恒为空串。
        let mut item = entry();
        item.cooldown_fixed(1000, 5000, "upstream 404");
        item.disable("12153 session dead");

        assert!(item.disabled);
        assert_eq!(item.reason, "12153 session dead", "禁用原因不得被清空");
        assert_eq!(item.until_ms, 0, "冷却域仍必须被清空");
        assert_eq!(item.cool_kind, None);
    }

    #[test]
    fn cost_tier_layering_and_ttl_expiry() {
        let mut item = entry();
        assert_eq!(item.cost_tier(1000, "m", 1000), CostTier::Unknown, "无观测");
        assert_eq!(item.cost_tier(1000, "", 1000), CostTier::Unknown, "空模型名");

        item.record_cost(1000, "free", 0.0, 100);
        assert_eq!(item.cost_tier(1000, "free", 6000), CostTier::Free);

        item.record_cost(1000, "paid", 5.0, 1000);
        assert_eq!(item.cost_tier(1000, "paid", 6000), CostTier::Paid);

        // 超过 TTL → 视为无观测
        assert_eq!(item.cost_tier(8000, "free", 6000), CostTier::Unknown);
    }

    #[test]
    fn record_cost_uses_ema_and_skips_invalid_samples() {
        let mut item = entry();
        assert!(!item.record_cost(1000, "", 1.0, 100), "空模型名不记录");
        assert!(!item.record_cost(1000, "m", 1.0, 0), "tokens<=0 不记录");

        // 首见直接赋值：1 credit / 1000 tokens → 每千 1.0
        assert!(item.record_cost(1000, "m", 1.0, 1000));
        let first = item.model_cost["m"].cost_per_1k;
        assert!((first - 1.0).abs() < 1e-9, "首见应直接赋值: {first}");

        // 第二次：cost EMA α=0.3 → 1.0*0.7 + 3.0*0.3 = 1.6
        item.record_cost(2000, "m", 3.0, 1000);
        let second = item.model_cost["m"].cost_per_1k;
        assert!((second - 1.6).abs() < 1e-9, "EMA α=0.3 计算错误: {second}");
        assert_eq!(item.model_cost["m"].samples, 2);
    }

    #[test]
    fn negative_credit_counts_as_free() {
        let mut item = entry();
        item.record_cost(1000, "m", -5.0, 1000);
        assert_eq!(item.model_cost["m"].cost_per_1k, 0.0);
        assert_eq!(item.cost_tier(1000, "m", 6000), CostTier::Free);
    }

    #[test]
    fn deduct_credits_rounds_and_clamps() {
        let mut item = entry();
        item.credits = 10;
        item.credits_expiring = 8;
        item.deduct_credits(0.4);
        assert_eq!(item.credits, 10, "0.4 四舍五入为 0");
        item.deduct_credits(0.6);
        assert_eq!(item.credits, 9);
        assert_eq!(item.credits_expiring, 8);

        item.deduct_credits(1000.0);
        assert_eq!(item.credits, 0, "扣穿必须钳 0");
        assert_eq!(item.credits_expiring, 0, "快过期子集同步压缩");
    }

    #[test]
    fn expiry_prefers_the_earlier_deadline() {
        let mut item = entry();
        assert_eq!(item.expiry_ms(), 0, "都未定 → 0");
        item.until_ms = 5000;
        assert_eq!(item.expiry_ms(), 5000);
        item.breaker_until_ms = 3000;
        assert_eq!(item.expiry_ms(), 3000, "取较早者");
        item.until_ms = 0;
        assert_eq!(item.expiry_ms(), 3000);
    }
}
