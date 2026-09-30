//! 豆包应用账号池模块。
//!
//! 架构对齐 [`crate::modules::trae`]：
//! - `store::` 读写 JSON（容错读 + 原子写）
//! - `paths::` 管理文件路径
//! - `handlers.rs` 挂 Tauri 命令
//!
//! ## 数据文件
//! - `doubao_accounts.json`：账号池（uid + 别名 + 会话字段 + 额度缓存）
//! - `doubao_snapshots/<uid>/`：各账号的快照槽（豆包客户端 profile 目录的拷贝）

pub mod account;
pub mod handlers;
pub mod paths;
pub mod session;
pub mod quota;
pub mod chats;
pub mod store;

pub use account::{DoubaoAccount, DoubaoAccountPool, DoubaoAccountView};
pub use paths::doubao_dir;
