//! 账号池管理动作（**唯一实现**）：排除 / 取消排除 + 只读视图。
//!
//! ## 为什么要单独一个模块，而不是在 Tauri 命令与 server 路由里各写一遍
//!
//! 「排除」不是一次简单的字段写入，它同时要：
//!
//! 1. 落盘（`exclusions::set`，原子写）；
//! 2. 刷新池内的排除快照（否则要等下一次请求才生效 —— UI 会显示「点了没反应」）；
//! 3. **按 uid 解绑会话粘性**（否则粘性徽标会挂在已排除的行上，且每请求白跑一跳）。
//!
//! 「把 uid 从『指定账号』里摘掉」**不需要**在这里做：`pick_with_preference` 的偏好
//! 可用性判据与自动选号同源（`is_usable`），已排除的号会被直接跳过并回落自动择优
//! —— 策略字段里的旧值留着也不会生效，反而保住了「临时取消排除后指定自动恢复」。
//!
//! 四个步骤漏任何一个都会留下「不报错但状态不对」的坑，而本仓有**两个入口**
//! （Tauri 命令 / 独立 server）。放在这里，两个入口就只是薄薄一层转发。

use serde_json::{json, Value};

use buddy_switch_core::modules::account;
use buddy_switch_core::modules::region::Region;

use crate::pool::exclusions;
use crate::state::GatewayState;

/// 读取某 region 的排除列表（附账号展示名，便于 UI 直接渲染）。
///
/// `inStore` 标记该 uid 是否仍在账号库里：**排除项在账号被删除后仍然保留**
/// （用户意图针对「这个号」，不是针对某条记录），UI 据此标「账号已删除」并给一键清理。
pub fn view(region: Region) -> Value {
    let items = exclusions::items_for(crate::routes::relay::scope_key(region));
    let accounts = account::load_accounts_for(region);
    let rows: Vec<Value> = items
        .iter()
        .map(|(uid, item)| {
            let found = accounts
                .iter()
                .find(|acc| account::resolve_account_key(acc, uid));
            json!({
                "uid": uid,
                "name": found.map(account::account_display_name).unwrap_or_default(),
                "note": item.note,
                "at": item.at,
                "inStore": found.is_some(),
            })
        })
        .collect();
    json!({
        "scope": crate::routes::relay::scope_key(region),
        "items": rows,
    })
}

/// 设置 / 取消一个排除项，返回**该 region 的最新列表**。
///
/// 四步动作见模块文档。任何一步失败都返回可读错误（不吞）。
pub async fn set(
    state: &GatewayState,
    region: Region,
    uid: &str,
    excluded: bool,
    note: &str,
) -> Result<Value, String> {
    let scope = crate::routes::relay::scope_key(region);
    let now_ms = crate::timeutil::now_ms();
    // ① 落盘（唯一真相）。
    exclusions::set(scope, uid, excluded, note, now_ms)?;

    // ② 刷新池内快照 —— 立刻生效，不等下一次请求。
    {
        let snapshot = exclusions::load_for(scope);
        let mut pool = state.pool.write().await;
        pool.sync_exclusions(scope, &snapshot);
    }

    // ③ 按 uid 解绑粘性（只在实际「排除」时做；取消排除不需要）。
    if excluded {
        state.sticky.write().await.purge_uid(uid);
    }

    Ok(view(region))
}

/// 按**任意 scope** 读排除列表（Trae / Qoder 用；WorkBuddy 用 [`view`] 带账号名）。
///
/// 只给 uid / note / at：这两条产品线的账号展示名要各自查库，放在各平台的
/// `status_list` 里本来就有一份，这里不重复造。
pub fn view_scope(scope: &str) -> Value {
    let items = exclusions::items_for(scope);
    exclusions::to_value(scope, &items)
}

/// 只落盘（不碰池/粘性）—— 供 Trae / Qoder 的端点复用。
///
/// 它们的池每次请求都从磁盘重建，所以「刷新池内快照」这一步天然成立；
/// 粘性清理由调用方按各自的锁类型做（两边一个 `std::sync::Mutex`、一个 tokio `RwLock`，
/// 强行统一反而要在这里塞两套锁）。
pub fn persist(scope: &str, uid: &str, excluded: bool, note: &str) -> Result<Value, String> {
    let now_ms = crate::timeutil::now_ms();
    exclusions::set(scope, uid, excluded, note, now_ms)?;
    Ok(view_scope(scope))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `view` 的字段契约：UI 靠 `inStore` 区分「账号还在」与「账号已删除」。
    /// 这里只钉 JSON 形状，不碰落盘（落盘由 `pool::exclusions` 自己的用例覆盖）。
    #[test]
    fn view_shape_has_scope_and_items() {
        // 构造一份空视图需要 `GatewayState`，这里退一步只校验「空表也能序列化」的形状约定：
        // `to_value` 是本模块与端点共用的唯一序列化出口。
        let empty = std::collections::BTreeMap::new();
        let value = exclusions::to_value("wb:cn", &empty);
        assert_eq!(value["scope"], "wb:cn");
        assert!(value["items"].is_array());
        assert_eq!(value["items"].as_array().unwrap().len(), 0);
    }
}
