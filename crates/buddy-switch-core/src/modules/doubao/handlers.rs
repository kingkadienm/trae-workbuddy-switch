//! 豆包操作层：**每条逻辑操作的唯一实现**，供两条通道共用。
//!
//! 本模块提供纯函数，`commands.rs`（Tauri）与 `api.rs`（HTTP）各自做薄包装。


use crate::modules::doubao::account::{
    account_view, load_current_uid, load_pool, now_str, save_current_uid, save_pool, DoubaoAccount,
    DoubaoAccountView,
};
use crate::modules::doubao::paths;

// ---------------------------------------------------------------------------
// 列表 / 探测
// ---------------------------------------------------------------------------

/// 列出全部账号（合并快照槽 + 账号池 + 当前登录标记）。
pub fn doubao_accounts_list() -> Result<Vec<DoubaoAccountView>, String> {
    let pool = load_pool();
    let current_uid = load_current_uid();
    let last_keepalive = pool.last_keepalive_at.clone();

    let mut views: Vec<DoubaoAccountView> = pool
        .accounts
        .iter()
        .map(|acc| {
            let mut view = account_view(acc, current_uid.as_deref());
            view.last_keepalive_at = last_keepalive.clone();
            view
        })
        .collect();

    // 补充：有快照但不在池中的账号
    let pool_uids: std::collections::HashSet<_> = pool.accounts.iter().map(|a| &a.user_id).collect();
    if let Ok(entries) = std::fs::read_dir(paths::snapshots_dir()) {
        for entry in entries.flatten() {
            let uid = entry.file_name().to_string_lossy().to_string();
            if pool_uids.contains(&uid) || uid.starts_with('.') {
                continue;
            }
            let (has_snapshot, size_bytes, file_count, last_modified) =
                crate::modules::doubao::account::snapshot_stats(&uid);
            views.push(DoubaoAccountView {
                user_id: uid.clone(),
                name: uid.clone(),
                note: String::new(),
                has_snapshot,
                size_bytes,
                file_count,
                last_modified,
                is_current: current_uid.as_deref() == Some(uid.as_str()),
                added_at: None,
                session_state: "none".to_string(),
                session_id: None,
                sid_guard: None,
                session_expire_at: None,
                cookies_synced_at: None,
                last_renew_at: None,
                session_source: None,
                ttwid: None,
                quota_level: None,
                quota_expire_at: None,
                quota_summary: None,
                quota_checked_at: None,
                last_keepalive_at: last_keepalive.clone(),
            });
        }
    }

    Ok(views)
}

/// 探测当前豆包客户端登录的 UID（读取豆包客户端写入的标记文件）。
pub fn doubao_detect_uid() -> Result<Option<String>, String> {
    Ok(load_current_uid())
}

// ---------------------------------------------------------------------------
// 保存 / 切换 / 移除
// ---------------------------------------------------------------------------

/// 保存/切换账号：将指定快照槽激活为当前账号。
///
/// 操作：
/// 1. 若账号不在池中，自动入池（name = uid）
/// 2. 写入当前登录标记
/// 3. 更新 last_active_at
pub fn doubao_account_save(
    user_id: String,
    name: Option<String>,
    note: Option<String>,
) -> Result<DoubaoAccountView, String> {
    if user_id.trim().is_empty() {
        return Err("user_id 不能为空".to_string());
    }

    let mut pool = load_pool();
    let current_uid = load_current_uid();

    // 查找或创建
    let account = match pool.accounts.iter_mut().find(|a| a.user_id == user_id) {
        Some(acc) => {
            if let Some(n) = name.filter(|s| !s.trim().is_empty()) {
                acc.name = n;
            }
            if let Some(note) = note {
                acc.note = note;
            }
            acc.last_active_at = Some(now_str());
            acc.clone()
        }
        None => {
            let acc = DoubaoAccount {
                user_id: user_id.clone(),
                name: name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| user_id.clone()),
                note: note.unwrap_or_default(),
                added_at: now_str(),
                last_active_at: Some(now_str()),
                ..DoubaoAccount::default()
            };
            pool.accounts.push(acc.clone());
            acc
        }
    };

    save_pool(&pool)?;
    save_current_uid(&user_id)?;

    let mut view = account_view(&account, current_uid.as_deref());
    view.last_keepalive_at = pool.last_keepalive_at;
    Ok(view)
}

/// 移除账号（从池中删除 + 可选删除快照槽）。
pub fn doubao_account_remove(
    user_id: String,
    remove_snapshot: bool,
) -> Result<(), String> {
    let mut pool = load_pool();
    pool.accounts.retain(|a| a.user_id != user_id);
    save_pool(&pool)?;

    if remove_snapshot {
        let dir = paths::snapshot_dir_for(&user_id);
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }

    // 若移除的是当前账号，清空标记
    if load_current_uid().map_or(false, |cur| cur == user_id) {
        let _ = std::fs::remove_file(paths::current_account_file());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 保活
// ---------------------------------------------------------------------------

/// 手动触发保活（更新池级 last_keepalive_at）。
pub fn doubao_keepalive_run() -> Result<(), String> {
    let mut pool = load_pool();
    pool.last_keepalive_at = Some(now_str());
    save_pool(&pool)
}
