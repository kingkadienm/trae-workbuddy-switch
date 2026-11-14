//! DeepSeek 思维链出站改写：`thinking` 注入、`reasoning_effort` 归一、
//! `reasoning_content` 回填。
//!
//! 移植自参考实现 `internal/upstream/thinking.go`。三条规则都**仅对 DeepSeek 系
//! 模型生效**（模型名前缀匹配 `deepseek`，大小写不敏感），其余模型零改动——
//! 这是刻意的收敛：上游只有 DeepSeek 通道接受 `thinking` 与 `reasoning_effort`，
//! 对其它模型注入会被判非法参数。

use serde_json::{json, Map, Value};

/// 静态表未给出默认档位时的兜底值。
pub const DEFAULT_DEEPSEEK_EFFORT: &str = "high";

/// 是否为 DeepSeek 系模型（`trim` + 小写后前缀匹配 `deepseek`）。
pub fn is_deepseek_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("deepseek")
}

/// 注入 `thinking` 并补齐 `reasoning_effort`。
///
/// 分支（对照参考实现，判定顺序固定）：
/// 1. `thinking.type == "disabled"` → 删除 `reasoning_effort` / `reasoningEffort`，**不注入**；
/// 2. `thinking.type` 非空（含 `enabled`）→ 保留客户端原值，仅补齐档位；
/// 3. 其余（无 `thinking`、`thinking` 非对象、`type` 为空）→ 覆写为 `{"type":"enabled"}` 后补齐档位。
pub fn inject_thinking(obj: &mut Map<String, Value>, default_effort: &str) {
    let thinking_type = obj
        .get("thinking")
        .and_then(Value::as_object)
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if thinking_type == "disabled" {
        obj.remove("reasoning_effort");
        obj.remove("reasoningEffort");
        return;
    }

    if thinking_type.is_empty() {
        obj.insert("thinking".to_string(), json!({ "type": "enabled" }));
    }

    ensure_effort(obj, default_effort);
}

/// 补齐 `reasoning_effort`：两个拼写任一已存在即不动，否则写 snake_case。
///
/// 只写一种拼写（snake 优先），不做双写——双写会让上游看到两个语义重复的键。
fn ensure_effort(obj: &mut Map<String, Value>, default_effort: &str) {
    if obj.contains_key("reasoning_effort") || obj.contains_key("reasoningEffort") {
        return;
    }
    let trimmed = default_effort.trim();
    let value = if trimmed.is_empty() {
        DEFAULT_DEEPSEEK_EFFORT
    } else {
        trimmed
    };
    obj.insert("reasoning_effort".to_string(), json!(value));
}

