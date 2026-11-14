//! 系统提示词体系：`passthrough` / `custom` 双模式与内容拦截降级门。
//!
//! 移植自参考实现 `internal/prompt/prompt.go`。两种模式的语义差异是刻意的：
//!
//! - **passthrough（默认）**：网关**不碰**客户端的 system/developer 消息，原样透传。
//!   这是缺省值——网关不应在用户不知情时替换其提示词。
//! - **custom**：网关用自己的提示词**替换**全部 system/developer 消息，从源头消除
//!   客户端模板句指纹（与 `sanitize` 的逐字改写互补：前者消 system 侧，后者洗
//!   user/assistant/tool 侧，两层独立不互替）。
//!
//! 降级门用于 `passthrough` 模式被上游内容审核拦截时的自救：改用中性提示词重试，
//! 并把该状态保持到次日 00:00（CST）。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::timeutil;

/// 内置默认提示词（`custom` 模式未指定 `prompt.file` 时使用）。
///
/// 内容移植自参考实现 `internal/prompt/defaultprompt.md`（MIT License,
/// 原始出处 https://github.com/Sliverkiss/workbuddy2api）。
pub const BUILTIN_SYSTEM_PROMPT: &str = include_str!("default_prompt.md");

/// 降级提示词：中性、不引导模型，仅保证请求可被受理。
pub const DEGRADED_SYSTEM_PROMPT: &str = "You are a helpful assistant. Respond in the user's language, follow the user's instructions, and be direct and concise.";

/// 提示词模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    /// 原样透传客户端 system/developer（缺省）。
    #[default]
    Passthrough,
    /// 用网关自带提示词替换。
    Custom,
}

impl PromptMode {
    /// 解析配置值；空值与缺失均回落 `passthrough`，非法值报错
    /// （**fail fast**，静默回落会让用户以为 custom 生效了却没生效）。
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "passthrough" => Ok(Self::Passthrough),
            "custom" => Ok(Self::Custom),
            other => Err(format!(
                "prompt.mode 取值非法：{other}（仅支持 passthrough / custom）"
            )),
        }
    }
}

/// 生效中的提示词设置。
#[derive(Debug, Clone, Default)]
pub struct PromptSettings {
    /// 模式。
    pub mode: PromptMode,
    /// 已加载的提示词正文（`passthrough` 下为空）。
    pub text: String,
}

impl PromptSettings {
    /// 按模式加载提示词。
    ///
    /// `custom` 且未给 `file` → 用内置默认；给了 `file` 但不可读 → **报错**
    /// （参考实现同样 fail fast：静默回落会让 custom 配置形同虚设）。
    pub fn load(mode: PromptMode, file: Option<&Path>) -> Result<Self, String> {
        match mode {
            PromptMode::Passthrough => Ok(Self {
                mode,
                text: String::new(),
            }),
            PromptMode::Custom => match file {
                None => Ok(Self {
                    mode,
                    text: BUILTIN_SYSTEM_PROMPT.to_string(),
                }),
                Some(path) => std::fs::read_to_string(path)
                    .map(|text| Self { mode, text })
                    .map_err(|error| {
                        format!("读取 prompt.file 失败（{}）：{error}", path.display())
                    }),
            },
        }
    }

    /// 本次请求应生效的替换提示词；`None` 表示不改写客户端 system。
    pub fn active_text(&self, degraded: bool) -> Option<&str> {
        match self.mode {
            PromptMode::Custom if !self.text.is_empty() => Some(self.text.as_str()),
            PromptMode::Passthrough if degraded => Some(DEGRADED_SYSTEM_PROMPT),
            _ => None,
        }
    }
}

/// 用 `system_prompt` 替换全部 system/developer 消息，并在头部插入一条 system。
///
/// - 解析失败或非对象 → 原样返回（不破坏不可解析的请求体）；
/// - 无 `messages` 字段（或非数组）→ 置为单条 system；
/// - user/assistant/tool 消息**逐字不动**。
pub fn rewrite_system_prompt(source: &str, system_prompt: &str) -> String {
    let Ok(mut body) = serde_json::from_str::<Value>(source) else {
        return source.to_string();
    };
    let Some(obj) = body.as_object_mut() else {
        return source.to_string();
    };

    match obj.get_mut("messages") {
        Some(Value::Array(messages)) => {
            messages.retain(|message| {
                !matches!(
                    message.get("role").and_then(Value::as_str),
                    Some("system") | Some("developer")
                )
            });
            messages.insert(0, json!({ "role": "system", "content": system_prompt }));
        }
        _ => {
            obj.insert(
                "messages".to_string(),
                json!([{ "role": "system", "content": system_prompt }]),
            );
        }
    }

    serde_json::to_string(&body).unwrap_or_else(|_| source.to_string())
}

/// 内容拦截降级门。
///
/// 语义：首次被上游内容审核拦截时开启降级期（截止次日 00:00 CST），期内所有
/// `passthrough` 请求直接改用降级提示词；**期内再次触发不延长截止时间**（不续期），
/// 避免被持续拦截时降级期无限滚动、永久偏离用户配置。
#[derive(Debug, Clone, Default)]
pub struct DegradeGate {
    until_ms: Option<i64>,
}

impl DegradeGate {
    /// 新建（未激活）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前是否处于降级期。
    pub fn active(&self, now_ms: i64) -> bool {
        self.until_ms.map(|until| now_ms < until).unwrap_or(false)
    }

