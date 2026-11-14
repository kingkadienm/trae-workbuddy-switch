//! 按密钥的用量计数（issue #76 步 3）—— 落盘 `gateway_key_usage.json`。
//!
//! ## 为什么单独一个文件、不进 `gateway_keys.json`
//!
//! Key 台账是**低频写**（创建 / 吊销 / 改策略），而用量是**每请求写**。
//! 两者合并在一个文件里，一次请求就要重写整份 Key 台账（含 hash）——
//! 既放大写放大，也让「改策略」与「记一笔用量」在锁上打架。
//!
//! ## 两个记账点（与规范 §6.2 对齐）
//!
//! - **请求数**（[`QuotaUnit::Request`]）：请求**进入网关后**记（业务代理没有 usage 回包，
//!   只有请求数可计）；
//! - **token**（[`QuotaUnit::Token`]）：流**结束时**记（`UsageTap` 的 sink）——
//!   请求前不知道用量，请求可能失败 ⇒ 记了就是假账。
//!
//! ## 周期重置：懒判定，不用定时器
//!
//! 每条记录带 `period_key`（`daily:2026-10-22` / `weekly:2026-W43` / `monthly:2026-10`）。
//! 查询或记账时发现「记录里的键 ≠ 当前键」⇒ `period_used` 视为 0 并**当场归零**。
//! 定时器方案在进程没开时也会「到期」，而懒判定天然正确（没人请求就不需要重置）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use buddy_switch_core::modules::config as core_config;

use super::apikey::{QuotaPeriod, QuotaUnit};

/// 一个口径（token 或请求数）的计数。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Counters {
    /// 累计用量（**不随周期重置**）—— `quota_period == None` 的判据就是它。
    pub total: u64,
    /// 记录时的周期键（`None` = 累计口径）。
    #[serde(default)]
    pub period_key: Option<String>,
    /// 本周期用量（周期键不符时视为 0）。
    #[serde(default)]
    pub period_used: u64,
}

impl Counters {
    /// 当前口径下的用量：有周期 ⇒ 周期键相符才认 `period_used`，否则 0；
    /// 无周期 ⇒ 累计 `total`。
    pub fn used_now(&self, period: QuotaPeriod, now_ms: i64) -> u64 {
        match period.period_key(now_ms) {
            None => self.total,
            Some(current) => {
                if self.period_key.as_deref() == Some(current.as_str()) {
                    self.period_used
                } else {
                    0
                }
            }
        }
    }
}

/// 单把 Key 的用量 —— **两个口径各自一份计数**。
///
/// ★ 为什么必须分开（2026-10-22 修的真 bug）：一次 LLM 请求同时产生
/// 「1 次请求」与「N 个 token」。若共用一份计数，按请求数计的 Key 会看到
/// `1 + N`、按 token 计的 Key 也会看到 `1 + N` —— 两个口径互相污染，
/// 配额判定全错。原实现里我写了 `let _ = unit;` 并注释「共用一份计数」，
/// 那是错的（当时只有请求数在记，token 还没接，所以没暴露）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyUsage {
    /// token 口径（LLM 请求）。
    #[serde(default)]
    pub tokens: Counters,
    /// 请求数口径（业务代理 / 每次调用）。
    #[serde(default)]
    pub requests: Counters,
}

impl KeyUsage {
    /// 取某个口径的计数。
    pub fn counters(&self, unit: QuotaUnit) -> &Counters {
        match unit {
            QuotaUnit::Token => &self.tokens,
            QuotaUnit::Request => &self.requests,
        }
    }

    /// 取某个口径的计数（可变）。
    fn counters_mut(&mut self, unit: QuotaUnit) -> &mut Counters {
        match unit {
            QuotaUnit::Token => &mut self.tokens,
            QuotaUnit::Request => &mut self.requests,
        }
    }
}

/// 按 Key 的用量账本（每次操作读盘，与 [`super::ApiKeyStore`] 同一取舍）。
pub struct KeyUsageStore {
    path: PathBuf,
    /// ★ **进程内串行化**（2026-10-22）。
    ///
    /// 账本是「读盘 → 改 → 写盘」，两个并发请求会**读到同一份旧值**，各自 +1 后
    /// 后写覆盖前写 ⇒ 计数丢失（比「超限一点」严重得多：配额可能永远不触发）。
    /// 所有写路径（含 [`Self::check_and_record`] 的读-判-写）都必须持这把锁。
    ///
    /// 为什么不换成内存计数器 + 定时落盘：账本要跨进程可见（桌面端与网关进程各持
    /// 一个实例读同一文件），内存态会分叉。锁只保护单次读改写，粒度足够细。
    lock: std::sync::Mutex<()>,
}

