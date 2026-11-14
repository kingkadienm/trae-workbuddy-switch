//! Trae JWT 解析。
//!
//! **不校验签名**——仅用于本地展示（user_id、到期时间）与「更新后防串号」比对。
//! 真正的鉴权由上游完成，本地校验签名只会因为密钥轮换而误判。
//!
//! 与 WorkBuddy 侧的凭据解析刻意分开：Trae 用 VSCode 系的 `Cloud-IDE-JWT <token>`
//! 前缀，payload 里用户 ID 位于 `data.id`（WorkBuddy 的 `workbuddy-desktop.info`
//! 是完全不同的结构）。两者唯一的共同点是都沿用 JWT 的 `exp` 约定。

use base64::Engine;

/// JWT 解析结果。字段全部 `Option`：任何一步失败都退化为「未知」，绝不 panic。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TraeJwt {
    /// 用户 ID（`data.id` → `auth_id` → `sub` 依次回落）。
    pub user_id: Option<String>,
    /// 剩余有效期（小时，可能为负表示已过期）。
    pub exp_hours: Option<f64>,
    /// `exp` 的 Unix 秒时间戳。
    pub exp_timestamp: Option<i64>,
}

impl TraeJwt {
    /// 到期状态：`ok` / `warn` / `expired` / `unknown`。
    ///
    /// 阈值与参考实现一致：>24h 为 `ok`，>0 且 ≤24h 为 `warn`。
    pub fn status(&self) -> &'static str {
        match self.exp_hours {
            Some(hours) if hours > 24.0 => "ok",
            Some(hours) if hours > 0.0 => "warn",
            Some(_) => "expired",
            None => "unknown",
        }
    }
}

/// 归一化 JWT：剥掉 `Cloud-IDE-JWT ` / `Bearer ` 前缀与首尾空白。
///
/// 用户从浏览器/代理日志粘贴时经常带上前缀或换行，这里统一收敛，
/// 使后续所有消费点（请求头、解析、展示）都拿到同一种形态。
pub fn normalize(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("Cloud-IDE-JWT ")
        .or_else(|| trimmed.strip_prefix("Bearer "))
        .unwrap_or(trimmed)
        .trim()
}

/// 组装请求头所需的 `Authorization` 值：`Cloud-IDE-JWT <token>`。
///
/// 已经是该前缀时原样返回，避免出现 `Cloud-IDE-JWT Cloud-IDE-JWT …`。
pub fn authorization_header(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("Cloud-IDE-JWT ") {
        trimmed.to_string()
    } else {
        format!("Cloud-IDE-JWT {}", normalize(trimmed))
    }
}

/// 解析 JWT 的 payload 段。失败返回 `None`。
fn decode_payload(raw: &str) -> Option<serde_json::Value> {
    let token = normalize(raw);
    let mut parts = token.split('.');
    parts.next()?; // header
    let payload_b64 = parts.next()?;
    // 先剥离填充符：payload 是 base64url 无填充编码，混入 `=` 会让 NO_PAD 解码失败。
    let payload_b64 = payload_b64.trim_end_matches('=');
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()
}

/// 解析 Trae JWT。
pub fn parse(raw: &str) -> TraeJwt {
    let Some(payload) = decode_payload(raw) else {
        return TraeJwt::default();
    };

    let user_id = payload
        .get("data")
        .and_then(|data| data.get("id"))
        .and_then(|value| {
            value
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| value.as_i64().map(|n| n.to_string()))
        })
        .or_else(|| {
            payload
                .get("auth_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            payload
                .get("sub")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

    // `exp` 在不同签发方下可能是整数、浮点或数字字符串，依次尝试。
    let exp_timestamp = payload.get("exp").and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|f| f as i64))
            .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()))
    });

    let exp_hours = exp_timestamp.map(|exp| {
        let now = chrono::Utc::now().timestamp();
        (exp - now) as f64 / 3600.0
    });

    TraeJwt {
        user_id,
        exp_hours,
        exp_timestamp,
    }
}

