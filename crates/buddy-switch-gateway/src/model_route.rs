//! 模型名前缀路由（`cn:` / `global:`）。
//!
//! # 与参考实现的关系（**注意这里有一处有意的安全取舍**）
//!
//! 参考实现允许客户端用模型名前缀覆盖路由，并把「Key 绑定 region」与「前缀路由」并存。
//! 本项目默认**只**按 API Key 绑定的 region 路由——这是刻意保留的安全属性：Key 的
//! region 与账号库的 region 强校验（安全红线 F），客户端无法自行把请求导向另一个域。
//!
//! 因此这里把前缀路由实现为**默认关闭的显式开关**（`allow_model_region_prefix`）：
//! - 开关关闭（默认）时，**同域前缀**仍然被剥离（纯归一化，不改路由，无安全影响）；
//!   但**跨域前缀**会被**明确拒绝**并给出可诊断的 400，而不是静默改名后让上游报
//!   「模型不存在」——后者会让人误以为是模型名写错，排查成本极高。
//! - 开关开启时，跨域前缀按前缀路由，并由既有的 region 校验兜底（选出的账号仍必须
//!   属于目标域，否则报 `region_mismatch`）。
//!
//! 这样「能力可用」与「默认安全」两者可以同时成立。

use serde_json::Value;

use buddy_switch_core::modules::region::Region;

/// 模型名解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRoute {
    /// 前缀声明的 region；`None` 表示无前缀。
    pub prefix_region: Option<Region>,
    /// 剥离前缀后的模型名。
    pub bare_model: String,
}

/// 解析模型名里的 region 前缀。
///
/// 规则（对照参考实现 `resolve_model.go`，**大小写敏感**）：
/// 取**第一个** `:`，当且仅当它前面的整段精确等于 `cn` 或 `global` 时剥离并返回该域；
/// 否则视为无前缀，模型名原样保留（例如 `GPT:x` / `gpt:x` / `deepseek-v4-pro`）。
pub fn split_model_prefix(model: &str) -> ModelRoute {
    match model.split_once(':') {
        Some((head, tail)) => {
            let prefix_region = match head {
                "cn" => Some(Region::Cn),
                "global" => Some(Region::Global),
                _ => None,
            };
            match prefix_region {
                Some(region) => ModelRoute {
                    prefix_region: Some(region),
                    bare_model: tail.to_string(),
                },
                None => ModelRoute {
                    prefix_region: None,
                    bare_model: model.to_string(),
                },
            }
        }
        None => ModelRoute {
            prefix_region: None,
            bare_model: model.to_string(),
        },
    }
}

/// 前缀的处置结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixDecision {
    /// 无前缀：按 Key 绑定的 region 处理，模型名不变。
    NoPrefix,
    /// 同域前缀：仅剥离前缀（不改路由），**无需开关**。
    StripOnly,
    /// 跨域前缀且开关已开启：按前缀路由。
    Reroute(Region),
    /// 跨域前缀但开关关闭：拒绝，并附上可诊断的原因。
    Rejected {
        /// 前缀声明的域。
        requested: Region,
        /// Key 绑定的域。
        key_region: Region,
    },
}

/// 按「Key 绑定的 region + 开关」决定前缀如何处置。
pub fn decide(key_region: Region, allow_reroute: bool, route: &ModelRoute) -> PrefixDecision {
    match route.prefix_region {
        None => PrefixDecision::NoPrefix,
        Some(requested) if requested == key_region => PrefixDecision::StripOnly,
        Some(requested) if allow_reroute => PrefixDecision::Reroute(requested),
        Some(requested) => PrefixDecision::Rejected {
            requested,
            key_region,
        },
    }
}

/// 把请求体顶层的 `model` 字段改写为剥离前缀后的名字。
///
/// 不可解析 / 非对象 / 无 `model` 字段时**原样返回**——归一化不该成为新的失败点。
pub fn rewrite_body_model(raw: &str, bare_model: &str) -> String {
    let Ok(mut body) = serde_json::from_str::<Value>(raw) else {
        return raw.to_string();
    };
    let Some(obj) = body.as_object_mut() else {
        return raw.to_string();
    };
    if !obj.contains_key("model") {
        return raw.to_string();
    }
    obj.insert("model".to_string(), Value::String(bare_model.to_string()));
    serde_json::to_string(&body).unwrap_or_else(|_| raw.to_string())
}

/// 请求体里**有没有**顶层 `model` 字段。
///
/// 降级要改写 body 的模型名；body 缺这个字段时 [`rewrite_body_model`] 是 no-op，
/// 而中继会用它自己的默认值（配置默认模型 / 清单第一条）发请求 —— 我们**不知道**那是哪个。
/// ⇒ 这种情况下**不能降级**，否则每跳实际发同一个模型，却告诉客户端「降到了备选」。
pub fn body_has_model(raw: &str) -> bool {
    let Ok(body) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    body.get("model").is_some()
}

/// 拒绝跨域前缀时的对外文案。
///
/// 刻意**同时说明原因与出路**：只报「不支持」会让人反复试错。
pub fn rejection_message(requested: Region, key_region: Region) -> String {
    format!(
        "该 API Key 绑定的是{}，不能通过模型名前缀 `{}:` 请求{}。\
         如需访问{}，请在「API 服务」页为该版本单独创建一个 Key；\
         或由管理员在网关配置中显式开启 `allow_model_region_prefix` 后重试。",
        buddy_switch_core::modules::region::region_display(key_region),
        region_prefix(requested),
        buddy_switch_core::modules::region::region_display(requested),
        buddy_switch_core::modules::region::region_display(requested),
    )
}

