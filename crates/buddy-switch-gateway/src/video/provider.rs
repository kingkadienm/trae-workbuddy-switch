//! 上游适配层：复用网关 UpstreamClient，不重试视频生成请求。
use std::collections::HashMap;

use buddy_switch_core::modules::region::{region_spec, Region};
use buddy_switch_core::modules::upstream::{UpstreamChatResult, UpstreamErrorKind};
use serde_json::{json, Value};

use super::models::{parse_video_models, VideoModel};
use crate::error::GatewayError;
use crate::state::GatewayState;

pub struct VideoProvider;

pub fn video_headers(account: &Value) -> HashMap<String, String> {
    let mut headers = buddy_switch_core::modules::account::build_auth_headers(account);
    headers.insert("X-Product".into(), "SaaS".into());
    headers
}

pub fn upstream_failure(status: u16, kind: UpstreamErrorKind, message: String) -> GatewayError {
    GatewayError::Upstream {
        kind,
        status,
        message,
        attempts: Vec::new(),
    }
}

pub fn response_body(value: &Value) -> Result<Value, GatewayError> {
    let mut node = value.clone();
    for _ in 0..16 {
        if let Some(code) = node.get("code") {
            let code = code
                .as_i64()
                .or_else(|| code.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| GatewayError::Internal("上游响应的业务码 code 格式错误".into()))?;
            if code != 0 {
                let message = node
                    .get("msg")
                    .or_else(|| node.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("上游业务请求失败");
                return Err(upstream_failure(
                    200,
                    UpstreamErrorKind::Server,
                    format!("上游视频业务失败（code={code}）：{message}"),
                ));
            }
        }
        let next = node.get("body").or_else(|| node.get("data"));
        match next {
            Some(Value::Object(_)) => node = next.unwrap().clone(),
            Some(Value::String(text)) => {
                node = serde_json::from_str(text)
                    .map_err(|_| GatewayError::Internal("上游视频 body 不是合法 JSON".into()))?;
            }
            _ if node.is_object() => return Ok(node),
            _ => {
                return Err(GatewayError::Internal(
                    "上游视频响应必须是 JSON 对象".into(),
                ))
            }
        }
    }
    Err(GatewayError::Internal("上游视频响应嵌套层级过深".into()))
}

pub async fn read_response(result: UpstreamChatResult) -> Result<Value, GatewayError> {
    match result {
        UpstreamChatResult::Ok(ok) => {
            let status = ok.response.status().as_u16();
            let value: Value = ok.response.json().await.map_err(|error| {
                upstream_failure(
                    status,
                    UpstreamErrorKind::Server,
                    format!("上游视频响应不是合法 JSON：{error}"),
                )
            })?;
            response_body(&value)?;
            Ok(value)
        }
        UpstreamChatResult::Err {
            status,
            kind,
            message,
        } => Err(upstream_failure(status, kind, message)),
    }
}

impl VideoProvider {
    pub async fn submit(
        state: &GatewayState,
        region: Region,
        account: &Value,
        body: &Value,
    ) -> UpstreamChatResult {
        let url = format!("{}/v2/videos/generations", region_spec(region).chat_base);
        state
            .upstream
            .forward_with(
                &url,
                video_headers(account),
                body.to_string(),
                None,
                &GatewayState::video_user_agent(region),
            )
            .await
    }

    pub async fn query(
        state: &GatewayState,
        region: Region,
        account: &Value,
        upstream_task_id: &str,
    ) -> UpstreamChatResult {
        let url = format!("{}/v2/videos/tasks", region_spec(region).chat_base);
        state
            .upstream
            .forward_with(
                &url,
                video_headers(account),
                json!({ "task_id": upstream_task_id }).to_string(),
                None,
                &GatewayState::video_user_agent(region),
            )
            .await
    }

    pub async fn fetch_video_models(
        state: &GatewayState,
        region: Region,
        account: &Value,
    ) -> Result<Vec<VideoModel>, GatewayError> {
        let url = format!("{}/v3/config", region_spec(region).chat_base);
        let result = state
            .upstream
            .forward_with_method(
                &url,
                video_headers(account),
                String::new(),
                None,
                &GatewayState::video_user_agent(region),
                reqwest::Method::GET,
            )
            .await;
        let value = read_response(result).await?;
        parse_video_models(&value, region.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_envelopes_and_business_codes_are_checked() {
        assert_eq!(
            response_body(&json!({"code":0,"data":{"body":{"id":"task"}}})).unwrap()["id"],
            "task"
        );
        assert_eq!(
            response_body(&json!({"body":"{\"code\":0,\"data\":{\"id\":\"task\"}}"})).unwrap()
                ["id"],
            "task"
        );
        for v in [
            json!({"code":1,"data":{"id":"bad"}}),
            json!({"code":0,"data":{"body":{"code":9,"id":"bad"}}}),
            json!({"code":false}),
            json!({"body":"invalid"}),
        ] {
            assert!(response_body(&v).is_err());
        }
    }

    #[test]
    fn headers_use_saas_without_refresh_token() {
        let headers = video_headers(
            &json!({"access_token":"fixture", "refresh_token":"never-send", "uid":"u1"}),
        );
        assert_eq!(headers["X-Product"], "SaaS");
        assert!(!headers
            .keys()
            .any(|k| k.to_ascii_lowercase().contains("refresh")));
    }
}
