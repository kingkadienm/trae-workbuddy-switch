//! 账号选择策略（`current` / `pinned` / `max_credits` / `earliest_expiry`；P0-8）。
//!
//! 中继热路径（`routes::relay` 把策略放在**首选**位）用 [`AccountSelector::select_with_pool`]：
//! `max_credits` / `earliest_expiry` 的候选优先读**池缓存**（后台刷新循环写入的
//! `credits` / `credits_expire_at_ms`，新鲜度 = `credits_refresh_interval_ms`，默认 30 分钟），
//! 零网络；条目过期或从未取过才按账号实时查（每号一次 `get_credit_expiry_for`）。
//! `current` / `pinned` 恒为纯本地解析。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use buddy_switch_core::modules::account;
use buddy_switch_core::modules::auth_file;
use buddy_switch_core::modules::config as core_config;
use buddy_switch_core::modules::credits;
use buddy_switch_core::modules::region::{self, Region, RegionMismatch};

use crate::error::GatewayError;
use crate::pool::{Pool, PoolEntry};
use crate::timeutil;

/// 账号选择策略（按 region 各自独立配置）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountStrategy {
    /// 默认：读该 region 认证文件指向的账号（跟随桌面端登录态）。
    Current,
    /// 固定账号：账号库按 id 选择。
    Pinned { account_id: String },
    /// 积分最多：在账号库中选剩余积分最高者。
    MaxCredits,
    /// 积分最先到期：在账号库中选「有效积分到期时间最早」者，
    /// 优先把快过期的积分用掉，减少浪费。
    EarliestExpiry,
}

impl Default for AccountStrategy {
    fn default() -> Self {
        AccountStrategy::Current
    }
}

/// 以严格大于选择积分最高账号；并列时保留先出现者。
fn select_max_credit_candidate<I>(candidates: I) -> Option<Value>
where
    I: IntoIterator<Item = (f64, Value)>,
{
    let mut best: Option<(f64, Value)> = None;
    for (remaining, account) in candidates {
        if best
            .as_ref()
            .map(|(current, _)| remaining > *current)
            .unwrap_or(true)
        {
            best = Some((remaining, account));
        }
    }
    best.map(|(_, account)| account)
}

/// 选「有效积分到期最早」的账号。
///
/// 排序键 `(soonest_expire_at, total_remaining)`：
/// - 只考虑 `totalRemaining > 0` 且能拿到 `soonestExpireAt` 的账号；
/// - 到期时间早者优先（严格小于），并列时**剩余多者优先**——同样是快过期，
///   先消耗存量最大的一笔，减少浪费；
/// - 仍并列时保留先出现者，与 `max_credits` 的并列语义一致。
fn select_earliest_expiry_candidate<I>(candidates: I) -> Option<Value>
where
    I: IntoIterator<Item = (Option<i64>, f64, Value)>,
{
    let mut best: Option<(i64, f64, Value)> = None;
    for (expire_at, remaining, account) in candidates {
        let Some(expire_at) = expire_at else { continue };
        if remaining <= 0.0 {
            continue;
        }
        let better = match &best {
            Some((current_expire, current_remaining, _)) => {
                expire_at < *current_expire
                    || (expire_at == *current_expire && remaining > *current_remaining)
            }
            None => true,
        };
        if better {
            best = Some((expire_at, remaining, account));
        }
    }
    best.map(|(_, _, account)| account)
}

