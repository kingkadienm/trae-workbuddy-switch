//! 统一网关的运行状态（进程内**唯一**实例）。
//!
//! # 为什么状态这么小
//!
//! 统一网关**不持池、不持目录缓存、不持请求日志** —— 那些都属于两个既有产品线自己的
//! [`crate::state::GatewayState`] / [`crate::trae::TraeGatewayState`]，而两者都是**进程内单例**
//! （`GatewayProduct::shared_state()`）。统一入口只做两件事：**按命名空间分派**、**聚合对外清单**。
//!
//! ⇒ 它自己只持三样：配置、统一密钥库、以及 router 构建期固化的请求体上限。
//! 这样「三个入口看到同一份池状态」是**结构性成立**的，而不是靠同步逻辑维持。

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::state::body_limit_bytes;

use super::apikey::UnifiedApiKeyStore;
use super::config::{unified_keys_file, UnifiedGatewayConfig};

/// 统一网关的运行状态。
///
/// `Clone` 是**廉价**的（字段全是 `Arc` / `Copy`）—— 与 [`crate::state::GatewayState`] 同款，
/// 让 handler 与 `GatewayProduct::is_enabled` 能各自持一份而不共享可变状态。
#[derive(Clone)]
pub struct UnifiedGatewayState {
    /// 运行配置（管理面保存后写回本句柄 ⇒ 独立监听立刻读到新值）。
    pub config: Arc<RwLock<UnifiedGatewayConfig>>,
    /// 统一密钥库（独立第三份，见 [`super::apikey`] 模块头）。
    pub keys: Arc<UnifiedApiKeyStore>,
    /// ★ 按密钥的用量账本（issue #76）—— 与另三条产品线**同一个实现**。
    pub key_usage: Arc<crate::key_usage::KeyUsageStore>,
    /// 进程启动时间（毫秒）。
    pub started_at: i64,
    /// 请求体上限（字节）。★ 在 **router 构建期**固化 § ⇒ 改配置需重启。
    pub body_limit_bytes: usize,
}

impl UnifiedGatewayState {
    /// 用给定配置构造（默认密钥库路径）。
    pub fn new(config: UnifiedGatewayConfig) -> Self {
        Self::with_keys(config, UnifiedApiKeyStore::new(unified_keys_file()))
    }

    /// 用给定配置 + 指定密钥库构造（便于测试）。
    ///
    /// 用量账本走**默认路径**（测试不写它 —— 只读；要隔离请用 [`Self::with_parts`]）。
    pub fn with_keys(config: UnifiedGatewayConfig, keys: UnifiedApiKeyStore) -> Self {
        Self::with_parts(
            config,
            keys,
            crate::key_usage::KeyUsageStore::new(super::config::unified_key_usage_file()),
        )
    }

    /// 用给定配置 + 密钥库 + 用量账本构造（**测试隔离用**：三个路径都能指到临时目录）。
    pub fn with_parts(
        config: UnifiedGatewayConfig,
        keys: UnifiedApiKeyStore,
        key_usage: crate::key_usage::KeyUsageStore,
    ) -> Self {
        Self {
            body_limit_bytes: body_limit_bytes(config.max_body_mb),
            config: Arc::new(RwLock::new(config)),
            keys: Arc::new(keys),
            key_usage: Arc::new(key_usage),
            started_at: buddy_switch_core::modules::config::now_ms(),
        }
    }

    /// 配置快照（克隆一份，避免跨 `await` 持锁）。
    pub async fn config_snapshot(&self) -> UnifiedGatewayConfig {
        self.config.read().await.clone()
    }

    /// 写回配置（管理面保存后调用；**不**重启监听 —— 端口/上限的生效边界由调用方说明）。
    pub async fn set_config(&self, config: UnifiedGatewayConfig) {
        *self.config.write().await = config;
    }
}

/// 进程内唯一的统一网关状态（两个宿主 + 管理面共用一份）。
///
/// ⚠️ 用 `OnceLock` 而不是 `lazy_static`：与 `WbGateway::shared_state` / `TraeGateway::shared_state`
/// 同款，进程内只读一次配置。
static SHARED: std::sync::OnceLock<UnifiedGatewayState> = std::sync::OnceLock::new();

/// 取（惰性初始化）进程内唯一的统一网关状态。
pub fn shared_state() -> &'static UnifiedGatewayState {
    SHARED.get_or_init(|| UnifiedGatewayState::new(UnifiedGatewayConfig::load()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state() -> (UnifiedGatewayState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-unified-state-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // 0 触发「0 视为未设置」的回落分支
        let config = UnifiedGatewayConfig {
            max_body_mb: 0,
            ..Default::default()
        };
        let state =
            UnifiedGatewayState::with_keys(config, UnifiedApiKeyStore::new(dir.join("keys.json")));
        (state, dir)
    }

    /// 请求体上限：配置 `0` 走**回落默认**（与 `GatewayConfig` 同款语义）。
    #[test]
    fn body_limit_falls_back_when_zero() {
        let (state, dir) = temp_state();
        assert_eq!(
            state.body_limit_bytes,
            crate::state::body_limit_bytes(crate::state::DEFAULT_MAX_BODY_MB)
        );
        assert!(state.body_limit_bytes > 0, "0 会让每个请求都 413");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `set_config` 之后快照立刻反映新值（管理面保存 ⇒ 独立监听读到）。
    #[tokio::test]
    async fn set_config_is_visible_to_the_next_snapshot() {
        let (state, dir) = temp_state();
        assert_eq!(
            state.config_snapshot().await.port,
            super::super::config::DEFAULT_UNIFIED_PORT
        );
        let mut next = state.config_snapshot().await;
        next.port = 57999;
        next.enabled = true;
        state.set_config(next).await;
        let snapshot = state.config_snapshot().await;
        assert_eq!(snapshot.port, 57999);
        assert!(snapshot.enabled);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 两个状态句柄各自独立（`shared_state` 是单例，但 `new` 出来的不互相污染）。
    #[tokio::test]
    async fn separate_states_do_not_share_config() {
        let (a, dir_a) = temp_state();
        let (b, dir_b) = temp_state();
        let mut config = a.config_snapshot().await;
        config.port = 12345;
        a.set_config(config).await;
        assert_eq!(
            b.config_snapshot().await.port,
            super::super::config::DEFAULT_UNIFIED_PORT,
            "两个状态实例不该共享配置"
        );
        std::fs::remove_dir_all(dir_a).unwrap();
        std::fs::remove_dir_all(dir_b).unwrap();
    }
}
