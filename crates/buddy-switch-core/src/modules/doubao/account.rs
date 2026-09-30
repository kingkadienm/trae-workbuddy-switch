//! 豆包账号池：数据模型 + 读写 API。
//!
//! 文件布局：
//! - `doubao_accounts.json`：账号池（`DoubaoAccountPool` → Vec<DoubaoAccount>）
//! - `doubao/snapshots/<uid>/`：各账号的快照槽（豆包客户端 profile 目录的拷贝）
//!
//! ## 命名约定
//! 全部 snake_case，与前端 `types.ts` 严格对齐；不加 `rename_all`。


use crate::modules::doubao::paths;
use crate::modules::trae::store as trae_store;

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// doubao_accounts.json 单条账号记录（serde default 向后兼容）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DoubaoAccount {
    /// 豆包 user_id（与快照槽目录名一致）
    pub user_id: String,
    /// 别名（展示名，默认 = user_id）
    #[serde(default)]
    pub name: String,
    /// 备注
    #[serde(default)]
    pub note: String,
    /// 入池时间（YYYY-MM-DD HH:MM:SS）
    #[serde(default)]
    pub added_at: String,
    /// 最近一次切换/保存登录态时间
    #[serde(default)]
    pub last_active_at: Option<String>,
    // ── 会话续期字段 ──
    /// 明文 sessionid（凭证等同密码：仅存本地文件）
    #[serde(default)]
    pub session_id: Option<String>,
    /// sid_guard 原文（滑动续期载体）
    #[serde(default)]
    pub sid_guard: Option<String>,
    /// 会话到期时间（由 sid_guard 解析）
    #[serde(default)]
    pub session_expire_at: Option<String>,
    /// 巡检判定：true=过期 / false=有效 / None=未知
    #[serde(default)]
    pub expired: Option<bool>,
    /// 最近一次 cookie 解密同步时间
    #[serde(default)]
    pub cookies_synced_at: Option<String>,
    /// 最近一次续期探活时间
    #[serde(default)]
    pub last_renew_at: Option<String>,
    /// 会话来源：live=当前 User Data / snapshot=快照槽解密
    #[serde(default)]
    pub session_source: Option<String>,
    /// ttwid 设备 Cookie（对话历史 API 登录校验必需）
    #[serde(default)]
    pub ttwid: Option<String>,
    // ── 会员额度缓存 ──
    /// 会员等级（None = 免费或未识别）
    #[serde(default)]
    pub quota_level: Option<String>,
    /// 会员到期时间
    #[serde(default)]
    pub quota_expire_at: Option<String>,
    /// 额度状态一句话
    #[serde(default)]
    pub quota_summary: Option<String>,
    /// 最近一次额度查询时间
    #[serde(default)]
    pub quota_checked_at: Option<String>,
}

impl DoubaoAccount {
    /// 会话状态：ok / expired / unknown / none
    pub fn session_state(&self) -> &'static str {
        match (&self.session_id, self.expired) {
            (None, _) => "none",
            (Some(_), Some(true)) => "expired",
            (Some(_), Some(false)) => "ok",
            (Some(_), None) => "unknown",
        }
    }
}

/// 账号池文件结构。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct DoubaoAccountPool {
    #[serde(default)]
    pub accounts: Vec<DoubaoAccount>,
    /// 最近一次 KeepAlive 保活时间
    #[serde(default)]
    pub last_keepalive_at: Option<String>,
}

