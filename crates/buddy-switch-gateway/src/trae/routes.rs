//! Trae 网关的四个端点与 Bearer 鉴权中间件。
//!
//! ## 与 WorkBuddy 网关路由的关系
//!
//! 只有 `POST /v1/chat/completions` 与 WorkBuddy 侧**同名**，实现却毫无共同点：
//! WorkBuddy 是「出站改写 → 透传 OpenAI SSE」，Trae 是「出站改写 → 转换私有 SOLO SSE」。
//! 因此本文件不引用 `crate::routes` 的任何内容，只有**错误响应体形状**刻意保持一致
//! （`{"error":{"message","type","code"}}`），让同一个 OpenAI 客户端两处都能读懂。
//!
//! ## 换号语义
//!
//! | 路径 | 连接期失败（HTTP 非 2xx / 传输层错误） | 流内失败（`event:error`） |
//! |:---|:---|:---|
//! | 流式 | 冷却该账号 → **换号重试**（最多 `max_rotate` 次） | 冷却该账号 → **不换号** |
//! | 非流式 | 冷却该账号 → **换号重试** | 冷却该账号 → **换号重试** |
//!
//! 差别来自「响应头是否已经发出」：流式一旦把 `200 + text/event-stream` 发给客户端，
//! 就只能把错误塞进事件流里（补一条 `data: {...error...}` 再 `data: [DONE]`）；
//! 非流式在聚合完成前一个字节都还没发给客户端，因此可以整轮重来。
//!
//! 流内失败也写回冷却，是为了让**下一轮**请求自动避开这个账号——否则每次都要先撞墙。

use std::collections::HashSet;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Extension, RawQuery, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use buddy_switch_core::modules::trae::account;
use buddy_switch_core::modules::trae::region::TraeRegion;
use buddy_switch_core::modules::trae::variant::TraeVariant;

use super::payload;
use super::pool::{classify_http, classify_solo, PickedTraeAccount, TraeErrKind, TraePool};
use super::sse::{self, TokenUsage};
use super::{
    now_secs, TraeGatewayState, TRAE_APP_ID, TRAE_IDE_VERSION,
    TRAE_IDE_VERSION_CODE, TRAE_LLM_CHAT_PATH,
};

/// 请求扩展：承载 Bearer Key 的**归属产品线**，供 handler 选择对应的账号池。
///
/// 用「请求扩展」而非给每个 handler 加参数：鉴权中间件解析出归属后写入，
/// `chat_completions` 读出，避免中间件 → handler 的签名穿透一堆函数。
#[derive(Clone, Copy, Debug)]
pub struct TraeKeyVariant(pub TraeVariant);

/// 日志里的端点名（与 WorkBuddy 网关同名字符串，便于统一聚合）。
const ENDPOINT: &str = "/v1/chat/completions";

/// 上游 UA。参考实现固定为 `TraeClient/TTNet`，改它没有好处。
const TRAE_USER_AGENT: &str = "TraeClient/TTNet";

// ---------------------------------------------------------------------------
// 处理器
// ---------------------------------------------------------------------------

/// `GET /health`：存活探针，**免鉴权**。
///
/// 附带账号池摘要：探活时顺手看一眼「还有没有可用账号」比再发一次 `/status` 省事。
/// 摘要取**默认变体**的池（探针无 Key、无归属，只能给一条产品线的概览）。
pub async fn health(State(state): State<TraeGatewayState>) -> Response {
    let variant = TraeVariant::default();
    let summary = {
        let mut pools = state.pools.lock().await;
        let pool = pools.entry(variant).or_insert_with(|| TraePool::for_variant(variant));
        pool.sync_for(variant);
        pool.summary(now_secs())
    };
    json_response(
        StatusCode::OK,
        json!({
            "status": "ok",
            "service": "trae-gateway",
            "running": true,
            "total_requests": state.total_requests.load(std::sync::atomic::Ordering::Relaxed),
            "pool": summary,
        }),
    )
}

