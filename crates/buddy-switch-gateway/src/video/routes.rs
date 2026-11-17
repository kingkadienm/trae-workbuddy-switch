//! Video Gateway 的三条 HTTP 路由。
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::Value;

use crate::error::GatewayError;
use crate::routes::{json_response, openai_error, parse_body};
use crate::state::GatewayState;

use crate::video::service::{query_video, submit_video, VideoSubmitInput};
use crate::video::store::VideoTaskStore;

/// 任务表句柄（与 `GatewayState::video_tasks` 同一个实例）。
pub fn shared_store(state: &GatewayState) -> std::sync::Arc<VideoTaskStore> {
    state.video_tasks.clone()
}

/// `GET /v1/video/models`：按 Key 绑定的 region 返回视频模型清单。
pub async fn video_models(State(state): State<GatewayState>, headers: HeaderMap) -> Response {
    let record = match crate::routes::authenticate(&state, &headers) {
        Ok(record) => record,
        Err(error) => return openai_error(error),
    };
    match crate::video::service::video_models(&state, &record).await {
        Ok(data) => json_response(
            StatusCode::OK,
            serde_json::json!({ "object": "list", "data": data }),
        ),
        Err(error) => openai_error(error),
    }
}

/// `POST /v1/videos`：提交一次视频生成任务；region 由 Key 决定（忽略调用方声明）。
pub async fn submit_video_handler(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let record = match crate::routes::authenticate(&state, &headers) {
        Ok(record) => record,
        Err(error) => return openai_error(error),
    };
    let parsed = match parse_body(&body) {
        Ok(value) => value,
        Err(error) => return openai_error(error),
    };
    let input = match VideoSubmitInput::parse(&parsed) {
        Ok(input) => input,
        Err(error) => return openai_error(error),
    };
    let store = shared_store(&state);
    match submit_video(&state, &store, &record, &input).await {
        Ok((_id, payload)) => json_response(StatusCode::OK, payload),
        Err(error) => openai_error(error),
    }
}

/// `GET /v1/videos/:id`：按本地任务 id 查询进度（固定用提交时的同一账号）。
pub async fn get_video_handler(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let record = match crate::routes::authenticate(&state, &headers) {
        Ok(record) => record,
        Err(error) => return openai_error(error),
    };
    let store = shared_store(&state);
    match query_video(&state, &store, &record, &id).await {
        Ok(Some(payload)) => json_response(StatusCode::OK, payload),
        Ok(None) => openai_error(GatewayError::BadRequest(format!(
            "视频任务 `{id}` 不存在（进程内任务表，进程重启即清空）"
        ))),
        Err(error) => openai_error(error),
    }
}

/// 供单测用：把任务状态收敛成与 `VideoTask::to_value` 同形的 JSON。
pub fn status_payload(task: &crate::video::store::VideoTask) -> Value {
    task.to_value()
}
