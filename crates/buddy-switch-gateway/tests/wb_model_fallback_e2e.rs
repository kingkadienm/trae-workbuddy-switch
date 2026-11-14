//! WorkBuddy **路由级**端到端：模型降级链真的会被走到吗？
//!
//! # 为什么必须有这一条
//!
//! `model_fallback.rs` 的 10 个单测全部是**纯函数**（`plan_for` / `trigger_for` / `normalize`）。
//! 它们证明不了「中继真的按这个计划跳了」—— 而这正是最容易写错的地方：
//!
//! - 探针票有没有按 hop 归还（漏还 ⇒ 主模型半开态卡 120s，日志无错）；
//! - 闸门暂停时是**零成本跳到下一跳**，还是照旧把 503 抛给客户端；
//! - 不可降级的失败（无可用账号）会不会被误当成「该降级」而白跑一跳。
//!
//! 单测一个都挡不住，只有把真实路由跑起来才看得见。
//!
//! # 为什么不需要 mock 上游
//!
//! 两条用例都让**主模型在发请求之前**就失败（闸门暂停 / 池里没有账号），
//! 所以一次上游请求都不会发出去。我们断言的是**决策与轨迹**（错误消息里有没有
//! 点名备选模型），不是上游的字节。
//!
//! # 环境隔离
//!
//! 与 `wb_model_gate_pause_e2e.rs` 同套路：`BUDDY_SWITCH_HOME` 指向临时目录，
//! 同进程内用一把互斥锁串行化（env 是进程级全局状态）。
//! ⚠️ `model_fallback::shared()` 是**进程级单例**，所以每条用例开头必须显式重置配置，
//! 否则上一条的链会漏到下一条。

use std::sync::Mutex;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use buddy_switch_core::modules::product::Scope;
use buddy_switch_core::modules::region::Region;
use buddy_switch_gateway::model_fallback::{self, FallbackChain, ScopeFallback};
use buddy_switch_gateway::{router, GatewayConfig, GatewayState};

/// 串行化所有触碰 `BUDDY_SWITCH_HOME` 与 `model_fallback::shared()` 的用例。
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn isolated_home(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wb-gw-fallback-e2e-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建隔离目录");
    std::env::set_var("BUDDY_SWITCH_HOME", &dir);
    dir
}

fn chat_request(key: &str, model: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {key}"))
        .body(Body::from(
            serde_json::json!({
                "model": model,
                "stream": false,
                "messages": [{ "role": "user", "content": "hi" }],
            })
            .to_string(),
        ))
        .expect("构造请求")
}

/// 把进程级降级配置重置成「只有给定链」。
async fn set_chains(chains: Vec<(&str, Vec<&str>)>) {
    let store = model_fallback::shared();
    let mut guard = store.write().await;
    let mut scoped = ScopeFallback {
        enabled: true,
        ..Default::default()
    };
    for (from, targets) in chains {
        scoped.chains.insert(
            from.to_string(),
            FallbackChain {
                targets: targets
                    .into_iter()
                    .map(buddy_switch_gateway::model_fallback::FallbackTarget::same_scope)
                    .collect(),
                // 空 `triggers` = 基线（闸门 / 6004 / 11102），正是本用例要的。
                triggers: Default::default(),
                enabled: true,
            },
        );
    }
    guard.set_scope(Scope::WbCn, scoped);
    guard.save().expect("降级配置必须能落盘");
}

/// 压一个模型进闸门暂停（默认 `gate_threshold = 2` ⇒ 两个不同账号各失败一次）。
async fn pause_gate(state: &GatewayState, model: &str) {
    let now_ms = buddy_switch_gateway::timeutil::now_ms();
    let mut pool = state.pool.write().await;
    assert!(
        !pool.note_model_failure(model, "u1", now_ms),
        "第 1 个账号不该开闸"
    );
    assert!(
        pool.note_model_failure(model, "u2", now_ms),
        "第 2 个账号必须开闸"
    );
}

