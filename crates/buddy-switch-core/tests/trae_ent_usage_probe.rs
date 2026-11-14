//! 诊断：Trae 积分接口的**请求体**到底该发什么。
//!
//! ## 背景
//!
//! `credits::post_json` 对所有路径都发空体 `{}`（沿用 Python 参考实现）。但从 Trae 客户端
//! 自己的缓存里翻出的自动生成 SDK 显示，真实客户端调用同一个接口时发的是结构化体：
//!
//! ```js
//! GetIdeUserEntUsageV2(e, t) {
//!   let r = e || {}, a = this.genBaseURL("/trae/api/v2/pay/ide_user_ent_usage"),
//!       i = {require_usage: r.require_usage, req_source: r.req_source,
//!            full_data: r.full_data, Request: r.Request};
//!   return this.request({url: a, method: "POST", data: i}, t)
//! }
//! ```
//!
//! 且同一份缓存里的真实调用点默认 `{require_usage: true, full_data: true}`。
//! **若服务端把 `full_data` 默认成 false，`{}` 就拿不到完整的
//! `user_entitlement_pack_list`**，`calc_remaining_credits` 会直接报错。
//!
//! ## 为什么需要真实账号
//!
//! 本机切换器的 Trae 账号库长期为空，这条路径**从未对真实上游跑通过** ——
//! 所以"发 `{}` 能用"这个前提没人验证过。改线上协议不能靠推理，只能实测。
//!
//! ## 用法
//!
//! 先用切换器登录一个 Trae 账号（或直接把 JWT 放进环境变量），然后：
//!
//! ```bash
//! # 用切换器账号库里的第一个账号
//! cargo test -p buddy-switch-core --test trae_ent_usage_probe -- --ignored --nocapture
//!
//! # 或显式指定 JWT（不落盘、不进日志）
//! TRAE_PROBE_JWT='<jwt>' cargo test -p buddy-switch-core \
//!     --test trae_ent_usage_probe -- --ignored --nocapture
//! ```
//!
//! **本测试只读、只发 GET 语义的查询请求，不签到、不改任何本地状态。**
//! 输出里**绝不打印 JWT**，只打印状态码、顶层字段名与 pack 数量。
//!
//! 默认 `#[ignore]`，不会在 CI / `cargo test --workspace` 里跑（它依赖真实凭据与网络）。

use std::collections::BTreeSet;

use buddy_switch_core::modules::trae::credits;
use buddy_switch_core::modules::trae::device::DeviceEntry;
use buddy_switch_core::modules::trae::endpoints_for;
use buddy_switch_core::modules::trae::variant::TraeVariant;
use buddy_switch_core::modules::trae::TRAE_ENTITLEMENT_PATH;

/// 真实客户端发的请求体（`f_*` 缓存里 `tp` 的默认参数）。
const CLIENT_BODY: &str = r#"{"require_usage":true,"full_data":true}"#;

/// 当前实现发的请求体。
const CURRENT_BODY: &str = "{}";

/// 取一个可用的 JWT：优先环境变量，其次切换器账号库的第一个账号。
fn probe_jwt() -> Option<String> {
    if let Ok(value) = std::env::var("TRAE_PROBE_JWT") {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    buddy_switch_core::modules::trae::account::entries()
        .into_iter()
        .map(|(_, account)| account.jwt)
        .find(|jwt| !jwt.trim().is_empty())
}

/// 按 `credits::post_json` 的方式发一次请求，但允许自定义请求体。
///
/// 刻意**不改 `post_json` 的签名**：这里直接复用公开的 `trae_http_client()` 与
/// `build_headers()`，与生产路径逐字节同构，不需要为诊断而改产品代码。
async fn post_with_body(body: &str, jwt: &str, device: &DeviceEntry) -> (u16, String) {
    let url = format!(
        "{}{TRAE_ENTITLEMENT_PATH}",
        endpoints_for(TraeVariant::default()).account_base
    );
    let mut request = credits::trae_http_client()
        .post(&url)
        .body(body.to_string());
    for (key, value) in credits::build_headers(jwt, device) {
        request = request.header(key, value);
    }
    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            (status, text)
        }
        Err(error) => (0, format!("{error}")),
    }
}

