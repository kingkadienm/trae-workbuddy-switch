//! Video Gateway P1 最小完整闭环。
//!
//! 目前覆盖三条路由：
//! - `GET  /v1/video/models`：列出当前 region 可用的视频模型；
//! - `POST /v1/videos`：提交一次视频生成任务（返回本地任务 id）；
//! - `GET  /v1/videos/:id`：按任务 id 查询进度（轮询上游）。
//!
//! 与 `/v1/models`、`/v1/chat/completions` 同一条鉴权纪律：归属域由 Key 决定，
//! 不接受调用方在请求体里声明的 region。P1 不做积分与下载（P3/P2），也不落盘。

pub mod models;
pub mod provider;
pub mod routes;
pub mod service;
pub mod store;
