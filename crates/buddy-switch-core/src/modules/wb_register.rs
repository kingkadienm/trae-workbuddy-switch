//! 国际版（Global）新账号注册激活与一次性 trial 加油包领取。
//!
//! 移植自 workbuddy2api-panel `internal/upstream/global_register.go` / `trial.go`：
//!
//! - 注册激活：`GET /auth/realms/copilot/overseas/user/register?userId=<uid>`，
//!   code 200 已激活；code 500 且 msg 含 "region required" 需先补地区。
//! - 补地区：`POST /billing/area/get-country-code` 拉国家列表（⚠️ data 是 JSON
//!   字符串双层信封，需二次解析），白名单 HK/MO/SG/TH/PH/MY/ID 过滤后取首个，
//!   `POST /console/login/account` 提交（幂等）。
//! - trial 加油包：`POST /billing/ide/trial`，幂等码 14051 = 已领过（非错误）。
//!
//! 仅 `Region::Global` 可用（CN 无这些端点），对齐 panel 的 realm 防线。

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::modules::account::{build_auth_headers, get_str};
use crate::modules::config::http_request;
use crate::modules::region::{region_of, region_spec, Region};

/// 注册链路走国际版 web 指纹（panel `global_register.go` 的 `globalWebUA`）：
/// 注册完善页是 web 流量，非桌面 CLI 指纹。
const GLOBAL_WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// 国际版 web 白名单（顺序对齐 web 展示集；`global_complete_registration`
/// 需补地区时取首个，即 HK）。
const COUNTRY_WHITELIST: &[&str] = &["HK", "MO", "SG", "TH", "PH", "MY", "ID"];

/// trial 幂等码 14051 的两种指纹：
/// - 200 响应外层信封 `{"code":14051,...}`；
/// - 非 2xx 时 `http_request` 把原始 body 塞进 message（前 500 字符截断）。
const TRIAL_ALREADY_MARKERS: &[&str] = &["14051"];

/// 统一信封取数：code（缺省 -1）+ 合并的 msg/message + data。
///
/// 与 `checkin.rs` 的 code 判断口径一致：`http_request` 对非 2xx 且 body 可解析时
/// 直接返回 body 原文（上游信封），不可解析时 `{"code": <status>, "message": …}`，
/// 两种形态都从这里取。
fn envelope(resp: &Value) -> (i64, String, Value) {
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    let mut msg = resp
        .get("msg")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(message) = resp.get("message").and_then(|v| v.as_str()) {
        if msg.is_empty() {
            msg = message.to_string();
        } else {
            msg.push(' ');
            msg.push_str(message);
        }
    }
    let data = resp.get("data").cloned().unwrap_or(Value::Null);
    (code, msg, data)
}

/// 请求头：官方对齐的认证头 + 注册链路的 web 指纹 UA 与 Origin/Referer。
fn register_headers(account: &Value) -> HashMap<String, String> {
    let mut headers = build_auth_headers(account);
    let base = region_spec(Region::Global).billing_base;
    headers.insert("User-Agent".to_string(), GLOBAL_WEB_UA.to_string());
    headers.insert("Origin".to_string(), base.to_string());
    headers.insert("Referer".to_string(), format!("{base}/"));
    headers
}

/// 非 Global 账号的统一防线（对齐 panel `Realm()=="global"` 判据）。
///
/// 只拒绝**明确 CN** 的账号（`domain` 非空且 `region_of` 判为 CN）；`domain` 缺失
/// （OAuth 采集的 global 账号 token 响应没有 domain 时即空串）信任调用方传入的
/// region，不误拒。
fn require_global(account: &Value) -> Result<(), String> {
    if let Some(domain) = get_str(account, "domain") {
        if region_of(&domain) == Region::Cn {
            return Err("仅国际版账号可用".to_string());
        }
    }
    Ok(())
}

/// 注册激活状态：`{activated, needsRegion, message}`。
///
/// code 200 = 已激活；code 500 且 msg 含 "region required"（不区分大小写）= 需补地区。
pub fn parse_register_status(resp: &Value) -> Value {
    let (code, msg, _) = envelope(resp);
    match code {
        200 => json!({"activated": true, "needsRegion": false, "message": "register success"}),
        500 if msg.to_lowercase().contains("region required") => {
            json!({"activated": false, "needsRegion": true, "message": msg})
        }
        _ => json!({"activated": false, "needsRegion": false, "message": msg}),
    }
}

