//! `POST /biz/*path`：**业务请求转发**（P12 T03 · 本地路由）。
//!
//! ⚠️ 路径语法为 **axum 0.7**的 `*path`（**不是** 0.8 的 `{*path}`）—— 写错会让
//! `router()` 在构建期 panic，网关完全起不来且编译全绿。
//!
//! ## 为什么需要它
//!
//! 「本地路由」下客户端把 base URL 指向本机网关（**明文 http**，**不需要 CA**），
//! 于是网关会收到本该发给上游的**业务请求**（签到 / 积分 / 旅行）。
//! 本路由把**白名单内**的路径转发到上游，**复用 [`crate::routes::relay`] 的治理层**
//! （选号 / 在途租约 / 冷却 / 熔断 / 换号重试）—— **不另写一份轮换逻辑**
//! （`relay` 是「把请求送到上游」的唯一实现，见其模块文档）。
//!
//! ## ★ 硬约束
//!
//! - **白名单外一律 404**（[`buddy_switch_core::modules::biz_allowlist::is_allowed_path`]，**fail-closed**）——
//!   **不静默、不猜测、不「顺便也代理」**；
//! - **★ 绝不劫持**（D29）：本路由是**客户端自愿指向**的本地服务，**无 hosts / DNS / 透明代理写路径**；
//! - **body 原样透传**（业务请求不是 LLM 协议，**不解析、不改写**）；**不落盘正文**。
//!
//! ## ⚠️ 与 LLM 路由的差异（**只有三处**）
//!
//! | | LLM（`chat` / `messages`） | 业务（本路由） |
//! |:--|:---|:---|
//! | 路径 | 固定 `/v2/chat/completions` | **由 URL 的 `{path}` 决定**（已在白名单） |
//! | 头 | `build_chat_headers` | `build_auth_headers` |
//! | body | 走 `chat_body_for` 改写 | **原样透传** |
//!
//! ⇒ 其余（选号 / 换号 / 熔断 / 账本）**完全共用** —— 这正是 `RelayKind` 的用途。

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};

use buddy_switch_core::modules::biz_allowlist::is_allowed_path;
use buddy_switch_core::modules::region::Region;

use crate::routes::relay::{self, BizMethod, RelayKind, RelayOutcome, RelayRequest};
use crate::routes::{authenticate, error_response, json_response, openai_error};
use crate::session_headers;
use crate::state::GatewayState;

/// 业务转发处理器。
pub async fn handler(
    State(state): State<GatewayState>,
    Path(path): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // ① 鉴权（与 LLM 路由**同一条**：Key 绑定 region，不读客户端声明）。
    let record = match authenticate(&state, &headers) {
        Ok(record) => record,
        Err(error) => return openai_error(error),
    };
    let Some(biz_method) = biz_method_of(&method) else {
        return json_response(
            StatusCode::METHOD_NOT_ALLOWED,
            serde_json::json!({
                "error": "业务转发只接受 GET / POST",
                "method": method.as_str(),
            }),
        );
    };
    // 归属域由 Key 决定 ⇒ 转成显式参数交给共用实现（统一网关从**统一 Key 的
    // `biz_namespace`** 取，见 `crate::unified::routes::biz`）。
    // ★ issue #76：收敛成 `KeyActor`（业务代理是「请求数」计量的主场景，没有 token 可计）。
    //   必须在 `state` 被 move 之前构造。
    let actor = std::sync::Arc::new(crate::apikey::KeyActor::new(
        record.id.clone(),
        record.policy(),
        state.key_usage.clone(),
        "wb",
    ));
    biz_for_region(state, record.region, path, body, biz_method, Some(actor)).await
}

/// 客户端方法 → 出站方法（**只允许 GET / POST**，其余明确 405）。
///
/// ⚠️ 这两种都要支持，不能只留 POST：白名单是**按前缀**收的，而同前缀下
/// `/activity/growth/streak`、`/heatmap` **只有 GET** —— 一律 POST 出去会被上游
/// 回 `404 page not found`（真实环境实测）。
pub fn biz_method_of(method: &Method) -> Option<BizMethod> {
    match *method {
        Method::GET => Some(BizMethod::Get),
        Method::POST => Some(BizMethod::Post),
        _ => None,
    }
}

