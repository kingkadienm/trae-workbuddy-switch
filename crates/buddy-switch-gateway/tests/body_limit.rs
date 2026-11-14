// 测试串行锁（`guard()` / `ENV_LOCK`）会被**故意**持有到用例结束：它防的是多个用例
// 同时改同一个进程级状态（环境变量 / 全局配置）。`#[tokio::test]` 默认 current-thread，
// 不存在「持锁跨线程唤醒」的风险；clippy::await_holding_lock 在这里是误报。
#![allow(clippy::await_holding_lock)]
//! 路由级测试：请求体上限必须**真的生效**，而不只是配置里存了个数字。
//!
//! # 为什么必须有这个测试
//!
//! axum 的 `DefaultBodyLimit` **默认只有 2MB**，而我们把上限改成了可配置。如果 `router()`
//! 忘了挂这个 layer，配置就是个**装饰品**——构建全绿、类型正确、行为不变。这类
//! 「零护栏接线」靠读代码或单测数字推导都发现不了，只能靠**端到端断言**。
//!
//! # 环境隔离
//!
//! `GatewayState::new` 会读 `~/.buddy-switch/` 下的网关配置与 Key 库，因此本测试先把
//! `BUDDY_SWITCH_HOME` 指向一个**已存在**的临时目录（该变量要求目录存在，否则会被忽略并
//! 回落真实 home）。集成测试是独立进程，故设置进程级 env 不会干扰 lib 单测；同进程内
//! 用一把互斥锁把两个用例串行化（env 是进程级全局状态）。

use std::sync::Mutex;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use buddy_switch_gateway::{body_limit_bytes, router, GatewayConfig, GatewayState};

/// 串行化所有触碰 `BUDDY_SWITCH_HOME` 的用例。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 把 `BUDDY_SWITCH_HOME` 指向一个已存在的临时目录，返回该目录。
fn isolated_home(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wb-gw-body-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("创建隔离目录");
    std::env::set_var("BUDDY_SWITCH_HOME", &dir);
    dir
}

fn chat_request(payload: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .expect("构造请求")
}

#[tokio::test]
async fn oversized_body_is_rejected_with_413() {
    // 容忍「锁中毒」：某个用例断言失败并持锁 panic 后，锁会被标记为 poisoned。
    // 若这里直接 `expect`，其余用例会连带在取锁处失败，掩盖它们各自的真实结果 ——
    // 变成"一处失败、满屏失败"，排查时反而看不清真正的缺陷。
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let home = isolated_home("over");

    // 上限 1MB
    let state = GatewayState::new(GatewayConfig {
        max_body_mb: 1,
        ..GatewayConfig::default()
    });
    let app = router(state);

    // 载荷约 1.5MB —— **刻意落在「超过配置的 1MB 上限」与「低于 axum 默认的 2MB」之间**。
    // 若载荷取 2MB 以上，即使 `router()` 忘了挂 layer，axum 的默认上限也会返回 413，
    // 测试就变成空洞断言、测不出接线缺陷。
    let payload = format!(
        r#"{{"model":"m","messages":[{{"role":"user","content":"{}"}}]}}"#,
        "a".repeat(1_500_000)
    );
    let response = app.oneshot(chat_request(payload)).await.expect("路由调用");

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "超出**配置**上限（1MB）的请求体必须被 413 拒绝。若这里是 401，说明 \
         `router()` 没把 DefaultBodyLimit 挂上（配置成了装饰品），或鉴权先于体积检查生效"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[tokio::test]
async fn body_within_limit_reaches_the_handler() {
    // 容忍「锁中毒」：某个用例断言失败并持锁 panic 后，锁会被标记为 poisoned。
    // 若这里直接 `expect`，其余用例会连带在取锁处失败，掩盖它们各自的真实结果 ——
    // 变成"一处失败、满屏失败"，排查时反而看不清真正的缺陷。
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let home = isolated_home("within");

    let state = GatewayState::new(GatewayConfig {
        max_body_mb: 8,
        ..GatewayConfig::default()
    });
    let app = router(state);

    // 小载荷应穿过体积检查并落到 handler（此处因缺 API Key 返回 401）——
    // 这证明路由是通的，且 413 的成因确实是体积而不是路由/鉴权配置错误。
    let response = app
        .oneshot(chat_request(r#"{"model":"m","messages":[]}"#.to_string()))
        .await
        .expect("路由调用");

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "体积合规的请求必须能到达 handler（断言 401 而非 413）"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn body_limit_derivation_covers_zero_and_extremes() {
    // 8MB 默认
    assert_eq!(
        body_limit_bytes(0),
        8 * 1024 * 1024,
        "0 视为未设置 → 默认 8MB"
    );
    assert_eq!(body_limit_bytes(1), 1024 * 1024);
    assert_eq!(body_limit_bytes(8), 8 * 1024 * 1024);
    // 极大值不得溢出（saturating）
    assert_eq!(body_limit_bytes(usize::MAX), usize::MAX);
}