    /// 降级截止时间（毫秒）。
    pub fn until_ms(&self) -> Option<i64> {
        self.until_ms
    }

    /// 触发降级；返回 `true` 表示**本次**开启了降级期（已在期内 → `false`，且不续期）。
    pub fn trigger(&mut self, now_ms: i64) -> bool {
        if self.active(now_ms) {
            return false;
        }
        self.until_ms = Some(timeutil::next_midnight(now_ms));
        true
    }

    /// 主动清除（用于测试与「手动恢复」）。
    pub fn clear(&mut self) {
        self.until_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cst_ms(hour: u32, minute: u32) -> i64 {
        use chrono::{NaiveDate, TimeZone};
        let naive = NaiveDate::from_ymd_opt(2026, 9, 16)
            .expect("合法日期")
            .and_hms_opt(hour, minute, 0)
            .expect("合法时刻");
        timeutil::cst_offset()
            .from_local_datetime(&naive)
            .single()
            .expect("无歧义")
            .timestamp_millis()
    }

    #[test]
    fn mode_parse_defaults_to_passthrough_and_rejects_unknown() {
        assert_eq!(PromptMode::parse("").unwrap(), PromptMode::Passthrough);
        assert_eq!(
            PromptMode::parse("  PASSTHROUGH ").unwrap(),
            PromptMode::Passthrough
        );
        assert_eq!(PromptMode::parse("custom").unwrap(), PromptMode::Custom);
        let error = PromptMode::parse("replace").unwrap_err();
        assert!(error.contains("replace"), "错误信息必须回显非法值: {error}");
    }

    #[test]
    fn passthrough_loads_no_text() {
        let settings = PromptSettings::load(PromptMode::Passthrough, None).unwrap();
        assert!(settings.text.is_empty());
        assert_eq!(
            settings.active_text(false),
            None,
            "passthrough 非降级期不改写"
        );
    }

    #[test]
    fn custom_without_file_uses_builtin() {
        let settings = PromptSettings::load(PromptMode::Custom, None).unwrap();
        assert_eq!(settings.text, BUILTIN_SYSTEM_PROMPT);
        assert!(settings.text.contains("核心立场"), "内置提示词应被内联编译");
        assert_eq!(settings.active_text(false), Some(BUILTIN_SYSTEM_PROMPT));
    }

    #[test]
    fn custom_with_missing_file_fails_fast() {
        let missing = std::path::Path::new("Z:/definitely/not/here/prompt.md");
        let error = PromptSettings::load(PromptMode::Custom, Some(missing)).unwrap_err();
        assert!(error.contains("prompt.file"), "错误须指明配置项: {error}");
    }

    #[test]
    fn passthrough_in_degrade_window_uses_degraded_prompt() {
        let settings = PromptSettings::load(PromptMode::Passthrough, None).unwrap();
        assert_eq!(settings.active_text(true), Some(DEGRADED_SYSTEM_PROMPT));
    }

    #[test]
    fn rewrite_replaces_all_system_and_developer_messages() {
        let source = r#"{"messages":[
            {"role":"system","content":"old system"},
            {"role":"developer","content":"old dev"},
            {"role":"user","content":"hi"},
            {"role":"assistant","content":"yo"}
        ]}"#;
        let rewritten: Value = serde_json::from_str(&rewrite_system_prompt(source, "NEW")).unwrap();
        let messages = rewritten["messages"].as_array().expect("数组");
        assert_eq!(messages.len(), 3, "两条旧提示词消息应被删除");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "NEW");
        assert_eq!(messages[1]["content"], "hi");
        assert_eq!(messages[2]["content"], "yo");
        for message in messages {
            assert_ne!(message["role"], "developer", "developer 也必须被清除");
        }
    }

    #[test]
    fn rewrite_inserts_single_system_when_messages_missing() {
        let rewritten: Value =
            serde_json::from_str(&rewrite_system_prompt(r#"{"model":"m"}"#, "NEW")).unwrap();
        assert_eq!(rewritten["model"], "m", "其它字段必须保留");
        assert_eq!(rewritten["messages"].as_array().expect("数组").len(), 1);
        assert_eq!(rewritten["messages"][0]["content"], "NEW");
    }

    #[test]
    fn rewrite_preserves_unparseable_body() {
        assert_eq!(rewrite_system_prompt("not json", "NEW"), "not json");
        assert_eq!(rewrite_system_prompt("[1,2]", "NEW"), "[1,2]");
    }

    #[test]
    fn degrade_gate_activates_until_next_midnight_and_does_not_extend() {
        let mut gate = DegradeGate::new();
        let now = cst_ms(10, 0);
        assert!(!gate.active(now));

        assert!(gate.trigger(now), "首次触发应开启降级期");
        assert!(gate.active(now));

        // 期内再次触发不续期：截止时间不变，且返回 false
        let later = cst_ms(20, 0);
        assert!(!gate.trigger(later), "期内触发不得续期");
        assert_eq!(gate.until_ms(), Some(timeutil::next_midnight(now)));

        // 跨过截止点后自动失效且可再次开启
        let after_midnight = timeutil::next_midnight(now) + 1;
        assert!(!gate.active(after_midnight));
        assert!(gate.trigger(after_midnight));
    }

    #[test]
    fn degrade_gate_clear_resets_state() {
        let mut gate = DegradeGate::new();
        let now = cst_ms(10, 0);
        gate.trigger(now);
        gate.clear();
        assert!(!gate.active(now));
        assert_eq!(gate.until_ms(), None);
    }
}
