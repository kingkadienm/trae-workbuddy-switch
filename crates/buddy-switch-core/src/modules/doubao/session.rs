//! 豆包会话续期模块（P3）。
//!
//! TODO：实现 sessionid/sid_guard 的滑动续期逻辑。
//! 当前仅提供占位，确保模块可编译。

use serde_json::Value;

/// 手动触发会话续期（占位实现）。
pub fn doubao_renew_run(
    _user_id: String,
) -> Result<Value, String> {
    Err("会话续期功能开发中".to_string())
}

/// 注册续期定时任务（占位）。
pub fn doubao_renew_task_register(
    _time: String,
) -> Result<(), String> {
    Err("续期调度功能开发中".to_string())
}

/// 查询续期任务状态（占位）。
pub fn doubao_renew_task_status() -> Result<String, String> {
    Ok("未注册".to_string())
}

/// 注销续期定时任务（占位）。
pub fn doubao_renew_task_unregister() -> Result<(), String> {
    Ok(())
}
