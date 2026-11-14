//! 桌面宿主（Tauri）的 **Qoder** 网关接入：独立监听句柄 + 进程内共享状态。
//!
//! 与 [`crate::trae_gateway`] **同构**（各持一个泛型
//! [`GatewayHost`](buddy_switch_gateway::GatewayHost)，只是产品线参数不同：
//! [`TraeGateway`](buddy_switch_gateway::trae::TraeGateway) vs
//! [`QoderGateway`](buddy_switch_gateway::qoder::QoderGateway)）。
//!
//! ## 与另两个网关的两处差异
//!
//! 1. **一个监听、两个区域**：区域由请求所用 API Key 的归属决定，两个区域的账号池
//!    各自独立（`qoder/accounts.{cn,global}.json`）⇒ 本句柄持有的是**整个网关**的
//!    生命周期，「看哪个区域」只是管理命令的入参。
//! 2. 凭据形态是 **COSY 签名**（非 JWT / 非信封），上游是 Qoder 官方 openapi
//!    （国内 `qoder.com.cn` / 国际 `qoder.com` 各一组候选主机）。
//!
//! ⚠️ 三个平台各持一个 router、各自端口（Qoder 57895 / Trae 57894 / WorkBuddy 57891），
//! **绝不 merge**（axum 同路径 merge 会 panic）。

use std::sync::Mutex;

use buddy_switch_gateway::qoder::{QoderGateway, QoderGatewayState};
use buddy_switch_gateway::{GatewayHost, GatewayProduct};

/// 进程内共享的 Qoder 网关状态（惰性初始化，读一次配置）。
///
/// 独立监听与管理命令复用**同一份**状态，因此「API 服务」页看到的请求日志、
/// 账号池与真实打进来的请求完全一致。
pub fn shared_state() -> QoderGatewayState {
    <QoderGateway as GatewayProduct>::shared_state()
}

/// 独立监听生命周期（Tauri managed state）。
///
/// 锁策略与 [`crate::trae_gateway::TraeGatewayRuntime`] 同款
/// （`std::sync::Mutex` + 锁外 `await`，因 `qoder_gateway_status` 需同步读运行态）。
pub struct QoderGatewayRuntime {
    host: Mutex<GatewayHost<QoderGateway>>,
}

impl Default for QoderGatewayRuntime {
    fn default() -> Self {
        Self {
            host: Mutex::new(GatewayHost::new()),
        }
    }
}

impl QoderGatewayRuntime {
    /// 新建（未启动）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 按当前配置启动 / 重启独立监听。返回实际监听地址；未启用时返回 `None`。
    pub async fn apply(&self) -> Result<Option<String>, String> {
        let mut host = self.take_host();
        let result = host.start().await;
        self.put_host(host);
        result
    }

    /// 是否运行中。
    pub fn is_running(&self) -> bool {
        self.host.lock().unwrap().is_running()
    }

    /// 运行中的监听地址。
    pub fn addr(&self) -> Option<String> {
        self.host.lock().unwrap().addr()
    }

    /// 取出宿主（在**锁外** `await` 用；[`GatewayHost`] 实现了 `Default`）。
    fn take_host(&self) -> GatewayHost<QoderGateway> {
        std::mem::take(&mut *self.host.lock().unwrap())
    }

    /// 放回宿主（与 [`QoderGatewayRuntime::take_host`] 配对）。
    fn put_host(&self, host: GatewayHost<QoderGateway>) {
        *self.host.lock().unwrap() = host;
    }
}