impl KeyUsageStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: std::sync::Mutex::new(()),
        }
    }

    /// 取锁（中毒也继续用：用量账本不该因为一次 panic 就永久不可写）。
    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn load(&self) -> BTreeMap<String, KeyUsage> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => BTreeMap::new(),
        }
    }

    fn save(&self, usage: &BTreeMap<String, KeyUsage>) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let content = serde_json::to_string_pretty(usage).map_err(|error| error.to_string())?;
        core_config::atomic_write(&self.path, &content).map_err(|error| error.to_string())
    }

    /// 记一笔用量（`unit` 决定累加到哪个口径；两个口径都维护同一份 `total`/`period_used`，
    /// 由**查询方**按 Key 自己的 `quota_unit` 取 —— 账本不认识策略，只记数）。
    ///
    /// 周期切换时**当场归零再累加**（否则跨周期的那一笔会被算进新周期）。
    pub fn record(
        &self,
        key_id: &str,
        unit: QuotaUnit,
        amount: u64,
        period: QuotaPeriod,
        now_ms: i64,
    ) {
        if amount == 0 {
            return;
        }
        let _guard = self.guard();
        self.record_locked(key_id, unit, amount, period, now_ms);
    }

    /// [`Self::record`] 的**不持锁**版本（调用方必须已持 [`Self::guard`]）。
    fn record_locked(
        &self,
        key_id: &str,
        unit: QuotaUnit,
        amount: u64,
        period: QuotaPeriod,
        now_ms: i64,
    ) {
        let mut usage = self.load();
        let counters = usage
            .entry(key_id.to_string())
            .or_default()
            .counters_mut(unit);
        counters.total = counters.total.saturating_add(amount);
        match period.period_key(now_ms) {
            None => {}
            Some(current) => {
                if counters.period_key.as_deref() != Some(current.as_str()) {
                    counters.period_key = Some(current);
                    counters.period_used = 0;
                }
                counters.period_used = counters.period_used.saturating_add(amount);
            }
        }
        if let Err(error) = self.save(&usage) {
            eprintln!("[gateway] 保存密钥用量失败: {error}");
        }
    }

    /// 请求数记账（进入网关时）。
    pub fn record_request(&self, key_id: &str, period: QuotaPeriod, now_ms: i64) {
        self.record(key_id, QuotaUnit::Request, 1, period, now_ms);
    }

    /// token 记账（流结束时；`tokens` = 该次请求的 prompt+completion）。
    pub fn record_tokens(&self, key_id: &str, tokens: u64, period: QuotaPeriod, now_ms: i64) {
        self.record(key_id, QuotaUnit::Token, tokens, period, now_ms);
    }

    /// ★ **原子**的「判定 + 计数」（2026-10-22）。
    ///
    /// ## 为什么必须是一个方法
    ///
    /// 原先是 `used_for()` 判、`record_request()` 记 —— 两个独立调用之间没有锁，
    /// 并发请求会**同时读到旧值**、同时通过判定，然后各自 +1 ⇒ 上限被突破 N 倍。
    /// 更糟的是两次读改写竞争还会**丢计数**（后写覆盖前写），配额可能永远不触发。
    ///
    /// 这里把「读 → 判 → 写」放在**同一把锁**内：
    /// - 未超限 ⇒ 记一笔，返回 `None`；
    /// - 已超限 ⇒ **不记这一笔**（请求被拒，不该计入），返回超限详情。
    ///
    /// 于是「恰好 limit 个请求通过」是**可保证**的（不是「大约」）。
    pub fn check_and_record(
        &self,
        key_id: &str,
        policy: &crate::apikey::KeyPolicy,
        now_ms: i64,
    ) -> Option<(
        u64,
        u64,
        crate::apikey::QuotaUnit,
        crate::apikey::QuotaPeriod,
    )> {
        let _guard = self.guard();
        let mut usage = self.load();
        {
            let counters = usage
                .entry(key_id.to_string())
                .or_default()
                .counters_mut(policy.quota_unit);
            let used = counters.used_now(policy.quota_period, now_ms);
            if let crate::apikey::QuotaVerdict::Exceeded {
                used,
                limit,
                unit,
                period,
            } = crate::apikey::quota_verdict_for_policy(policy, used)
            {
                // 超限：**不记账**（被拒的请求不该消耗额度）。
                return Some((used, limit, unit, period));
            }
        }
        /* 未超限：**在同一份 `usage` 上**记账并落盘 —— 不重新 `load()`
        （重新读会多一次 IO，也让「读到的就是刚判过的那个值」变得不显然）。 */
        {
            let counters = usage
                .entry(key_id.to_string())
                .or_default()
                .counters_mut(crate::apikey::QuotaUnit::Request);
            counters.total = counters.total.saturating_add(1);
            if let Some(current) = policy.quota_period.period_key(now_ms) {
                if counters.period_key.as_deref() != Some(current.as_str()) {
                    counters.period_key = Some(current);
                    counters.period_used = 0;
                }
                counters.period_used = counters.period_used.saturating_add(1);
            }
        }
        if let Err(error) = self.save(&usage) {
            eprintln!("[gateway] 保存密钥用量失败: {error}");
        }
        None
    }

    /// 查询某把 Key 在**指定口径**下的用量。
    ///
    /// ★ `unit` 必传：两个口径各记各的（见 [`KeyUsage`] 的文档）。按 Key 自己的
    /// `quota_unit` 传进来 —— 那是「这个上限按什么计」的唯一定义。
    pub fn used_for(&self, key_id: &str, unit: QuotaUnit, period: QuotaPeriod, now_ms: i64) -> u64 {
        self.load()
            .get(key_id)
            .map(|entry| entry.counters(unit).used_now(period, now_ms))
            .unwrap_or(0)
    }

    /// 管理面快照（含原始周期键，供前端显示「本周期已用」）。
    pub fn snapshot(&self, key_id: &str) -> Option<KeyUsage> {
        self.load().get(key_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> KeyUsageStore {
        let path = std::env::temp_dir().join(format!(
            "buddy-switch-gateway-key-usage-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        KeyUsageStore::new(path)
    }

    /// 2026-10-22 12:00 UTC 的毫秒（固定值 ⇒ 周期键可预期）。
    const NOON: i64 = 1_781_388_000_000;
    const NEXT_DAY: i64 = NOON + 86_400_000;

    /// ★ **并发下恰好放行 `limit` 个**（2026-10-22 加锁前这条会红）。
    ///
    /// 可证伪：
    /// - 去掉 `KeyUsageStore::lock`（或改成「先 `used_for` 判、再 `record` 记」两步）
    ///   ⇒ 多个线程同时读到同一个旧值，放行数 **大于** limit，且 `used` 可能小于 limit
    ///   （读改写竞争丢计数）。
    /// - 把「超限不记账」改回「超限也记」⇒ `used` 会大于 limit。
    #[test]
    fn concurrent_check_and_record_admits_exactly_the_limit() {
        use crate::apikey::{KeyPolicy, QuotaMode, QuotaPeriod, QuotaUnit, ScopeMode};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let store = Arc::new(temp_store());
        let policy = KeyPolicy {
            quota_mode: QuotaMode::Limited,
            quota_limit: 10,
            quota_unit: QuotaUnit::Request,
            quota_period: QuotaPeriod::Daily,
            scope_mode: ScopeMode::All,
            scope_accounts: Vec::new(),
            scope_groups: Vec::new(),
        };
        let admitted = Arc::new(AtomicUsize::new(0));

        // 32 个线程各抢一次（远大于上限 10）。
        let mut handles = Vec::new();
        for _ in 0..32 {
            let store = store.clone();
            let policy = policy.clone();
            let admitted = admitted.clone();
            handles.push(std::thread::spawn(move || {
                if store
                    .check_and_record("k-concurrent", &policy, NOON)
                    .is_none()
                {
                    admitted.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for handle in handles {
            handle.join().expect("线程不应 panic");
        }

        assert_eq!(
            admitted.load(Ordering::Relaxed),
            10,
            "并发下必须**恰好**放行 limit 个（多放 = 判定非原子；少放 = 丢计数）"
        );
        assert_eq!(
            store.used_for("k-concurrent", QuotaUnit::Request, QuotaPeriod::Daily, NOON),
            10,
            "被拒的请求不该消耗额度（used 必须停在 limit）"
        );
    }

    #[test]
    fn request_counting_accumulates_within_a_period() {
        let store = temp_store();
        store.record_request("k1", QuotaPeriod::Daily, NOON);
        store.record_request("k1", QuotaPeriod::Daily, NOON);
        assert_eq!(
            store.used_for("k1", QuotaUnit::Request, QuotaPeriod::Daily, NOON),
            2
        );
        // 累计口径看到的是同一个数（每日口径下 total 也在涨）。
        assert_eq!(
            store.used_for("k1", QuotaUnit::Request, QuotaPeriod::None, NOON),
            2
        );
    }

    /// ★ 回归：一次请求同时记「1 次请求」与「N 个 token」时，两个口径**不得互相污染**。
    ///
    /// 改造前两者共用一份计数 ⇒ 按请求数计的 Key 会看到 `1 + N`。这个用例是
    /// 那处 bug 的护栏：删掉 `KeyUsage::counters` 的分家就会红。
    #[test]
    fn token_and_request_counters_are_independent() {
        let store = temp_store();
        store.record_request("k1", QuotaPeriod::None, NOON);
        store.record_tokens("k1", 1500, QuotaPeriod::None, NOON);
        assert_eq!(
            store.used_for("k1", QuotaUnit::Request, QuotaPeriod::None, NOON),
            1,
            "请求数只算请求"
        );
        assert_eq!(
            store.used_for("k1", QuotaUnit::Token, QuotaPeriod::None, NOON),
            1500,
            "token 只算 token"
        );
    }

    #[test]
    fn daily_period_resets_on_the_next_day_but_total_keeps_growing() {
        let store = temp_store();
        store.record_request("k1", QuotaPeriod::Daily, NOON);
        store.record_request("k1", QuotaPeriod::Daily, NOON);
        store.record_request("k1", QuotaPeriod::Daily, NEXT_DAY);
        assert_eq!(
            store.used_for("k1", QuotaUnit::Request, QuotaPeriod::Daily, NEXT_DAY),
            1,
            "新的一天从零开始"
        );
        assert_eq!(
            store.used_for("k1", QuotaUnit::Request, QuotaPeriod::Daily, NOON),
            0,
            "昨天已是上一周期"
        );
        assert_eq!(
            store.used_for("k1", QuotaUnit::Request, QuotaPeriod::None, NEXT_DAY),
            3,
            "累计口径不重置"
        );
    }

    #[test]
    fn cumulative_period_never_resets() {
        let store = temp_store();
        store.record_request("k1", QuotaPeriod::None, NOON);
        store.record_request("k1", QuotaPeriod::None, NEXT_DAY + 86_400_000 * 30);
        assert_eq!(
            store.used_for("k1", QuotaUnit::Request, QuotaPeriod::None, NEXT_DAY),
            2,
            "不重置 ⇒ 只累加"
        );
    }

    #[test]
    fn token_accounting_accumulates_on_its_own_counter() {
        let store = temp_store();
        store.record_tokens("k1", 1200, QuotaPeriod::Monthly, NOON);
        store.record_tokens("k1", 800, QuotaPeriod::Monthly, NOON);
        assert_eq!(
            store.used_for("k1", QuotaUnit::Token, QuotaPeriod::Monthly, NOON),
            2000
        );
        let snap = store.snapshot("k1").unwrap();
        assert_eq!(snap.tokens.total, 2000);
        assert_eq!(snap.tokens.period_used, 2000);
        assert_eq!(snap.requests.total, 0, "请求数口径不得被 token 记账污染");
    }

    #[test]
    fn zero_amount_is_a_noop_and_missing_key_reads_zero() {
        let store = temp_store();
        store.record_tokens("k1", 0, QuotaPeriod::None, NOON);
        assert_eq!(
            store.used_for("k1", QuotaUnit::Token, QuotaPeriod::None, NOON),
            0
        );
        assert_eq!(
            store.used_for("absent", QuotaUnit::Request, QuotaPeriod::Daily, NOON),
            0
        );
        assert!(store.snapshot("absent").is_none());
    }

    #[test]
    fn two_instances_share_state_via_disk() {
        let path = std::env::temp_dir().join(format!(
            "buddy-switch-gateway-key-usage-shared-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let a = KeyUsageStore::new(path.clone());
        let b = KeyUsageStore::new(path);
        a.record_request("k1", QuotaPeriod::None, NOON);
        assert_eq!(
            b.used_for("k1", QuotaUnit::Request, QuotaPeriod::None, NOON),
            1,
            "另一实例应看得到"
        );
    }
}
