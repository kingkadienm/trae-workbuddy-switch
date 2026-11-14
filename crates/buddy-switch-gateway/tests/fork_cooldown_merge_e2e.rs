//! fork 严格冷却与新版真实路由的集成：不发上游请求、不把限流错报认证失败。
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use buddy_switch_core::modules::{account, region::Region};
use buddy_switch_gateway::{
    pool::{RealmTag, UpstreamEvent},
    router, GatewayConfig, GatewayState,
};
use tower::ServiceExt;

#[tokio::test]
async fn active_account_cooldown_returns_429_without_reusing_credentials() {
    let home = std::env::temp_dir().join(format!("fork-merge-cooldown-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("BUDDY_SWITCH_HOME", &home);
    let accounts = vec![
        serde_json::json!({"uid":"merge-cold", "id":"merge-cold", "access_token":"fixture-token", "domain":"workbuddy.cn"}),
    ];
    account::save_accounts_for(Region::Cn, &accounts).unwrap();
    let state = GatewayState::new(GatewayConfig::default());
    let (_, key) = state.keys.create("merge-test".into(), Region::Cn);
    let now = buddy_switch_gateway::timeutil::now_ms();
    {
        let mut pool = state.pool.write().await;
        pool.sync_accounts(&accounts, Some(RealmTag::Cn));
        pool.apply_upstream_error(
            "merge-cold",
            "deepseek-v4-flash",
            &UpstreamEvent::SoftRate {
                reset_at_ms: Some(now + 60000),
                model_scoped: false,
            },
            now,
        );
    }
    let request = Request::builder().method("POST").uri("/v1/chat/completions")
        .header("content-type","application/json").header("authorization",format!("Bearer {key}"))
        .body(Body::from(r#"{"model":"deepseek-v4-flash","stream":false,"messages":[{"role":"user","content":"fixture"}]}"#)).unwrap();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        router(state).oneshot(request),
    )
    .await
    .expect("冷却判定不得触发上游网络")
    .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(
        !text.contains("no_credential"),
        "限流不能变成认证错误：{text}"
    );
    assert!(text.contains("冷却"), "必须给出可行动原因：{text}");
}