/// Bearer 鉴权中间件：`/health` 免鉴权，其余端点校验 `Authorization: Bearer <key>`。
///
/// 与参考实现的一处**有意差异**：没有任何可用 Key 时**拒绝**而不是放行。参考实现
/// 「留空即不鉴权」意味着任何人只要猜到 7864 端口就能白嫖额度；本网关要求 Key 由
/// 用户显式创建（或经旧 `settings.apiKey` 兼容读得来），不存在「用户忘了配」的放行场景，
/// 所以宁严勿宽。
///
/// 鉴权通过后把 `record.variant`（**归属产品线**）写进请求扩展
/// （[`TraeKeyVariant`]），下游据此选择对应的账号池；同时 `touch` 刷新最近使用时间。
pub async fn bearer_auth(
    State(state): State<TraeGatewayState>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }

    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
                .map(str::trim)
        })
        .filter(|value| !value.is_empty());

    match presented {
        None => openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            "缺少凭据：请在 Authorization 头携带 `Bearer <API Key>`",
        ),
        Some(key) => match state.key_store.verify(key) {
            Some(record) => {
                // 归属产品线透传给 handler（决定用哪个账号池）。
                request
                    .extensions_mut()
                    .insert(TraeKeyVariant(record.variant));
                state.key_store.touch(&record.id);
                next.run(request).await
            }
            None => openai_error(
                StatusCode::UNAUTHORIZED,
                "invalid_request_error",
                "API Key 无效：请到「API 服务」页创建或复制一把可用的 Key",
            ),
        },
    }
}

/// `GET /status`：运行状态 + 账号明细 + 诊断。
///
/// 可选 query `variant`（`trae_work` / `trae_cn`）决定看哪个账号池；
/// 缺失或无法识别一律回落 [`TraeVariant::default`]（TraeWork），与旧行为一致。
pub async fn status(
    State(state): State<TraeGatewayState>,
    RawQuery(query): RawQuery,
) -> Response {
    let variant = parse_variant_query(query.as_deref());
    // 复用管理面那份组装逻辑：`/status` 与「API 服务」页显示的必须是同一份状态。
    let config = state.config_snapshot().await;
    let addr = Some(format!("{}:{}", config.bind_addr, config.port));
    let body =
        super::status_view(&state, true, addr, env!("CARGO_PKG_VERSION"), variant).await;
    json_response(StatusCode::OK, body)
}

/// 从 query 串解析变体参数（最简实现，与 server 的 `query_value` 同风格）。
fn parse_variant_query(query: Option<&str>) -> TraeVariant {
    query
        .unwrap_or("")
        .split('&')
        .find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key == "variant").then(|| TraeVariant::parse(value).unwrap_or_default())
        })
        .unwrap_or_default()
}

/// `GET /v1/models`：**随上游刷新**的模型清单（issue #4）。
///
/// 清单来源与「API 服务」页的模型卡**同源**：读该 Key **归属产品线**的客户端
/// `state.vscdb` 缓存（上游下发）。归属由鉴权中间件经请求扩展传入。
///
/// ## 两处刻意的取舍
///
/// 1. **归属用必需提取器**（不是 `Option<Extension<…>>`）：中间件对除 `/health`
///    外的所有路由都会注入它，**拿不到就说明鉴权链断了** —— 那属于配置错误，
///    应当 500 响亮失败，而不是静默回落默认变体（那会把「链断了」伪装成正常）。
/// 2. **读库放进阻塞池**：`models_response_for` 会同步读 SQLite（客户端可能在写，
///    最多等 core 侧设的 `busy_timeout` 那个窗口）。直接在 async handler 里做会占住 worker。
///
/// 客户端没启动过 / 没登录时读不到 ⇒ 回落静态清单（见 [`payload::models_response_for`]），
/// 保证 OpenAI 客户端**开箱即可列出模型**，而不是返回空表或报错。
pub async fn models(Extension(key_variant): Extension<TraeKeyVariant>) -> Response {
    let variant = key_variant.0;
    // `models_response_for` 自己**不失败**（读不到就回落静态清单），所以这里的
    // `JoinError` 只可能是运行时正在关闭 —— 用静态清单兜底，别让「列模型」挂掉。
    let body = tokio::task::spawn_blocking(move || payload::models_response_for(variant))
        .await
        .unwrap_or_else(|_| payload::models_response());
    json_response(StatusCode::OK, body)
}

