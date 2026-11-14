//! 账号级**限额护栏**（G1 余额地板 / G2 当日 token / G4 单模型当日 token）+ 402 余额停放。
//!
//! ## 为什么在网关侧，而不是账号库
//!
//! 护栏判的是「**这台网关**还能不能拿这个号去转发」，计数来源是网关自己的记账漏斗
//! （[`super::QoderGatewayState::record_request`]）。把它写进账号库会让「凭据文件」
//! 变成账本，而账号库是要被导出 / 导入 / 手工编辑的。因此计数落在**独立文件**
//! `qoder/gateway_usage.{region}.json`（按区域分家，见 `core::qoder::paths`）。
//!
//! ## 三条硬约束（与参考实现 R1 / R2 / R3 同源）
//!
//! 1. **判据纯内存读**：选号路径上**绝不**打上游。余额是慢变数据，每个请求多一次往返
//!    会直接拉高端到端延迟 —— 所以余额只由「额度视图」写快照（见 [`note_credits`]）。
//! 2. **未知一律放行**（fail-open）：计数未知 / 余额未知 / 快照过期 ⇒ 不拦。
//!    「猜着拦」的代价是把好账号标成耗尽：参考实现踩过「护栏停号 → 402 永不解封 →
//!    汇总显示 0」那一整串。
//! 3. **计数来自单一漏斗，不从环形日志推导**：请求日志是**环形的**，整窗重算会把
//!    「今日已用量」算小（本仓 `token_daily::gateway_round` 的水位线教训同源）。
//!    这里的计数是**只增**的（按日桶），落盘节流 5s，崩溃最多丢几秒的计数。
//!
//! ## 与 G3（当日 credit 上限）的关系
//!
//! **刻意不做**：上游不按请求返回 credit 成本，账户级「今日 consumed credits」只有
//! 客户端日志那一路（量的是 IDE 自己的用量，不是网关转发的量）。两者混用会让网关在
//! 用户自己写代码时被莫名拦下 —— 宁可没有，也不摆一个语义错误的开关。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use buddy_switch_core::modules::qoder::paths;
use buddy_switch_core::modules::qoder::region::QoderRegion;

use super::QoderGatewayConfig;
use crate::timeutil;

/// 计数保留天数。护栏只看「今日」，多留几天是为了排查跨天 / 时区问题。
pub const QUOTA_RETENTION_DAYS: usize = 7;

/// 余额快照的**有效期**（30 分钟）。
///
/// 余额是慢变数据，但也不能拿「昨天看到的余额」去拦今天的请求：过期即视为**未知**
/// （放行），等下一次额度视图刷新再判。
pub const CREDITS_SNAPSHOT_TTL_MS: i64 = 30 * 60 * 1000;

/// 落盘节流（毫秒）。一次请求一次写盘太贵，护栏计数允许这点误差。
const FLUSH_INTERVAL_MS: i64 = 5_000;

fn now_ms() -> i64 {
    crate::timeutil::now_ms()
}

// ---------------------------------------------------------------------------
// 限额（来自网关配置）
// ---------------------------------------------------------------------------

/// 三条**可配**的限额（`0` = 关闭该条）。
// ⚠️ 只用 `PartialEq`：`daily_credit_limit` 是 `f64`（`Eq` 不适用）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct QuotaLimits {
    /// G1：余额地板 —— `remaining <= reserve` 即停（含等号）。
    pub reserve_credits: i64,
    /// G2：当日 token 总额（账号级）。
    pub daily_token_limit: i64,
    /// G3：当日 **credit** 总额（账号级；上游 `usage.credits` 是浮点）。
    pub daily_credit_limit: f64,
    /// G4：单模型当日 token 额。
    pub model_daily_token_limit: i64,
}

impl QuotaLimits {
    /// 从网关配置取（**归一在这里**：负数一律按 0 = 关闭处理，别把负值带上判据）。
    pub fn from_config(config: &QoderGatewayConfig) -> Self {
        Self {
            reserve_credits: config.reserve_credits.max(0),
            daily_token_limit: config.daily_token_limit.max(0),
            // 浮点阈值：NaN / 负数一律按 0（关）处理，别把 NaN 带上判据（比较恒假）。
            daily_credit_limit: if config.daily_credit_limit.is_finite() {
                config.daily_credit_limit.max(0.0)
            } else {
                0.0
            },
            model_daily_token_limit: config.model_daily_token_limit.max(0),
        }
    }

    /// 是否有任何一条真的开着。
    pub fn any(&self) -> bool {
        self.reserve_credits > 0
            || self.daily_token_limit > 0
            || self.daily_credit_limit > 0.0
            || self.model_daily_token_limit > 0
    }
}

/// 被护栏拦下的**原因**（每一种都能对用户说清，不留「莫名其妙不可用」）。
#[derive(Debug, Clone, PartialEq)]
pub enum QuotaBlock {
    /// 402 余额停放中（等下一个额度重置窗口）。
    Parked { until_ms: i64, reason: String },
    /// G1 余额地板。
    Reserve { reserve: i64, remaining: i64 },
    /// G2 当日 token 总额。
    DailyTokens { limit: i64, used: u64 },
    /// G3 当日 credit 总额。
    DailyCredits { limit: f64, used: f64 },
    /// G4 单模型当日 token。
    ModelTokens {
        model: String,
        limit: i64,
        used: u64,
    },
}

