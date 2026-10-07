//! AutoClaw（智谱澳龙云端沙箱）桥接。
//!
//! 对照参考实现 `internal/autoclaw/client.go`：
//! AutoClaw 作为独立部署的 Python 服务（autoclaw2api，端口 7865）自治运行，
//! 本模块只做三件事：
//!   1. 健康探测（/healthz）
//!   2. 模型列表拉取（/v1/models）并归一化
//!   3. OpenAI 兼容反向代理（含 SSE 流式透传）
//!
//! 账号池 / 状态机 / 签到全部由 autoclaw2api 自治，Rust 侧不做重复实现。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// AutoClaw 桥接配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoclawConfig {
    /// 是否启用 AutoClaw 桥接（默认关闭）。
    pub enabled: bool,
    /// AutoClaw 服务基址（如 `http://127.0.0.1:7865`）。
    pub base_url: String,
    /// AutoClaw 服务的 API 密钥（与主网关统一）。
    pub api_key: String,
    /// 模型列表缓存 TTL（秒，默认 60）。
    #[serde(default = "default_model_cache_ttl")]
    pub model_cache_ttl_secs: u64,
}

fn default_model_cache_ttl() -> u64 {
    60
}

impl AutoclawConfig {
    /// 是否为有效配置（已启用且有 base_url）。
    pub fn is_active(&self) -> bool {
        self.enabled && !self.base_url.is_empty()
    }

    /// 规范化 base_url（去掉末尾斜杠）。
    pub fn normalized_base(&self) -> String {
        self.base_url.trim_end_matches('/').to_string()
    }
}

// ---------------------------------------------------------------------------
// 模型缓存（60s TTL）
// ---------------------------------------------------------------------------

/// 模型集合的 60 秒 TTL 缓存。
#[derive(Debug, Clone, Default)]
struct KnownCache {
    ttl: Duration,
    set: Arc<RwLock<Option<(HashMap<String, bool>, std::time::Instant)>>>,
}

impl KnownCache {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            set: Arc::new(RwLock::new(None)),
        }
    }

    /// 尝试从缓存获取；返回 `None` 表示缓存未命中或已过期。
    async fn get(&self) -> Option<HashMap<String, bool>> {
        let guard = self.set.read().await;
        if let Some((ref set, ref at)) = *guard {
            if at.elapsed() < self.ttl {
                return Some(set.clone());
            }
        }
        None
    }

    /// 写入缓存。
    async fn put(&self, set: HashMap<String, bool>) {
        let mut guard = self.set.write().await;
        *guard = Some((set, std::time::Instant::now()));
    }

    /// 使缓存失效。
    async fn invalidate(&self) {
        let mut guard = self.set.write().await;
        *guard = None;
    }
}

// ---------------------------------------------------------------------------
// 模型列表响应
// ---------------------------------------------------------------------------

/// AutoClaw /v1/models 响应。
#[derive(Debug, Clone, Deserialize)]
pub struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    id: String,
    #[serde(default)]
    owned_by: String,
}

// ---------------------------------------------------------------------------
// 健康检查响应
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct HealthResponse {
    healthy: i64,
}

// ---------------------------------------------------------------------------
// AutoClaw 客户端
// ---------------------------------------------------------------------------

/// AutoClaw2api 的最小客户端。
///
/// 线程安全：可 `Clone` 后跨任务共享。
#[derive(Clone)]
pub struct AutoclawClient {
    pub base_url: String,
    pub api_key: String,
    pub http: Client,
    cache: KnownCache,
    cache_ttl: Duration,
    /// 上一次已知的模型数量（用于健康判断辅助）。
    known_count: Arc<AtomicU64>,
    /// 最近一次健康检查结果。
    healthy: Arc<AtomicBool>,
}

