//! OAuth 扫码登录采集（复刻 cockpit 流程）。
//!
//! 对照 server.py `oauth_start` / `oauth_poll`。
//!
//! **region 化**：新增 `oauth_start_for` / `oauth_poll_for`；旧 CN 签名保留为薄包装。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::modules::account;
use crate::modules::checkin;
use crate::modules::config::{
    http_request, norm_ts, now_ms, now_secs, OAUTH_TIMEOUT_SECONDS, WORKBUDDY_API_PREFIX,
};
use crate::modules::credits;
use crate::modules::region::{region_spec, Region};
use crate::modules::wb_register;

#[derive(Default)]
struct OAuthInfo {
    region: Region,
    state: String,
    expires_at: i64,
    done: bool,
    result: Option<Value>,
    error: Option<String>,
}

static OAUTH_STATES: OnceLock<Mutex<HashMap<String, OAuthInfo>>> = OnceLock::new();

fn oauth_states() -> &'static Mutex<HashMap<String, OAuthInfo>> {
    OAUTH_STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Global（workbuddy.ai）web 设备登录专用请求头，与 panel `loginHTTP` 同口径：
/// CLI UA + XHR 头 + 按域切 Origin/Referer。`http_request` 的 headers 参数可逐请求
/// 覆盖默认桌面 UA，因此无需另建 client。
fn web_login_headers(base: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    headers.insert("User-Agent".to_string(), "CLI/2.63.2 CodeBuddy/2.63.2".to_string());
    headers.insert("X-Requested-With".to_string(), "XMLHttpRequest".to_string());
    headers.insert("Origin".to_string(), base.to_string());
    headers.insert("Referer".to_string(), format!("{base}/"));
    headers.insert("Accept".to_string(), "application/json, text/plain, */*".to_string());
    headers
}