/// 某 region 的前缀字面量。
pub fn region_prefix(region: Region) -> &'static str {
    match region {
        Region::Cn => "cn",
        Region::Global => "global",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_recognizes_both_prefixes_case_sensitively() {
        assert_eq!(
            split_model_prefix("global:glm-5.2"),
            ModelRoute {
                prefix_region: Some(Region::Global),
                bare_model: "glm-5.2".to_string(),
            }
        );
        assert_eq!(
            split_model_prefix("cn:deepseek-v4-pro"),
            ModelRoute {
                prefix_region: Some(Region::Cn),
                bare_model: "deepseek-v4-pro".to_string(),
            }
        );
    }

    #[test]
    fn split_ignores_non_region_heads_and_case_variants() {
        // 大小写敏感：`GPT` / `gpt` 都不是合法前缀，整串保留
        for model in ["GPT:x", "gpt:x", "Global:x", "CN:x", "deepseek-v4-pro"] {
            assert_eq!(
                split_model_prefix(model),
                ModelRoute {
                    prefix_region: None,
                    bare_model: model.to_string(),
                },
                "{model} 不应被识别为前缀"
            );
        }
    }

    #[test]
    fn split_takes_only_the_first_colon() {
        // 前段是 cn → 剥离第一段，其余冒号原样保留在模型名里
        assert_eq!(
            split_model_prefix("cn:vendor:model"),
            ModelRoute {
                prefix_region: Some(Region::Cn),
                bare_model: "vendor:model".to_string(),
            }
        );
        // 前段不是 cn/global → 整串保留（含后续冒号）
        assert_eq!(
            split_model_prefix("vendor:cn:model"),
            ModelRoute {
                prefix_region: None,
                bare_model: "vendor:cn:model".to_string(),
            }
        );
    }

    #[test]
    fn split_handles_empty_prefix_and_empty_tail() {
        assert_eq!(
            split_model_prefix("cn:"),
            ModelRoute {
                prefix_region: Some(Region::Cn),
                bare_model: String::new(),
            }
        );
        assert_eq!(
            split_model_prefix(""),
            ModelRoute {
                prefix_region: None,
                bare_model: String::new()
            }
        );
        assert_eq!(
            split_model_prefix(":x"),
            ModelRoute {
                prefix_region: None,
                bare_model: ":x".to_string()
            }
        );
    }

    #[test]
    fn same_region_prefix_is_stripped_without_reroute() {
        let route = split_model_prefix("cn:glm-5.2");
        assert_eq!(
            decide(Region::Cn, false, &route),
            PrefixDecision::StripOnly,
            "同域前缀即使开关关闭也应剥离（不改路由，无安全影响）"
        );
    }

    #[test]
    fn cross_region_prefix_is_rejected_by_default() {
        let route = split_model_prefix("global:glm-5.2");
        assert_eq!(
            decide(Region::Cn, false, &route),
            PrefixDecision::Rejected {
                requested: Region::Global,
                key_region: Region::Cn,
            },
            "跨域前缀在开关关闭时必须拒绝，而不是静默改名"
        );
    }

    #[test]
    fn cross_region_prefix_reroutes_only_when_enabled() {
        let route = split_model_prefix("global:glm-5.2");
        assert_eq!(
            decide(Region::Cn, true, &route),
            PrefixDecision::Reroute(Region::Global)
        );
    }

    #[test]
    fn no_prefix_keeps_key_region() {
        let route = split_model_prefix("glm-5.2");
        assert_eq!(decide(Region::Cn, false, &route), PrefixDecision::NoPrefix);
        assert_eq!(
            decide(Region::Global, true, &route),
            PrefixDecision::NoPrefix
        );
    }

    #[test]
    fn rewrite_replaces_top_level_model_only() {
        let raw = r#"{"model":"global:glm-5.2","messages":[{"role":"user","content":"keep"}]}"#;
        let rewritten: Value = serde_json::from_str(&rewrite_body_model(raw, "glm-5.2")).unwrap();
        assert_eq!(rewritten["model"], "glm-5.2");
        assert_eq!(
            rewritten["messages"][0]["content"], "keep",
            "其它字段必须原样保留"
        );
    }

    #[test]
    fn rewrite_is_noop_for_unusable_input() {
        assert_eq!(rewrite_body_model("not json", "x"), "not json");
        assert_eq!(rewrite_body_model("[1,2]", "x"), "[1,2]");
        // 无 model 字段：不改写，也不需要新增字段
        assert_eq!(rewrite_body_model(r#"{"a":1}"#, "x"), r#"{"a":1}"#);
    }

    #[test]
    fn rejection_message_names_both_regions_and_the_flag() {
        let message = rejection_message(Region::Global, Region::Cn);
        assert!(message.contains("global:"), "应回显被请求的前缀: {message}");
        assert!(
            message.contains("allow_model_region_prefix"),
            "应指出开关名: {message}"
        );
    }

    #[test]
    fn region_prefix_round_trips_with_split() {
        for region in [Region::Cn, Region::Global] {
            let model = format!("{}:glm-5.2", region_prefix(region));
            assert_eq!(
                split_model_prefix(&model).prefix_region,
                Some(region),
                "{model} 应解析回 {region:?}"
            );
        }
    }
}
