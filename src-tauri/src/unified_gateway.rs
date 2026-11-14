//! 桌面宿主（Tauri）的**统一网关**接入（第三个产品线，默认 `127.0.0.1:57892`）。
//!
//! 与 [`crate::gateway`]（WorkBuddy 57891）/ [`crate::trae_gateway`]（Trae 57894）/
//! [`crate::qoder_gateway`]（Qoder 57895）平行，**各持一个泛型 [`GatewayHost`]** ——
//! 四个监听入口、四个端口、四套 Key 库。
//!
//! ⚠️ 「第三个产品线」说的是**数据/策略**维度（WB / Trae / Qoder 之后的第四个入口）；
//! 端口分工上它是**第四个监听**。`src-tauri/src/lib.rs` 的 `setup` 必须有四段
//! `spawn + apply()` —— 只 `manage` 不 `apply` 会让开关「看起来开着、实际没监听」
//! （issue #52，统一网关就曾漏了这一段）。
//!
//! ## 为什么必须是独立端口
//!
//! `unified::router()` 挂的 `/v1/models` / `/v1/chat/completions` 与两个既有网关**逐字相同**，
//! axum 的同路径 `merge` 会**运行期 panic**（不是编译期）。故它只能走自己的端口；
//! 桌面端只经 `/api/unified-gateway/*` 走管理面。
//!
//! ## 与两个既有宿主的差异：**不持有账号池相关状态**
//!
//! 统一网关的状态里只有「配置 + 统一密钥库」（见 `unified::state` 模块头）——
//! 账号池与模型目录属于两个既有产品线的进程内单例，它只是**借用**。
//! ⇒ 因此本模块不需要任何「池同步 / 目录刷新」的托管逻辑，比另两个宿主更薄。

use std::sync::Mutex;

use buddy_switch_gateway::unified::{UnifiedGateway, UnifiedGatewayState};
use buddy_switch_gateway::{GatewayHost, GatewayProduct};

/// 进程内共享的统一网关状态（惰性初始化，读一次配置）。
///
/// 独立监听与管理面复用**同一份**状态 ⇒ 监听 57892 上的真实请求与配置改动互相可见。
pub fn shared_state() -> UnifiedGatewayState {
    <UnifiedGateway as GatewayProduct>::shared_state()
}

/// 独立监听生命周期（Tauri managed state）。
///
/// ## 为什么用 `std::sync::Mutex`（而非 `tokio::sync::Mutex`）
///
/// 与 [`crate::gateway::GatewayRuntime`] 同款：状态查询命令是**同步**的，需要同步读
/// `is_running` / `addr`。为**不跨 `await` 持有 std 锁**，`apply` 先把 [`GatewayHost`]
/// **整体取出**（`mem::take`），在锁外 `await` 之后再放回。
pub struct UnifiedGatewayRuntime {
    host: Mutex<GatewayHost<UnifiedGateway>>,
}

impl Default for UnifiedGatewayRuntime {
    fn default() -> Self {
        Self {
            host: Mutex::new(GatewayHost::new()),
        }
    }
}

impl UnifiedGatewayRuntime {
    /// 新建（未启动）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 按当前配置启动 / 重启独立监听。返回实际监听地址；未启用时返回 `None`。
    ///
    /// 用 [`shared_state`] 起监听，使独立监听与管理面共享同一份配置与密钥库。
    /// ★ 起不来**不阻断**另两个入口（三个入口互不牵连）。
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
    fn take_host(&self) -> GatewayHost<UnifiedGateway> {
        std::mem::take(&mut *self.host.lock().unwrap())
    }

    /// 放回宿主（与 [`UnifiedGatewayRuntime::take_host`] 配对）。
    fn put_host(&self, host: GatewayHost<UnifiedGateway>) {
        *self.host.lock().unwrap() = host;
    }
}