/// `POST /v1/chat/completions`。
///
/// 归属产品线由鉴权中间件经请求扩展（[`TraeKeyVariant`]）传入，决定用哪个账号池。
pub async fn chat_completions(
    State(state): State<TraeGatewayState>,
    Extension(key_variant): Extension<TraeKeyVariant>,
    _headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let variant = key_variant.0;
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                &format!("请求体不是合法 JSON：{error}"),
            )
        }
    };

    let config = state.config_snapshot().await;
    let model = payload::model_of(&parsed, &config.default_model);
    let stream = payload::wants_stream(&parsed);
    let max_rotate = config.max_rotate.max(1);
    let chat_id = format!("chatcmpl-{}", uuid_like());

    if stream {
        stream_chat(
            &state,
            &body,
            &config.default_model,
            &model,
            &chat_id,
            max_rotate,
            started,
            variant,
        )
        .await
    } else {
        aggregate_chat(
            &state,
            &body,
            &config.default_model,
            &model,
            &chat_id,
            max_rotate,
            started,
            variant,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// 流式
// ---------------------------------------------------------------------------

/// 流式路径：先换号直到拿到 2xx，再把响应体交给转换任务。
#[allow(clippy::too_many_arguments)]
async fn stream_chat(
    state: &TraeGatewayState,
    body: &Bytes,
    default_model: &str,
    model: &str,
    chat_id: &str,
    max_rotate: usize,
    started: Instant,
    variant: TraeVariant,
) -> Response {
    let mut tried: HashSet<String> = HashSet::new();
    let mut last: Option<UpstreamFailure> = None;

    for _ in 0..max_rotate {
        match attempt_once(state, body, default_model, &mut tried, variant).await {
            None => break,
            Some(AttemptResult::Failed { failure, .. }) => last = Some(failure),
            Some(AttemptResult::Ok { account, response }) => {
                let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(64);
                let task_state = state.clone();
                let task_model = model.to_string();
                let task_chat_id = chat_id.to_string();
                tokio::spawn(async move {
                    let outcome = sse::stream_convert(response, tx, &task_chat_id, &task_model).await;
                    settle(
                        &task_state,
                        &account,
                        &task_model,
                        true,
                        started,
                        outcome.error,
                        outcome.usage,
                        variant,
                    )
                    .await;
                });
                return sse_response(rx);
            }
        }
    }

    // 没拿到任何可用上游：这里**还没发过响应头**，可以正常返回 JSON 错误。
    let failure = last.unwrap_or_else(|| no_account_failure_sync(state, variant));
    settle_failure(state, model, true, started, &failure, variant).await;
    openai_error(
        StatusCode::from_u16(failure.status).unwrap_or(StatusCode::BAD_GATEWAY),
        &failure.code,
        &failure.message,
    )
}

// ---------------------------------------------------------------------------
// 非流式
// ---------------------------------------------------------------------------

/// 非流式路径：本地聚合。**流内错误也换号**——聚合完成前客户端一个字节都没收到。
#[allow(clippy::too_many_arguments)]
async fn aggregate_chat(
    state: &TraeGatewayState,
    body: &Bytes,
    default_model: &str,
    model: &str,
    chat_id: &str,
    max_rotate: usize,
    started: Instant,
    variant: TraeVariant,
) -> Response {
    let mut tried: HashSet<String> = HashSet::new();
    let mut last: Option<UpstreamFailure> = None;

    for _ in 0..max_rotate {
        let (account, response) =
            match attempt_once(state, body, default_model, &mut tried, variant).await {
                None => break,
                Some(AttemptResult::Failed { failure, .. }) => {
                    last = Some(failure);
                    continue;
                }
                Some(AttemptResult::Ok { account, response }) => (account, response),
            };

        let (payload, error, usage) = sse::aggregate(response, chat_id, model).await;

        match (payload, error) {
            (Some(payload), None) => {
                settle(state, &account, model, false, started, None, usage, variant).await;
                return json_response(StatusCode::OK, payload);
            }
            (_, Some((code, message))) => {
                // 流内错误：冷却该账号 → 换下一个账号整轮重来（响应头还没发）。
                last = Some(
                    record_stream_error(state, &account, code, &message, variant).await,
                );
            }
            _ => {
                // 既没有 payload 也没有错误：上游给了个空流。按 5xx 处理并换号。
                last = Some(
                    record_stream_error(state, &account, 0, "上游返回空事件流", variant).await,
                );
            }
        }
    }

    let failure = last.unwrap_or_else(|| no_account_failure_sync(state, variant));
    settle_failure(state, model, false, started, &failure, variant).await;
    openai_error(
        StatusCode::from_u16(failure.status).unwrap_or(StatusCode::BAD_GATEWAY),
        &failure.code,
        &failure.message,
    )
}

// ---------------------------------------------------------------------------
// 选号与出站
// ---------------------------------------------------------------------------

/// 一次出站尝试的结果。
enum AttemptResult {
    /// 上游返回 2xx（响应体尚未读取）。
    Ok {
        account: PickedTraeAccount,
        response: reqwest::Response,
    },
    /// 出站失败：已分类、已写回冷却文件。
    Failed {
        #[allow(dead_code)]
        account: PickedTraeAccount,
        failure: UpstreamFailure,
    },
}

/// 选号 + 出站一次。`tried` 在内部累加，调用方反复调用即可自动换号。
///
/// 返回 `None` 表示**已经挑不出新账号**（不是失败，是没得试了）。
async fn attempt_once(
    state: &TraeGatewayState,
    body: &Bytes,
    default_model: &str,
    tried: &mut HashSet<String>,
    variant: TraeVariant,
) -> Option<AttemptResult> {
    let picked = {
        let mut pools = state.pools.lock().await;
        let pool = pools.entry(variant).or_insert_with(|| TraePool::for_variant(variant));
        // 每次选号前重新同步：另一个入口（签到页 / 桌面端）可能刚写了冷却或刷新了积分。
        pool.sync_for(variant);
        pool.pick(now_secs(), tried)
    }?;
    tried.insert(picked.uid.clone());

    let converted = payload::prepare_llm_chat_body(
        body,
        default_model,
        &picked.uid,
        &picked.device_id,
        &picked.machine_id,
    );

    match send_llm_chat(state, &picked, &converted).await {
        Ok(response) => Some(AttemptResult::Ok {
            account: picked,
            response,
        }),
        Err((status, detail)) => {
            let kind = classify_http(status);
            let failure = UpstreamFailure {
                // 传输层错误没有 HTTP 状态码，对客户端统一报 502（网关上游不可达）。
                status: if status == 0 { 502 } else { status },
                code: if status == 0 {
                    "upstream_unreachable".to_string()
                } else {
                    kind.as_str().to_string()
                },
                message: format!(
                    "账号「{}」上游失败（HTTP {}）：{}",
                    picked.name,
                    if status == 0 { "—".into() } else { status.to_string() },
                    detail
                ),
            };
            {
                let mut pools = state.pools.lock().await;
                pools
                    .entry(variant)
                    .or_insert_with(|| TraePool::for_variant(variant))
                    .apply_error(&picked.uid, kind, &failure.message);
            }
            *state.last_error.write().await = Some(failure.message.clone());
            Some(AttemptResult::Failed {
                account: picked,
                failure,
            })
        }
    }
}

/// 发一次 `llm_utils_chat`：拿到响应头即返回，不读 body。
///
/// 头部逐字对齐参考实现（`x-ide-token` 用裸 JWT，**不带** `Cloud-IDE-JWT ` 前缀，
/// 前缀只在 `Authorization` 场景使用）。`accept-encoding` 显式写 `identity`：
/// 本 crate 的 reqwest 未开 gzip/br/zstd 特性，若上游压缩返回，读出来就是乱码。
async fn send_llm_chat(
    state: &TraeGatewayState,
    account: &PickedTraeAccount,
    body: &[u8],
) -> Result<reqwest::Response, (u16, String)> {
    let url = format!("{}{TRAE_LLM_CHAT_PATH}", state.upstream);
    let trace_id = trace_id();

    let response = state
        .http
        .post(&url)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "*/*")
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::USER_AGENT, TRAE_USER_AGENT)
        .header(header::REFERER, &url)
        .header("x-ide-token", &account.jwt)
        .header("x-app-id", TRAE_APP_ID)
        .header("x-app-version", "default")
        .header("x-app-version-code", TRAE_IDE_VERSION_CODE)
        .header("x-ide-version", TRAE_IDE_VERSION)
        .header("x-ide-version-code", TRAE_IDE_VERSION_CODE)
        .header("x-ide-version-type", "stable")
        .header("x-device-type", "windows")
        .header("x-device-brand", "CREFG-XX")
        .header("x-device-cpu", "Intel")
        .header("x-device-id", &account.device_id)
        .header("x-machine-id", &account.machine_id)
        .header("x-os-version", "Windows 11 Home China")
        .header("request-traffic-type", "prod")
        .header("package-type", "stable_cn")
        .header("x-lgw-req-sdk-type", "3")
        .header("x-lscbd-aid", "787976")
        .header("x-lscbd-platform", "windows")
        .header("x-ss-dp", "787976")
        .header("app-version", TRAE_IDE_VERSION)
        .header("x-custom-trace-id", &trace_id[..16])
        .header(
            "x-flow-traceparent",
            format!("04-{}-{}-01", &trace_id[3..35], uuid_like()),
        )
        .header("x-tt-trace-id", &trace_id)
        .header("x-request-id", format!("req_{}", uuid_like()))
        .body(body.to_vec())
        .send()
        .await
        .map_err(|error| (0u16, describe_transport_error(&error)))?;

    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(response);
    }

    // 错误体可能很长（含堆栈），截断后再回传与落日志。
    let text = response.text().await.unwrap_or_default();
    Err((status, preview(&text, 300)))
}

