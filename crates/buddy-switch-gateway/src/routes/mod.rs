//! 路由组装、鉴权与错误响应辅助。

pub mod biz;
pub mod chat;
pub mod cross;
/// 模型降级链的 hop 循环骨架（Trae / Qoder 两条自有中复用；WB 走 `relay` 自己的）。
pub mod fallback;
pub mod health;
pub mod messages;
pub mod models;
pub mod pool_admin;
pub mod relay;
pub mod responses;
pub mod status;

use axum::body::Bytes;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::apikey::ApiKeyRecord;
use crate::error::GatewayError;
use crate::state::GatewayState;

/// 从请求头提取 API Key：优先 `x-api-key`，其次 `Authorization: Bearer <key>`。
pub(crate) fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
    {
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw)
        .trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// 校验 API Key 并返回记录（Key 绑定 region，不读客户端声明）。
pub(crate) fn authenticate(
    state: &GatewayState,
    headers: &HeaderMap,
) -> Result<ApiKeyRecord, GatewayError> {
    let presented = extract_bearer(headers).ok_or_else(|| {
        GatewayError::Unauthorized(
            "缺少凭据：请在 Authorization 头携带 `Bearer <API Key>`".to_string(),
        )
    })?;
    let record = state
        .keys
        .verify(&presented)
        .ok_or_else(|| GatewayError::Unauthorized("API Key 无效或已吊销".to_string()))?;
    state.keys.touch(&record.id);
    Ok(record)
}

/// JSON 响应。
pub(crate) fn json_response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// 错误响应：状态码 + 错误体 + 可选 `Retry-After`。
///
/// `Retry-After` 只在**有确定退避依据**时给（见 [`GatewayError::retry_after_secs`]）——
/// 目前是 `pool_busy`（503）。不写这个头，客户端只能靠猜：常见结果是立刻重试，
/// 把刚空出来的在途额度又打满，于是「明明有号、却一直 503」。
pub(crate) fn error_response(error: &GatewayError, body: Value) -> Response {
    let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut response = json_response(status, body);
    if let Some(secs) = error.retry_after_secs() {
        if let Ok(value) = header::HeaderValue::from_str(&secs.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
    }
    response
}

/// OpenAI 端点错误响应。
pub(crate) fn openai_error(error: GatewayError) -> Response {
    // 先取体再传引用：`openai_body` 借 `&self`，`error_response` 也要读 `status/retry_after`。
    let body = error.openai_body();
    error_response(&error, body)
}

/// Anthropic 端点错误响应。
pub(crate) fn anthropic_error(error: GatewayError) -> Response {
    let body = error.anthropic_body();
    error_response(&error, body)
}

/// 把降级轨迹挂到错误体**顶层**（`{"error":…, "fallback":…}`）。
///
/// ## 为什么不塞进 `GatewayError`
///
/// 那个类型每个变体的状态码与 `type` 都被契约测试钉着（客户端靠 `type` 决定
/// 重试与否）。为一个**可选诊断字段**动它，风险远大于收益。
/// 顶层多一个键是纯增量：老客户端忽略它，新客户端能编程判断。
///
/// ## 为什么不能只追加到 message 文本
///
/// 追加文本只对 `Upstream` 变体可行 —— 而「降级试过但最后一跳连账号都没选出来」
/// 时，最终错误是 `NoCredential`（没有可变 message 字段）。那正是最需要轨迹的时候。
fn attach_fallback(body: &mut Value, fallback: Option<Value>) {
    let Some(info) = fallback else {
        return;
    };
    if let Some(object) = body.as_object_mut() {
        object.insert("fallback".to_string(), info);
    }
}

/// OpenAI 端点错误响应 + 降级轨迹。
pub(crate) fn openai_error_with_fallback(error: GatewayError, fallback: Option<Value>) -> Response {
    let mut body = error.openai_body();
    attach_fallback(&mut body, fallback);
    error_response(&error, body)
}

/// Anthropic 端点错误响应 + 降级轨迹。
pub(crate) fn anthropic_error_with_fallback(
    error: GatewayError,
    fallback: Option<Value>,
) -> Response {
    let mut body = error.anthropic_body();
    attach_fallback(&mut body, fallback);
    error_response(&error, body)
}

/// 解析请求体为 JSON（空体视为空对象）。
pub(crate) fn parse_body(bytes: &Bytes) -> Result<Value, GatewayError> {
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(bytes)
        .map_err(|error| GatewayError::BadRequest(format!("请求体不是合法 JSON: {error}")))
}

/// 构造 SSE 响应（用于流式透传/转换）。
pub(crate) fn sse_response<S>(stream: S) -> Response
where
    S: futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(axum::body::Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buddy_switch_core::modules::region::Region;
    use buddy_switch_core::modules::upstream::UpstreamErrorKind;

    use crate::error::NoCredentialCause;

    /// `pool_busy` ⇒ 503 **且**带 `Retry-After`；认证类与限流类**不得**带该头。
    ///
    /// 阳性对照：删掉 `error_response` 里的头注入 ⇒ 第一条断言必红；
    /// 反过来「给所有错误都加头」⇒ 后两条断言红（编一个假的退避时长比不给更糟）。
    #[test]
    fn pool_busy_carries_retry_after_and_others_do_not() {
        let busy = openai_error(GatewayError::PoolBusy {
            region: Region::Cn,
            total: 2,
        });
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            busy.headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("2"),
            "503 必须告诉客户端等多久，否则它会立刻重试，把刚空出来的额度又打满"
        );

        let no_credential = anthropic_error(GatewayError::NoCredential {
            region: Region::Cn,
            cause: NoCredentialCause::StoreEmpty,
        });
        assert_eq!(no_credential.status(), StatusCode::UNAUTHORIZED);
        assert!(
            no_credential.headers().get(header::RETRY_AFTER).is_none(),
            "认证类错误没有「等一会儿就好」这回事，带 Retry-After 只会拖延用户去登录"
        );

        // 限流 / 余额不足的真实解冻时刻在上游响应里，本仓拿不到可信值 ⇒ 刻意不给头。
        let rate_limited = openai_error(GatewayError::Upstream {
            kind: UpstreamErrorKind::SoftRate,
            status: 429,
            message: "429 rate limit".to_string(),
            attempts: Vec::new(),
        });
        assert_eq!(rate_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            rate_limited.headers().get(header::RETRY_AFTER).is_none(),
            "不臆造退避时长：宁可让客户端自己指数退避"
        );
    }
}
