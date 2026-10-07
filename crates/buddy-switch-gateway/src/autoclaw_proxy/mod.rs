//! AutoClaw 反向代理：将请求原样转发到 autoclaw2api 服务。
//!
//! 对照参考实现 `internal/autoclaw/client.go` 的 `Proxy()` 方法：
//! 用 reqwest 做 SSE 透传 + 鉴权注入。
//!
//! 以 `tower::Service` 形式实现，挂载为 `Router::fallback_service`。

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, Uri};
use axum::response::Response;
use http_body_util::BodyExt;
use tower::Service;

use crate::autoclaw::AutoclawClient;

/// AutoClaw 反向代理 Service。
///
/// 实现 `tower::Service` 以支持通过 `Router::fallback_service` 挂载。
#[derive(Clone)]
pub struct AutoclawProxyService {
    client: AutoclawClient,
}

impl AutoclawProxyService {
    /// 构造代理服务。
    pub fn new(client: AutoclawClient) -> Self {
        Self { client }
    }
}

impl Service<Request<Body>> for AutoclawProxyService {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<
        Box<
            dyn std::future::Future<Output = Result<Self::Response, Self::Error>>
                + Send
                + 'static,
        >,
    >;

    fn poll_ready(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let client = self.client.clone();

        Box::pin(async move {
            // 提取原始路径和查询字符串
            let req_uri = req.uri().clone();
            let path = req_uri.path();
            let query = req_uri.query();
            let path_and_query = if let Some(q) = query {
                format!("{}?{}", path, q)
            } else {
                path.to_string()
            };

            // 构建目标 URL：将 autoclaw 基址与原始路径拼接
            let target_url_str = client.base_url.clone() + &path_and_query;
            let target_url = match target_url_str.parse::<Uri>() {
                Ok(url) => url,
                Err(_) => {
                    return Ok(client.error_response("invalid target URL"));
                }
            };

            // 读取请求体
            let (parts, body) = req.into_parts();
            let body_bytes = match body.collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(_) => {
                    return Ok(client.error_response("failed to read request body"));
                }
            };

            // 构建转发请求
            let mut forward_req = Request::builder()
                .method(parts.method)
                .uri(target_url)
                .version(parts.version);

            // 复制请求头
            for (key, value) in parts.headers.iter() {
                if !key.eq("host") && !key.eq("connection") {
                    forward_req = forward_req.header(key, value);
                }
            }

            // 注入鉴权头
            if !client.api_key.is_empty() {
                forward_req = forward_req.header(
                    "Authorization",
                    format!("Bearer {}", client.api_key),
                );
            }

            let forward_req = match forward_req.body(Body::from(body_bytes)) {
                Ok(req) => req,
                Err(_) => {
                    return Ok(client.error_response("failed to build forward request"));
                }
            };

            // 提取请求方法和 URI（reqwest::Client::request 需要独立参数）
            let req_method = forward_req.method().clone();
            let req_uri_str = forward_req.uri().to_string();

            // 发送请求并透传响应
            match client.http.request(req_method, req_uri_str).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let mut builder = Response::builder().status(status);

                    // 复制响应头
                    for (key, value) in resp.headers().iter() {
                        if !key.eq("connection") && !key.eq("transfer-encoding") {
                            builder = builder.header(key, value);
                        }
                    }

                    let body = Body::from_stream(resp.bytes_stream());
                    match builder.body(body) {
                        Ok(response) => Ok(response),
                        Err(_) => Ok(client.error_response("failed to build response")),
                    }
                }
                Err(_) => Ok(client.error_response("upstream unreachable")),
            }
        })
    }
}
