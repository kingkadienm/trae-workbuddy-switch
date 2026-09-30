//! Trae 的 OpenAI 兼容网关（`/v1/chat/completions`）。
//!
//! ## 为什么与 WorkBuddy 网关**不共用**一套实现
//!
//! 两者除了「都是 OpenAI 兼容」之外没有任何共同点：
//!
//! | 维度 | WorkBuddy 网关（`crate::pool` / `routes`） | Trae 网关（本模块） |
//! |:---|:---|:---|
//! | 归属域 | `Region`（cn / global），凭据随域隔离 | 无域概念，账号库只有一套 |
//! | 凭据形态 | `access_token` + `domain` | `Cloud-IDE-JWT <jwt>` |
//! | 上游协议 | 直接转发 OpenAI 请求体 | 需改写成 `llm_utils_chat` 专用请求体 |
//! | 上游响应 | OpenAI SSE，可原样透传 | 私有 SOLO SSE（`event: output` 等），**必须转换** |
//! | 路由挂载 | merge 进宿主 `/v1/*` | 独立监听（否则 `/v1/chat/completions` 与 WorkBuddy 撞车） |
//!
//! 因此本模块是**平行的第二套实现**，刻意不复用 `pool` / `protocol` / `outbound`：
//! 强行抽象只会让两侧都变得难改。共用的只有真正与产品无关的基础件——
//! [`crate::logging::RequestLog`]（只存 `Value`）与 [`crate::error`] 的错误形状。
//!
//! ## 与 Trae 账号模块的关系
//!
//! 账号、设备标识、冷却、剩余积分**全部复用** `buddy_switch_core::modules::trae`：
//! 网关不另建账号库，也不另建冷却文件——否则「签到页显示正常、网关页显示冷却中」
//! 这类不一致会立刻出现。网关的错误会经
//! [`buddy_switch_core::modules::trae::credits::save_cooldown_for`] 写回**该产品线自己的**
//! 冷却文件（`account_cooldowns.json` / `account_cooldowns.trae_cn.json`），
//! 与签到页的变体分家口径一致：两个页面看到的是同一份状态。

pub mod apikey;
pub mod payload;
pub mod pool;
pub mod routes;
pub mod sse;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex, RwLock};

use buddy_switch_core::modules::config as core_config;
use buddy_switch_core::modules::trae::variant::TraeVariant;
use buddy_switch_core::modules::trae::{paths, TRAE_DEFAULT_API_PORT};

use crate::logging::RequestLog;
use crate::GatewayHandle;

use pool::{TraePool, TraePoolSummary};

// ---------------------------------------------------------------------------
// 上游常量
// ---------------------------------------------------------------------------

/// Trae SOLO 上游基址（**CN 默认值**）。
///
/// 注意与 `modules::trae::endpoints_for(..).account_base`（`https://api.trae.cn`，签到/积分用）
/// **不是同一个主机**：对话走 mchost.guru 的 agent 网关。两者不可互换。
///
/// 取值与 [`buddy_switch_core::modules::trae::variant`] 的 CN 端点表逐字一致
/// （实测两条 CN 产品线都是这个主机）；国际池改由 `routes::attempt_once`
/// 按区域取端点表的 `agent_host`（`https://core-normal.trae.ai`，2026-09-30），
/// 本常量只作 CN 默认值与「测试 override 未改」的判据。
pub const TRAE_AGENT_HOST: &str = "https://trae-api-cn.mchost.guru";

/// SOLO 对话接口路径（消耗 IDE 积分，product_id 208）。
pub const TRAE_LLM_CHAT_PATH: &str = "/api/agent/v3/llm_utils_chat";

/// 客户端 App ID（参考实现硬编码值，非机密）。
pub const TRAE_APP_ID: &str = "6eefa01c-1036-4c7e-9ca5-d891f63bfcd8";

/// IDE 版本号与版本码，随上游校验字段一同下发。
pub const TRAE_IDE_VERSION: &str = "0.1.50";
pub const TRAE_IDE_VERSION_CODE: &str = "20260811";

/// `function` 字段取值。
pub const TRAE_FUNCTION: &str = "solo_work_lite";

/// 未指定模型时的默认值。
pub const TRAE_DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// 单请求体上限默认值（MB）。与 WorkBuddy 网关保持一致。
pub const DEFAULT_MAX_BODY_MB: usize = 8;