/// 账号是否已在账号库中被标记停用（`deactivated: true`）。
///
/// 策略的自动择优（`max_credits` / `earliest_expiry`）必须跳过停用账号——
/// 停用通常是会话失效（上游恒回 401），选它只会浪费一次上游往返。
/// 2026-10-17 实测：已停用的 `王正帅` 在 max_credits 下被反复选中，持续 401。
fn is_deactivated(account: &Value) -> bool {
    account
        .get("deactivated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 账号选择器。
pub struct AccountSelector;

impl AccountSelector {
    /// 返回该 region 选中的账号（含 `access_token` / `domain` / `uid`）。
    ///
    /// 无池缓存（管理页 / 目录刷新等低频路径）：`max_credits` / `earliest_expiry`
    /// 按账号库全量实时查。池缓存可用时走 [`Self::select_with_pool`]。
    pub async fn select(
        &self,
        region: Region,
        strategy: &AccountStrategy,
    ) -> Result<Value, GatewayError> {
        self.select_with_pool(None, region, strategy).await
    }

    /// 池缓存版的选号（中继热路径用，见模块头）。
    ///
    /// `pool` 为 `None` 时等价于 [`Self::select`]。`max_credits` / `earliest_expiry`
    /// 的候选取自**池条目**（跳过 `disabled` / `deactivated`，与池选号同一套治理状态）：
    /// 条目缓存新鲜（`credits_refreshed_ms > 0` 且距 `now` 小于
    /// `credits_refresh_interval_ms`）直接用池里的 `credits`（+`credits_expire_at_ms`），
    /// **零网络**；否则对该 uid 取账号记录并实时查（每号一次，串行）。
    ///
    /// 安全红线与 [`Self::select`] 相同：选中的账号过 [`ensure_region`] + 信封凭据拦截。
    pub async fn select_with_pool(
        &self,
        pool: Option<&Pool>,
        region: Region,
        strategy: &AccountStrategy,
    ) -> Result<Value, GatewayError> {
                let account = match strategy {
            AccountStrategy::Current => {
                // 安全红线 F：先用 core 的校验读取（region 不符 → RegionMismatch，
                // 且不发起任何上游请求），再导入账号并二次防御校验。
                auth_file::read_auth_file_checked_for(region)
                    .map_err(GatewayError::RegionMismatch)?;
                let account = auth_file::import_from_auth_file_for(region)
                    .ok_or(GatewayError::NoCredential { region })?;
                ensure_region(&account, region)?;
                account
            }
            AccountStrategy::Pinned { account_id } => {
                let account = account::find_account_for(region, account_id)
                    .ok_or(GatewayError::NoCredential { region })?;
                ensure_region(&account, region)?;
                account
            }
            AccountStrategy::MaxCredits => select_max_credits(pool, region).await?,
            AccountStrategy::EarliestExpiry => select_earliest_expiry(pool, region).await?,
        };

        // 加密信封凭据不得送进网关：`build_chat_headers` 会把信封折成**空 `Bearer`**，
        // 上游只会回 401，而用户看到的是网关的 HTML 错误页。
        //
        // ★ 这条是「本机导入接受信封」（core `imported_account_from_root` 的
        //   `plain_text_or_envelope` 改造）的**必要配套**：改造之前 `Current` 策略会因
        //   导入失败而报 `NoCredential`（响亮、可行动），改造之后就会静默发出空 Bearer。
        //   各策略都可能选出信封账号（`Pinned` / 择优走账号库，而库里在明文过期后
        //   也会被信封接管），故拦在 match 之后而不是各分支里。
        if account::is_envelope(&account, "access_token") {
            return Err(GatewayError::EncryptedCredential { region });
        }
        ensure_region(&account, region)?;
        Ok(account)
    }
}

/// 池缓存新鲜度判据：从未取过（`0`）或超出刷新间隔一律视为过期。
///
/// 与后台刷新循环（`crate::credits_refresh::needs_refresh`）**同一套口径**——
/// 缓存不新鲜时本策略实时查，新鲜时直接用池读数，后台循环负责把缓存养新鲜。
fn is_credit_cache_fresh(entry: &PoolEntry, now_ms: i64, interval_ms: i64) -> bool {
    entry.credits_refreshed_ms > 0
        && now_ms.saturating_sub(entry.credits_refreshed_ms) < interval_ms
}

/// 池条目 → 账号记录（池是权威：已删 uid 在库里查无此号时直接跳过，
/// 等待下一次请求的 `sync_accounts` 裁剪掉它）。
fn pool_account(pool: &Pool, uid: &str, region: Region) -> Option<Value> {
    pool.get(uid)
        .and_then(|_| account::find_account_for(region, uid))
}

/// `max_credits`：池缓存优先，过期条目才实时查（每号一次，串行）。
///
/// 池为 `None`（管理页 / 目录刷新等低频路径）时退化为账号库全量实时查——
/// 与改造前行为一致（展示层「每次请求时实时择优」的既有语义）。
async fn select_max_credits(pool: Option<&Pool>, region: Region) -> Result<Value, GatewayError> {
    let now_ms = timeutil::now_ms();
    let mut candidates: Vec<(f64, Value)> = Vec::new();

    match pool {
        Some(pool) => {
            let interval = pool.config().credits_refresh_interval_ms.max(0);
            for entry in pool.entries() {
                if entry.disabled || entry.deactivated {
                    continue;
                }
                let Some(account) = pool_account(pool, &entry.uid, region) else {
                    continue;
                };
                if is_deactivated(&account) {
                    continue;
                }
                if is_credit_cache_fresh(entry, now_ms, interval) {
                    // 池缓存直接可用：零网络。
                    candidates.push((entry.credits.max(0) as f64, account));
                    continue;
                }
                // 缓存过期 → 实时查（region 修正：旧实现 Global 号误查 CN 端点）。
                let credits_value = credits::get_credit_expiry_for(region, &account).await;
                let remaining = credits_value
                    .get("totalRemaining")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                candidates.push((remaining, account));
            }
        }
        None => {
            let accounts = account::load_accounts_for(region);
            for account in accounts {
                if is_deactivated(&account) {
                    continue;
                }
                let credits_value = credits::get_credit_expiry_for(region, &account).await;
                let remaining = credits_value
                    .get("totalRemaining")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                candidates.push((remaining, account));
            }
        }
    }

    // 过滤禁用/停用/已删后可能为空（全库停用），不得 `expect` 直接 panic。
    select_max_credit_candidate(candidates.into_iter())
        .ok_or(GatewayError::NoCredential { region })
}

/// `earliest_expiry`：池缓存优先（含 `credits_expire_at_ms` 到期时刻），
/// 过期条目才实时查。池为 `None` 时同 [`select_max_credits`] 走全库实时查。
async fn select_earliest_expiry(pool: Option<&Pool>, region: Region) -> Result<Value, GatewayError> {
    let now_ms = timeutil::now_ms();
    let mut candidates: Vec<(Option<i64>, f64, Value)> = Vec::new();

    match pool {
        Some(pool) => {
            let interval = pool.config().credits_refresh_interval_ms.max(0);
            for entry in pool.entries() {
                if entry.disabled || entry.deactivated {
                    continue;
                }
                let Some(account) = pool_account(pool, &entry.uid, region) else {
                    continue;
                };
                if is_deactivated(&account) {
                    continue;
                }
                if is_credit_cache_fresh(entry, now_ms, interval) {
                    candidates.push((
                        entry.credits_expire_at_ms,
                        entry.credits.max(0) as f64,
                        account,
                    ));
                    continue;
                }
                let credits_value = credits::get_credit_expiry_for(region, &account).await;
                let remaining = credits_value
                    .get("totalRemaining")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let expire_at = credits_value
                    .get("soonestExpireAt")
                    .and_then(Value::as_i64);
                candidates.push((expire_at, remaining, account));
            }
        }
        None => {
            let accounts = account::load_accounts_for(region);
            for account in accounts {
                if is_deactivated(&account) {
                    continue;
                }
                let credits_value = credits::get_credit_expiry_for(region, &account).await;
                let remaining = credits_value
                    .get("totalRemaining")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let expire_at = credits_value
                    .get("soonestExpireAt")
                    .and_then(Value::as_i64);
                candidates.push((expire_at, remaining, account));
            }
        }
    }

    select_earliest_expiry_candidate(candidates.into_iter())
        .ok_or(GatewayError::NoCredential { region })
}

/// 构造某 region 的策略展示对象：`{ region, strategy, selected, error?, note? }`。
///
/// `selected` 只做**本地**解析（`current` 读认证文件 / `pinned` 查账号库），
/// 因此该接口恒定快速、无网络请求。`max_credits` 需要按实时积分对**全部**账号
/// 逐个发起请求择优，不适合在管理页轮询时预取，故返回 `selected: null` 并附
/// `note` 说明，由前端提示「每次请求时实时择优」。
pub async fn describe_strategy(region: Region, strategy: &AccountStrategy) -> Value {
    let mut described = serde_json::json!({
        "region": region,
        "strategy": serde_json::to_value(strategy).unwrap_or(Value::Null),
        "selected": Value::Null,
    });
    match strategy {
        AccountStrategy::MaxCredits => {
            described["note"] =
                serde_json::json!("max_credits 在每次请求时择优（池缓存新鲜时零网络，过期才实时查），此处不预取");
        }
        AccountStrategy::EarliestExpiry => {
            described["note"] = serde_json::json!(
                "earliest_expiry 在每次请求时按积分到期时间择优（池缓存新鲜时零网络，过期才实时查），此处不预取"
            );
        }
        _ => match AccountSelector.select(region, strategy).await {
            Ok(account) => {
                // account_meta 已剥离 token，可直接下发前端。
                described["selected"] = account::account_meta(&account);
            }
            Err(error) => {
                described["error"] = serde_json::json!(error.message());
            }
        },
    }
    described
}

/// 校验凭据 domain 所属 region 与目标 region 一致（安全红线 F）。
pub fn ensure_region(account: &Value, region: Region) -> Result<(), GatewayError> {
    let domain = account
        .get("domain")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let actual = region::region_of(&domain);
    if actual == region {
        return Ok(());
    }
    let spec = region::region_spec(region);
    Err(GatewayError::RegionMismatch(RegionMismatch {
        actual_domain: domain,
        expected_file: spec.auth_filename.to_string(),
        env_var: spec.auth_env.to_string(),
        actual_region: actual,
        expected_region: region,
    }))
}

/// 策略持久化文件（与 `gateway_config.json` 同目录）。
pub fn strategy_file() -> PathBuf {
    region::gateway_config_file().with_file_name("gateway_strategy.json")
}

/// 读取各 region 策略（缺失回落空表 → 各处默认 `current`）。
pub fn load_strategies() -> HashMap<Region, AccountStrategy> {
    let file = strategy_file();
    if let Ok(text) = std::fs::read_to_string(&file) {
        if let Ok(map) = serde_json::from_str::<HashMap<Region, AccountStrategy>>(&text) {
            return map;
        }
    }
    HashMap::new()
}

/// 保存各 region 策略。
pub fn save_strategies(map: &HashMap<Region, AccountStrategy>) -> Result<(), String> {
    let file = strategy_file();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let content = serde_json::to_string_pretty(map).map_err(|error| error.to_string())?;
    core_config::atomic_write(&file, &content).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn is_credit_cache_fresh_requires_read_value_and_within_interval() {
        let now = 1_700_000_000_000i64;
        let mut entry = crate::pool::PoolEntry::new("u1");
        entry.credits_refreshed_ms = 0;
        assert!(!is_credit_cache_fresh(&entry, now, 60_000), "从未取过（0）不新鲜");
        entry.credits_refreshed_ms = now - 30_000;
        assert!(is_credit_cache_fresh(&entry, now, 60_000), "间隔内新鲜");
        entry.credits_refreshed_ms = now - 60_000;
        assert!(!is_credit_cache_fresh(&entry, now, 60_000), "恰达阈值即过期");
    }

    #[test]
    fn max_credits_selection_keeps_first_tie_and_uses_zero_for_missing_values() {
        let candidates = vec![
            (10.0, json!({"id": "first"})),
            (10.0, json!({"id": "tie"})),
            (0.0, json!({"id": "missing"})),
        ];
        let selected = select_max_credit_candidate(candidates.into_iter()).unwrap();
        assert_eq!(selected["id"], "first");

        let empty: Vec<(f64, Value)> = Vec::new();
        assert!(select_max_credit_candidate(empty.into_iter()).is_none());
    }

    #[test]
    fn earliest_expiry_selection_prefers_soonest_then_largest_remaining() {
        let candidates = vec![
            (Some(200), 5.0, json!({"id": "later"})),
            (Some(100), 3.0, json!({"id": "sooner-small"})),
            (Some(100), 8.0, json!({"id": "sooner-large"})),
            (Some(50), 0.0, json!({"id": "no-remaining"})),
            (None, 9.0, json!({"id": "no-expiry"})),
        ];
        let selected = select_earliest_expiry_candidate(candidates.into_iter()).unwrap();
        assert_eq!(selected["id"], "sooner-large");

        // 全部无效（无到期时间或无剩余）→ None（调用方回落 NoCredential）。
        let all_invalid = vec![
            (None, 9.0, json!({"id": "a"})),
            (Some(50), 0.0, json!({"id": "b"})),
        ];
        assert!(select_earliest_expiry_candidate(all_invalid.into_iter()).is_none());

        let empty: Vec<(Option<i64>, f64, Value)> = Vec::new();
        assert!(select_earliest_expiry_candidate(empty.into_iter()).is_none());
    }

    #[test]
    fn is_deactivated_only_flags_explicit_true() {
        // 缺字段 / false / 非布尔都不是停用——只有显式 true 才是。
        assert!(!is_deactivated(&json!({})));
        assert!(!is_deactivated(&json!({"deactivated": false})));
        assert!(!is_deactivated(&json!({"deactivated": null})));
        assert!(is_deactivated(&json!({"deactivated": true})));
    }

    #[test]
    fn strategy_serde_roundtrip() {
        assert_eq!(
            serde_json::to_value(AccountStrategy::Current).unwrap(),
            json!({"kind": "current"})
        );
        assert_eq!(
            serde_json::to_value(AccountStrategy::Pinned {
                account_id: "a1".into()
            })
            .unwrap(),
            json!({"kind": "pinned", "account_id": "a1"})
        );
        assert_eq!(
            serde_json::to_value(AccountStrategy::MaxCredits).unwrap(),
            json!({"kind": "max_credits"})
        );
        assert_eq!(
            serde_json::to_value(AccountStrategy::EarliestExpiry).unwrap(),
            json!({"kind": "earliest_expiry"})
        );

        let parsed: AccountStrategy = serde_json::from_value(json!({"kind": "pinned", "account_id": "x"})).unwrap();
        assert!(matches!(parsed, AccountStrategy::Pinned { account_id } if account_id == "x"));
    }

    #[test]
    fn ensure_region_documents_legacy_missing_domain_behavior() {
        // CN defaults unknown/legacy records for backward compatibility;
        // Global remains fail-closed because an absent domain is not global evidence.
        let legacy = json!({"access_token": "t"});
        assert!(ensure_region(&legacy, Region::Cn).is_ok());
        let err = ensure_region(&legacy, Region::Global).unwrap_err();
        assert_eq!(err.status(), 401);
        assert_eq!(err.type_name(), "region_mismatch");
    }

    #[test]
    fn ensure_region_accepts_matching_and_rejects_mismatch() {
        let cn = json!({"domain": "www.codebuddy.cn", "access_token": "t"});
        assert!(ensure_region(&cn, Region::Cn).is_ok());

        let global = json!({"domain": "www.workbuddy.ai", "access_token": "t"});
        assert!(ensure_region(&global, Region::Global).is_ok());

        // 国际凭据出现在国内 region → 拒绝
        let err = ensure_region(&global, Region::Cn).unwrap_err();
        assert_eq!(err.status(), 401);
        assert_eq!(err.type_name(), "region_mismatch");
        let message = err.message();
        assert!(message.contains("workbuddy.ai"), "含实际 domain: {message}");
        // core 的 RegionMismatch::message() 由「实际 region 展示名 / 期望 region 展示名 /
        // 实际 domain / 环境变量名 / 期望 region 展示名」构成，不直接含认证文件名。
        assert!(
            message.contains(region::region_display(Region::Cn)),
            "含期望 region 展示名: {message}"
        );
        assert!(message.contains("WORKBUDDY_AUTH_FILE"), "含环境变量名: {message}");
    }
}
