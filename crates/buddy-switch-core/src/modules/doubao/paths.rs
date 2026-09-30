//! 豆包模块的文件路径管理。
//!
//! 所有文件落在 `doubao/` 子目录下，与 Trae 模块的 `trae/` 平级。

use crate::modules::config::store_dir;
use std::path::PathBuf;

/// 豆包数据根目录：`<store>/doubao/`
pub fn doubao_dir() -> PathBuf {
    store_dir().join("doubao")
}

/// 账号池文件：`doubao/doubao_accounts.json`
pub fn accounts_file() -> PathBuf {
    doubao_dir().join("doubao_accounts.json")
}

/// 快照槽根目录：`doubao/snapshots/<uid>/`
pub fn snapshots_dir() -> PathBuf {
    doubao_dir().join("snapshots")
}

/// 单个账号的快照槽目录：`doubao/snapshots/<uid>/`
pub fn snapshot_dir_for(uid: &str) -> PathBuf {
    snapshots_dir().join(uid)
}

/// 当前登录账号标记文件（豆包客户端写入，我们读取探测）
pub fn current_account_file() -> PathBuf {
    doubao_dir().join("current_account.txt")
}

/// 会话凭证文件（自动抓取 / 手动录入）
pub fn credentials_file() -> PathBuf {
    doubao_dir().join("credentials.json")
}

/// 续期任务注册表
pub fn renew_tasks_file() -> PathBuf {
    doubao_dir().join("renew_tasks.json")
}

/// 额度查询任务注册表
pub fn quota_tasks_file() -> PathBuf {
    doubao_dir().join("quota_tasks.json")
}

/// 豆包日志文件
pub fn log_file() -> PathBuf {
    doubao_dir().join("doubao.log")
}