/// uid 安全校验（panel 同口径）：只放行 `[A-Za-z0-9_-]`，长度 ≤ 64。
/// 上游返回的 uid 会参与拼本地文件名，路径字符（`.` / `/` 等）一律拒绝。
fn valid_uid(uid: &str) -> bool {
    let uid = uid.trim();
    if uid.is_empty() || uid.len() > 64 {
        return false;
    }
    uid.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 发起 CN 登录。
pub async fn oauth_start() -> Result<Value, String> {
    oauth_start_for(Region::Cn).await
}

/// 发起指定 region 的登录：向官方申请 state，返回 loginId / verificationUri / expiresIn。
///
/// Global 与 panel 的 web 设备登录同口径：`platform=CLI` + CLI UA / XHR / Origin 头
/// （[`web_login_headers`]）；CN 保持既有 workbuddy 平台参数与默认桌面 UA。
pub async fn oauth_start_for(region: Region) -> Result<Value, String> {
    let spec = region_spec(region);
    let base = spec.billing_base;
    let platform = if region == Region::Global {
        "CLI"
    } else {
        spec.platform
    };
    let web_headers = (region == Region::Global).then(|| web_login_headers(base));
    let login_id = format!("wb_{}", uuid::Uuid::new_v4().simple());
    let url = format!("{base}{WORKBUDDY_API_PREFIX}/auth/state?platform={platform}");
    let resp = http_request(&url, "POST", Some(json!({})), web_headers.as_ref()).await;
    let data = resp.get("data").cloned().unwrap_or_else(|| json!({}));
    let state = data
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if state.is_empty() {
        let snippet = serde_json::to_string(&resp)
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect::<String>();
        return Err(format!("auth/state 响应缺少 state: {snippet}"));
    }
    let auth_url = data
        .get("authUrl")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("auth_url").and_then(|v| v.as_str()))
        .or_else(|| data.get("url").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{base}/login?state={state}"));

    let mut map = oauth_states().lock().unwrap();
    map.insert(
        login_id.clone(),
        OAuthInfo {
            region,
            state,
            expires_at: now_secs() + OAUTH_TIMEOUT_SECONDS,
            ..Default::default()
        },
    );
    drop(map);

    Ok(json!({
        "loginId": login_id,
        "verificationUri": auth_url,
        "expiresIn": OAUTH_TIMEOUT_SECONDS,
    }))
}

/// 轮询一次 CN 官方 token 接口。
pub async fn oauth_poll(login_id: &str) -> Value {
    oauth_poll_for(Region::Cn, login_id).await
}

/// 轮询一次指定 region 的官方 token 接口。成功则拉取账号信息并入库。
pub async fn oauth_poll_for(region: Region, login_id: &str) -> Value {
    let base = region_spec(region).billing_base;
    let state = {
        let mut map = oauth_states().lock().unwrap();
        let Some(info) = map.get_mut(login_id) else {
            return json!({"done": true, "error": "登录请求不存在"});
        };
        // 登录会话在发起时绑定 region；用其他 region 轮询同一 loginId 属调用方错误，
        // 直接拒绝，避免把 CN 的登录态写进国际版账号库（反之亦然）。
        if info.region != region {
            return json!({"done": true, "error": "登录请求与目标版本不匹配"});
        }
        if info.done {
            return json!({"done": true, "result": info.result.clone(), "error": info.error.clone()});
        }
        if now_secs() > info.expires_at {
            info.done = true;
            info.error = Some("登录超时".to_string());
            return json!({"done": true, "error": "登录超时"});
        }
        info.state.clone()
    };

    let web_headers = (region == Region::Global).then(|| web_login_headers(base));
    let url = format!("{base}{WORKBUDDY_API_PREFIX}/auth/token?state={state}");
    let resp = http_request(&url, "GET", None, web_headers.as_ref()).await;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 && code != 200 {
        return json!({"done": false});
    }
    let data = resp.get("data").cloned().unwrap_or_else(|| json!({}));
    let access_token = data
        .get("accessToken")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("access_token").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    if access_token.is_empty() {
        return json!({"done": false});
    }

    // 拉取账号信息
    let account_url = format!("{base}{WORKBUDDY_API_PREFIX}/login/account?state={state}");
    let mut headers = web_headers.unwrap_or_default();
    headers.insert(
        "Authorization".to_string(),
        format!("Bearer {access_token}"),
    );
    let domain = data.get("domain").and_then(|v| v.as_str()).unwrap_or("");
    if !domain.is_empty() {
        headers.insert("X-Domain".to_string(), domain.to_string());
    }
    let acc_resp = http_request(&account_url, "GET", None, Some(&headers)).await;
    let acc_data = acc_resp.get("data").cloned().unwrap_or_else(|| json!({}));

    let uid = acc_data
        .get("uid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // Global 口径（panel 同）：uid 会参与拼本地文件名，路径字符一律拒绝；
    // 空 uid 也拒绝（panel：token 已发但账号信息缺失，要求重试而非落空号）。
    if region == Region::Global {
        match uid.as_deref() {
            Some(uid_str) if valid_uid(uid_str) => {}
            _ => {
                let mut map = oauth_states().lock().unwrap();
                if let Some(info) = map.get_mut(login_id) {
                    info.done = true;
                    info.error = Some("未获取到合法 uid，已拒绝保存（请重试）".to_string());
                }
                return json!({"done": true, "error": "未获取到合法 uid，已拒绝保存（请重试）"});
            }
        }
    }
    let nickname = acc_data
        .get("nickname")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let email = oauth_profile_email(&acc_data);

    let expires_at = norm_ts(data.get("expiresAt").or_else(|| data.get("expires_at")));
    let expires_at = match expires_at {
        Some(v) => Some(v),
        None => data
            .get("expiresIn")
            .and_then(|v| v.as_i64())
            .map(|e| now_ms() + e * 1000),
    };
    let refresh_expires_at = norm_ts(
        data.get("refreshExpiresAt")
            .or_else(|| data.get("refresh_expires_at")),
    );
    let refresh_expires_at = match refresh_expires_at {
        Some(v) => Some(v),
        None => data
            .get("refreshExpiresIn")
            .and_then(|v| v.as_i64())
            .map(|e| now_ms() + e * 1000),
    };

    let account = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "uid": uid,
        "nickname": nickname,
        "email": email,
        "enterpriseName": acc_data.get("enterpriseName"),
        "enterpriseId": acc_data.get("enterpriseId"),
        "access_token": access_token,
        "refresh_token": data.get("refreshToken").and_then(|v| v.as_str())
            .or_else(|| data.get("refresh_token").and_then(|v| v.as_str()))
            .map(|s| s.to_string()),
        "token_type": data.get("tokenType").and_then(|v| v.as_str())
            .or_else(|| data.get("token_type").and_then(|v| v.as_str()))
            .unwrap_or("Bearer")
            .to_string(),
        "domain": domain.to_string(),
        "expiresAt": expires_at,
        "refreshExpiresAt": refresh_expires_at,
        "auth_raw": data,
        "profile_raw": acc_data,
        "createdAt": now_ms(),
    });

    let account = match account::save_collected_account_for(region, account) {
        Ok(saved) => saved,
        Err(error) => {
            let error = format!("保存账号失败: {error}");
            let mut map = oauth_states().lock().unwrap();
            if let Some(info) = map.get_mut(login_id) {
                info.done = true;
                info.error = Some(error.clone());
            }
            return json!({"done": true, "error": error});
        }
    };

    let account_meta = account::account_meta_for(region, &account);
    // 添加账号后置任务（移植 workbuddy2api-panel 的 login 闭环）：CN 自动签到；
    // Global 注册激活（需补地区取白名单首个 HK）+ trial 加油包；两者都补查一次积分。
    // 全部**不阻断登录结果**——账号已落库，任务失败只进 postTasks.message。
    let post_tasks = run_post_tasks(region, &account).await;

    let result = {
        let mut out = account_meta.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("postTasks".to_string(), post_tasks.clone());
        }
        out
    };

    let mut map = oauth_states().lock().unwrap();
    if let Some(info) = map.get_mut(login_id) {
        info.done = true;
        info.result = Some(result.clone());
    }
    drop(map);

    json!({"done": true, "result": result})
}