/// 解析国家列表：`data` 可能是 JSON 字符串（双层信封）或对象，需二次解析；
/// 取内层 `data.list` 后按白名单过滤（保持顺序）。
pub fn parse_country_list(resp: &Value) -> Vec<Value> {
    let (_, _, data) = envelope(resp);
    let inner = match data {
        Value::String(text) => serde_json::from_str::<Value>(text.trim())
            .unwrap_or_else(|_| Value::Object(serde_json::Map::new())),
        value => value.clone(),
    };
    let list = inner
        .get("data")
        .and_then(|v| v.get("list"))
        .or_else(|| inner.get("list"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let by_code: std::collections::HashMap<String, Value> = list
        .iter()
        .filter_map(|c| {
            let ios2 = c.get("IOS2")?.as_str()?.to_string();
            Some((ios2, c.clone()))
        })
        .collect();
    COUNTRY_WHITELIST
        .iter()
        .filter_map(|code| by_code.get(*code))
        .cloned()
        .collect()
}

/// 已领过 trial（幂等码 14051）判定：只看外层响应，不看 data。
pub fn trial_already_claimed(resp: &Value) -> bool {
    let haystack = format!("{} {}", resp.get("code").map(|v| v.to_string()).unwrap_or_default(), resp.get("message").and_then(|v| v.as_str()).unwrap_or_default());
    TRIAL_ALREADY_MARKERS
        .iter()
        .any(|m| haystack.contains(m))
}

/// 查询 Global 账号注册激活状态。
pub async fn global_register_status(account: &Value) -> Result<Value, String> {
    require_global(account)?;
    let uid = get_str(account, "uid").ok_or_else(|| "缺少 uid".to_string())?;
    let base = region_spec(Region::Global).billing_base;
    let url = format!(
        "{base}/auth/realms/copilot/overseas/user/register?userId={uid}"
    );
    let resp = http_request(&url, "GET", None, Some(&register_headers(account)))
        .await;
    Ok(parse_register_status(&resp))
}

/// 拉取 Global 可注册地区（白名单过滤后）。
pub async fn global_fetch_countries(account: &Value) -> Result<Vec<Value>, String> {
    require_global(account)?;
    let base = region_spec(Region::Global).billing_base;
    let url = format!("{base}/billing/area/get-country-code");
    let resp =
        http_request(&url, "POST", Some(json!({"filterForbidden": 1})), Some(&register_headers(account)))
            .await;
    let (code, msg, _) = envelope(&resp);
    if code != 0 {
        return Err(format!("get-country-code: {msg} (code={code})"));
    }
    let list = parse_country_list(&resp);
    if list.is_empty() {
        return Err("no countries available".to_string());
    }
    Ok(list)
}

/// 提交注册地区（幂等）。`country` 来自 `global_fetch_countries`。
pub async fn global_submit_region(account: &Value, country: &Value) -> Result<(), String> {
    require_global(account)?;
    let base = region_spec(Region::Global).billing_base;
    let url = format!("{base}/console/login/account");
    let body = json!({
        "attributes": {
            "countryCode": [country.get("Code").cloned().unwrap_or(Value::Null)],
            "countryFullName": [country.get("EnName").cloned().unwrap_or(Value::Null)],
            "countryName": [country.get("IOS2").cloned().unwrap_or(Value::Null)],
        }
    });
    let resp =
        http_request(&url, "POST", Some(body), Some(&register_headers(account))).await;
    let (code, msg, _) = envelope(&resp);
    if code != 0 {
        return Err(format!("submit region: {msg} (code={code})"));
    }
    Ok(())
}

/// 一键注册激活：查状态 → 需补地区则取白名单首个（HK）提交 → 复查。
/// 返回 `{activated, message}`；已激活直接返回。
pub async fn global_complete_registration(account: &Value) -> Value {
    let status = match global_register_status(account).await {
        Ok(s) => s,
        Err(error) => return json!({"activated": false, "message": error}),
    };
    if status["activated"] == json!(true) {
        return json!({"activated": true, "message": "register success"});
    }
    if status["needsRegion"] != json!(true) {
        return json!({
            "activated": false,
            "message": format!("register not activated: {}", status["message"]),
        });
    }
    let countries = match global_fetch_countries(account).await {
        Ok(list) => list,
        Err(error) => {
            return json!({"activated": false, "message": format!("fetch countries: {error}")})
        }
    };
    if let Err(error) = global_submit_region(account, &countries[0]).await {
        return json!({"activated": false, "message": format!("submit region: {error}")});
    }
    match global_register_status(account).await {
        Ok(recheck) => {
            if recheck["activated"] == json!(true) {
                json!({"activated": true, "message": "register success"})
            } else {
                json!({
                    "activated": false,
                    "message": format!(
                        "register still not activated after region submit: {}",
                        recheck["message"]
                    ),
                })
            }
        }
        Err(error) => json!({"activated": false, "message": error}),
    }
}

/// 领取一次性 trial 加油包。返回 `{claimed, message}`；已领过（14051）视为幂等成功。
pub async fn claim_trial(account: &Value) -> Value {
    if require_global(account).is_err() {
        return json!({"claimed": false, "message": "仅国际版账号可用"});
    }
    let base = region_spec(Region::Global).billing_base;
    let url = format!("{base}/billing/ide/trial");
    let resp =
        http_request(&url, "POST", Some(json!({})), Some(&register_headers(account)))
            .await;
    if trial_already_claimed(&resp) {
        return json!({"claimed": false, "message": "已领取过 trial（幂等）"});
    }
    let (code, msg, _) = envelope(&resp);
    if code == 0 {
        json!({"claimed": true, "message": "trial 领取成功"})
    } else {
        json!({"claimed": false, "message": format!("trial 领取失败: {msg} (code={code})")})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_list_unwraps_double_envelope_and_filters_whitelist() {
        // data 是 JSON 字符串（双层信封），list 里混入非白名单国家。
        let inner = json!({"data": {"list": [
            {"IOS2": "HK", "EnName": "Hong Kong", "Code": "344"},
            {"IOS2": "US", "EnName": "United States", "Code": "840"},
            {"IOS2": "SG", "EnName": "Singapore", "Code": "702"},
        ]}});
        let resp = json!({"code": 0, "data": serde_json::json!(serde_json::to_string(&inner).unwrap())});
        let list = parse_country_list(&resp);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["IOS2"], json!("HK"));
        assert_eq!(list[1]["IOS2"], json!("SG"));
    }

    #[test]
    fn country_list_plain_object_envelope_also_parses() {
        let resp = json!({"code": 0, "data": {"data": {"list": [
            {"IOS2": "MO", "EnName": "Macao", "Code": "446"},
        ]}}});
        let list = parse_country_list(&resp);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["IOS2"], json!("MO"));
    }

    #[test]
    fn trial_already_claimed_matches_14051_in_outer_or_message() {
        assert!(trial_already_claimed(&json!({"code": 14051, "message": "already claimed"})));
        assert!(trial_already_claimed(
            &json!({"code": 400, "message": "上游返回原文 {\"code\":14051,\"msg\":\"...\"}"})
        ));
        assert!(!trial_already_claimed(&json!({"code": 0})));
    }

    #[test]
    fn register_status_regions_required_only_on_code_500() {
        let ok = parse_register_status(&json!({"code": 200}));
        assert_eq!(ok["activated"], json!(true));
        let need =
            parse_register_status(&json!({"code": 500, "msg": "region required"}));
        assert_eq!(need["needsRegion"], json!(true));
        let other = parse_register_status(&json!({"code": 500, "msg": "boom"}));
        assert_eq!(other["needsRegion"], json!(false));
    }

    /// 明确 CN domain 必须被前置拒绝；Global domain 与缺失 domain 放行
    /// （OAuth 采集的 global 账号 token 响应可能没有 domain，信任调用方 region）。
    #[test]
    fn require_global_rejects_cn_account() {
        assert!(require_global(&json!({"uid": "u1", "domain": "codebuddy.cn"})).is_err());
        assert!(require_global(&json!({"uid": "u1", "domain": "www.workbuddy.ai"})).is_ok());
        assert!(require_global(&json!({"uid": "u1"})).is_ok());
    }
}
