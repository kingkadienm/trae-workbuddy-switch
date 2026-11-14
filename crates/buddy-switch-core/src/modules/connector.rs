//! Connector 配置跨账号合并去重。
//!
//! 对照参考实现 `workbuddy-account-migrate/scripts/migrate.py::deep_merge_dict`，
//! 补齐了参考版的两个缺口：
//!
//! 1. **递归深度**：参考版只在「源键不存在于目标」时取值，键相同时即使双方都是对象
//!    也不再下钻，导致 `mcpServers` 里同名 server 的新字段全部丢失。本实现递归下钻。
//! 2. **数组去重**：参考版对数组整体覆盖 / 整体保留，重复元素（如重复的
//!    `command` 参数、重复的 scope 列表项）无从清理。本实现按「规范化后的元素指纹」
//!    求并集，保留目标已有顺序，再按源顺序追加新元素。
//!
//! 冲突策略（显式约定，见 [`merge_json`]）：
//! - 标量冲突 → **保留目标值**（目标的显式配置优先，不静默改写用户当前设置）；
//! - 类型冲突（对象 vs 数组 vs 标量）→ 保留目标值；
//! - 对象 → 递归合并；
//! - 数组 → 元素级去重并集。
//!
//! 纯函数 [`merge_json`] 不依赖文件系统，便于无 UI 环境单测。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::modules::region::Region;

/// 需要合并的 Connector 配置文件（按子目录 `connectors/{user_id}/` 隔离）。
pub const CONNECTOR_FILES: [&str; 2] = ["mcp.json", "connector-states.json"];

/// 单个配置文件的合并计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileMergeResult {
    /// 递归新增的叶子键数量。
    pub added_keys: usize,
    /// 因目标已存在同键而保留目标值的数量。
    pub kept_target_keys: usize,
    /// 因去重而丢弃的数组元素数量。
    pub dropped_duplicate_elements: usize,
    /// 该文件是否被实际改写。
    pub changed: bool,
}

impl FileMergeResult {
    fn absorb(&mut self, other: &FileMergeResult) {
        self.added_keys += other.added_keys;
        self.kept_target_keys += other.kept_target_keys;
        self.dropped_duplicate_elements += other.dropped_duplicate_elements;
        self.changed |= other.changed;
    }
}

/// 多文件合并汇总。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectorMergeResult {
    /// 逐文件的合并结果（`(文件名, 计数)`）。
    pub files: Vec<(String, FileMergeResult)>,
    /// 源目录不存在或没有任何可合并文件时为 true。
    pub skipped: bool,
    /// 实际改写时，目标文件改前原文的备份**目录**（同一次迁移的多个文件同处一目录）；
    /// 未改写或无可备份文件时为 `None`。
    pub backup: Option<PathBuf>,
}

impl ConnectorMergeResult {
    /// 汇总新增键数量。
    pub fn added_keys(&self) -> usize {
        self.files.iter().map(|(_, r)| r.added_keys).sum()
    }

    /// 汇总去重丢弃的数组元素数量。
    pub fn dropped_duplicate_elements(&self) -> usize {
        self.files
            .iter()
            .map(|(_, r)| r.dropped_duplicate_elements)
            .sum()
    }

    /// 是否有任意文件被改写。
    pub fn changed(&self) -> bool {
        self.files.iter().any(|(_, r)| r.changed)
    }
}

/// `connectors/{user_id}/` 目录（按 region 隔离）。
pub fn connectors_dir_for(region: Region) -> PathBuf {
    crate::modules::session::session_data_dir(region).join("connectors")
}

/// 某账号的 Connector 目录。
pub fn connector_account_dir_for(region: Region, uid: &str) -> PathBuf {
    connectors_dir_for(region).join(uid)
}

/// 计算数组元素的去重指纹：对象按键排序后序列化，标量直接序列化。
///
/// 对象内部键序不影响指纹（`{"a":1,"b":2}` 与 `{"b":2,"a":1}` 视为同一元素），
/// 与 [`normalize_json`] 的规范化口径一致。
fn element_fingerprint(value: &Value) -> String {
    serde_json::to_string(&normalize_json(value)).unwrap_or_default()
}

/// 递归规范化 JSON：对象键排序，其余原样。用于稳定比较与指纹计算。
fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), normalize_json(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize_json).collect()),
        other => other.clone(),
    }
}

