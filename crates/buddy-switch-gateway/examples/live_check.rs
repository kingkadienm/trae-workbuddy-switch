//! 用**真实账号**对 Trae 网关做一次端到端冒烟（**会真实调用上游，消耗少量积分**）。
//!
//! # 与 `tests/trae_gateway_e2e.rs` 的区别
//!
//! 那个用 **mock 上游 + 隔离 home**，验证的是「协议 / 换号 / 鉴权」这些**我们能控制**的层。
//! 这个用**真实 home**（`~/.buddy-switch`）与**真实上游**，因此能验证 mock 覆盖不到的
//! 那一层：真实账号能不能被选中、上游认不认我们改写的请求体。
//!
//! 运行：`cargo run -p buddy-switch-gateway --example live_check`
//!
//! # 安全措施
//!
//! - 用**临时 Key**（名字带 pid），跑完**必定删除**（成功与失败都删）；
//! - 只发**两次极短**请求（一次非流式、一次流式），prompt 是 `ping`；
//! - **只读**账号库 / 冷却文件，不写入任何账号状态。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use buddy_switch_core::modules::trae::{paths, variant::TraeVariant};
use buddy_switch_gateway::trae::{
    apikey::TraeApiKeyStore, router, TraeGatewayConfig, TraeGatewayState,
};

#[tokio::main]
async fn main() {
    let state = TraeGatewayState::new(TraeGatewayConfig::default());
    let store = TraeApiKeyStore::new(paths::api_gateway_keys_file());

    // 先清掉本工具**上次遗留**的临时 Key（上次可能因为崩溃没走到 cleanup）。
    sweep_leftovers(&store);

    // 临时 Key：用 pid 命名，避免与用户自己的 Key 混淆；跑完删。
    let name = format!("live-check-{}", std::process::id());
    let (record, plaintext) = store.create(name.clone(), TraeVariant::default());
    println!("[setup] 已创建临时 Key「{name}」前缀 {}", record.prefix);

    let result = run_checks(&state, &plaintext).await;

    remove_key(&store, &record.id, &record.prefix);

    if let Err(error) = result {
        eprintln!("\n[FAIL] {error}");
        std::process::exit(1);
    }
    println!("\n[OK] 真实链路两项检查全部通过");
}

/// 删除一把 Key：`delete` 要求**先吊销**，直接删会报「请先吊销该 API Key 再删除」。
fn remove_key(store: &TraeApiKeyStore, id: &str, prefix: &str) {
    if let Err(error) = store.revoke(id) {
        eprintln!("[cleanup] ⚠️ 吊销失败（{prefix}）：{error}");
    }
    match store.delete(id) {
        Ok(()) => println!("[cleanup] 临时 Key {prefix} 已吊销并删除"),
        Err(error) => eprintln!("[cleanup] ⚠️ 删除失败（请手动删 {prefix}）：{error}"),
    }
}

/// 清理本工具遗留的临时 Key（按名字前缀识别，**不碰**用户自己的 Key）。
fn sweep_leftovers(store: &TraeApiKeyStore) {
    for record in store.list() {
        if record.name.starts_with("live-check-") {
            println!("[setup] 发现遗留临时 Key「{}」，清理", record.name);
            remove_key(store, &record.id, &record.prefix);
        }
    }
}

async fn run_checks(state: &TraeGatewayState, key: &str) -> Result<(), Box<dyn std::error::Error>> {
    // ---- 1. 非流式：Anthropic 协议 ----
    let body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "ping"}],
    })
    .to_string();

    let response = router(state.clone())
        .oneshot(request("/v1/messages", key, &body)?)
        .await?;
    let status = response.status();
    let text = body_text(response).await;
    println!("\n=== /v1/messages（非流式）===");
    println!("HTTP {status}");
    println!("{}", preview(&text, 600));

    if status != StatusCode::OK {
        return Err(format!("非流式请求失败（HTTP {status}）：{}", preview(&text, 400)).into());
    }
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("非流式响应不是 JSON：{}", preview(&text, 300)))?;
    if value["type"] != "message" {
        return Err(format!("响应 type 不是 message：{}", preview(&text, 300)).into());
    }
    let content = value["content"].as_array().cloned().unwrap_or_default();
    if content.is_empty() {
        return Err(format!("content 为空：{}", preview(&text, 300)).into());
    }
    println!(
        "[check1] ✓ Anthropic message 形状正确，正文 {} 字，usage in={} out={}",
        content[0]["text"].as_str().map(str::len).unwrap_or(0),
        value["usage"]["input_tokens"],
        value["usage"]["output_tokens"],
    );

    // ---- 2. 流式：OpenAI 协议（确认另一条入口也没被本次改动带坏）----
    let body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [{"role": "user", "content": "ping"}],
        "stream": true,
    })
    .to_string();

    let response = router(state.clone())
        .oneshot(request("/v1/chat/completions", key, &body)?)
        .await?;
    let status = response.status();
    let text = body_text(response).await;
    println!("\n=== /v1/chat/completions（流式）===");
    println!("HTTP {status}");
    println!("{}", preview(&text, 600));

    if status != StatusCode::OK {
        return Err(format!("流式请求失败（HTTP {status}）：{}", preview(&text, 400)).into());
    }
    if !text.contains("data:") {
        return Err(format!("流式响应不像 SSE：{}", preview(&text, 300)).into());
    }
    println!("[check2] ✓ OpenAI SSE 正常（未泄漏 SOLO 事件名）");

    Ok(())
}

fn request(uri: &str, key: &str, body: &str) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {key}"))
        .body(Body::from(body.to_string()))
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).to_string()
}

fn preview(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    format!(
        "{}…（共 {} 字）",
        trimmed.chars().take(limit).collect::<String>(),
        trimmed.chars().count()
    )
}