impl AutoclawClient {
    /// 构造客户端。
    pub fn new(config: &AutoclawConfig) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            base_url: config.normalized_base(),
            api_key: config.api_key.clone(),
            http,
            cache: KnownCache::new(Duration::from_secs(config.model_cache_ttl_secs)),
            cache_ttl: Duration::from_secs(config.model_cache_ttl_secs),
            known_count: Arc::new(AtomicU64::new(0)),
            healthy: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 更新配置（热更新时调用）。
    pub fn update_config(&mut self, config: &AutoclawConfig) {
        self.base_url = config.normalized_base();
        self.api_key = config.api_key.clone();
        self.cache_ttl = Duration::from_secs(config.model_cache_ttl_secs);
        self.cache = KnownCache::new(self.cache_ttl);
        self.known_count.store(0, Ordering::SeqCst);
    }

    /// 检查 AutoClaw 服务是否可达且至少有一个健康账号。
    pub async fn is_healthy(&self) -> bool {
        let url = format!("{}/healthz", self.base_url);
        let req = match self.http.get(&url).build() {
            Ok(r) => r,
            Err(_) => {
                self.healthy.store(false, Ordering::SeqCst);
                return false;
            }
        };

        match self.http.execute(req).await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<HealthResponse>().await {
                    Ok(h) => {
                        let ok = h.healthy > 0;
                        self.healthy.store(ok, Ordering::SeqCst);
                        ok
                    }
                    Err(_) => {
                        self.healthy.store(false, Ordering::SeqCst);
                        false
                    }
                }
            }
            _ => {
                self.healthy.store(false, Ordering::SeqCst);
                false
            }
        }
    }

    /// 获取缓存的健康状态（不发起网络请求）。
    pub fn cached_healthy(&self) -> bool {
        self.healthy.load(Ordering::SeqCst)
    }

    /// 拉取 AutoClaw 的模型列表，把 owned_by 归一为 "autoclaw"。
    pub async fn fetch_models(&self) -> Vec<ModelEntry> {
        let url = format!("{}/v1/models", self.base_url);
        let mut req = self.http.get(&url);
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<ModelsResponse>().await {
                    Ok(mut models) => {
                        let count = models.data.len() as u64;
                        self.known_count.store(count, Ordering::SeqCst);

                        // 归一化 owned_by，并做旧名兼容
                        for m in &mut models.data {
                            // zai_glm-5.3-flash → glm-5.3-flash（与 cn 条目合并）
                            if m.id.eq_ignore_ascii_case("zai_glm-5.3-flash") {
                                m.id = "glm-5.3-flash".to_string();
                            }
                            m.owned_by = "autoclaw".to_string();
                        }
                        models.data
                    }
                    Err(_) => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// 获取已知模型集合（含缓存）。
    pub async fn known_models(&self) -> HashMap<String, bool> {
        // 先尝试缓存
        if let Some(cached) = self.cache.get().await {
            return cached;
        }

        let models = self.fetch_models().await;
        let mut set = HashMap::with_capacity(models.len() * 2);
        for m in &models {
            set.insert(m.id.clone(), true);
            set.insert(m.id.to_lowercase(), true);
        }
        // 旧名兼容
        set.insert("zai_glm-5.3-flash".to_string(), true);

        self.cache.put(set.clone()).await;
        set
    }

    /// 判断模型是否属于 AutoClaw 目录。
    pub async fn known_model(&self, model: &str) -> bool {
        let set = self.known_models().await;
        set.contains_key(model) || set.contains_key(&model.to_lowercase())
    }

    /// 已知模型数量（用于健康判断辅助）。
    pub fn known_count(&self) -> u64 {
        self.known_count.load(Ordering::SeqCst)
    }

    /// 使模型缓存失效。
    pub async fn invalidate_cache(&self) {
        self.cache.invalidate().await;
    }

    /// 构建反向代理请求（将请求转发到 AutoClaw 服务）。
    pub fn proxy_request(&self, mut req: Request<Body>) -> Result<Request<Body>, Response> {
        let path_and_query = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");

        let target_uri = match format!("{}{}", self.base_url, path_and_query).parse() {
            Ok(uri) => uri,
            Err(_) => {
                return Err(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from("autoclaw: invalid target URL"))
                    .unwrap());
            }
        };

        *req.uri_mut() = target_uri;

        // 注入鉴权头
        if !self.api_key.is_empty() {
            req.headers_mut()
                .insert("Authorization", format!("Bearer {}", self.api_key).parse().unwrap());
        }

        Ok(req)
    }

    /// 构建错误响应。
    pub fn error_response(&self, message: &str) -> Response {
        Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header("Content-Type", "application/json")
            .body(Body::from(format!(
                r#"{{"error":{{"message":"autoclaw upstream unreachable: {}","type":"api_error","code":"bad_gateway"}}}}"#,
                message.replace('"', "'")
            )))
            .unwrap()
    }
}