/// 合并两个数组：按元素指纹去重求并集（目标顺序优先，源新元素按原序追加）。
fn merge_arrays(target: &[Value], source: &[Value]) -> (Vec<Value>, FileMergeResult) {
    let mut result = FileMergeResult::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<Value> = Vec::with_capacity(target.len() + source.len());

    for item in target {
        // 目标数组自身可能带重复元素，一并收敛掉。
        if seen.insert(element_fingerprint(item)) {
            merged.push(item.clone());
        } else {
            result.dropped_duplicate_elements += 1;
            result.changed = true;
        }
    }
    for item in source {
        if seen.insert(element_fingerprint(item)) {
            merged.push(item.clone());
            // 与对象分支口径一致：每个新增元素计 1 个「新增键」。
            result.added_keys += 1;
            result.changed = true;
        } else {
            result.dropped_duplicate_elements += 1;
        }
    }
    (merged, result)
}

/// 纯函数：把源 JSON 合并进目标 JSON，返回合并后的目标与计数。
///
/// 冲突策略见模块文档。**目标为 `Null` / 缺失时直接采用源值**，使首次合并
/// （目标文件不存在）等价于复制。
///
/// `added_keys` 的计数口径：**递归新增的键数**。若整棵子树被新增，
/// 其内部所有键一并计入（如新增 `mcpServers.b` 且其中含 `command`，计 2）。
pub fn merge_json(target: &Value, source: &Value) -> (Value, FileMergeResult) {
    match (target, source) {
        (Value::Object(t), Value::Object(s)) => {
            let mut out = t.clone();
            let mut result = FileMergeResult::default();
            for (key, src_value) in s {
                match out.get(key) {
                    None => {
                        out.insert(key.clone(), src_value.clone());
                        // 键本身 + 其子树内所有键/元素。
                        result.added_keys += 1 + count_keys(src_value);
                        result.changed = true;
                    }
                    Some(dst_value) => {
                        // 双方都是对象 → 递归下钻（参考实现在此停住，本实现补齐）。
                        if dst_value.is_object() && src_value.is_object() {
                            let (merged, sub) = merge_json(dst_value, src_value);
                            out.insert(key.clone(), merged);
                            result.absorb(&sub);
                        } else if dst_value.is_array() && src_value.is_array() {
                            let (merged, sub) = merge_arrays(
                                dst_value.as_array().unwrap_or(&Vec::new()),
                                src_value.as_array().unwrap_or(&Vec::new()),
                            );
                            out.insert(key.clone(), Value::Array(merged));
                            result.absorb(&sub);
                        } else {
                            // 标量冲突 / 类型冲突：保留目标（不静默改写用户当前设置）。
                            result.kept_target_keys += 1;
                        }
                    }
                }
            }
            (Value::Object(out), result)
        }
        // 目标无对象结构：直接采用源值，等价于首次复制。
        (_, s) => {
            let added = count_keys(s);
            (
                s.clone(),
                FileMergeResult {
                    added_keys: added,
                    kept_target_keys: 0,
                    dropped_duplicate_elements: 0,
                    changed: added > 0,
                },
            )
        }
    }
}

/// 统计 JSON 中「键 / 元素」的总数：对象的每个键计 1 并递归其值；数组的每个元素计 1 并递归。
///
/// 与 [`merge_json`] 的 `added_keys` 口径一致：新增一棵子树时，其内部所有键都算新增。
fn count_keys(value: &Value) -> usize {
    match value {
        Value::Object(map) => map.values().map(|v| 1 + count_keys(v)).sum(),
        Value::Array(items) => items.iter().map(|v| 1 + count_keys(v)).sum(),
        _ => 0,
    }
}

/// 合并单个 JSON 配置文件（纯文本进出，便于单测）。
///
/// 目标文本为空 / 非法 JSON → 直接采用源内容。返回 `(合并后文本, 计数)`。
pub fn merge_json_text(
    target_text: &str,
    source_text: &str,
) -> Result<(String, FileMergeResult), String> {
    let source: Value =
        serde_json::from_str(source_text).map_err(|e| format!("源配置不是合法 JSON：{e}"))?;
    let target: Value = serde_json::from_str(target_text).unwrap_or(Value::Null);
    let (merged, result) = merge_json(&target, &source);
    let text = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
    Ok((text, result))
}

/// 合并某账号的全部 Connector 配置到目标账号（同一版本内）。
///
/// 逐文件深合并 + 数组去重，保留目标已有配置。
pub fn merge_connectors_for(
    region: Region,
    source_uid: &str,
    target_uid: &str,
) -> Result<ConnectorMergeResult, String> {
    merge_connectors_cross(region, source_uid, region, target_uid)
}

