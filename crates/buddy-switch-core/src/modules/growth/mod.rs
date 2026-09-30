//! 成长任务自动化（对照参考实现 `workbuddy2api-panel` 的「任务中心」）。
//!
//! - `tasks`：任务列表（默认 + 小程序 mp 口径合并去重）/ 接受 / 接受全部 / 领奖
//! - `events`：桌面指纹、mp 指纹、chat_request_send 等事件构造器（纯函数）
//! - `autotasks`：19 个可自动任务动作 + 「一键完成」（panel autotask.go 同口径）
//! - `queue`：全账号扫描 + 并发执行队列 + 调度钩子（panel taskcenter.go 同口径）
//!
//! 仅 CN 有成长任务体系：所有生产入口对 `Region::Global` 直接返回 `unsupported`，
//! 不发起任何上游调用（panel D4 门控同款）。

pub mod autotasks;
pub mod events;
pub mod queue;
pub mod tasks;