/// 单轮最多换号次数。
pub const DEFAULT_MAX_ROTATE: usize = 3;

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// Trae 网关运行配置（落盘 `~/.buddy-switch/trae/api_gateway.json`）。
///
/// 字段名沿用 WorkBuddy 网关的 snake_case 约定（同一管理面的两套网关不该有两种风格），
/// 同时接受 camelCase 别名以容忍前端写法差异。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TraeGatewayConfig {
    /// 是否启用独立监听。
    pub enabled: bool,
    /// 监听地址，默认 `127.0.0.1`。
    #[serde(alias = "bindAddr")]
    pub bind_addr: String,
    /// 监听端口，默认 [`TRAE_DEFAULT_API_PORT`]（7864，与 WorkBuddy 的 57891 错开）。
    pub port: u16,
    /// 是否允许非回环监听（默认 false）。
    #[serde(alias = "allowNonLoopback")]
    pub allow_non_loopback: bool,
    /// 请求日志保留条数。
    #[serde(alias = "logKeep")]
    pub log_keep: usize,
    /// 是否记录 prompt / response 正文。
    #[serde(alias = "logBodies")]
    pub log_bodies: bool,
    /// 请求体上限（MB；`0` 视为未设置并回落默认）。
    #[serde(alias = "maxBodyMb")]
    pub max_body_mb: usize,
    /// 未指定模型时的默认模型名。
    #[serde(alias = "defaultModel")]
    pub default_model: String,
    /// 单轮最多换号次数。
    #[serde(alias = "maxRotate")]
    pub max_rotate: usize,
}

impl Default for TraeGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_addr: "127.0.0.1".to_string(),
            port: TRAE_DEFAULT_API_PORT,
            allow_non_loopback: false,
            log_keep: 200,
            log_bodies: false,
            max_body_mb: DEFAULT_MAX_BODY_MB,
            default_model: TRAE_DEFAULT_MODEL.to_string(),
            max_rotate: DEFAULT_MAX_ROTATE,
        }
    }
}

impl TraeGatewayConfig {
    /// 从 `~/.buddy-switch/trae/api_gateway.json` 读取；缺失/损坏回落默认值。
    pub fn load() -> Self {
        let file = paths::api_gateway_file();
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Ok(config) = serde_json::from_str::<TraeGatewayConfig>(&text) {
                return config;
            }
        }
        TraeGatewayConfig::default()
    }

    /// 原子写回配置文件。
    pub fn save(&self) -> Result<(), String> {
        let file = paths::api_gateway_file();
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let content = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        core_config::atomic_write(&file, &content).map_err(|error| error.to_string())
    }

    /// 对外 Base URL（不含 `/v1` 后缀，供接入指引展示）。
    pub fn base_url(&self) -> String {
        let host = if self.bind_addr.trim().is_empty() {
            "127.0.0.1"
        } else {
            self.bind_addr.trim()
        };
        format!("http://{host}:{}", self.port)
    }
}

/// 请求体上限（MB）→ 字节数；`0` 视为未设置并回落默认。
pub fn body_limit_bytes(max_body_mb: usize) -> usize {
    let mb = if max_body_mb == 0 {
        DEFAULT_MAX_BODY_MB
    } else {
        max_body_mb
    };
    mb.saturating_mul(1024 * 1024)
}

// ---------------------------------------------------------------------------
// 运行状态
// ---------------------------------------------------------------------------

