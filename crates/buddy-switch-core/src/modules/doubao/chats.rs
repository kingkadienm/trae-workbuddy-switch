//! 豆包对话数据管理模块（备份/恢复/导出）。
//!
//! TODO：实现对话数据的备份、恢复和导出功能。
//! 当前仅提供占位，确保模块可编译。

use serde_json::Value;

/// 获取对话历史（占位）。
pub fn doubao_history() -> Result<Vec<Value>, String> {
    Err("对话历史功能开发中".to_string())
}

/// 备份聊天数据（占位）。
pub fn doubao_chatdata_backup(
    _user_id: String,
) -> Result<Value, String> {
    Err("聊天备份功能开发中".to_string())
}

/// 恢复聊天数据（占位）。
pub fn doubao_chatdata_restore(
    _user_id: String,
) -> Result<Value, String> {
    Err("聊天恢复功能开发中".to_string())
}

/// 获取聊天数据信息（占位）。
pub fn doubao_chatdata_info(
    _user_id: String,
) -> Result<Value, String> {
    Err("聊天数据信息功能开发中".to_string())
}

/// 导出对话（占位）。
pub fn doubao_export_chats(
    _user_id: String,
) -> Result<Value, String> {
    Err("对话导出功能开发中".to_string())
}