/// 概括一次响应，供对照：状态码、顶层字段、pack 数量、错误文案。
#[derive(Debug)]
struct Summary {
    status: u16,
    top_keys: BTreeSet<String>,
    pack_count: Option<usize>,
    has_pack_list: bool,
    note: String,
}

fn summarize(status: u16, body: &str) -> Summary {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let mut top_keys = BTreeSet::new();
    let mut pack_count = None;
    let mut has_pack_list = false;

    if let Some(value) = parsed.as_ref() {
        if let Some(object) = value.as_object() {
            for key in object.keys() {
                top_keys.insert(key.clone());
            }
            if let Some(list) = object.get("user_entitlement_pack_list") {
                has_pack_list = true;
                pack_count = list.as_array().map(|items| items.len());
            }
        }
    }

    // 只留一小段错误文案用于定位，且绝不含凭据。
    let note = if parsed.is_none() {
        body.chars().take(160).collect::<String>()
    } else if !has_pack_list {
        "(JSON 合法，但没有 user_entitlement_pack_list)".to_string()
    } else {
        String::new()
    };

    Summary {
        status,
        top_keys,
        pack_count,
        has_pack_list,
        note,
    }
}

#[tokio::test]
#[ignore = "需要真实 Trae 凭据与网络；用 --ignored 手动运行"]
async fn probe_ent_usage_request_body() {
    let Some(jwt) = probe_jwt() else {
        eprintln!(
            "跳过：没有可用凭据。请先用切换器登录一个 Trae 账号，\
             或设置 TRAE_PROBE_JWT 环境变量。"
        );
        return;
    };

    let device = credits::device_for_jwt(&jwt).expect("JWT 应能解析出 user id 并派生出设备");
    eprintln!(
        "使用账号 {}（JWT 长度 {}，不回显内容）\n",
        device.device_id.chars().take(6).collect::<String>() + "…",
        jwt.len()
    );

    let (s_cur, b_cur) = post_with_body(CURRENT_BODY, &jwt, &device).await;
    let (s_new, b_new) = post_with_body(CLIENT_BODY, &jwt, &device).await;

    let cur = summarize(s_cur, &b_cur);
    let new = summarize(s_new, &b_new);

    println!("=== 当前实现：body = {CURRENT_BODY}");
    println!("  状态码          : {}", cur.status);
    println!("  含 pack_list    : {}", cur.has_pack_list);
    println!("  pack 数量       : {:?}", cur.pack_count);
    println!(
        "  顶层字段        : {:?}",
        cur.top_keys.iter().collect::<Vec<_>>()
    );
    if !cur.note.is_empty() {
        println!("  备注            : {}", cur.note);
    }

    println!("\n=== 真实客户端：body = {CLIENT_BODY}");
    println!("  状态码          : {}", new.status);
    println!("  含 pack_list    : {}", new.has_pack_list);
    println!("  pack 数量       : {:?}", new.pack_count);
    println!(
        "  顶层字段        : {:?}",
        new.top_keys.iter().collect::<Vec<_>>()
    );
    if !new.note.is_empty() {
        println!("  备注            : {}", new.note);
    }

    println!("\n=== 结论");
    match (cur.has_pack_list, new.has_pack_list) {
        (true, true) => println!(
            "  两者都拿得到 pack_list → 服务端对 full_data 有合理默认，**当前实现没问题**，不要改。"
        ),
        (false, true) => println!(
            "  **当前实现拿不到 pack_list，客户端那套能拿到** → 确认是真 bug。\n  \
             改法：`post_json` 的 body 由 `{{}}` 改为 {CLIENT_BODY}（或给积分路径单独用它）。"
        ),
        (false, false) => println!(
            "  两者都拿不到 pack_list → 不是请求体的问题。看状态码：\n  \
             401/403 说明凭据失效；其他看上面的备注与顶层字段。"
        ),
        (true, false) => {
            println!("  当前实现能拿到、客户端那套反而拿不到 → 保持现状，并把客户端那套排除掉。")
        }
    }
    if cur.status != new.status {
        println!(
            "  注意：两次状态码不同（{} vs {}），以各自的实际响应为准。",
            cur.status, new.status
        );
    }
}