/// 可 `Arc` 共享的 Trae 网关状态。
#[derive(Clone)]
pub struct TraeGatewayState {
    /// 运行配置。
    pub config: Arc<RwLock<TraeGatewayConfig>>,
    /// **按产品线分家的账号池**：Key 的归属（`variant`）决定用哪个池。
    ///
    /// 用 `HashMap<TraeVariant, TraePool>` 而非「一个池 + 每条目带 variant」：
    /// - 两条产品线的账号库、冷却文件、积分缓存**本就分家**（`*_for(variant)`），
    ///   分成两个池可以各自 `sync_for`，不必在选号里再过滤；
    /// - 避免「同一 uid 在两条线是两个不同账号」被错误合并
    ///   （`core::modules::trae::account` 的 `find_for` 注释）。
    pub pools: Arc<Mutex<HashMap<TraeVariant, TraePool>>>,
    /// 请求日志（与 WorkBuddy 网关同一个 [`RequestLog`]，只是换了落盘路径）。
    pub log: Arc<RequestLog>,
    /// 多 API Key 存储（哈希 + 前缀 + **归属产品线**；含旧 `settings.apiKey` 兼容读）。
    pub key_store: Arc<apikey::TraeApiKeyStore>,
    /// 进程启动时刻（毫秒）。
    pub started_at: i64,
    /// 累计请求数。
    pub total_requests: Arc<AtomicU64>,
    /// 最近一次错误摘要。
    pub last_error: Arc<RwLock<Option<String>>>,
    /// 出站 HTTP 客户端。
    pub http: reqwest::Client,
    /// 请求体上限字节数（构造期固化，见 [`body_limit_bytes`]）。
    pub body_limit_bytes: usize,
    /// 出站上游基址，默认 [`TRAE_AGENT_HOST`]。
    ///
    /// 做成字段而不是直接引用常量，是为了让集成测试能把出站打到**本地 mock 上游**：
    /// 真实 Trae 需要有效 JWT，而鉴权、换号、SSE 转换这三块逻辑恰恰最需要端到端验证。
    ///
    /// 2026-09-30 起生产路径**按池区域取端点**（CN → 本常量值、国际 → 国际表
    /// `agent_host`，见 `routes::attempt_once`）：本字段仅在「被显式改离默认值」时
    /// 对全部变体生效（测试 override），未改时等价于按区域分家的行为。
    pub upstream: String,
}

impl TraeGatewayState {
    /// 依据配置构造一份完整运行状态。
    ///
    /// **不在此处自动生成 Key**：多 Key 化后 Key 只存哈希，自动生成的明文无处可取，
    /// 会造出一把「存在但无人知道明文」的幽灵 Key。改为由用户在「API 服务」页显式创建
    /// （[`apikey::create_response`]），或由 [`ensure_api_key`] 在需要时兜底生成。
    /// 老用户则经 [`apikey::TraeApiKeyStore::load`] 的 legacy 回落继续可用。
    pub fn new(config: TraeGatewayConfig) -> Self {
        let log = Arc::new(RequestLog::new(
            paths::api_gateway_log_file(),
            config.log_keep,
            config.log_bodies,
        ));
        Self {
            config: Arc::new(RwLock::new(config.clone())),
            pools: Arc::new(Mutex::new(HashMap::new())),
            log,
            key_store: Arc::new(apikey::TraeApiKeyStore::new(
                paths::api_gateway_keys_file(),
            )),
            started_at: core_config::now_ms(),
            total_requests: Arc::new(AtomicU64::new(0)),
            last_error: Arc::new(RwLock::new(None)),
            http: build_http_client(),
            body_limit_bytes: body_limit_bytes(config.max_body_mb),
            upstream: TRAE_AGENT_HOST.to_string(),
        }
    }

    /// 读取配置快照。
    pub async fn config_snapshot(&self) -> TraeGatewayConfig {
        self.config.read().await.clone()
    }

    /// 记一次请求（计数 + 日志 + 最近错误）。
    ///
    /// 日志条目字段名与 WorkBuddy 网关的 [`RequestMeta::to_value`] 对齐
    /// （`ts` / `endpoint` / `method` / `account` / `model` / `status` / `latencyMs` /
    /// `promptTokens` / `completionTokens` / `stream`），只是多了 `error` 与 `variant`。
    /// 这样「Token 统计」页可以用同一套归一化逻辑聚合两侧日志，且能按变体过滤。
    ///
    /// [`RequestMeta::to_value`]: crate::logging::RequestMeta::to_value
    #[allow(clippy::too_many_arguments)]
    pub async fn record_request(
        &self,
        endpoint: &'static str,
        model: &str,
        status: u16,
        uid: &str,
        latency_ms: i64,
        stream: bool,
        prompt_tokens: u64,
        completion_tokens: u64,
        error: Option<String>,
        variant: TraeVariant,
    ) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.log.record(serde_json::json!({
            "ts": core_config::now_ms(),
            "endpoint": endpoint,
            "method": "POST",
            "account": uid,
            "model": model,
            "status": status,
            "latencyMs": latency_ms,
            "promptTokens": prompt_tokens,
            "completionTokens": completion_tokens,
            "stream": stream,
            "error": error,
            // 归属产品线：Token 统计页据此出「变体范围条」（筛选维度）。
            // 用 as_str() 的下划线形态，与 Key/查询参数口径一致（勿用派生 serde）。
            "variant": variant.as_str(),
        }));
        if let Some(message) = error {
            *self.last_error.write().await = Some(message);
        }
    }
}

