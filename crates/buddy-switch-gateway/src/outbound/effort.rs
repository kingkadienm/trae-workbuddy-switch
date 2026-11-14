//! 模型档位目录（`reasoning_effort` 的静态能力表）与档位降级。
//!
//! 移植自参考实现 `internal/upstream/effort_catalog.go`。分 CN / Global 两张表：
//! 同一模型名在两域的可用档位与默认档位可能不同（例如 `deepseek-v4.1-flash`
//! 在 CN 为 `low/high/max`，在 Global 仅 `high`）。
//!
//! 约定：
//! - 远端目录（`/v3/config` 的 `reasoning.supportedEfforts`）非空时为**权威**，
//!   静态表仅作回落；本模块只负责静态表与降级算法。
//! - 档位强度序固定为 off < minimal < low < medium < high < xhigh < max。

use std::collections::HashMap;
use std::sync::OnceLock;

use buddy_switch_core::modules::region::Region;

/// DeepSeek 系模型的默认档位（静态表未给出 default 时的兜底）。
pub const DEFAULT_DEEPSEEK_EFFORT: &str = "high";

/// 单个模型的档位规格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortSpec {
    /// 支持档位（小写规范名）。
    pub efforts: Vec<String>,
    /// 默认档位；`None` 表示静态表未声明。
    pub default_effort: Option<String>,
}

impl EffortSpec {
    fn of(efforts: &[&str], default: Option<&str>) -> Self {
        Self {
            efforts: efforts.iter().map(|value| value.to_string()).collect(),
            default_effort: default.map(str::to_string),
        }
    }
}

/// 档位强度序；未知档位返回 `None`（调用方据此判定「非已知档，不处理」）。
///
/// 入参做 `trim` 容错（HTTP 客户端偶发带上空白），大小写敏感——
/// 与参考实现的 map 精确查找一致。
pub fn effort_rank(name: &str) -> Option<u8> {
    match name.trim() {
        "off" => Some(0),
        "minimal" => Some(1),
        "low" => Some(2),
        "medium" => Some(3),
        "high" => Some(4),
        "xhigh" => Some(5),
        "max" => Some(6),
        _ => None,
    }
}

/// 档位列表是否包含某档位（`EffortListing` 的 default 合法性判定）。
pub fn contains_effort(efforts: &[String], candidate: &str) -> bool {
    efforts.iter().any(|effort| effort == candidate)
}

/// 国内版（CN）静态档位表。
pub fn cn_effort_table() -> &'static HashMap<&'static str, EffortSpec> {
    static TABLE: OnceLock<HashMap<&'static str, EffortSpec>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = HashMap::new();
        table.insert(
            "deepseek-v4-flash",
            EffortSpec::of(&["low", "high", "max"], None),
        );
        table.insert(
            "deepseek-v4.1-flash",
            EffortSpec::of(&["low", "high", "max"], Some("high")),
        );
        table.insert(
            "deepseek-v4-pro",
            EffortSpec::of(&["low", "high", "xhigh"], Some("high")),
        );
        table.insert("hy4-preview", EffortSpec::of(&["high"], Some("high")));
        table.insert("hy4-preview-x", EffortSpec::of(&["high"], None));
        table.insert("hy3", EffortSpec::of(&["low", "high"], Some("high")));
        table.insert("hy3-x", EffortSpec::of(&["low", "high"], Some("high")));
        table.insert(
            "glm-5.3",
            EffortSpec::of(&["low", "high", "max"], Some("high")),
        );
        table.insert(
            "glm-5.3-flash",
            EffortSpec::of(&["low", "high", "max"], Some("high")),
        );
        table.insert("glm-5.2", EffortSpec::of(&["high", "xhigh"], Some("high")));
        table.insert("glm-5.1", EffortSpec::of(&["medium"], None));
        table.insert("glm-5v-turbo", EffortSpec::of(&["medium"], None));
        table.insert("kimi-k3-1", EffortSpec::of(&["medium"], None));
        table.insert("kimi-k2.7", EffortSpec::of(&["medium"], None));
        table.insert("kimi-k2.6", EffortSpec::of(&["medium"], None));
        table.insert("minimax-m3", EffortSpec::of(&["medium"], None));
        table
    })
}

