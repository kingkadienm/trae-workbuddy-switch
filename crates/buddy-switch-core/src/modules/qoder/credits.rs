//! Qoder **额度 / 套餐**视图：逐账号 quota + plan，附区域汇总。
//!
//! ## 诚实性约束（与 `api::parse_quota` 的守卫串成一条线）
//!
//! - 单账号取数失败**不吞**：该行 `ok=false + error`，其余账号照常展示
//!   （一个 401 的账号不能把整页额度变成空）。
//! - 退化响应（解析不了）在 [`api::parse_quota`] 已拒绝 ⇒ 这里永远不会把
//!   「看不懂」写成 `remaining: 0` 再谎报给用户。
//! - 汇总只累加 `ok` 的行，并**报出成功/失败账号数**——部分失败绝不被伪装成全额。

use serde_json::{json, Value};

use super::account;
use super::api;
use super::region::QoderRegion;

/// 区域额度视图：`{ region, summary:{ okAccounts, failedAccounts, remaining, total }, accounts: [...] }`。
pub async fn region_view(region: QoderRegion) -> Value {
    let file = account::load_accounts_for(region);
    let mut rows = Vec::new();
    let mut ok_accounts = 0u32;
    let mut failed_accounts = 0u32;
    let mut remaining_sum = 0i64;
    let mut total_sum = 0i64;

    for account in &file.accounts {
        let name = account::display_name(account);
        let Some(access) = account
            .access_token
            .as_deref()
            .filter(|token| !token.is_empty())
        else {
            failed_accounts += 1;
            rows.push(json!({
                "uid": account.uid,
                "name": name,
                "ok": false,
                "error": "没有可调用的 access token",
            }));
            continue;
        };
        match api::quota_usage(region, &account.uid, access).await {
            Ok(quota) => {
                ok_accounts += 1;
                remaining_sum += quota.remaining;
                total_sum += quota.total;
                // 套餐名单独取（失败不影响额度展示 —— 套餐只是展示信息）。
                let plan = api::user_plan(region, &account.uid, access)
                    .await
                    .ok()
                    .and_then(|payload| {
                        payload
                            .get("planName")
                            .or_else(|| payload.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
                rows.push(json!({
                    "uid": account.uid,
                    "name": name,
                    "ok": true,
                    "remaining": quota.remaining,
                    "used": quota.used,
                    "total": quota.total,
                    "isExceeded": quota.is_exceeded,
                    "plan": plan,
                    "raw": quota.raw,
                }));
            }
            Err(error) => {
                failed_accounts += 1;
                rows.push(json!({
                    "uid": account.uid,
                    "name": name,
                    "ok": false,
                    "error": error,
                }));
            }
        }
    }

    json!({
        "region": region.as_str(),
        "summary": {
            "okAccounts": ok_accounts,
            "failedAccounts": failed_accounts,
            "remaining": remaining_sum,
            "total": total_sum,
        },
        "accounts": rows,
    })
}