/// 出站客户端。
///
/// - `no_proxy()`：与 Trae 账号模块同理，绝不能走进本机 MITM 代理，否则请求会在
///   自己的代理里打转。
/// - 只设 `connect_timeout` 与 `read_timeout`，**不设总超时**：SSE 对话的合法时长
///   由模型输出长度决定，加总超时会把正常的长回答掐断。
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(300))
        .pool_max_idle_per_host(20)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 确保 Key 库里**至少有一把可用 Key**（load + legacy 优先，都没有才新建）。
///
/// 语义（T01）：返回**新建时**的明文（`Some`）；若已存在至少一把未吊销的 Key，
/// 则不新建、返回 `None` —— 明文不落库、不可复原，因此已有 Key 的明文无从返回。
///
/// 升级用户：`key_store.load()` 会回落 `settings.apiKey`，故这里不会给他们凭空造 Key，
/// 走的是「已存在 → 返回 None」分支，行为与升级前一致。
///
/// 新装用户：键库与 `settings.apiKey` 皆空 → 这里生成一把 `sk-trae-<32hex>` 并落库，
/// 明文由调用方（首启流程 / 管理命令）一次性呈现，之后不再可取。
pub fn ensure_api_key(store: &apikey::TraeApiKeyStore) -> Option<String> {
    if store.list().iter().any(|record| !record.is_revoked()) {
        return None;
    }
    let (_record, plaintext) = store.create("默认 Key".to_string(), TraeVariant::default());
    Some(plaintext)
}

/// 生成新 Key 明文：`sk-trae-` + 32 位 hex。
///
/// 仅用于测试 / 诊断的形态校验；正常发号走 [`apikey::TraeApiKeyStore::create`]
/// （它同时算出前缀与哈希并落库）。
pub fn generate_api_key() -> String {
    let secret = uuid::Uuid::new_v4().simple().to_string();
    format!("sk-trae-{secret}")
}

// ---------------------------------------------------------------------------
// 对外状态视图
// ---------------------------------------------------------------------------

/// `GET /api/trae/gateway/status` 的响应契约。
///
/// 字段命名与 WorkBuddy 网关的 `GatewayStatusView` 保持一致（snake_case），
/// 便于同一套前端归一化逻辑复用。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TraeGatewayStatusView {
    pub enabled: bool,
    pub running: bool,
    pub addr: Option<String>,
    pub base_url: String,
    pub bind_addr: String,
    pub port: u16,
    pub allow_non_loopback: bool,
    pub version: String,
    /// 累计请求数。
    pub total_requests: u64,
    /// 最近一次错误摘要。
    pub last_error: Option<String>,
    /// API Key 的脱敏展示（`sk-trae-abcd…`）。
    pub api_key_prefix: String,
    /// 账号池摘要。
    pub pool: TraePoolSummary,
}

impl TraeGatewayStatusView {
    /// 由配置派生；`running` / `addr` / `version` / 运行时字段由调用方补充。
    pub fn from_config(config: &TraeGatewayConfig) -> Self {
        Self {
            enabled: config.enabled,
            running: false,
            addr: None,
            base_url: config.base_url(),
            bind_addr: config.bind_addr.clone(),
            port: config.port,
            allow_non_loopback: config.allow_non_loopback,
            version: String::new(),
            total_requests: 0,
            last_error: None,
            api_key_prefix: String::new(),
            pool: TraePoolSummary::default(),
        }
    }
}

/// Key 脱敏：保留前缀与末 4 位，中间省略。
///
/// 返回的字符串**不包含**可复原的完整 Key，因此可安全下发到前端。
pub fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.len() <= 12 {
        return format!("{}…", &trimmed[..trimmed.len().min(4)]);
    }
    format!("{}…{}", &trimmed[..12], &trimmed[trimmed.len() - 4..])
}

/// 当前 Unix 秒。
///
/// 池的冷却与积分到期时间都以**秒**为单位（与 `credits::expire_times` 一致），
/// 全模块统一从这里取，避免某处混进毫秒。
pub fn now_secs() -> i64 {
    chrono::Local::now().timestamp()
}