// ---------------------------------------------------------------------------
// 收尾：写回池 + 落日志
// ---------------------------------------------------------------------------

/// 一次请求的收尾（流式与非流式共用）。
#[allow(clippy::too_many_arguments)]
async fn settle(
    state: &TraeGatewayState,
    account: &PickedTraeAccount,
    model: &str,
    stream: bool,
    started: Instant,
    error: Option<(i64, String)>,
    usage: TokenUsage,
    variant: TraeVariant,
) {
    let latency_ms = started.elapsed().as_millis() as i64;

    let message = match &error {
        Some((code, detail)) => Some(format!(
            "账号「{}」流内错误（code={code}）：{}",
            account.name,
            preview(detail, 200)
        )),
        None => None,
    };

    if let Some((code, detail)) = &error {
        let kind = classify_solo(*code, detail);
        if kind != TraeErrKind::None {
            let reason = message.clone().unwrap_or_default();
            state
                .pools
                .lock()
                .await
                .entry(variant)
                .or_insert_with(|| TraePool::for_variant(variant))
                .apply_error(&account.uid, kind, &reason);
        }
    } else {
        state
            .pools
            .lock()
            .await
            .entry(variant)
            .or_insert_with(|| TraePool::for_variant(variant))
            .note_success(&account.uid);
    }

    // 状态码恒为 200：SSE 已经以 200 开头发出，错误只能体现在事件里。
    state
        .record_request(
            ENDPOINT,
            model,
            200,
            &account.uid,
            latency_ms,
            stream,
            usage.prompt,
            usage.completion,
            message,
            variant,
        )
        .await;
}