impl QuotaBlock {
    /// 稳定的机器可读代号（写进池诊断 / 接口，便于界面与日志对齐）。
    pub fn code(&self) -> &'static str {
        match self {
            QuotaBlock::Parked { .. } => "parked",
            QuotaBlock::Reserve { .. } => "reserve",
            QuotaBlock::DailyTokens { .. } => "daily_tokens",
            QuotaBlock::DailyCredits { .. } => "daily_credits",
            QuotaBlock::ModelTokens { .. } => "model_tokens",
        }
    }

    /// 给用户看的一句话（不含平台术语，直接可展示）。
    pub fn message(&self) -> String {
        match self {
            QuotaBlock::Parked { until_ms, reason } => format!(
                "余额耗尽停放中（{} 后自动恢复）{}",
                reset_text(*until_ms),
                if reason.is_empty() {
                    String::new()
                } else {
                    format!("：{reason}")
                }
            ),
            QuotaBlock::Reserve { reserve, remaining } => {
                format!("已触发余额地板：剩余 {remaining} ≤ 保留 {reserve}，本账号暂停使用")
            }
            QuotaBlock::DailyTokens { limit, used } => {
                format!("已达当日 token 上限（{used} / {limit}）")
            }
            QuotaBlock::DailyCredits { limit, used } => {
                format!("已达当日 credit 上限（{used:.2} / {limit:.2}）")
            }
            QuotaBlock::ModelTokens { model, limit, used } => {
                format!("模型 {model} 已达当日上限（{used} / {limit}）")
            }
        }
    }
}

/// 把「解冻时刻」写成可读文本（跨天时说日期，避免「10:00 后」被误读成今天）。
fn reset_text(until_ms: i64) -> String {
    let now = now_ms();
    let until = timeutil::cst_datetime(until_ms);
    if timeutil::cst_date_str(until_ms) == timeutil::cst_date_str(now) {
        format!("今天 {}", until.format("%H:%M"))
    } else {
        format!("{} {}", until.format("%m-%d"), until.format("%H:%M"))
    }
}

/// 下一个「额度重置窗口」（每日 10:00 UTC+8）的**毫秒**时间戳。
///
/// 与签到共用同一个窗口口径（[`buddy_switch_core::modules::qoder::campaign::next_checkin_window`]）
/// —— 额度与签到名额在同一时刻刷新，凭空另定一个时刻只会两边错开。
pub fn next_reset_ms(now: i64) -> i64 {
    let seconds = buddy_switch_core::modules::qoder::campaign::next_checkin_window(now / 1000).0;
    seconds.saturating_mul(1000)
}

// ---------------------------------------------------------------------------
// 计数与停放（可落盘）
// ---------------------------------------------------------------------------

/// 一天里一个账号的用量。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DayUsage {
    /// `uid -> tokens`（prompt + completion 之和）。
    #[serde(default)]
    tokens: BTreeMap<String, u64>,
    /// `uid -> model -> tokens`（G4 的计数）。
    #[serde(default, rename = "perModel")]
    per_model: BTreeMap<String, BTreeMap<String, u64>>,
    /// `uid -> credits`（G3 的计数；上游 `usage.credits` 是**浮点**）。
    #[serde(default)]
    credits: BTreeMap<String, f64>,
}

/// 一次「余额耗尽停放」。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ParkRecord {
    /// 解冻时刻（毫秒）。
    #[serde(default)]
    until: i64,
    #[serde(default)]
    reason: String,
}

/// 护栏的持久状态（**按区域一份**）。
///
/// 字段全部 `default`：老版本文件 / 手写坏文件都能读（读不出就是空表 = 全放行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuotaStore {
    #[serde(default = "schema_version")]
    version: u32,
    /// 记账当日的日期串（`YYYY-MM-DD`，CST）。
    #[serde(default)]
    date: String,
    #[serde(default)]
    days: BTreeMap<String, DayUsage>,
    #[serde(default)]
    parked: BTreeMap<String, ParkRecord>,
}

fn schema_version() -> u32 {
    1
}

impl Default for QuotaStore {
    fn default() -> Self {
        Self {
            version: schema_version(),
            date: String::new(),
            days: BTreeMap::new(),
            parked: BTreeMap::new(),
        }
    }
}