/// 组装管理面状态视图（Tauri 命令与 webui 路由**共用**）。
///
/// `running` / `addr` / `version` 由宿主提供——只有宿主知道监听是否真的起来了、
/// 应用版本是多少。池摘要与账号明细针对**指定变体**的池现算，因此两个宿主看到的永远是同一份。
///
/// **响应键集合不变**（`trae/mod.rs` 的 `status_view_keys_are_pinned` 护栏仍通过）：
/// `variant` 只是入参，不进入响应体——池摘要本身已隐含「这是哪条产品线」，
/// 多一个键反而要与前端重新对齐形状。
pub async fn status_view(
    state: &TraeGatewayState,
    running: bool,
    addr: Option<String>,
    version: &str,
    variant: TraeVariant,
) -> serde_json::Value {
    let now = now_secs();
    let config = state.config_snapshot().await;
    let mut view = TraeGatewayStatusView::from_config(&config);
    view.running = running;
    view.addr = addr;
    view.version = version.to_string();
    view.total_requests = state.total_requests.load(Ordering::Relaxed);
    view.last_error = state.last_error.read().await.clone();
    // 最近一把未吊销 Key 的前缀（缺省空串；明文不可复原，故只给前缀）。
    view.api_key_prefix = state
        .key_store
        .list()
        .into_iter()
        .filter(|record| !record.is_revoked())
        .max_by_key(|record| record.created_at)
        .map(|record| record.prefix)
        .unwrap_or_default();

    let (summary, accounts, diagnose) = {
        let mut pools = state.pools.lock().await;
        let pool = pools.entry(variant).or_insert_with(|| TraePool::for_variant(variant));
        pool.sync_for(variant);
        (
            pool.summary(now),
            pool.status_list(now),
            pool.diagnose(now),
        )
    };
    view.pool = summary;

    let mut body = match serde_json::to_value(&view) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    body.insert("accounts".into(), serde_json::json!(accounts));
    body.insert("diagnose".into(), serde_json::json!(diagnose));
    body.insert("upstream".into(), serde_json::json!(state.upstream));
    serde_json::Value::Object(body)
}

// ---------------------------------------------------------------------------
// 路由与监听
// ---------------------------------------------------------------------------

/// 组装 Trae 网关路由（**不含 fallback**）。
///
/// 挂载 `GET /health`、`GET /status`、`GET /v1/models`、`POST /v1/chat/completions`。
/// 鉴权由 [`routes::bearer_auth`] 统一处理，`/health` 免鉴权。
pub fn router(state: TraeGatewayState) -> axum::Router {
    use axum::routing::{get, post};
    let body_limit = state.body_limit_bytes;
    axum::Router::new()
        .route("/health", get(routes::health))
        .route("/status", get(routes::status))
        .route("/v1/models", get(routes::models))
        .route("/v1/chat/completions", post(routes::chat_completions))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            routes::bearer_auth,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(body_limit))
        .with_state(state)
    // 不设 fallback：宿主可能把它 merge 进更大的 Router。
}

/// 按配置启动独立监听。
///
/// 与 WorkBuddy 网关一致：只监听回环地址，除非显式允许；端口占用返回**可读错误**
/// 而不是 panic。
pub async fn spawn_listener(state: TraeGatewayState) -> anyhow::Result<GatewayHandle> {
    let config = state.config_snapshot().await;
    let bind_addr = config.bind_addr.clone();
    let port = config.port;

    let ip: IpAddr = bind_addr
        .parse()
        .map_err(|_| anyhow::anyhow!("Trae 网关监听地址无效：{bind_addr}（应为 IPv4/IPv6 字面量）"))?;

    if !ip.is_loopback() && !config.allow_non_loopback {
        return Err(anyhow::anyhow!(
            "已拒绝监听非回环地址 {bind_addr}：局域网内任何设备都可消耗你的 Trae 积分，\
             请确认可信网络后，在设置中显式开启『允许局域网访问』再重试"
        ));
    }

    let addr = SocketAddr::new(ip, port);
    let listener = TcpListener::bind(addr).await.map_err(|error| {
        anyhow::anyhow!("无法监听 {addr}：端口可能被占用或不可用（{error}）。请在设置中改用其他端口。")
    })?;
    let local = listener.local_addr().unwrap_or(addr);

    let app = router(state);
    let (tx, rx) = oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;
    });

    Ok(GatewayHandle::new(local, tx, join))
}