/// 流内错误 → 写回冷却 + 构造可重试的失败。
async fn record_stream_error(
    state: &TraeGatewayState,
    account: &PickedTraeAccount,
    code: i64,
    detail: &str,
    variant: TraeVariant,
) -> UpstreamFailure {
    let kind = classify_solo(code, detail);
    let message = format!(
        "账号「{}」流内错误（code={code}）：{}",
        account.name,
        preview(detail, 200)
    );
    if kind != TraeErrKind::None {
        state
            .pools
            .lock()
            .await
            .entry(variant)
            .or_insert_with(|| TraePool::for_variant(variant))
            .apply_error(&account.uid, kind, &message);
    }
    *state.last_error.write().await = Some(message.clone());
    UpstreamFailure {
        status: 502,
        code: kind.as_str().to_string(),
        message,
    }
}

/// 落一条失败请求日志（用于「网关页能看到最近为什么全失败」）。
async fn settle_failure(
    state: &TraeGatewayState,
    model: &str,
    stream: bool,
    started: Instant,
    failure: &UpstreamFailure,
    variant: TraeVariant,
) {
    state
        .record_request(
            ENDPOINT,
            model,
            failure.status,
            "",
            started.elapsed().as_millis() as i64,
            stream,
            0,
            0,
            Some(failure.message.clone()),
            variant,
        )
        .await;
}