/// 签到结果 → 展示文案（纯映射，便于测试）。
fn checkin_task_message(checkin: &Value) -> Value {
    let status = checkin
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();
    match status.as_str() {
        "success" => json!("签到成功"),
        "already" => json!("今日已签到"),
        other => json!(format!(
            "签到失败: {}",
            checkin
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or(other)
        )),
    }
}

/// 积分查询结果 → 展示文案（纯映射，便于测试）。
fn credits_task_message(credits: &Value) -> Value {
    if credits.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        let mut msg = format!(
            "积分余额 {}/{}",
            credits
                .get("totalRemaining")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            credits
                .get("totalCapacity")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        );
        if let Some(date) = format_soonest_expire_ms(
            credits.get("soonestExpireAt").and_then(|v| v.as_i64()),
        ) {
            msg.push_str(&format!("，最早到期 {date}"));
        }
        json!(msg)
    } else {
        json!(format!(
            "积分查询失败: {}",
            credits
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("未知原因")
        ))
    }
}

fn format_soonest_expire_ms(ms: Option<i64>) -> Option<String> {
    let secs = ms? / 1000;
    Some(
        chrono::DateTime::from_timestamp(secs, 0)?
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string(),
    )
}

/// 添加账号成功后的自动任务，返回前端可直接展示状态的扁平结构。
///
/// 字段口径：
/// - `checkin`（CN）：签到状态文案，取自 [`checkin::checkin_account_for`]（内置并发守卫与惰性刷新）。
/// - `register` / `trial`（Global）：注册激活与 trial 领取结果，失败带原因。
/// - `credits`：积分摘要（两 region 都有），失败带原因。
async fn run_post_tasks(region: Region, account: &Value) -> Value {
    let mut tasks = serde_json::Map::new();
    if region == Region::Cn {
        let checkin = checkin::checkin_account_for(region, account).await;
        tasks.insert("checkin".to_string(), checkin_task_message(&checkin));
    } else {
        let register = wb_register::global_complete_registration(account).await;
        let register_msg = if register["activated"] == json!(true) {
            "注册激活成功".to_string()
        } else {
            register["message"]
                .as_str()
                .map(|s| format!("注册激活: {s}"))
                .unwrap_or_default()
        };
        tasks.insert("register".to_string(), json!(register_msg));
        let trial = wb_register::claim_trial(account).await;
        let trial_msg = if trial["claimed"] == json!(true) {
            "trial 加油包领取成功".to_string()
        } else {
            trial["message"]
                .as_str()
                .map(|s| format!("trial: {s}"))
                .unwrap_or_default()
        };
        tasks.insert("trial".to_string(), json!(trial_msg));
    }
    let credits = credits::get_credit_expiry_for(region, account).await;
    tasks.insert("credits".to_string(), credits_task_message(&credits));
    Value::Object(tasks)
}