/// Trae 网关请求日志文件路径（供管理命令透出）。
pub fn log_file() -> PathBuf {
    paths::api_gateway_log_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn keys_of(value: &serde_json::Value) -> BTreeSet<&str> {
        value
            .as_object()
            .expect("serialized value must be an object")
            .keys()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn config_serializes_with_pinned_snake_case_keys() {
        let value = serde_json::to_value(TraeGatewayConfig::default()).expect("serialize config");
        let expected: BTreeSet<&str> = [
            "enabled",
            "bind_addr",
            "port",
            "allow_non_loopback",
            "log_keep",
            "log_bodies",
            "max_body_mb",
            "default_model",
            "max_rotate",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys_of(&value), expected, "config key set must be pinned");
    }

    #[test]
    fn config_defaults_are_loopback_and_off_by_default() {
        let config = TraeGatewayConfig::default();
        assert!(!config.enabled, "默认不得自动开始监听");
        assert_eq!(config.bind_addr, "127.0.0.1");
        assert!(!config.allow_non_loopback);
        assert_eq!(config.port, TRAE_DEFAULT_API_PORT);
        // 端口必须与 WorkBuddy 网关（57891）错开，否则同时启用会互相抢端口。
        assert_ne!(config.port, 57891);
        assert_eq!(config.base_url(), format!("http://127.0.0.1:{TRAE_DEFAULT_API_PORT}"));
    }

    #[test]
    fn config_accepts_camel_case_aliases_and_missing_fields() {
        let config: TraeGatewayConfig = serde_json::from_str(
            r#"{"enabled":true,"bindAddr":"0.0.0.0","port":60001,"allowNonLoopback":true,
                "logKeep":10,"logBodies":true,"maxBodyMb":4,"defaultModel":"glm-5.3","maxRotate":5}"#,
        )
        .expect("camelCase aliases must deserialize");
        assert!(config.enabled);
        assert_eq!(config.bind_addr, "0.0.0.0");
        assert_eq!(config.port, 60001);
        assert!(config.allow_non_loopback);
        assert_eq!(config.log_keep, 10);
        assert!(config.log_bodies);
        assert_eq!(config.max_body_mb, 4);
        assert_eq!(config.default_model, "glm-5.3");
        assert_eq!(config.max_rotate, 5);

        let empty: TraeGatewayConfig = serde_json::from_str("{}").expect("empty must parse");
        assert_eq!(empty.port, TraeGatewayConfig::default().port);
    }

    #[test]
    fn body_limit_falls_back_to_default_when_zero() {
        assert_eq!(body_limit_bytes(0), DEFAULT_MAX_BODY_MB * 1024 * 1024);
        assert_eq!(body_limit_bytes(2), 2 * 1024 * 1024);
    }

    #[test]
    fn api_key_mask_never_leaks_the_middle() {
        let key = "sk-trae-0123456789abcdef0123456789abcdef";
        let masked = mask_api_key(key);
        assert!(masked.starts_with("sk-trae-0123"));
        assert!(masked.ends_with("cdef"));
        // 中段绝不出现在脱敏结果里。
        assert!(!masked.contains("456789abcdef0123456789ab"));
        assert!(masked.len() < key.len());

        assert_eq!(mask_api_key(""), "");
        assert_eq!(mask_api_key("short"), "shor…");
    }

    #[test]
    fn generated_api_key_has_stable_shape() {
        let key = generate_api_key();
        assert!(key.starts_with("sk-trae-"));
        assert_eq!(key.len(), "sk-trae-".len() + 32);
        assert!(key["sk-trae-".len()..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(key, generate_api_key(), "每次生成必须不同");
    }

    #[test]
    fn status_view_keys_are_pinned() {
        let view = TraeGatewayStatusView::from_config(&TraeGatewayConfig::default());
        let value = serde_json::to_value(&view).expect("serialize status");
        let expected: BTreeSet<&str> = [
            "enabled",
            "running",
            "addr",
            "base_url",
            "bind_addr",
            "port",
            "allow_non_loopback",
            "version",
            "total_requests",
            "last_error",
            "api_key_prefix",
            "pool",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys_of(&value), expected, "status key set must be pinned");
        // 不得出现 camelCase 键。
        assert!(!value.as_object().unwrap().contains_key("baseUrl"));
        assert!(!value.as_object().unwrap().contains_key("apiKey"));
    }
}