impl QuotaStore {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        buddy_switch_core::modules::config::atomic_write(path, &text)
            .map_err(|error| error.to_string())
    }

    /// 跨天：把日期推到 `today`，顺手裁掉超期桶。
    ///
    /// ⚠️ 只改 `date` 与裁剪，**不清**当日桶 —— 天数由 `days` 的键表达，
    /// 「今日用量」一律按 `days[today]` 读（不依赖内存里有个「今天」的指针）。
    fn roll(&mut self, today: &str) {
        if self.date == today {
            return;
        }
        self.date = today.to_string();
        self.prune();
    }

    /// 只保留最近 [`QUOTA_RETENTION_DAYS`] 天（ISO 日期串排序即时间顺序）。
    ///
    /// ⚠️ 必须在新桶**写完之后**再裁：先裁后写会多留一天（裁到 7 条再插新的一天
    /// 就是 8 条）。文件是长期存在的，多留一天就是每天多长一条。
    fn prune(&mut self) {
        if self.days.len() <= QUOTA_RETENTION_DAYS {
            return;
        }
        let keep: Vec<String> = self
            .days
            .keys()
            .rev()
            .take(QUOTA_RETENTION_DAYS)
            .cloned()
            .collect();
        self.days
            .retain(|day, _| keep.iter().any(|kept| kept == day));
    }

    fn record(&mut self, today: &str, uid: &str, model: &str, tokens: u64, credits: f64) {
        self.roll(today);
        if uid.is_empty() || (tokens == 0 && credits <= 0.0) {
            return;
        }
        let day = self.days.entry(today.to_string()).or_default();
        if tokens > 0 {
            *day.tokens.entry(uid.to_string()).or_insert(0) += tokens;
        }
        if credits > 0.0 {
            *day.credits.entry(uid.to_string()).or_insert(0.0) += credits;
        }
        if !model.is_empty() {
            *day.per_model
                .entry(uid.to_string())
                .or_default()
                .entry(model.to_string())
                .or_insert(0) += tokens;
        }
        self.prune();
    }

    fn tokens_today(&self, today: &str, uid: &str) -> Option<u64> {
        self.days.get(today)?.tokens.get(uid).copied()
    }

    fn credits_today(&self, today: &str, uid: &str) -> Option<f64> {
        self.days.get(today)?.credits.get(uid).copied()
    }

    fn model_tokens_today(&self, today: &str, uid: &str, model: &str) -> Option<u64> {
        self.days
            .get(today)?
            .per_model
            .get(uid)?
            .get(model)
            .copied()
    }

    /// 记一次停放（**只延长不缩短**：短窗口不该覆盖长窗口）。
    fn park(&mut self, uid: &str, until: i64, reason: &str) {
        let record = self.parked.entry(uid.to_string()).or_default();
        if until > record.until {
            record.until = until;
        }
        if !reason.is_empty() {
            record.reason = reason.to_string();
        }
    }

    fn release(&mut self, uid: &str) -> bool {
        self.parked.remove(uid).is_some()
    }

    /// 当前有效的停放时刻（过期的顺手清掉，避免文件里堆陈年记录）。
    fn parked_until(&mut self, uid: &str, now: i64) -> Option<ParkRecord> {
        match self.parked.get(uid) {
            Some(record) if record.until > now => Some(record.clone()),
            Some(_) => {
                self.parked.remove(uid);
                None
            }
            None => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 运行时（进程级）
// ---------------------------------------------------------------------------

/// 余额快照（由额度视图写入；**只用于 G1**）。
#[derive(Debug, Clone, Copy)]
struct CreditsSnapshot {
    remaining: i64,
    at_ms: i64,
}

#[derive(Default)]
struct Runtime {
    stores: HashMap<QoderRegion, QuotaStore>,
    /// `(region, uid) -> 余额快照`。
    credits: HashMap<(QoderRegion, String), CreditsSnapshot>,
    last_flush: HashMap<QoderRegion, i64>,
}

/// 是否允许落盘。**只有单测会关掉它**：全局单例 + 真实 home 的组合在 `cargo test`
/// 里会往开发者的 `~/.buddy-switch/qoder/` 写文件（那是**用户数据目录**，不是临时目录）。
static PERSIST_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn persist_enabled() -> bool {
    PERSIST_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

fn runtime() -> &'static Mutex<Runtime> {
    static RUNTIME: OnceLock<Mutex<Runtime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(Runtime::default()))
}

fn store_path(region: QoderRegion) -> PathBuf {
    paths::gateway_usage_file_for(region)
}

/// 拿区域 store（首次访问时从盘上读一次）。
fn with_store<R>(region: QoderRegion, action: impl FnOnce(&mut QuotaStore) -> R) -> R {
    let mut guard = match runtime().lock() {
        Ok(guard) => guard,
        // 锁中毒仍继续：护栏是**旁路**能力，不能因为别处 panic 就把转发卡死。
        Err(poisoned) => poisoned.into_inner(),
    };
    let stores = &mut guard.stores;
    let store = stores
        .entry(region)
        .or_insert_with(|| QuotaStore::load(&store_path(region)));
    action(store)
}

/// 记一次转发用量（**唯一计数入口**：`record_request` 调用）。
///
/// `credits` 取自上游 `usage.credits`（真机确认每帧都有，浮点）；缺失按 0 记 ——
/// 那是「少记」的方向，最坏结果是 G3 晚一点触发（fail-open），不会误拦。
pub fn record_usage(region: QoderRegion, uid: &str, model: &str, tokens: u64, credits: f64) {
    if uid.is_empty() || (tokens == 0 && credits <= 0.0) {
        return;
    }
    let now = now_ms();
    let today = timeutil::cst_date_str(now);
    let path = store_path(region);
    let mut flush = false;
    let mut guard = match runtime().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    {
        let store = guard
            .stores
            .entry(region)
            .or_insert_with(|| QuotaStore::load(&path));
        store.record(&today, uid, model, tokens, credits);
        let last = guard.last_flush.get(&region).copied().unwrap_or(0);
        if now.saturating_sub(last) >= FLUSH_INTERVAL_MS {
            guard.last_flush.insert(region, now);
            flush = true;
        }
    }
    if flush && persist_enabled() {
        if let Some(store) = guard.stores.get(&region) {
            // 写盘失败**不影响转发**（护栏是旁路）：下一次节流点会再试。
            let _ = store.save(&path);
        }
    }
}

/// **纯判据**：给定一份用量账 + 余额快照，判这个账号现在能不能用 `model` 转发。
///
/// 抽成纯函数（不碰全局）有两个好处：单测可以精确构造边界，且**界面与选号
/// 共用同一份判据**（`snapshot` 也走它），不会出现「面板说能、选号说不能」。
fn decide(
    store: &mut QuotaStore,
    credits: Option<CreditsSnapshot>,
    limits: QuotaLimits,
    today: &str,
    uid: &str,
    model: &str,
    now: i64,
) -> Option<QuotaBlock> {
    if uid.is_empty() {
        return None;
    }
    // ① 402 停放：与限额开关**无关**（上游说没钱了，开不开护栏都不该继续打）。
    if let Some(record) = store.parked_until(uid, now) {
        return Some(QuotaBlock::Parked {
            until_ms: record.until,
            reason: record.reason,
        });
    }
    if !limits.any() {
        return None;
    }
    // ② G1 余额地板（余额未知 / 快照过期 ⇒ `None`，放行）。
    if limits.reserve_credits > 0 {
        if let Some(credits) = credits {
            if credits.remaining <= limits.reserve_credits {
                return Some(QuotaBlock::Reserve {
                    reserve: limits.reserve_credits,
                    remaining: credits.remaining,
                });
            }
        }
    }
    // ③ G2 当日 token（未知 ⇒ 放行）。
    if limits.daily_token_limit > 0 {
        if let Some(used) = store.tokens_today(today, uid) {
            if used as i64 >= limits.daily_token_limit {
                return Some(QuotaBlock::DailyTokens {
                    limit: limits.daily_token_limit,
                    used,
                });
            }
        }
    }
    // ③-b G3 当日 credit（未知 ⇒ 放行）。
    if limits.daily_credit_limit > 0.0 {
        if let Some(used) = store.credits_today(today, uid) {
            if used >= limits.daily_credit_limit {
                return Some(QuotaBlock::DailyCredits {
                    limit: limits.daily_credit_limit,
                    used,
                });
            }
        }
    }
    // ④ G4 单模型当日 token（模型为空 ⇒ 无从判，放行）。
    if limits.model_daily_token_limit > 0 && !model.is_empty() {
        if let Some(used) = store.model_tokens_today(today, uid, model) {
            if used as i64 >= limits.model_daily_token_limit {
                return Some(QuotaBlock::ModelTokens {
                    model: model.to_string(),
                    limit: limits.model_daily_token_limit,
                    used,
                });
            }
        }
    }
    None
}

/// 判「这个账号现在能不能用 `model` 转发」。`None` = 放行。
pub fn blocked(
    limits: QuotaLimits,
    region: QoderRegion,
    uid: &str,
    model: &str,
) -> Option<QuotaBlock> {
    if uid.is_empty() {
        return None;
    }
    let now = now_ms();
    let today = timeutil::cst_date_str(now);
    let path = store_path(region);
    let mut guard = match runtime().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    // 余额快照过期 ⇒ 视为未知（fail-open），不让「昨天看到的余额」拦今天的请求。
    let credits = guard
        .credits
        .get(&(region, uid.to_string()))
        .copied()
        .filter(|snapshot| now.saturating_sub(snapshot.at_ms) <= CREDITS_SNAPSHOT_TTL_MS);
    let store = guard
        .stores
        .entry(region)
        .or_insert_with(|| QuotaStore::load(&path));
    decide(store, credits, limits, &today, uid, model, now)
}

/// 额度视图写入的**余额快照**（G1 的唯一数据来源）。
///
/// ⚠️ 只在用户（或后台）真的查过额度时才有数据 —— 没查过 ⇒ G1 恒不触发（fail-open）。
pub fn note_credits(region: QoderRegion, uid: &str, remaining: i64, now: i64) {
    if uid.is_empty() {
        return;
    }
    let mut guard = match runtime().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.credits.insert(
        (region, uid.to_string()),
        CreditsSnapshot {
            remaining,
            at_ms: now,
        },
    );
}

/// 把一次**额度视图**的返回喂进护栏快照（G1 的唯一数据来源）。
///
/// ## 语义
///
/// - `remaining > 0` ⇒ 写快照，并**解除 402 停放**（余额是真回来了，不是「没查到」）；
/// - `remaining <= 0` ⇒ 只写快照。**不**因为「查到时为 0」就停放：额度视图是**观测**，
///   停放要等上游 402 真的发生（观测与拒绝不是一回事，混用会误停可用账号）。
/// - `ok=false` 的账号**不写**（保留上一次读数；读失败不等于余额变了）。
pub fn note_credits_view(region: QoderRegion, view: &Value) {
    let Some(rows) = view.get("accounts").and_then(Value::as_array) else {
        return;
    };
    let now = now_ms();
    for row in rows {
        if row.get("ok").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(uid) = row.get("uid").and_then(Value::as_str) else {
            continue;
        };
        let Some(remaining) = row.get("remaining").and_then(Value::as_i64) else {
            continue;
        };
        note_credits(region, uid, remaining, now);
        if remaining > 0 && parked_until(region, uid).is_some() {
            release_park(region, uid);
        }
    }
}

/// 上游说「余额耗尽」（HTTP 402）⇒ 停放到下一个额度重置窗口。
///
/// 返回解冻时刻（毫秒）。
pub fn park_exhausted(region: QoderRegion, uid: &str, reason: &str) -> i64 {
    let now = now_ms();
    let until = next_reset_ms(now);
    let path = store_path(region);
    let mut guard = match runtime().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let store = guard
        .stores
        .entry(region)
        .or_insert_with(|| QuotaStore::load(&path));
    store.park(uid, until, reason);
    if persist_enabled() {
        let _ = store.save(&path);
    }
    until
}

/// 余额刷新看到 `remaining > 0` ⇒ 解除停放（**唯一**的解封路径之一）。
///
/// 与 [`note_credits`] 分开是因为「知道余额」不等于「余额可用」：只有真的 > 0 才解。
pub fn release_park(region: QoderRegion, uid: &str) -> bool {
    let path = store_path(region);
    let mut guard = match runtime().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let store = guard
        .stores
        .entry(region)
        .or_insert_with(|| QuotaStore::load(&path));
    let released = store.release(uid);
    if released && persist_enabled() {
        let _ = store.save(&path);
    }
    released
}

/// 某账号是否在停放中（给管理面看；`None` = 不在停放）。
pub fn parked_until(region: QoderRegion, uid: &str) -> Option<(i64, String)> {
    let now = now_ms();
    with_store(region, |store| {
        store
            .parked_until(uid, now)
            .map(|record| (record.until, record.reason))
    })
}

/// 管理面视图：`{ date, limits:{…}, accounts:[…] }`。
///
/// `accounts` 列出**当前池里的账号**（由调用方给名字），用量 / 停放状态由本模块填 ——
/// 没有用量的账号 `tokens=0`（与「未知」区分：这里的 0 是真的今天还没转发过，
/// 因为账号一进池就会出现在这张表里）。
pub fn snapshot(
    region: QoderRegion,
    limits: QuotaLimits,
    accounts: &[(String, String)],
    now: i64,
) -> Value {
    let today = timeutil::cst_date_str(now);
    // ① 先在**短锁**内冻结一份纯数据（不在这里判拦截：判拦截自己要拿锁）。
    struct Row {
        uid: String,
        name: String,
        tokens: u64,
        /// 今日已消耗 credits（G3 的读数；未知 = 0，与「今天还没转发过」同形）。
        credits_today: f64,
        per_model: Vec<(String, u64)>,
        parked: Option<(i64, String)>,
        credits: Option<(i64, i64)>,
    }
    let frozen: Vec<Row> = {
        let mut guard = match runtime().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let path = store_path(region);
        // 拆开借用：`stores` 要可变、`credits` 只读（同一个 `guard` 不能同时借两次）。
        let Runtime {
            stores, credits, ..
        } = &mut *guard;
        let store = stores
            .entry(region)
            .or_insert_with(|| QuotaStore::load(&path));
        accounts
            .iter()
            .map(|(uid, name)| {
                let day = store.days.get(&today);
                Row {
                    uid: uid.clone(),
                    name: name.clone(),
                    tokens: day
                        .and_then(|day| day.tokens.get(uid))
                        .copied()
                        .unwrap_or(0),
                    credits_today: day
                        .and_then(|day| day.credits.get(uid))
                        .copied()
                        .unwrap_or(0.0),
                    per_model: day
                        .and_then(|day| day.per_model.get(uid))
                        .map(|models| {
                            models
                                .iter()
                                .map(|(model, tokens)| (model.clone(), *tokens))
                                .collect()
                        })
                        .unwrap_or_default(),
                    parked: store
                        .parked_until(uid, now)
                        .map(|record| (record.until, record.reason)),
                    credits: credits
                        .get(&(region, uid.clone()))
                        .copied()
                        .filter(|snapshot| {
                            now.saturating_sub(snapshot.at_ms) <= CREDITS_SNAPSHOT_TTL_MS
                        })
                        .map(|snapshot| (snapshot.remaining, snapshot.at_ms)),
                }
            })
            .collect()
    };

    // ② 逐条判一次「现在会不会被拦」，界面与选号用**同一个函数**（不会两处口径）。
    let rows: Vec<Value> = frozen
        .into_iter()
        .map(|row| {
            let block = blocked(limits, region, &row.uid, "");
            json!({
                "uid": row.uid,
                "name": row.name,
                "tokens": row.tokens,
                // 今日消耗 credits（浮点；与「余额」是两件事，字段名刻意不同）。
                "credits": row.credits_today,
                "perModel": row
                    .per_model
                    .iter()
                    .map(|(model, tokens)| json!({ "model": model, "tokens": tokens }))
                    .collect::<Vec<Value>>(),
                "blocked": block.as_ref().map(QuotaBlock::code),
                "blockedReason": block.as_ref().map(QuotaBlock::message),
                "parkedUntil": row.parked.as_ref().map(|(until, _)| *until),
                "parkedReason": row.parked.as_ref().map(|(_, reason)| reason.clone()),
                "creditsRemaining": row.credits.map(|(remaining, _)| remaining),
                "creditsAt": row.credits.map(|(_, at)| at),
            })
        })
        .collect();

    json!({
        "date": today,
        "limits": {
            "reserveCredits": limits.reserve_credits,
            "dailyTokenLimit": limits.daily_token_limit,
            "dailyCreditLimit": limits.daily_credit_limit,
            "modelDailyTokenLimit": limits.model_daily_token_limit,
            "enabled": limits.any(),
        },
        "accounts": rows,
    })
}

/// 开关落盘（**只给测试**：单测不许写开发者的 `~/.buddy-switch`）。
#[doc(hidden)]
pub fn set_persistence_for_test(enabled: bool) {
    PERSIST_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// 清空进程级状态（**只给测试**：全局单例不能让用例互相污染）。
#[doc(hidden)]
pub fn reset_for_test() {
    let mut guard = match runtime().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.stores.clear();
    guard.credits.clear();
    guard.last_flush.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: &str = "2026-10-21";

    fn limits(reserve: i64, daily: i64, per_model: i64) -> QuotaLimits {
        QuotaLimits {
            reserve_credits: reserve,
            daily_token_limit: daily,
            daily_credit_limit: 0.0,
            model_daily_token_limit: per_model,
        }
    }

    /// 构造 CST 时刻的毫秒时间戳（测试用；与 `timeutil` 的测试助手同口径）。
    fn cst_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        use chrono::TimeZone;
        let naive = chrono::NaiveDate::from_ymd_opt(year, month, day)
            .expect("合法日期")
            .and_hms_opt(hour, minute, 0)
            .expect("合法时刻");
        timeutil::cst_offset()
            .from_local_datetime(&naive)
            .single()
            .expect("无歧义 CST 时刻")
            .timestamp_millis()
    }

    fn credits(remaining: i64) -> Option<CreditsSnapshot> {
        Some(CreditsSnapshot {
            remaining,
            at_ms: now_ms(),
        })
    }

    /// 三条限额**都关**时：不论用了多少都放行（缺省状态不能凭空拦人）。
    ///
    /// 可证伪：把 `decide` 里的 `if !limits.any() { return None }` 删掉并让某条判据
    /// 无条件生效 ⇒ 第一条断言变红。
    #[test]
    fn all_limits_off_never_blocks() {
        let mut store = QuotaStore::default();
        store.record(TODAY, "u1", "m1", 10_000_000, 0.0);
        assert_eq!(
            decide(
                &mut store,
                credits(0),
                limits(0, 0, 0),
                TODAY,
                "u1",
                "m1",
                0
            ),
            None
        );
        // 未知账号同样放行（不能因为「没记录」而拦）。
        assert_eq!(
            decide(&mut store, None, limits(0, 0, 0), TODAY, "u2", "m1", 0),
            None
        );
    }

    /// G2 当日 token：达到即拦（含等号），**未达到**与**未知**都放行。
    #[test]
    fn daily_tokens_blocks_at_the_limit_inclusive() {
        let mut store = QuotaStore::default();
        store.record(TODAY, "u1", "m1", 500, 0.0);
        let l = limits(0, 1000, 0);
        assert_eq!(
            decide(&mut store, None, l, TODAY, "u1", "m1", 0),
            None,
            "500 < 1000 必须放行"
        );
        store.record(TODAY, "u1", "m1", 500, 0.0);
        assert_eq!(
            decide(&mut store, None, l, TODAY, "u1", "m1", 0),
            Some(QuotaBlock::DailyTokens {
                limit: 1000,
                used: 1000
            }),
            "恰好等于上限就要拦（否则第 1001 个 token 已经超了）"
        );
        // 另一个账号不受影响（护栏是**账号级**的）。
        assert_eq!(decide(&mut store, None, l, TODAY, "u2", "m1", 0), None);
    }

    /// G4 单模型：只拦那个模型，同账号的其它模型照常可用。
    #[test]
    fn model_tokens_only_block_the_named_model() {
        let mut store = QuotaStore::default();
        store.record(TODAY, "u1", "expensive", 900, 0.0);
        let l = limits(0, 0, 900);
        assert!(matches!(
            decide(&mut store, None, l, TODAY, "u1", "expensive", 0),
            Some(QuotaBlock::ModelTokens { .. })
        ));
        assert_eq!(
            decide(&mut store, None, l, TODAY, "u1", "cheap", 0),
            None,
            "别的模型不能被连坐"
        );
        // 模型名为空（诊断 / 列表场景）⇒ 无从判 ⇒ 放行。
        assert_eq!(decide(&mut store, None, l, TODAY, "u1", "", 0), None);
    }

    /// G3 当日 credit：浮点累加（上游 `usage.credits` 是小数），达到即拦。
    ///
    /// 可证伪：把 `DayUsage.credits` 的累加删掉（只记 token）⇒ 第一条断言拿到 `None`。
    #[test]
    fn daily_credits_accumulate_as_floats() {
        let mut store = QuotaStore::default();
        // 三发、每发 0.35 credit（真机实测量级）。
        for _ in 0..3 {
            store.record(TODAY, "u1", "m1", 100, 0.35);
        }
        let mut l = limits(0, 0, 0);
        l.daily_credit_limit = 1.0;
        // 1.05 >= 1.0 ⇒ 拦。
        match decide(&mut store, None, l, TODAY, "u1", "m1", 0) {
            Some(QuotaBlock::DailyCredits { limit, used }) => {
                assert_eq!(limit, 1.0);
                assert!((used - 1.05).abs() < 1e-9, "浮点要累计: {used}");
            }
            other => panic!("应命中 G3: {other:?}"),
        }
        // 只记 credits、没 token 的报告也要入账（否则免费/极小请求永远不计）。
        store.record(TODAY, "u2", "m1", 0, 0.8);
        assert!((store.credits_today(TODAY, "u2").unwrap() - 0.8).abs() < 1e-9);
        // 阈值关 ⇒ 不拦。
        l.daily_credit_limit = 0.0;
        assert_eq!(decide(&mut store, None, l, TODAY, "u1", "m1", 0), None);
        // NaN 阈值按「关」处理（否则比较恒假，等于永远不拦，看起来像坏开关）。
        let config = QoderGatewayConfig {
            daily_credit_limit: f64::NAN,
            ..QoderGatewayConfig::default()
        };
        assert_eq!(config.quota_limits().daily_credit_limit, 0.0);
    }

    /// G1 余额地板：`remaining <= reserve` 即拦；**余额未知 ⇒ 放行**（fail-open）。
    #[test]
    fn reserve_floor_blocks_only_when_balance_is_known() {
        let mut store = QuotaStore::default();
        let l = limits(100, 0, 0);
        assert!(matches!(
            decide(&mut store, credits(100), l, TODAY, "u1", "m1", 0),
            Some(QuotaBlock::Reserve { .. })
        ));
        assert_eq!(
            decide(&mut store, credits(101), l, TODAY, "u1", "m1", 0),
            None,
            "高于地板要放行"
        );
        assert_eq!(
            decide(&mut store, None, l, TODAY, "u1", "m1", 0),
            None,
            "余额未知必须放行（猜着拦会把好号标成耗尽）"
        );
    }

    /// 402 停放：到点自动失效（并顺手清掉记录），且与限额开关无关。
    #[test]
    fn parking_expires_without_any_limit_enabled() {
        let mut store = QuotaStore::default();
        store.park("u1", 1_000, "余额耗尽");
        assert!(matches!(
            decide(&mut store, None, limits(0, 0, 0), TODAY, "u1", "m1", 999),
            Some(QuotaBlock::Parked { .. })
        ));
        // 到点后放行，且记录被清掉（文件里不堆陈年记录）。
        assert_eq!(
            decide(&mut store, None, limits(0, 0, 0), TODAY, "u1", "m1", 1_000),
            None
        );
        assert!(store.parked.is_empty(), "过期停放要被清掉");
        assert!(!store.release("u1"), "已经清掉的再解一次应返回 false");
    }

    /// 停放**只延长不缩短**（短窗口不能覆盖长窗口），解除要显式调用。
    #[test]
    fn park_only_extends_and_release_is_explicit() {
        let mut store = QuotaStore::default();
        store.park("u1", 5_000, "第一次");
        store.park("u1", 3_000, "");
        assert_eq!(store.parked["u1"].until, 5_000);
        assert_eq!(store.parked["u1"].reason, "第一次", "空原因不覆盖旧原因");
        assert!(store.release("u1"));
        assert!(store.parked.is_empty());
    }

    /// 计数：按日分桶、跨天从 0 开始、且**只保留最近 N 天**。
    #[test]
    fn counting_rolls_over_days_and_prunes_old_buckets() {
        let mut store = QuotaStore::default();
        store.record("2026-10-20", "u1", "m1", 100, 0.0);
        store.record(TODAY, "u1", "m1", 300, 0.0);
        assert_eq!(store.tokens_today(TODAY, "u1"), Some(300));
        assert_eq!(
            store.tokens_today("2026-10-19", "u1"),
            None,
            "没记的日期是未知"
        );
        // 第二天：今日桶从 0 起（不是接着昨天算）。
        store.roll("2026-10-22");
        assert_eq!(store.tokens_today("2026-10-22", "u1"), None);
        assert_eq!(
            store.tokens_today(TODAY, "u1"),
            Some(300),
            "昨天的账不能被抹掉"
        );

        // 超过保留天数 ⇒ 裁掉最旧的。
        for day in 1..=QUOTA_RETENTION_DAYS + 3 {
            store.record(&format!("2026-11-{day:02}"), "u1", "m1", 1, 0.0);
        }
        assert!(
            store.days.len() <= QUOTA_RETENTION_DAYS,
            "{:?}",
            store.days.keys()
        );
    }

    /// 计数遇到空 uid / 0 token 不写（否则会造出「空账号用了一堆」的假账）。
    #[test]
    fn counting_ignores_empty_uid_and_zero_tokens() {
        let mut store = QuotaStore::default();
        store.record(TODAY, "", "m1", 100, 0.0);
        store.record(TODAY, "u1", "m1", 0, 0.0);
        assert!(!store.days.contains_key(TODAY));
    }

    /// 配置里的负数一律归一成 0（关闭）—— 负阈值会让**所有**账号被拦。
    #[test]
    fn negative_limits_are_normalized_to_off() {
        let config = QoderGatewayConfig {
            reserve_credits: -5,
            daily_token_limit: -1,
            model_daily_token_limit: -9,
            ..QoderGatewayConfig::default()
        };
        let limits = config.quota_limits();
        assert_eq!(limits, QuotaLimits::default());
        assert!(!limits.any());
    }

    /// 停放到**下一个 10:00（UTC+8）**，跨日边界正确（与签到共用同一个重置窗口）。
    #[test]
    fn parking_targets_the_next_reset_window() {
        // 2026-10-21 13:48 CST
        let now = cst_ms(2026, 10, 21, 13, 48);
        let until = next_reset_ms(now);
        assert_eq!(
            timeutil::cst_date_str(until),
            "2026-10-22",
            "13:48 之后的下一个 10:00 是明天"
        );
        assert!(until > now);
        // 09:00 ⇒ 当天 10:00。
        let nine = cst_ms(2026, 10, 21, 9, 0);
        assert_eq!(next_reset_ms(nine) - nine, 3_600_000);
    }

    /// 落盘 → 读回：用量与停放都要**逐字**往返（重启后护栏不能失忆）。
    #[test]
    fn store_round_trips_through_disk() {
        let dir =
            std::env::temp_dir().join(format!("bs-quota-test-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("gateway_usage.cn.json");

        let mut store = QuotaStore::default();
        store.record(TODAY, "u1", "m1", 1200, 0.0);
        store.park("u1", 9_999, "余额耗尽");
        store.save(&path).expect("写盘");

        let loaded = QuotaStore::load(&path);
        assert_eq!(loaded.tokens_today(TODAY, "u1"), Some(1200));
        assert_eq!(loaded.model_tokens_today(TODAY, "u1", "m1"), Some(1200));
        assert_eq!(loaded.parked["u1"].until, 9_999);
        assert_eq!(loaded.date, TODAY);

        // 坏文件 / 缺失 ⇒ 空账（全放行），不能 panic。
        std::fs::write(&path, b"{not json").expect("写坏文件");
        let broken = QuotaStore::load(&path);
        assert!(broken.days.is_empty() && broken.parked.is_empty());
        let missing = QuotaStore::load(&dir.join("nope.json"));
        assert!(missing.days.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★ 全局 API 全链路：记账 → 判拦；余额快照 → 地板；402 停放 → 判停 / 解封。
    ///
    /// 关闭落盘（`set_persistence_for_test(false)`）：单测**绝不**往开发者的
    /// `~/.buddy-switch/qoder/` 写文件（那是用户数据目录）。
    ///
    /// 可证伪：把 `record_tokens` 里的 `store.record(...)` 删掉 ⇒ 第一条断言拿到 `None`。
    #[test]
    fn global_api_records_blocks_and_parks() {
        reset_for_test();
        set_persistence_for_test(false);
        let region = QoderRegion::Cn;

        // ① 记账 → 达阈值即拦（`today` 由系统时钟决定，这里只用相对量）。
        record_usage(region, "uid-global", "m1", 600, 0.0);
        assert_eq!(
            blocked(limits(0, 10_000, 0), region, "uid-global", "m1"),
            None,
            "600 < 10000 必须放行"
        );
        assert!(matches!(
            blocked(limits(0, 500, 0), region, "uid-global", "m1"),
            Some(QuotaBlock::DailyTokens { used: 600, .. })
        ));
        // 单模型限额只影响该模型。
        assert!(matches!(
            blocked(limits(0, 0, 500), region, "uid-global", "m1"),
            Some(QuotaBlock::ModelTokens { .. })
        ));
        assert_eq!(blocked(limits(0, 0, 500), region, "uid-global", "m2"), None);

        // ② 余额快照 → G1（未知时放行，写入后才拦）。
        assert_eq!(blocked(limits(300, 0, 0), region, "uid-global", "m1"), None);
        note_credits(region, "uid-global", 250, now_ms());
        assert!(matches!(
            blocked(limits(300, 0, 0), region, "uid-global", "m1"),
            Some(QuotaBlock::Reserve { .. })
        ));

        // ③ 402 停放：与限额开关无关；解封后恢复。
        let until = park_exhausted(region, "uid-global", "HTTP 402");
        assert!(until > now_ms(), "解冻时刻必须在未来");
        assert!(matches!(
            blocked(QuotaLimits::default(), region, "uid-global", "m1"),
            Some(QuotaBlock::Parked { .. })
        ));
        assert!(release_park(region, "uid-global"));
        assert_eq!(
            blocked(QuotaLimits::default(), region, "uid-global", "m1"),
            None
        );

        // ④ 视图：与判据同源（这里拿得到的就是选号时的结论）。
        let view = snapshot(
            region,
            limits(0, 500, 0),
            &[("uid-global".into(), "n".into())],
            now_ms(),
        );
        assert_eq!(view["accounts"][0]["tokens"], 600);
        assert_eq!(view["accounts"][0]["blocked"], "daily_tokens");
        assert_eq!(view["limits"]["dailyTokenLimit"], 500);

        reset_for_test();
        set_persistence_for_test(true);
    }

    /// 拦截原因必须**可读**（用户能据此行动），而不是一个代号。
    #[test]
    fn block_messages_are_actionable() {
        assert!(QuotaBlock::DailyTokens {
            limit: 10,
            used: 10
        }
        .message()
        .contains("10"));
        assert!(QuotaBlock::Reserve {
            reserve: 100,
            remaining: 0
        }
        .message()
        .contains("余额地板"));
        assert_eq!(
            QuotaBlock::ModelTokens {
                model: "m".into(),
                limit: 1,
                used: 2
            }
            .code(),
            "model_tokens"
        );
        assert_eq!(
            QuotaBlock::Parked {
                until_ms: now_ms(),
                reason: String::new()
            }
            .code(),
            "parked"
        );
    }
}