/// 一趟业务转发（**认证之后的全部逻辑**）—— 产品线入口与统一网关**共用**这一份实现。
///
/// `state` **按值**传入（`Arc` 句柄，克隆极廉价）⇒ 函数体内既有的 `&state` 写法一行都不用改。
/// 不接收 `headers`：认证之后一律用网关自己的账号凭据重建出站头（见 `build_auth_headers`）。
pub async fn biz_for_region(
    state: GatewayState,
    region: Region,
    path: String,
    body: Bytes,
    method: BizMethod,
    // ★ issue #76：调用方的 `KeyActor`（`None` = 无 Key 的本地/测试调用 ⇒ 不判配额、不记账）。
    key: Option<std::sync::Arc<crate::apikey::KeyActor>>,
) -> Response {
    /* ★ issue #76：**业务转发也要判配额**（请求数口径）。
    这条路径没有上游 usage（业务代理的响应不是 LLM 流）⇒ token 口径对它天然无意义；
    但「这把 Key 被用了多少次」是事实，超限就该拒 —— 否则业务转发是配额的**绕过口**。 */
    if let Some(actor) = key.as_deref() {
        if let Some(message) =
            actor.check_quota_and_count(buddy_switch_core::modules::config::now_ms())
        {
            return json_response(
                StatusCode::TOO_MANY_REQUESTS,
                serde_json::json!({
                    "error": "key_quota_exceeded",
                    "message": message,
                }),
            );
        }
    }
    // ★ E1：计一次「今日代理请求」。放在**鉴权之后、白名单校验之前** ——
    //   KPI 问的是「今天有多少请求走到本机网关」，含被白名单拒绝的（那是真实流量）。
    //   ⚠️ 这是**业务转发**路径；LLM 路由（chat / messages）另有各自行 为，不在此计。
    state.proxy_kpi.record_request();

    // ② ★ 白名单校验 —— **fail-closed**：不在册一律 404（明确拒绝，不静默）。
    let full_path = format!("/{path}");
    if !is_allowed_path(&full_path) {
        return json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({
                "error": "路径不在业务白名单内",
                "path": full_path,
            }),
        );
    }

    // ③ body **原样透传**（空体合法：签到等就是 POST 空体）。
    let prepared_body = String::from_utf8_lossy(&body).to_string();

    // ④ 走 `relay`（**复用治理层**），`kind = Biz`。
    let outcome = relay::relay(
        &state,
        RelayRequest {
            openai_shape: false,
            region,
            kind: RelayKind::Biz {
                path: full_path.clone(),
                method,
            },
            // 业务请求没有「模型」概念：传空串 ⇒ 冷却/熔断按「无模型」维度记，**不与 LLM 混淆**。
            model: String::new(),
            prepared_body,
            // 业务请求不走出站改写管线（body 原样透传）⇒ 脱敏位恒 false，
            // 也就永远不会被降级门误判成「内容前缀拦截」。
            sanitized: false,
            conversation_request_id: session_headers::new_hex_id(),
            conversation_id: None,
            trace_id: None,
            key_policy: key.as_ref().map(|actor| actor.policy.clone()),
            key_id: key.as_ref().map(|actor| actor.key_id.clone()),
        },
    )
    .await;

    match outcome {
        // 跨作用域降级：目标产品线已经产出了成型响应（SSE / JSON / 错误体都已封装），
        // 直接返回，不再做协议转换或聚合。
        // 业务请求（签到 / 积分 / 旅行）没有模型概念，不会产生跨作用域降级 ——
        // 这个分支只为穷尽 match。
        Ok(RelayOutcome::Ready { response, .. }) => return response,
        Ok(RelayOutcome::Ok { response, .. }) => {
            // ⑤ **透传响应**（状态码 + 正文），**不做协议转换**。
            let status = response.status();
            let bytes = response.bytes().await.unwrap_or_default();
            Response::builder()
                .status(status)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        // ★ 走 `error_response` 而不是自己拼 `json_response`：否则这条路径会漏掉
        //   `Retry-After`（`pool_busy` 的唯一用途就是告诉客户端「等 2 秒再试」）。
        Err(failure) => error_response(
            &failure.error,
            serde_json::json!({
                "error": failure.error.openai_body(),
                "upstream": failure.upstream_message,
            }),
        ),
    }
}