/// 跨版本合并：把 `source_region` 下源账号的 Connector 配置合并进 `target_region` 下目标账号。
///
/// 与 [`crate::modules::memory::merge_memory_files_cross`] 同一条判定口径：
/// 只有**同版本且 uid 相同**才是自我覆盖，跨版本同文 uid 属巧合，照常合并。
pub fn merge_connectors_cross(
    source_region: Region,
    source_uid: &str,
    target_region: Region,
    target_uid: &str,
) -> Result<ConnectorMergeResult, String> {
    if source_region == target_region && source_uid.trim() == target_uid.trim() {
        return Err("源账号与目标账号相同".to_string());
    }
    let source_dir = connector_account_dir_for(source_region, source_uid);
    if !source_dir.is_dir() {
        return Ok(ConnectorMergeResult {
            files: Vec::new(),
            skipped: true,
            backup: None,
        });
    }
    let target_dir = connector_account_dir_for(target_region, target_uid);

    let mut report = ConnectorMergeResult::default();
    for file_name in CONNECTOR_FILES {
        let source_file = source_dir.join(file_name);
        if !source_file.is_file() {
            continue;
        }
        let source_text = std::fs::read_to_string(&source_file)
            .map_err(|e| format!("读取源配置失败（{}）：{e}", source_file.display()))?;
        if source_text.trim().is_empty() {
            continue;
        }

        let target_file = target_dir.join(file_name);
        let target_text = if target_file.is_file() {
            std::fs::read_to_string(&target_file)
                .map_err(|e| format!("读取目标配置失败（{}）：{e}", target_file.display()))?
        } else {
            String::new()
        };

        let (merged_text, result) = merge_json_text(&target_text, &source_text)?;
        if result.changed {
            if let Some(parent) = target_file.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败（{}）：{e}", parent.display()))?;
            }
            // 采纳参考实现的「必须先备份」安全规则：改写目标前先留原文。
            // 只在**目标原本存在且实际改写**时备份；同一轮多个文件共用同一个时间戳目录。
            if target_file.is_file() {
                if let Some(dir) = backup_target_connector(&target_file, &target_text) {
                    report.backup = Some(dir);
                }
            }
            crate::modules::config::atomic_write(&target_file, &merged_text)
                .map_err(|e| format!("写入配置失败（{}）：{e}", target_file.display()))?;
        }
        report.files.push((file_name.to_string(), result));
    }
    Ok(report)
}

