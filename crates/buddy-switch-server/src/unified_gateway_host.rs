//! webui / CLI 形态的**统一网关**宿主（第三个产品线；默认 `127.0.0.1:57892`）。
//!
//! 与 [`crate::gateway_host`]（WorkBuddy）/ [`crate::trae_gateway_host`]（Trae）平行，
//! **各持一个泛型 [`GatewayHost`]** —— 三个入口、三个端口、三套 Key 库。
//!
//! ## ⚠️ 为什么不把统一网关的路由 merge 进 webui 的 API Router
//!
//! `buddy_switch_gateway::unified::router()` 挂的 `/v1/models` 与两个既有网关**逐字相同**，
//! axum 的同路径 `merge` 会**运行期 panic**（不是编译期）。故它只能走**自己的端口**；
//! webui 侧只暴露 `/api/unified-gateway/*` 管理面（待接）。
//!
//! 这不是缺陷而是刻意设计：统一入口的职责是「**跨产品线的单一入口**」，
//! 混在 webui 的端口上还得靠路径前缀区分，客户端配置会变得难以理解。

use std::sync::OnceLock;

use tokio::sync::Mutex;

use buddy_switch_gateway::unified::{UnifiedGateway, UnifiedGatewayState};
use buddy_switch_gateway::{GatewayHost, GatewayProduct};

static HOST: OnceLock<Mutex<GatewayHost<UnifiedGateway>>> = OnceLock::new();

fn host() -> &'static Mutex<GatewayHost<UnifiedGateway>> {
    HOST.get_or_init(|| Mutex::new(GatewayHost::new()))
}

/// 按当前配置应用（启动 / 重启）独立监听；配置为 disabled 等价于停止。
///
/// ★ 三个入口各自独立：本函数失败**不阻断**另两个 —— 关掉统一入口不影响任何既有客户端。
pub async fn apply() -> Result<Option<String>, String> {
    host().lock().await.start().await
}

/// 进程内共享的统一网关状态（惰性初始化，读一次配置）。
///
/// 独立监听与管理面复用**同一份**状态 ⇒ 监听 57892 上的真实请求与配置改动互相可见。
/// 但**账号池与模型目录不在本状态里** —— 它们属于两个既有产品线的进程内单例，
/// 统一入口只是借用（见 `unified::state` 模块头）。
pub fn shared_state() -> UnifiedGatewayState {
    <UnifiedGateway as GatewayProduct>::shared_state()
}

/// 查询运行状态（运行中, 监听地址）。
pub async fn status() -> (bool, Option<String>) {
    let host = host().lock().await;
    (host.is_running(), host.addr())
}