/// 前端合并视图：快照槽 + 账号池别名 + 当前账号标记。
///
/// 序列化走 camelCase，与前端 `src/lib/types.ts` 的 `DoubaoAccount` 对齐
/// （Tauri 的 `rename_all` 只作用于命令**入参**，出参形状由此处的 serde 属性决定，
/// 两条通道共用同一份）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoubaoAccountView {
    pub user_id: String,
    pub name: String,
    pub note: String,
    pub has_snapshot: bool,
    pub size_bytes: u64,
    pub file_count: u64,
    pub last_modified: String,
    pub is_current: bool,
    pub added_at: Option<String>,
    // ── 会话状态 ──
    pub session_state: String,
    pub session_id: Option<String>,
    pub sid_guard: Option<String>,
    pub session_expire_at: Option<String>,
    pub cookies_synced_at: Option<String>,
    pub last_renew_at: Option<String>,
    pub session_source: Option<String>,
    pub ttwid: Option<String>,
    // ── 会员额度缓存 ──
    pub quota_level: Option<String>,
    pub quota_expire_at: Option<String>,
    pub quota_summary: Option<String>,
    pub quota_checked_at: Option<String>,
    /// 池级：最近一次 KeepAlive 保活时间
    pub last_keepalive_at: Option<String>,
}

// ---------------------------------------------------------------------------
// I/O
// ---------------------------------------------------------------------------

/// 读取账号池（文件缺失/解析失败 → 空池）。
pub fn load_pool() -> DoubaoAccountPool {
    trae_store::read_json(paths::accounts_file().as_path())
}

/// 写入账号池（原子写）。
pub fn save_pool(pool: &DoubaoAccountPool) -> Result<(), String> {
    trae_store::write_json(paths::accounts_file().as_path(), pool)
}

/// 读取当前登录 UID（豆包客户端写入的标记文件）。
pub fn load_current_uid() -> Option<String> {
    let path = paths::current_account_file();
    if !path.exists() {
        return None;
    }
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 写入当前登录 UID 标记。
pub fn save_current_uid(uid: &str) -> Result<(), String> {
    let path = paths::current_account_file();
    std::fs::write(path, uid.trim()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 视图构造
// ---------------------------------------------------------------------------

/// 统计快照槽目录大小。
pub fn snapshot_stats(uid: &str) -> (bool, u64, u64, String) {
    let dir = paths::snapshot_dir_for(uid);
    if !dir.exists() {
        return (false, 0, 0, "—".to_string());
    }
    let mut total_bytes: u64 = 0;
    let mut file_count: u64 = 0;
    let mut last_modified: Option<std::time::SystemTime> = None;

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    total_bytes = total_bytes.saturating_add(metadata.len());
                    file_count += 1;
                    if let Ok(accessed) = metadata.accessed() {
                        if last_modified.map_or(true, |cur| accessed > cur) {
                            last_modified = Some(accessed);
                        }
                    }
                }
            }
        }
    }

    let last_mod = last_modified
        .map(|t| {
            use std::time::UNIX_EPOCH;
            t.duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| {
                    let ts = d.as_secs() as i64;
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                })
                .unwrap_or_else(|| "—".to_string())
        })
        .unwrap_or_else(|| "—".to_string());

    (true, total_bytes, file_count, last_mod)
}

/// 将账号池条目转为前端合并视图。
pub fn account_view(account: &DoubaoAccount, current_uid: Option<&str>) -> DoubaoAccountView {
    let (has_snapshot, size_bytes, file_count, last_modified) = snapshot_stats(&account.user_id);
    let is_current = current_uid.map_or(false, |cur| cur == account.user_id);

    DoubaoAccountView {
        user_id: account.user_id.clone(),
        name: account.name.clone(),
        note: account.note.clone(),
        has_snapshot,
        size_bytes,
        file_count,
        last_modified,
        is_current,
        added_at: Some(account.added_at.clone()),
        session_state: account.session_state().to_string(),
        session_id: account.session_id.clone(),
        sid_guard: account.sid_guard.clone(),
        session_expire_at: account.session_expire_at.clone(),
        cookies_synced_at: account.cookies_synced_at.clone(),
        last_renew_at: account.last_renew_at.clone(),
        session_source: account.session_source.clone(),
        ttwid: account.ttwid.clone(),
        quota_level: account.quota_level.clone(),
        quota_expire_at: account.quota_expire_at.clone(),
        quota_summary: account.quota_summary.clone(),
        quota_checked_at: account.quota_checked_at.clone(),
        last_keepalive_at: None, // 池级字段在 handlers 中补充
    }
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 当前时间字符串（YYYY-MM-DD HH:MM:SS）。
pub fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