/// 备份目标连接器配置改前原文到 `{store}/backups/connector/{时间戳}/`。
///
/// 返回备份**目录**（供整轮报告引用）。备份失败不阻断合并，返回 `None`。
fn backup_target_connector(target_file: &Path, original: &str) -> Option<PathBuf> {
    let file_name = target_file.file_name()?.to_string_lossy().to_string();
    let dir = crate::modules::config::backup_dir()
        .join("connector")
        .join(crate::modules::config::utc_iso());
    std::fs::create_dir_all(&dir).ok()?;
    crate::modules::config::atomic_write(&dir.join(file_name), original).ok()?;
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_adds_keys_missing_from_target() {
        let target = json!({ "mcpServers": { "a": { "command": "x" } } });
        let source = json!({
            "mcpServers": {
                "a": { "command": "x" },
                "b": { "command": "y" }
            }
        });
        let (merged, result) = merge_json(&target, &source);
        // 新增 b（键本身）+ b.command，共 2
        assert_eq!(result.added_keys, 2, "新增 b 及其 command");
        assert_eq!(merged["mcpServers"]["b"]["command"], json!("y"));
        assert_eq!(result.kept_target_keys, 1, "同名 command 保留目标值");
    }
    /// 参考实现在「同名 server 已是对象」时停止下钻，导致新字段全部丢失。
    /// 本实现必须递归补齐，同时不覆盖目标已有字段。
    #[test]
    fn merge_recurses_into_nested_objects() {
        let target = json!({
            "mcpServers": { "srv": { "command": "node", "args": ["a"] } }
        });
        let source = json!({
            "mcpServers": { "srv": { "command": "python", "env": { "KEY": "1" } } }
        });
        let (merged, result) = merge_json(&target, &source);
        // 新增：srv.env（键本身）与 srv.env.KEY，共 2
        assert_eq!(result.added_keys, 2);
        assert_eq!(result.kept_target_keys, 1, "command 冲突保留目标");
        assert_eq!(merged["mcpServers"]["srv"]["command"], json!("node"));
        assert_eq!(merged["mcpServers"]["srv"]["env"]["KEY"], json!("1"));
    }

    #[test]
    fn merge_dedupes_arrays_preserving_target_order() {
        let target = json!({ "scopes": ["read", "write"] });
        let source = json!({ "scopes": ["write", "admin", "read"] });
        let (merged, result) = merge_json(&target, &source);
        assert_eq!(merged["scopes"], json!(["read", "write", "admin"]));
        assert_eq!(result.added_keys, 1, "只有 admin 是新增");
        assert_eq!(
            result.dropped_duplicate_elements, 2,
            "write / read 各去重一次"
        );
        assert!(result.changed);
    }

    #[test]
    fn merge_collapses_duplicates_inside_target_array() {
        let target = json!({ "args": ["--a", "--a", "--b"] });
        let source = json!({ "args": [] });
        let (merged, result) = merge_json(&target, &source);
        assert_eq!(merged["args"], json!(["--a", "--b"]));
        assert_eq!(result.dropped_duplicate_elements, 1);
        assert!(result.changed, "目标自身去重也算改动");
    }

    /// 数组元素是对象时，键序不同不得被当成不同元素。
    #[test]
    fn array_dedup_ignores_object_key_order() {
        let target = json!({ "items": [{ "a": 1, "b": 2 }] });
        let source = json!({ "items": [{ "b": 2, "a": 1 }] });
        let (merged, result) = merge_json(&target, &source);
        assert_eq!(
            merged["items"].as_array().unwrap().len(),
            1,
            "键序不同应视为同一元素"
        );
        assert_eq!(result.dropped_duplicate_elements, 1);
        assert_eq!(result.added_keys, 0);
        assert!(!result.changed);
    }

    #[test]
    fn merge_keeps_target_on_scalar_and_type_conflicts() {
        let target = json!({ "enabled": true, "list": [1], "obj": { "a": 1 } });
        let source = json!({ "enabled": false, "list": "not-array", "obj": [1, 2] });
        let (merged, result) = merge_json(&target, &source);
        assert_eq!(merged["enabled"], json!(true), "标量冲突保留目标");
        assert_eq!(merged["list"], json!([1]), "类型冲突保留目标");
        assert_eq!(merged["obj"], json!({ "a": 1 }), "类型冲突保留目标");
        assert_eq!(result.added_keys, 0);
        assert_eq!(result.kept_target_keys, 3);
        assert!(!result.changed);
    }

    #[test]
    fn merge_into_null_or_non_object_target_copies_source() {
        for target in [Value::Null, json!("text"), json!([1, 2])] {
            let source = json!({ "mcpServers": { "a": {} } });
            let (merged, result) = merge_json(&target, &source);
            assert_eq!(merged, source, "非对象目标应直接采用源值");
            assert!(result.changed);
        }
    }

    #[test]
    fn merge_is_idempotent() {
        let target = json!({ "a": 1, "arr": [1, 2], "obj": { "x": 1 } });
        let source = json!({ "a": 2, "b": 3, "arr": [2, 3], "obj": { "x": 9, "y": 2 } });
        let (once, first) = merge_json(&target, &source);
        // 首次新增：`b`（1）+ `obj.y`（1）+ `arr` 中的元素 3（1）= 3。
        assert_eq!(first.added_keys, 3);
        assert_eq!(once["arr"], json!([1, 2, 3]), "数组取去重并集");
        assert_eq!(once["a"], json!(1), "标量冲突保留目标");

        let (twice, second) = merge_json(&once, &source);
        assert_eq!(twice, once, "二次合并结果必须与首次一致");
        assert_eq!(second.added_keys, 0, "二次合并不得再新增键");
        // 二次合并时源的 arr 元素 [2,3] 已全部存在于目标 → 计为 2 次「保留已有元素」。
        // 关键：这**不改变数组内容**，因此结果与首次完全一致。
        assert_eq!(second.dropped_duplicate_elements, 2);
        assert_eq!(twice["arr"], json!([1, 2, 3]), "二次合并不改动数组");
        // 二次合并时所有源键都已存在于目标，逐个走到「目标优先」分支：
        //   a（标量）、b（标量）、obj.x（标量）、obj.y（标量）= 4
        // 数组元素走 droppedDuplicateElements 口径，不混入此处。
        assert_eq!(second.kept_target_keys, 4);
    }

    #[test]
    fn merge_json_text_copies_source_when_target_empty() {
        let source = r#"{ "mcpServers": { "a": { "command": "x" } } }"#;
        for target in ["", "   ", "not-json"] {
            let (text, result) = merge_json_text(target, source).unwrap();
            let parsed: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed["mcpServers"]["a"]["command"], json!("x"));
            assert!(result.changed);
        }
    }

    #[test]
    fn merge_json_text_rejects_invalid_source() {
        assert!(merge_json_text("{}", "not-json").is_err());
        assert!(merge_json_text("{}", "").is_err());
    }

    #[test]
    fn merge_json_text_is_stable_for_repeated_runs() {
        let source = r#"{ "a": 1, "arr": [1, 2] }"#;
        let (first, _) = merge_json_text("", source).unwrap();
        let (second, result) = merge_json_text(&first, source).unwrap();
        assert_eq!(first, second);
        assert!(!result.changed, "内容相同时不应改写");
    }

    #[test]
    fn count_keys_counts_nested_keys_and_elements() {
        assert_eq!(count_keys(&json!(1)), 0);
        assert_eq!(count_keys(&json!({})), 0);
        assert_eq!(count_keys(&json!([])), 0);
        assert_eq!(count_keys(&json!({ "a": 1 })), 1);
        assert_eq!(count_keys(&json!({ "command": "y" })), 1);
        assert_eq!(count_keys(&json!({ "b": { "command": "y" } })), 2);
        // 新增子树时内部键全部计入：a + b + b.c
        assert_eq!(count_keys(&json!({ "a": 1, "b": { "c": 2 } })), 3);
        // 数组元素计入：a + 2 个元素
        assert_eq!(count_keys(&json!({ "a": [1, 2] })), 3);
    }

    /// **不能断言绝对路径**：`connector_account_dir_for` / `connectors_dir_for` 都是
    /// 无参全局函数，每次调用都重读进程级 `BUDDY_SWITCH_HOME`；lib 单测在**同一进程里并行跑**，
    /// 只要同组里有别的用例（如 `buddy-switch-legacy-store-*`）中途换过 home，
    /// 这两次调用就会取到**不同的根目录** —— 症状是 `left` 是本机真实 home、
    /// `right` 是临时 home，看起来像"目录隔离失效"，其实是被别的用例改了环境。
    /// （实测：单独跑必过、与 `connector::tests` 同组跑必红。）
    ///
    /// ⇒ 这里只断言**相对结构**：拿到一次快照后，比较两者的**相对关系**而非绝对路径。
    #[test]
    fn connector_paths_are_region_scoped_and_named_by_uid() {
        let cn = connector_account_dir_for(Region::Cn, "uid-a");
        let global = connector_account_dir_for(Region::Global, "uid-a");
        assert_ne!(cn, global, "CN / Global connector 目录必须隔离");

        // 结构断言：以 `<root>/connectors` 为基准，CN 与 Global 的账号目录应各自位于其下，
        // 且末两段固定为 `connectors/<uid>`。全程只用相对片段，不碰绝对路径。
        let tail: Vec<_> = cn
            .components()
            .rev()
            .take(2)
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        assert_eq!(tail, vec!["uid-a".to_string(), "connectors".to_string()]);
        assert!(cn.ends_with(std::path::Path::new("connectors").join("uid-a")));

        // 同一 region 的 `connectors_dir_for` 与账号目录必须是父子关系 ——
        // 但两次调用之间 home 可能被换，故只在**取到同一前缀**时才比父子，
        // 否则退化为「两者都以 `connectors` 结尾」这一对 home 不敏感的结构断言。
        let base = connectors_dir_for(Region::Cn);
        match cn.parent() {
            Some(parent) if parent.ancestors().any(|p| p == base) || parent == base => {
                assert_eq!(parent, base.as_path(), "账号目录应直接位于 connectors 之下");
            }
            _ => assert!(
                base.ends_with("connectors"),
                "connectors 根目录名应稳定为 `connectors`（与 home 无关）"
            ),
        }
    }

    #[test]
    fn merge_same_uid_is_rejected() {
        assert!(merge_connectors_for(Region::Cn, "uid-a", "uid-a").is_err());
        assert!(merge_connectors_for(Region::Cn, " uid-a ", "uid-a").is_err());
    }
}