/// ★★ 主模型闸门暂停 + 配了链 ⇒ **必须跳到备选**，而不是把 503 抛给客户端。
///
/// 池里没有任何账号，所以备选也会失败 —— 但那正是要看的东西：
/// 错误消息里必须**点名备选模型**（证明真的跳了），并说明「降级未成功」。
///
/// 变异验证：
/// - 把 `relay_hops` 里的 `continue` 改成 `break`（不跳下一跳）⇒ 错误里不会有备选名，本用例红；
/// - 把 `trigger_for` 里 `ModelGatePaused` 的 `Gate` 分支删掉 ⇒ 同上，红；
/// - 把 `allowed && plan.len() > hop + 1` 的 `allowed` 恒设 false ⇒ 同上，红。
#[tokio::test]
async fn gate_paused_primary_falls_back_to_next_hop() {
    let _guard = guard();
    let _home = isolated_home("fallback");
    set_chains(vec![("glm-5.3", vec!["deepseek-v3"])]).await;

    let state = GatewayState::new(GatewayConfig::default());
    let (_record, key) = state.keys.create("e2e-fb".into(), Region::Cn);
    let app = router(state.clone());

    pause_gate(&state, "glm-5.3").await;

    let response = app
        .oneshot(chat_request(&key, "glm-5.3"))
        .await
        .expect("请求应完成");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("读取响应体");
    let text = String::from_utf8_lossy(&bytes).to_string();

    assert_ne!(status, StatusCode::OK, "池里没有账号，不可能成功：{text}");

    // ★ 轨迹是**结构化字段**（`fallback`），不是拼进 message 的文本 ——
    //   因为最后一跳的错误是 `no_credential`（没有可变 message），
    //   而「试过备选但连账号都没选出来」正是最需要轨迹的时候。
    let value: serde_json::Value = serde_json::from_str(&text).expect("错误体应是 JSON");
    let fallback = value
        .get("fallback")
        .expect("降级失败必须带 `fallback` 轨迹（否则用户以为网关没配好）");
    assert_eq!(fallback["origin"], "glm-5.3", "轨迹要写清从谁降下来");
    let tried: Vec<&str> = fallback["tried"]
        .as_array()
        .expect("`tried` 必须是数组")
        .iter()
        .filter_map(|item| item.as_str())
        .collect();
    assert!(
        tried.contains(&"deepseek-v3"),
        "降级必须真的跳到备选（轨迹里要点名它）：{tried:?}"
    );

    // 事件流里也要有这一笔，且 `result` 是 failed（试过备选）而不是 blocked。
    let events = model_fallback::recent_events();
    let latest = events.first().expect("必须记一条降级事件");
    assert_eq!(latest.from, "glm-5.3");
    assert!(
        latest.path.contains(&"deepseek-v3".to_string()),
        "事件路径必须包含被尝试的备选：{:?}",
        latest.path
    );
    assert_eq!(
        latest.result, "failed",
        "试过备选但没成 ⇒ failed（blocked 表示「按规则一次都没降级」）"
    );
}

/// ★ 不可降级的失败（池里没有账号）**不许**白跑一跳。
///
/// `pool_unavailable_error` 不是 `Trigger` 里的任何一档（它连上游都没打到），
/// 所以必须原样抛出，错误里不能出现「降级未成功」这种误导性文案。
#[tokio::test]
async fn unavailable_pool_does_not_trigger_fallback() {
    let _guard = guard();
    let _home = isolated_home("nofallback");
    set_chains(vec![("glm-5.3", vec!["deepseek-v3"])]).await;

    let state = GatewayState::new(GatewayConfig::default());
    let (_record, key) = state.keys.create("e2e-fb2".into(), Region::Cn);
    let app = router(state.clone());

    // 注意：**不压闸门** —— 失败原因就是「池里没有可用账号」。
    // 事件是进程级共享的（`recent_events`），先记基线，只看本次新增的那几条。
    let before = model_fallback::recent_events().len();
    let response = app
        .oneshot(chat_request(&key, "glm-5.3"))
        .await
        .expect("请求应完成");
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("读取响应体");
    let text = String::from_utf8_lossy(&bytes).to_string();

    let value: serde_json::Value = serde_json::from_str(&text).expect("错误体应是 JSON");
    assert!(
        value.get("fallback").is_none(),
        "「没有可用账号」不是可降级类型，不许附带降级轨迹：{text}"
    );
    let events = model_fallback::recent_events();
    let added = events.len().saturating_sub(before);
    assert!(
        events
            .iter()
            .take(added)
            .all(|event| event.result != "failed"),
        "这条请求不该产生「试过备选」的事件（新增 {added} 条）：{:?}",
        &events[..added.min(events.len())]
    );
}
