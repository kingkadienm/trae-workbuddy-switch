//! webui / CLI 形态的 **Qoder** 网关宿主（与 Trae 宿主平行；默认 `127.0.0.1:57895`）。
//!
//! 与 [`crate::gateway_host`] / [`crate::trae_gateway_host`] 平行、**各持一个泛型
//! [`GatewayHost`]**（原因见 `buddy_switch_gateway::qoder` 模块文档与
//! [`buddy_switch_gateway::GatewayProduct`]）。
//!
//! ## 为什么不把 Qoder 路由 merge 进 webui 的 API Router
//!
//! `buddy_switch_gateway::qoder::router()` 也挂 `/v1/models` 与 `/v1/chat/completions`，
//! 与 WorkBuddy / Trae 网关的路径**逐字相同**。axum 的 `merge` 遇到同路径会直接 panic，
//! 所以 Qoder 网关只能走**自己的端口**（57895）；webui 侧只暴露 `/api/qoder/gateway/*` 管理面。
//!
//! ## 一个监听、两个区域
//!
//! Qoder 网关**只有一个**独立监听：区域由请求所用 API Key 的归属（`QoderKeyRegion`）
//! 决定，两个区域的账号池各自独立（`qoder/accounts.{cn,global}.json`）。
//! 因此本宿主持有的是**整个网关**的生命周期，管理面的区域只是「看哪本池」的参数。

use std::sync::OnceLock;

use tokio::sync::Mutex;

use buddy_switch_gateway::qoder::{QoderGateway, QoderGatewayState};
use buddy_switch_gateway::{GatewayHost, GatewayProduct};

static HOST: OnceLock<Mutex<GatewayHost<QoderGateway>>> = OnceLock::new();

/// 进程内共享的 Qoder 网关状态（惰性初始化，读一次配置）。
pub fn shared_state() -> QoderGatewayState {
    <QoderGateway as GatewayProduct>::shared_state()
}

fn host() -> &'static Mutex<GatewayHost<QoderGateway>> {
    HOST.get_or_init(|| Mutex::new(GatewayHost::new()))
}

/// 按当前配置应用（启动 / 重启）独立监听。
pub async fn apply() -> Result<Option<String>, String> {
    host().lock().await.start().await
}

/// 查询运行状态（运行中, 监听地址）。
pub async fn status() -> (bool, Option<String>) {
    let host = host().lock().await;
    (host.is_running(), host.addr())
}