/// 池里挑不出账号时的失败：带上**逐账号原因**，且**文案必须点名用户当前所在的区域**。
///
/// 只回一句「没有可用账号」等于把排查成本转嫁给用户；把 `diagnose()` 的结果拼进去，
/// 用户能立刻看出是「全部冷却中」还是「积分都过期了」。
///
/// 按区域取名的原因：Key 的**归属就是区域**（账号库、冷却、积分都按区域分家），
/// 账号管理页的切换器也是「国内版 / 国际版」两个 Tab。文案若按程序位取名
/// （`Trae Work` / `Trae`），用户会被指到界面上**根本不存在**的入口去加账号。
fn no_account_failure_sync(state: &TraeGatewayState, variant: TraeVariant) -> UpstreamFailure {
    // 这里不能 await（调用点在 `unwrap_or_else` 里），用阻塞锁读一次内存视图即可：
    // 池状态在 `attempt_once` 里刚同步过，读到的不会比磁盘旧。
    let diagnose = match state.pools.try_lock() {
        Ok(pools) => pools
            .get(&variant)
            .map(|pool| pool.diagnose(now_secs()))
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    // 选中的池为空时，顺手看一眼**另一个区域**是否有账号：若另一个有而这个没有，
    // 用户多半是「账号加在了另一个区域、Key 却归属这个区域」——直接点出该怎么改。
    //
    // ⚠️ 必须遍历 [`TraeRegion::all`]，**不能**用 `TraeVariant::all()`：后者只有两条
    // **国内**程序位，于是「国内 Key + 国内库为空 + 国际版库有账号」这一情形
    // 永远得不到提示 —— 真正可能有账号的那条线根本不在候选里。提示要双向覆盖，
    // 否则国际版接入后，只在「国际版 Key 配国内账号」这一个方向上给指引。
    let other_region_with_accounts = if diagnose.is_empty() {
        TraeRegion::all()
            .into_iter()
            .filter(|candidate| *candidate != variant.region())
            .find(|candidate| !account::entries_for_region(*candidate).is_empty())
    } else {
        None
    };
    UpstreamFailure {
        status: 503,
        code: "no_healthy_account".to_string(),
        message: no_account_message(variant, &diagnose, other_region_with_accounts),
    }
}

/// 空池文案的唯一构造点（纯函数，便于单测钉住「区域名必须出现」）。
///
/// `other_region_with_accounts` 为「另一个区域有账号」时的提示对象；`None` 表示不提。
///
/// 取名一律走 [`TraeVariant::region`] + [`TraeRegion::display_name`]：
/// `TraeWork` 与 `Trae` **同属国内区域**，因此两者产出的文案**逐字相同**。
fn no_account_message(
    variant: TraeVariant,
    diagnose: &[String],
    other_region_with_accounts: Option<TraeRegion>,
) -> String {
    let name = variant.region().display_name();
    let base = if diagnose.is_empty() {
        format!(
            "「{name}」没有可用账号：请先在「账号管理」切到 {name} 添加账号，\
             或在「一键签到」页查看冷却原因"
        )
    } else {
        format!("「{name}」没有可用账号：{}", diagnose.join("、"))
    };
    match other_region_with_accounts {
        Some(other) => {
            let other_name = other.display_name();
            format!("{base}（检测到 {other_name} 有账号：请改用归属 {other_name} 的 API Key）")
        }
        None => base,
    }
}

/// 出站失败的统一形状。
struct UpstreamFailure {
    status: u16,
    code: String,
    message: String,
}

// ---------------------------------------------------------------------------
// 响应与工具
// ---------------------------------------------------------------------------

/// JSON 响应。
fn json_response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// OpenAI 端点错误响应。
fn openai_error(status: StatusCode, code: &str, message: &str) -> Response {
    json_response(status, sse::error_body(code, message))
}

/// SSE 响应。
fn sse_response(rx: mpsc::Receiver<Result<Bytes, std::io::Error>>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(sse::body_from_receiver(rx))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 32 位 hex，用作 trace / request id 的原料。
fn uuid_like() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// W3C traceparent 形态的上游追踪号：`00-<32hex>-<32hex>-01`（71 字符）。
///
/// 长度是硬约束：`x-custom-trace-id` 取 `[..16]`、`x-flow-traceparent` 取 `[3..35]`，
/// 越界会 panic。所以这里不做「短一点更省」的优化。
fn trace_id() -> String {
    format!("00-{}-{}-01", uuid_like(), uuid_like())
}

/// 截断到 `max` 个**字符**（不是字节），避免把多字节字符切一半。
fn preview(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(max).collect();
    format!("{head}…（共 {} 字符）", trimmed.chars().count())
}

/// 传输层失败的**可读**描述。
///
/// 用户该看到「连接超时」而不是 `error sending request for url (...)`，
/// 但原始错误也不能丢——排查时它是唯一线索。
///
/// 实现在 core 的 `modules::net`，**本 crate 不再自己维护一份**：
/// 两个 crate 各写一遍必然出现「同一个故障两种说法」，而且 core 那份还会
/// 额外展开 `source` 链（DNS / TCP / TLS / 超时的具体原因），排查时才够用。
fn describe_transport_error(error: &reqwest::Error) -> String {
    buddy_switch_core::modules::net::describe_transport_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_id_has_the_length_the_header_slices_require() {
        let trace = trace_id();
        // 71 = "00-" + 32 + "-" + 32 + "-01"
        assert_eq!(trace.len(), 71, "trace id 长度是切片安全的前提");
        assert!(trace.starts_with("00-"));
        assert!(trace.ends_with("-01"));
        // 这两处切片一旦越界就是 panic，必须在这里钉住。
        assert_eq!(&trace[..16].len(), &16);
        assert_eq!(&trace[3..35].len(), &32);
        assert_ne!(trace, trace_id());
    }

    #[test]
    fn uuid_like_is_32_hex_chars() {
        let value = uuid_like();
        assert_eq!(value.len(), 32);
        assert!(value.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn preview_truncates_by_char_not_by_byte() {
        assert_eq!(preview("abc", 10), "abc");
        assert_eq!(preview("  abc  ", 10), "abc");
        // 中文按字符截断：3 个字符不该被腰斩成乱码。
        let text = "一二三四五";
        let cut = preview(text, 3);
        assert!(cut.starts_with("一二三"));
        assert!(cut.contains("共 5 字符"));
        assert!(!cut.contains('\u{FFFD}'));
    }

    #[test]
    fn error_body_is_openai_shaped() {
        let response = openai_error(StatusCode::UNAUTHORIZED, "invalid_request_error", "缺少凭据");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = sse::error_body("no_healthy_account", "没有可用账号");
        assert_eq!(body["error"]["code"], "no_healthy_account");
        assert_eq!(body["error"]["type"], "api_error");
        assert_eq!(body["error"]["message"], "没有可用账号");
    }

    #[test]
    fn no_account_failure_message_is_actionable_and_region_aware() {
        // 这条文案是用户唯一能看到的排障线索：必须包含「下一步做什么」且**点名区域**。
        let cn = no_account_message(TraeVariant::TraeWork, &[], None);
        assert!(cn.contains("账号管理"), "{cn}");
        assert!(cn.contains("一键签到"), "{cn}");
        assert!(cn.contains("国内版"), "文案必须点名选中的区域: {cn}");

        // ⚠️ 契约在 2026-09-21 变了：网关的号池按**区域**分区（`pool.rs::sync_for`
        // 走 `entries_for_region`），Key 的归属也是区域，因此文案按区域取名。
        // `TraeWork` 与 `Trae` 是**程序位**、同属国内区域 ⇒ 两段文案必须**逐字相同**。
        // 若哪天又按程序位取名（用户会在账号管理页找不到那个入口），这条 `assert_eq!` 会红。
        let trae = no_account_message(TraeVariant::Trae, &[], None);
        assert_eq!(cn, trae, "国内两个程序位共用一本账号库，文案不得按程序位分叉");

        // 国际版必须是另一个名字（否则用户分不清该去哪一版加账号）。
        let global = no_account_message(TraeVariant::Global, &[], None);
        assert!(global.contains("国际版"), "{global}");
        assert_ne!(cn, global, "两个区域的文案必须可区分: {cn} / {global}");

        // 有逐账号原因时，原因必须出现在文案里。
        let with_reason = no_account_message(
            TraeVariant::Trae,
            &["主号(7481920:冷却中,积分=0)".to_string()],
            None,
        );
        assert!(with_reason.contains("国内版"), "{with_reason}");
        assert!(with_reason.contains("冷却中"), "{with_reason}");

        // 「另一个区域有账号」的提示必须**双向**都给。
        // 方向①：国际版 Key + 只有国内账号（R8①的原始场景）。
        let hinted_global = no_account_message(TraeVariant::Global, &[], Some(TraeRegion::Cn));
        assert!(
            hinted_global.contains("国际版"),
            "选中的区域必须出现: {hinted_global}"
        );
        assert!(
            hinted_global.contains("国内版"),
            "另一个有账号的区域必须出现（否则用户不知道该往哪加 Key）: {hinted_global}"
        );
        // 方向②：国内版 Key + 只有国际版账号。**这条在修复前做不到** ——
        // 旧实现遍历的是 `TraeVariant::all()`（两条国内程序位），候选里根本没有国际版。
        let hinted_cn = no_account_message(TraeVariant::TraeWork, &[], Some(TraeRegion::Global));
        assert!(hinted_cn.contains("国内版"), "{hinted_cn}");
        assert!(
            hinted_cn.contains("国际版"),
            "另一个有账号的区域必须出现: {hinted_cn}"
        );
    }

    #[test]
    fn status_query_variant_parsing_falls_back_to_default() {
        assert_eq!(parse_variant_query(Some("variant=trae_cn")), TraeVariant::Trae);
        assert_eq!(parse_variant_query(Some("variant=trae_work")), TraeVariant::TraeWork);
        // 缺失 / 未知 → 默认（TraeWork），老客户端行为不变。
        assert_eq!(parse_variant_query(None), TraeVariant::default());
        assert_eq!(parse_variant_query(Some("")), TraeVariant::default());
        assert_eq!(parse_variant_query(Some("variant=unknown")), TraeVariant::default());
        assert_eq!(parse_variant_query(Some("days=7")), TraeVariant::default());
    }
}