/// 国际版（Global）静态档位表。
pub fn global_effort_table() -> &'static HashMap<&'static str, EffortSpec> {
    static TABLE: OnceLock<HashMap<&'static str, EffortSpec>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = HashMap::new();
        table.insert("fast-model", EffortSpec::of(&["medium"], None));
        table.insert("balanced-model", EffortSpec::of(&["medium"], None));
        table.insert("primary-model", EffortSpec::of(&["high"], None));
        table.insert("hy4-preview-f", EffortSpec::of(&["high"], Some("high")));
        table.insert("hy3", EffortSpec::of(&["low", "high"], Some("high")));
        // Global 的 deepseek-v4.1-flash 仅支持 high（与 CN 不同）。
        table.insert("deepseek-v4.1-flash", EffortSpec::of(&["high"], None));
        for model in [
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
        ] {
            table.insert(
                model,
                EffortSpec::of(&["low", "medium", "high", "xhigh", "max"], Some("high")),
            );
        }
        for model in ["gpt-5.5", "gpt-5.4"] {
            table.insert(
                model,
                EffortSpec::of(&["low", "medium", "high", "xhigh"], Some("high")),
            );
        }
        table.insert("gpt-5.3-codex", EffortSpec::of(&["medium"], None));
        table.insert("gemini-3.5-flash", EffortSpec::of(&["medium"], None));
        table.insert(
            "glm-5.3",
            EffortSpec::of(&["low", "high", "max"], Some("high")),
        );
        table.insert("glm-5.2", EffortSpec::of(&["high", "xhigh"], Some("high")));
        table.insert("kimi-k3", EffortSpec::of(&["medium"], None));
        table.insert("kimi-k2.6", EffortSpec::of(&["medium"], None));
        table
    })
}

/// 按 region 取静态表（`realm` 归一：Global 用国际表，其余用 CN 表）。
pub fn static_effort_cap(region: Region) -> &'static HashMap<&'static str, EffortSpec> {
    match region {
        Region::Global => global_effort_table(),
        Region::Cn => cn_effort_table(),
    }
}

/// 取该模型在静态表中的默认档位；未声明则回落 [`DEFAULT_DEEPSEEK_EFFORT`]。
pub fn lookup_default_effort(region: Region, model: &str) -> String {
    static_effort_cap(region)
        .get(model.trim())
        .and_then(|spec| spec.default_effort.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_DEEPSEEK_EFFORT.to_string())
}

/// 档位降级结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortAdjustment {
    /// 改写后的档位。
    pub effort: String,
    /// `true` 表示请求档位高于该模型所有支持档位，已落到最低支持档（floored）。
    pub floored: bool,
}