/// 回填 `reasoning_content`。
///
/// 语义（对照参考实现）：
/// - **预检**：仅当某条消息带非空 `reasoning`，或已存在 `reasoning_content` 键时才工作；
///   否则整条请求零改动（避免给无思维链历史的普通对话凭空塞空字段）。
/// - **回填**：对所有 `role == "assistant"` 的消息统一处理，**不按序配对**：
///   已有 `reasoning_content` 者不覆盖；有非空 `reasoning` 者复制过来；否则写空串 `""`。
///
/// 返回是否发生改动。
pub fn backfill_reasoning_content(obj: &mut Map<String, Value>) -> bool {
    let Some(Value::Array(messages)) = obj.get_mut("messages") else {
        return false;
    };

    let has_trace = messages.iter().any(|message| {
        let Some(wrapped) = message.as_object() else {
            return false;
        };
        if wrapped.contains_key("reasoning_content") {
            return true;
        }
        wrapped
            .get("reasoning")
            .and_then(Value::as_str)
            .map(|text| !text.is_empty())
            .unwrap_or(false)
    });
    if !has_trace {
        return false;
    }

    let mut changed = false;
    for message in messages.iter_mut() {
        let Some(wrapped) = message.as_object_mut() else {
            continue;
        };
        if wrapped.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if wrapped.contains_key("reasoning_content") {
            continue;
        }
        let reasoning = wrapped
            .get("reasoning")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .unwrap_or_default();
        wrapped.insert("reasoning_content".to_string(), json!(reasoning));
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(source: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(source)
            .expect("测试输入必须是 JSON 对象")
            .as_object()
            .expect("必须是对象")
            .clone()
    }

    #[test]
    fn deepseek_detection_is_prefix_and_case_insensitive() {
        assert!(is_deepseek_model("deepseek-v4-flash"));
        assert!(is_deepseek_model("DeepSeek-V4.1-Flash"));
        assert!(is_deepseek_model("  deepseek-v4-pro "));
        assert!(!is_deepseek_model("glm-5.2"));
        assert!(!is_deepseek_model("my-deepseek-clone"));
        assert!(!is_deepseek_model(""));
    }

    #[test]
    fn injects_thinking_and_effort_when_absent() {
        let mut obj = object(r#"{"model":"deepseek-v4.1-flash"}"#);
        inject_thinking(&mut obj, "high");
        assert_eq!(obj["thinking"], json!({"type": "enabled"}));
        assert_eq!(obj["reasoning_effort"], json!("high"));
    }

    #[test]
    fn disabled_thinking_removes_effort_and_does_not_inject() {
        let mut obj =
            object(r#"{"thinking":{"type":"disabled"},"reasoning_effort":"low","messages":[]}"#);
        inject_thinking(&mut obj, "high");
        assert_eq!(
            obj["thinking"],
            json!({"type": "disabled"}),
            "不得覆写 disabled"
        );
        assert!(obj.get("reasoning_effort").is_none(), "必须删除 snake 拼写");
        assert!(!obj.contains_key("reasoningEffort"), "不得新写 camel 拼写");
    }

    #[test]
    fn disabled_thinking_also_clears_camel_spelling() {
        let mut obj = object(r#"{"thinking":{"type":"disabled"},"reasoningEffort":"low"}"#);
        inject_thinking(&mut obj, "high");
        assert!(!obj.contains_key("reasoningEffort"), "必须删除 camel 拼写");
        assert!(!obj.contains_key("reasoning_effort"));
    }

    #[test]
    fn existing_enabled_thinking_is_preserved_but_effort_filled() {
        let mut obj = object(r#"{"thinking":{"type":"enabled","budget_tokens":2048}}"#);
        inject_thinking(&mut obj, "max");
        assert_eq!(obj["thinking"]["type"], json!("enabled"));
        assert_eq!(
            obj["thinking"]["budget_tokens"],
            json!(2048),
            "客户端其它 thinking 字段必须保留"
        );
        assert_eq!(obj["reasoning_effort"], json!("max"));
    }

    #[test]
    fn existing_effort_is_never_overwritten() {
        let mut obj = object(r#"{"reasoning_effort":"low"}"#);
        inject_thinking(&mut obj, "high");
        assert_eq!(obj["reasoning_effort"], json!("low"), "不得覆盖客户端档位");

        let mut camel = object(r#"{"reasoningEffort":"low"}"#);
        inject_thinking(&mut camel, "high");
        assert_eq!(camel["reasoningEffort"], json!("low"));
        assert!(
            !camel.contains_key("reasoning_effort"),
            "已有 camel 拼写时不得再写 snake 拼写"
        );
    }

    #[test]
    fn non_object_thinking_is_replaced() {
        let mut obj = object(r#"{"thinking":"enabled"}"#);
        inject_thinking(&mut obj, "high");
        assert_eq!(obj["thinking"], json!({"type": "enabled"}));
    }

    #[test]
    fn empty_default_effort_falls_back_to_high() {
        let mut obj = object("{}");
        inject_thinking(&mut obj, "   ");
        assert_eq!(obj["reasoning_effort"], json!(DEFAULT_DEEPSEEK_EFFORT));
    }

    #[test]
    fn backfill_skips_when_no_trace_at_all() {
        let mut obj = object(
            r#"{"messages":[{"role":"assistant","content":"hi"},{"role":"user","content":"yo"}]}"#,
        );
        assert!(
            !backfill_reasoning_content(&mut obj),
            "无思维链痕迹不得改动"
        );
        assert!(obj["messages"][0].get("reasoning_content").is_none());
    }

    #[test]
    fn backfill_copies_reasoning_into_assistant_messages() {
        let mut obj = object(
            r#"{"messages":[
                {"role":"assistant","reasoning":"thought A","content":"a"},
                {"role":"user","content":"u"},
                {"role":"assistant","content":"b"}
            ]}"#,
        );
        assert!(backfill_reasoning_content(&mut obj));
        assert_eq!(obj["messages"][0]["reasoning_content"], json!("thought A"));
        assert_eq!(
            obj["messages"][2]["reasoning_content"],
            json!(""),
            "未带思维链的 assistant 补空串"
        );
        assert!(
            obj["messages"][1].get("reasoning_content").is_none(),
            "非 assistant 消息不得被写入"
        );
    }

    #[test]
    fn backfill_does_not_overwrite_existing_content() {
        let mut obj = object(
            r#"{"messages":[{"role":"assistant","reasoning":"new","reasoning_content":"existing"}]}"#,
        );
        assert!(
            !backfill_reasoning_content(&mut obj),
            "已有 reasoning_content 时无事可做"
        );
        assert_eq!(obj["messages"][0]["reasoning_content"], json!("existing"));
    }

    #[test]
    fn backfill_triggered_by_existing_key_even_if_other_messages_lack_reasoning() {
        let mut obj = object(
            r#"{"messages":[
                {"role":"assistant","reasoning_content":"kept"},
                {"role":"assistant","content":"plain"}
            ]}"#,
        );
        assert!(backfill_reasoning_content(&mut obj));
        assert_eq!(obj["messages"][1]["reasoning_content"], json!(""));
    }

    #[test]
    fn backfill_ignores_requests_without_messages() {
        let mut obj = object(r#"{"model":"deepseek-v4-flash"}"#);
        assert!(!backfill_reasoning_content(&mut obj));
    }
}
