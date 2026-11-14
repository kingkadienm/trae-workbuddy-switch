//! `GET /health`（兼容别名 `/healthz`）：免鉴权，返回可用性摘要（P1-6）。

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use buddy_switch_core::modules::{account, auth_file, region::Region};

use crate::routes::json_response;
use crate::state::GatewayState;

/// 健康检查处理器。
pub async fn handler(State(state): State<GatewayState>) -> Response {
    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "regions": {
                "cn": region_available(Region::Cn),
                "global": region_available(Region::Global),
            },
            "version": env!("CARGO_PKG_VERSION"),
            "startedAt": state.started_at,
        }),
    )
}

/// 某 region 是否具备可用账号（认证文件或账号库非空）。
fn region_available(region: Region) -> bool {
    auth_file::read_auth_file_for(region).is_some()
        || !account::load_accounts_for(region).is_empty()
}
