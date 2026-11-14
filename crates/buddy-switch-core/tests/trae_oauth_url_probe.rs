//! 真机探针：确认「区域标识 → 授权页产品线」在**真实客户端环境**下落在 TRAE 线。
//!
//! 单测用的是**合成事实**（`synthetic_facts`），它能钉住 URL 的形状，但回答不了
//! 「本机这两个客户端实际读到的 device_id / machine_id / 版本号是什么」以及
//! 「授权页域是不是本区域的」。本探针走的是**和界面完全同一条**入口
//! （`handlers::parse_variant_id` → `oauth_login_start_for`），因此能同时兜住
//! 「区域解析」与「设备身份来源」两类缺陷 —— 后者曾经指向另一代客户端（`TRAE SOLO`）。
//!
//! 运行：`cargo test -p buddy-switch-core --test trae_oauth_url_probe -- --ignored --nocapture`
//!
//! 只读客户端 + 起一次本地回调监听（随即取消）：不写账号库、不改客户端状态。
//!
//! ⚠️ 会占用固定回调端口 `17388`：Trae 客户端正在授权时端口被占，本探针会打印
//! `发起登录失败: …`（那是环境问题，不是缺陷）。输出含本机 `device_id` / `machine_id`，
//! 贴到别处前先脱敏。

use buddy_switch_core::modules::trae::handlers;
use serde_json::Value;

fn param(uri: &str, key: &str) -> String {
    let query = uri.split_once('?').map(|(_, q)| q).unwrap_or("");
    query
        .split('&')
        .find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == key).then(|| v.to_string())
        })
        .unwrap_or_default()
}

fn host(uri: &str) -> String {
    uri.split_once('?')
        .map(|(base, _)| base.to_string())
        .unwrap_or_else(|| uri.to_string())
}

#[tokio::test]
#[ignore = "真机探针：需要本机已安装 Trae 客户端，默认不跑"]
async fn probe_region_authorize_url() {
    for region in ["cn", "global"] {
        let variant = handlers::parse_variant_id(Some(region));
        println!(
            "\n=========== region={region} → variant={} ===========",
            variant.as_str()
        );

        match handlers::oauth_login_start_for(variant).await {
            Ok(value) => {
                let uri = value
                    .get("verificationUri")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                println!("授权页基址 : {}", host(&uri));
                for key in [
                    "auth_from",
                    "client_id",
                    "login_channel",
                    "auth_type",
                    "redirect",
                    "hide_saas_login",
                    "machine_id",
                    "device_id",
                    "x_machine_id",
                    "x_device_id",
                    "x_app_version",
                    "x_app_type",
                    "plugin_version",
                    "channel_name",
                    "code_challenge_method",
                ] {
                    let value = param(&uri, key);
                    println!(
                        "  {key:22} = {}",
                        if value.is_empty() {
                            "(缺失)".to_string()
                        } else {
                            value
                        }
                    );
                }
                println!(
                    "参数总个数 : {}",
                    uri.split_once('?')
                        .map(|(_, q)| q.split('&').count())
                        .unwrap_or(0)
                );
                if let Some(cred) = value.get("deviceCredential") {
                    println!("设备凭证诊断 : {cred}");
                }
                let login_id = value
                    .get("loginId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                println!("取消会话 : {}", handlers::oauth_login_cancel(&login_id));
            }
            Err(error) => println!("发起登录失败: {error}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