fn oauth_profile_email(profile: &Value) -> Option<String> {
    profile
        .get("email")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_profile_without_email_does_not_use_nickname_or_uid() {
        let profile = json!({"uid": "u-1", "nickname": "同名用户"});
        assert_eq!(oauth_profile_email(&profile), None);
    }

    #[test]
    fn oauth_profile_keeps_factual_email() {
        let profile = json!({"email": " user@example.com "});
        assert_eq!(
            oauth_profile_email(&profile).as_deref(),
            Some("user@example.com")
        );
    }

    #[test]
    fn valid_uid_rejects_path_traversal_and_allows_uuid_shape() {
        assert!(valid_uid("0d1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8"));
        assert!(valid_uid("u_12345"));
        assert!(!valid_uid(""), "空 uid 必须拒绝");
        assert!(!valid_uid("../../evil"), "路径穿越必须拒绝");
        assert!(!valid_uid("a/b"), "斜杠必须拒绝");
        assert!(!valid_uid("a.b"), "点号必须拒绝");
        assert!(!valid_uid(&"x".repeat(65)), "超长必须拒绝");
    }

    #[test]
    fn web_login_headers_match_panel_cli_profile() {
        let headers = web_login_headers("https://www.workbuddy.ai");
        assert_eq!(headers.get("User-Agent").unwrap(), "CLI/2.63.2 CodeBuddy/2.63.2");
        assert_eq!(headers.get("X-Requested-With").unwrap(), "XMLHttpRequest");
        assert_eq!(headers.get("Origin").unwrap(), "https://www.workbuddy.ai");
        assert_eq!(headers.get("Referer").unwrap(), "https://www.workbuddy.ai/");
    }

    /// 轮询必须校验 loginId 绑定的 region（PRD G1）：用另一版轮询同一 loginId
    /// 必须在任何 HTTP 请求**之前**被拒绝，否则会把 CN 的登录态写进国际版账号库
    /// （反之亦然）。
    ///
    /// 这里直接往 `OAUTH_STATES` 里种一条记录来构造交错，因此不依赖网络，
    /// 也不会真的发起请求。用完清理，避免污染其它用例。
    #[tokio::test]
    async fn poll_rejects_login_id_bound_to_other_region() {
        let cn_login = format!("wb_test_{}", uuid::Uuid::new_v4().simple());
        let global_login = format!("wb_test_{}", uuid::Uuid::new_v4().simple());
        let unbound = "wb_test_never_inserted";

        {
            let mut map = oauth_states().lock().unwrap();
            map.insert(
                cn_login.clone(),
                OAuthInfo {
                    region: Region::Cn,
                    state: "state-cn".to_string(),
                    expires_at: now_secs() + OAUTH_TIMEOUT_SECONDS,
                    ..Default::default()
                },
            );
            map.insert(
                global_login.clone(),
                OAuthInfo {
                    region: Region::Global,
                    state: "state-global".to_string(),
                    expires_at: now_secs() + OAUTH_TIMEOUT_SECONDS,
                    ..Default::default()
                },
            );
        }

        // Global 轮询 CN 发起的 loginId → 拒绝。
        let crossed = oauth_poll_for(Region::Global, &cn_login).await;
        assert_eq!(crossed["done"], json!(true));
        assert_eq!(crossed["error"], json!("登录请求与目标版本不匹配"));

        // 反向：CN 轮询 Global 发起的 loginId → 同样拒绝。
        let crossed_rev = oauth_poll_for(Region::Cn, &global_login).await;
        assert_eq!(crossed_rev["done"], json!(true));
        assert_eq!(crossed_rev["error"], json!("登录请求与目标版本不匹配"));

        // 未知 loginId → 登录请求不存在（同样在 I/O 之前返回）。
        let unknown = oauth_poll_for(Region::Cn, unbound).await;
        assert_eq!(unknown["done"], json!(true));
        assert_eq!(unknown["error"], json!("登录请求不存在"));

        let mut map = oauth_states().lock().unwrap();
        map.remove(&cn_login);
        map.remove(&global_login);
    }

    /// 后置任务纯映射：登录结果与任务结果在文案层合并（任务失败不改变登录成功）。
    #[test]
    fn post_task_messages_map_checkin_and_credits() {
        assert_eq!(
            checkin_task_message(&json!({"result": "success"})),
            json!("签到成功")
        );
        assert_eq!(
            checkin_task_message(&json!({"result": "already"})),
            json!("今日已签到")
        );
        assert!(checkin_task_message(&json!({
            "result": "error",
            "error": "网络超时"
        }))
        .to_string()
        .contains("签到失败: 网络超时"));

        let ok = credits_task_message(&json!({
            "ok": true,
            "totalRemaining": 120.0,
            "totalCapacity": 300.0,
            "soonestExpireAt": 1_750_000_000_000_i64
        }));
        let text = ok.to_string();
        assert!(text.contains("120"), "应含剩余量: {text}");
        assert!(text.contains("300"), "应含总量: {text}");
        // 1_750_000_000_000 ms = 2025-06-15 23:46 UTC（按本机时区渲染，
        // 东半球为 06-16）——断言覆盖两个候选日期。
        assert!(
            text.contains("2025-06-15") || text.contains("2025-06-16"),
            "应含到期日: {text}"
        );

        let fail = credits_task_message(&json!({
            "ok": false,
            "error": "上游 500"
        }));
        assert!(fail.to_string().contains("积分查询失败: 上游 500"));
    }
}