/// 从 JWT 取用户 ID。
pub fn user_id_of(raw: &str) -> Option<String> {
    parse(raw).user_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// 构造一个真实形态的 Trae JWT（`Cloud-IDE-JWT` + 载荷 data.id/exp）。
    fn make_jwt(user_id: &str, exp_offset_secs: i64) -> String {
        let exp = chrono::Utc::now().timestamp() + exp_offset_secs;
        let payload = serde_json::json!({
            "data": { "id": user_id },
            "exp": exp,
        });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        format!("Cloud-IDE-JWT header.{b64}.signature")
    }

    #[test]
    fn parse_extracts_user_id_and_expiry() {
        let jwt = make_jwt("1234567890123456", 48 * 3600);
        let info = parse(&jwt);
        assert_eq!(info.user_id.as_deref(), Some("1234567890123456"));
        assert!(info.exp_hours.unwrap() > 47.0 && info.exp_hours.unwrap() <= 48.0);
        assert_eq!(info.status(), "ok");
    }

    #[test]
    fn parse_tolerates_prefixes_and_padding() {
        let jwt = make_jwt("999", 10 * 3600);
        // 带 Bearer 前缀
        assert_eq!(
            parse(&format!("Bearer {jwt}")).user_id.as_deref(),
            Some("999")
        );
        // 带 Cloud-IDE-JWT 前缀（裸 token 再包一层）
        let bare = normalize(&jwt).to_string();
        assert_eq!(
            parse(&format!("Cloud-IDE-JWT {bare}")).user_id.as_deref(),
            Some("999")
        );
        // 带首尾空白与换行
        assert_eq!(parse(&format!("  {jwt}\n")).user_id.as_deref(), Some("999"));
    }

    #[test]
    fn parse_accepts_padded_base64_payload() {
        // 某些签发方会补 `=`，必须仍能解析。
        let payload = serde_json::json!({ "data": { "id": "42" } });
        let padded =
            base64::engine::general_purpose::URL_SAFE.encode(serde_json::to_vec(&payload).unwrap());
        let jwt = format!("h.{padded}.s");
        assert_eq!(parse(&jwt).user_id.as_deref(), Some("42"));
    }

    #[test]
    fn parse_falls_back_to_auth_id_then_sub() {
        let payload = serde_json::json!({ "auth_id": "auth-1" });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        assert_eq!(
            parse(&format!("h.{b64}.s")).user_id.as_deref(),
            Some("auth-1")
        );

        let payload = serde_json::json!({ "sub": "sub-1" });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        assert_eq!(
            parse(&format!("h.{b64}.s")).user_id.as_deref(),
            Some("sub-1")
        );
    }

    #[test]
    fn parse_numeric_user_id_and_string_exp() {
        let exp = chrono::Utc::now().timestamp() + 3600;
        let payload = serde_json::json!({ "data": { "id": 12345 }, "exp": exp.to_string() });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let info = parse(&format!("h.{b64}.s"));
        assert_eq!(info.user_id.as_deref(), Some("12345"));
        assert!(info.exp_timestamp.is_some());
    }

    #[test]
    fn parse_returns_unknown_on_garbage() {
        for bad in ["", "not-a-jwt", "only.two", "a.b.c"] {
            let info = parse(bad);
            assert_eq!(info.user_id, None, "{bad}");
            assert_eq!(info.status(), "unknown", "{bad}");
        }
    }

    #[test]
    fn status_thresholds_match_reference() {
        assert_eq!(
            TraeJwt {
                user_id: None,
                exp_hours: Some(25.0),
                exp_timestamp: None
            }
            .status(),
            "ok"
        );
        assert_eq!(
            TraeJwt {
                user_id: None,
                exp_hours: Some(24.0),
                exp_timestamp: None
            }
            .status(),
            "warn"
        );
        assert_eq!(
            TraeJwt {
                user_id: None,
                exp_hours: Some(-0.5),
                exp_timestamp: None
            }
            .status(),
            "expired"
        );
        assert_eq!(TraeJwt::default().status(), "unknown");
    }

    #[test]
    fn authorization_header_normalizes_once() {
        assert_eq!(authorization_header("abc"), "Cloud-IDE-JWT abc");
        assert_eq!(
            authorization_header("Cloud-IDE-JWT abc"),
            "Cloud-IDE-JWT abc"
        );
        assert_eq!(authorization_header("  Bearer abc  "), "Cloud-IDE-JWT abc");
        // 不重复加前缀（回归：曾出现 Cloud-IDE-JWT Cloud-IDE-JWT 双前缀）
        assert_eq!(
            authorization_header("Cloud-IDE-JWT Cloud-IDE-JWT abc"),
            "Cloud-IDE-JWT Cloud-IDE-JWT abc"
        );
    }
}