// ---------------------------------------------------------------------------
// 便捷函数
// ---------------------------------------------------------------------------

/// 创建默认禁用的 AutoClaw 配置。
pub fn disabled_config() -> AutoclawConfig {
    AutoclawConfig::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_is_inactive() {
        let cfg = disabled_config();
        assert!(!cfg.is_active());
    }

    #[test]
    fn active_config_requires_base_url() {
        let mut cfg = AutoclawConfig {
            enabled: true,
            base_url: String::new(),
            ..Default::default()
        };
        assert!(!cfg.is_active());

        cfg.base_url = "http://127.0.0.1:7865".to_string();
        assert!(cfg.is_active());
    }

    #[test]
    fn normalized_base_strips_trailing_slash() {
        let cfg = AutoclawConfig {
            base_url: "http://127.0.0.1:7865/".to_string(),
            ..Default::default()
        };
        assert_eq!(cfg.normalized_base(), "http://127.0.0.1:7865");
    }

    #[test]
    fn proxy_request_replaces_uri_and_injects_auth() {
        use axum::http::Uri;

        let client = AutoclawClient {
            base_url: "http://127.0.0.1:7865".to_string(),
            api_key: "secret-key".to_string(),
            http: reqwest::Client::new(),
            cache: KnownCache::new(Duration::from_secs(60)),
            cache_ttl: Duration::from_secs(60),
            known_count: Arc::new(AtomicU64::new(0)),
            healthy: Arc::new(AtomicBool::new(false)),
        };

        let req = Request::builder()
            .method("POST")
            .uri("http://gateway:57891/v1/chat/completions")
            .body(Body::empty())
            .unwrap();

        let result = client.proxy_request(req);
        assert!(result.is_ok());
        let proxied = result.unwrap();
        assert_eq!(proxied.uri(), &Uri::from_static("http://127.0.0.1:7865/v1/chat/completions"));
        assert_eq!(
            proxied.headers().get("Authorization").unwrap(),
            "Bearer secret-key"
        );
    }

    #[test]
    fn proxy_request_without_api_key_skips_auth_header() {
        use axum::http::Uri;

        let client = AutoclawClient {
            base_url: "http://127.0.0.1:7865".to_string(),
            api_key: String::new(),
            http: reqwest::Client::new(),
            cache: KnownCache::new(Duration::from_secs(60)),
            cache_ttl: Duration::from_secs(60),
            known_count: Arc::new(AtomicU64::new(0)),
            healthy: Arc::new(AtomicBool::new(false)),
        };

        let req = Request::builder()
            .method("GET")
            .uri("http://gateway:57891/healthz")
            .body(Body::empty())
            .unwrap();

        let result = client.proxy_request(req);
        assert!(result.is_ok());
        let proxied = result.unwrap();
        assert_eq!(proxied.uri(), &Uri::from_static("http://127.0.0.1:7865/healthz"));
        assert!(proxied.headers().get("Authorization").is_none());
    }
}