/// 按模型支持档位归一 `reasoning_effort`。
///
/// 返回 `None` 表示**不改写**（无档位信息 / 模型不在静态表 / 已是合法档）。
/// 规则（对照参考实现 `normalizeReasoningEffort`，模型名**精确匹配**）：
/// - 取「不高于请求档」中的最高支持档；
/// - 若所有支持档都高于请求档，取最低支持档（floored）；
/// - 与请求档相同则不改写。
pub fn adjust_effort(region: Region, model: &str, requested: &str) -> Option<EffortAdjustment> {
    let spec = static_effort_cap(region).get(model.trim())?;
    if spec.efforts.is_empty() {
        return None;
    }
    let requested_rank = effort_rank(requested)?;

    let mut at_or_below: Option<(u8, &str)> = None;
    for effort in &spec.efforts {
        let Some(rank) = effort_rank(effort) else {
            continue;
        };
        if rank <= requested_rank
            && at_or_below
                .map(|(current, _)| rank > current)
                .unwrap_or(true)
        {
            at_or_below = Some((rank, effort.as_str()));
        }
    }

    let (chosen, floored) = match at_or_below {
        Some((_, effort)) => (effort.to_string(), false),
        None => {
            let mut lowest: Option<(u8, &str)> = None;
            for effort in &spec.efforts {
                let Some(rank) = effort_rank(effort) else {
                    continue;
                };
                if lowest.map(|(current, _)| rank < current).unwrap_or(true) {
                    lowest = Some((rank, effort.as_str()));
                }
            }
            (lowest?.1.to_string(), true)
        }
    };

    if chosen == requested {
        return None;
    }
    Some(EffortAdjustment {
        effort: chosen,
        floored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_rank_orders_strength_and_rejects_unknown() {
        assert_eq!(effort_rank("off"), Some(0));
        assert_eq!(effort_rank("medium"), Some(3));
        assert_eq!(effort_rank("max"), Some(6));
        assert_eq!(effort_rank(" high "), Some(4), "空白容错");
        assert_eq!(effort_rank("HIGH"), None, "大小写敏感");
        assert_eq!(effort_rank("turbo"), None);
        assert_eq!(effort_rank(""), None);
    }

    #[test]
    fn default_effort_uses_table_then_falls_back_to_high() {
        assert_eq!(
            lookup_default_effort(Region::Cn, "deepseek-v4.1-flash"),
            "high"
        );
        // 表中无 default 声明 → 回落 high
        assert_eq!(
            lookup_default_effort(Region::Cn, "deepseek-v4-flash"),
            "high"
        );
        // 模型不在表中 → 回落 high
        assert_eq!(lookup_default_effort(Region::Cn, "unknown-model"), "high");
    }

    #[test]
    fn region_tables_differ_for_same_model() {
        // deepseek-v4.1-flash: CN 三档 / Global 仅 high —— 双域隔离的证据
        assert_eq!(cn_effort_table()["deepseek-v4.1-flash"].efforts.len(), 3);
        assert_eq!(
            global_effort_table()["deepseek-v4.1-flash"].efforts.len(),
            1
        );
        assert_eq!(lookup_default_effort(Region::Global, "glm-5.2"), "high");
        assert_eq!(lookup_default_effort(Region::Cn, "glm-5.1"), "high");
    }

    #[test]
    fn adjusts_down_to_nearest_supported() {
        // deepseek-v4-pro: low/high/xhigh，请求 max → 降到 xhigh（存在 ≤ 请求档的档位，非 floor）
        let adjustment = adjust_effort(Region::Cn, "deepseek-v4-pro", "max").unwrap();
        assert_eq!(adjustment.effort, "xhigh");
        assert!(!adjustment.floored);

        // gpt-5.5（Global）: low/medium/high/xhigh，请求 max → xhigh，同样非 floor
        let adjustment = adjust_effort(Region::Global, "gpt-5.5", "max").unwrap();
        assert_eq!(adjustment.effort, "xhigh");
        assert!(!adjustment.floored);
    }

    #[test]
    fn below_lowest_supported_is_a_floor() {
        // glm-5.2: high/xhigh，请求 medium（无 ≤medium 的档）→ 落到最低支持档 high
        let adjustment = adjust_effort(Region::Cn, "glm-5.2", "medium").unwrap();
        assert_eq!(adjustment.effort, "high");
        assert!(adjustment.floored, "无更低档位时必须标记 floored");
    }

    #[test]
    fn floors_when_all_supported_are_higher() {
        // glm-5.1: 仅 medium，请求 low（无 ≤low 的档）→ 落到最低支持档 medium
        let adjustment = adjust_effort(Region::Cn, "glm-5.1", "low").unwrap();
        assert_eq!(adjustment.effort, "medium");
        assert!(adjustment.floored, "全高于请求档必须标记 floored");
    }

    #[test]
    fn returns_none_when_no_change_needed() {
        assert_eq!(adjust_effort(Region::Cn, "glm-5.1", "medium"), None);
        assert_eq!(adjust_effort(Region::Cn, "hy4-preview-x", "high"), None);
    }

    #[test]
    fn returns_none_for_unknown_model_or_effort() {
        assert_eq!(adjust_effort(Region::Cn, "not-a-model", "high"), None);
        assert_eq!(adjust_effort(Region::Cn, "glm-5.2", "turbo"), None);
        assert_eq!(adjust_effort(Region::Cn, "glm-5.2", ""), None);
    }

    #[test]
    fn model_match_is_exact_not_prefix() {
        // 前缀相同的模型不得互相套用档位表
        assert_eq!(adjust_effort(Region::Cn, "glm-5.2-preview", "medium"), None);
        assert!(adjust_effort(Region::Cn, "glm-5.2", "medium").is_some());
    }

    #[test]
    fn global_single_effort_model_never_downgrades_silently() {
        // Global 的 deepseek-v4.1-flash 仅 high：请求 high 不改写
        assert_eq!(
            adjust_effort(Region::Global, "deepseek-v4.1-flash", "high"),
            None
        );
        // 请求 low → 全高于 → floored 到 high
        let adjustment = adjust_effort(Region::Global, "deepseek-v4.1-flash", "low").unwrap();
        assert_eq!(adjustment.effort, "high");
        assert!(adjustment.floored);
    }

    #[test]
    fn contains_effort_is_exact() {
        let efforts = vec!["low".to_string(), "high".to_string()];
        assert!(contains_effort(&efforts, "high"));
        assert!(!contains_effort(&efforts, "HIGH"));
        assert!(!contains_effort(&efforts, "max"));
    }
}
