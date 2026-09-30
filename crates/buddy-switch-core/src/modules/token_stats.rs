//! 本地 WorkBuddy / CodeBuddy CLI / CodeBuddy IDE Token 统计。
//!
//! 这个模块是统计数据的唯一归属：日志只在这里解码、去重和按时间聚合，
//! Tauri 与 HTTP 层只负责转发结果。响应只包含聚合数字和脱敏标识，不返回
//! 消息正文、arguments 或认证信息。
//!
//! CodeBuddy IDE 不写 JSONL，用量在 `CodeBuddyExtension/Data/**/history/**/index.json`
//! 的 `requests[].usage` 中；消息正文文件（`messages/`）不会被扫描。

use chrono::{Datelike, Local, Timelike};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::modules::region::{Region, RegionFilter};
use crate::modules::usage;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Usage {
    input: u64,
    output: u64,
    read: u64,
    write: u64,
}

#[derive(Clone, Debug, Default)]
struct Totals {
    usage: Usage,
    records: u64,
}

#[derive(Clone, Debug)]
struct SessionTotals {
    key: String,
    title: Option<String>,
    project: String,
    session_id: String,
    totals: Totals,
}

impl Totals {
    fn add(&mut self, usage: Usage) {
        self.usage.input = self.usage.input.saturating_add(usage.input);
        self.usage.output = self.usage.output.saturating_add(usage.output);
        self.usage.read = self.usage.read.saturating_add(usage.read);
        self.usage.write = self.usage.write.saturating_add(usage.write);
        self.records = self.records.saturating_add(1);
    }

    fn value(&self) -> Value {
        let cache_hit_rate = (self.usage.input > 0)
            .then(|| self.usage.read as f64 / self.usage.input as f64);
        // `input` already includes cache reads; expose the same headline total
        // used by the dashboard without double-counting the cached portion.
        let total = self
            .usage
            .input
            .saturating_add(self.usage.output)
            .saturating_add(self.usage.write);
        json!({
            "total": total,
            "input": self.usage.input,
            "output": self.usage.output,
            "cacheRead": self.usage.read,
            "cacheWrite": self.usage.write,
            "uncachedInput": self.usage.input.saturating_sub(self.usage.read),
            "records": self.records,
            "cacheHitRate": cache_hit_rate,
        })
    }
}

/// Read a non-negative integer from a JSON number or string.
fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| value.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as u64))
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

fn field(object: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| object.get(*key).and_then(number))
}

fn positive_field(object: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(number)
            .filter(|value| *value > 0)
    })
}

fn cached_input_field(object: &Map<String, Value>) -> u64 {
    // Providers have emitted both flat aliases and OpenAI-compatible nested
    // details. Prefer a positive flat alias so a stale `cache_read...: 0`
    // field cannot hide a populated `prompt_cache_hit_tokens` value.
    positive_field(
        object,
        &[
            "cache_read_input_tokens",
            "cacheReadInputTokens",
            "prompt_cache_hit_tokens",
            "cached_tokens",
        ],
    )
    .or_else(|| {
        object
            .get("prompt_tokens_details")
            .and_then(Value::as_object)
            .and_then(|details| positive_field(details, &["cached_tokens"]))
    })
    .or_else(|| {
        object
            .get("inputTokensDetails")
            .and_then(Value::as_array)
            .and_then(|details| {
                details.iter().find_map(|detail| {
                    detail
                        .as_object()
                        .and_then(|detail| positive_field(detail, &["cached_tokens"]))
                })
            })
    })
    .unwrap_or(0)
}

const CACHE_WRITE_KEYS: &[&str] = &[
    "cache_write_input_tokens",
    "cacheWriteInputTokens",
    "cache_creation_input_tokens",
    "prompt_cache_write_tokens",
];

fn usage_fields(object: &Map<String, Value>) -> Usage {
    Usage {
        input: field(object, &["input_tokens", "inputTokens", "prompt_tokens"]).unwrap_or(0),
        output: field(
            object,
            &["output_tokens", "outputTokens", "completion_tokens"],
        )
        .unwrap_or(0),
        read: cached_input_field(object),
        write: positive_field(object, CACHE_WRITE_KEYS).unwrap_or(0),
    }
}

fn usage_object(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value?.as_object().filter(|object| {
        // Input is the required anchor for a usage record. It may legitimately
        // be zero (for example a provider reports output-only retries), so do
        // not use `input > 0` as the validity check.
        field(object, &["input_tokens", "inputTokens", "prompt_tokens"]).is_some()
    })
}

/// Decode one record. Usage precedence is message.usage > providerData.usage >
/// top-level usage. Cache-write metadata may only exist on a non-selected
/// usage object or rawUsage, so those objects are consulted without counting
/// their input/output again.
fn usage(value: &Value) -> Option<Usage> {
    let provider = value.get("providerData");
    let candidates = [
        value.get("message").and_then(|message| message.get("usage")),
        provider.and_then(|data| data.get("usage")),
        value.get("usage"),
    ];
    let selected = candidates.iter().copied().find_map(usage_object)?;
    let mut result = usage_fields(selected);

    if result.write == 0 {
        result.write = candidates
            .iter()
            .copied()
            .filter_map(|candidate| candidate.and_then(Value::as_object))
            .chain(
                provider
                    .and_then(|data| data.get("rawUsage"))
                    .and_then(Value::as_object),
            )
            .find_map(|object| positive_field(object, CACHE_WRITE_KEYS))
            .unwrap_or(0);
    }

    // prompt_cache_miss_tokens is deliberately not a write alias: current
    // WorkBuddy/CodeBuddy logs use it for newly computed (uncached) input,
    // while their explicit cache-write fields may legitimately remain zero.

    Some(result)
}

fn timestamp(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("ts"))
        .and_then(|timestamp| {
            timestamp
                .as_i64()
                .or_else(|| timestamp.as_u64().and_then(|n| i64::try_from(n).ok()))
                .or_else(|| timestamp.as_str()?.trim().parse::<i64>().ok())
        })
}

fn date(value: &Value) -> Option<String> {
    let timestamp = timestamp(value)?;
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|date| date.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

fn hour(value: &Value) -> Option<String> {
    let timestamp = timestamp(value)?;
    chrono::DateTime::from_timestamp_millis(timestamp).map(|date| {
        let local = date.with_timezone(&Local);
        format!("{}-{}", local.weekday().num_days_from_monday(), local.hour())
    })
}

fn model(value: &Value) -> String {
    value
        .get("providerData")
        .and_then(|data| data.get("model"))
        .or_else(|| value.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or("未知模型")
        .to_string()
}

fn files(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Subagent logs duplicate parent-session context and are not part
            // of either product's primary usage accounting.
            if path.file_name().and_then(|name| name.to_str()) != Some("subagents") {
                files(&path, output);
            }
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
            output.push(path);
        }
    }
}

fn project_name(root: &Path, file: &Path) -> String {
    let name = file
        .strip_prefix(root)
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|component| component.as_os_str().to_str())
        .filter(|name| !name.is_empty() && !name.ends_with(".jsonl"));
    match name {
        // Product directories commonly encode the complete absolute path.
        // Returning that would leak a user name and parent directories.
        Some(name) if !name.starts_with("Users-") && !name.starts_with("home-") => {
            name.to_string()
        }
        _ => "未知项目".to_string(),
    }
}

fn record_project(value: &Value, fallback: &str) -> String {
    value
        .get("cwd")
        .and_then(Value::as_str)
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 120)
        .unwrap_or(fallback)
        .to_string()
}

fn non_empty_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn groups(groups: HashMap<String, Totals>) -> Vec<Value> {
    let mut values: Vec<_> = groups
        .into_iter()
        .map(|(key, totals)| {
            let mut value = totals.value();
            value["key"] = json!(key);
            value
        })
        .collect();
    values.sort_by(|left, right| {
        total_value(right).cmp(&total_value(left))
    });
    values
}

fn session_groups(sessions: Vec<SessionTotals>) -> Vec<Value> {
    let mut values: Vec<_> = sessions
        .into_iter()
        .map(|session| {
            let mut value = session.totals.value();
            value["key"] = json!(session.key);
            value["title"] = json!(session.title);
            value["project"] = json!(session.project);
            value["sessionId"] = json!(session.session_id);
            value
        })
        .collect();
    values.sort_by(|left, right| {
        total_value(right).cmp(&total_value(left))
    });
    values
}

fn total_value(value: &Value) -> u64 {
    value
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            value
                .get("input")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_add(value.get("output").and_then(Value::as_u64).unwrap_or(0))
                .saturating_add(value.get("cacheWrite").and_then(Value::as_u64).unwrap_or(0))
        })
}

#[derive(Default)]
struct SourceCollector {
    total: Totals,
    models: HashMap<String, Totals>,
    projects: HashMap<String, Totals>,
    sessions: Vec<SessionTotals>,
    daily: HashMap<String, Totals>,
    daily_by_model: HashMap<String, HashMap<String, Totals>>,
    hours: HashMap<String, Totals>,
    parse_errors: u64,
    coverage_start_at: Option<i64>,
    coverage_end_at: Option<i64>,
    session_key_counts: HashMap<String, usize>,
}

impl SourceCollector {
    fn note_parse_error(&mut self) {
        self.parse_errors = self.parse_errors.saturating_add(1);
    }

    fn add(&mut self, usage: Usage, value: &Value, project: &str) {
        self.total.add(usage);
        let model_name = model(value);
        self.models
            .entry(model_name.clone())
            .or_default()
            .add(usage);
        self.projects
            .entry(project.to_string())
            .or_default()
            .add(usage);
        if let Some(day) = date(value) {
            self.daily.entry(day.clone()).or_default().add(usage);
            self.daily_by_model
                .entry(model_name)
                .or_default()
                .entry(day)
                .or_default()
                .add(usage);
        }
        if let Some(hour) = hour(value) {
            self.hours.entry(hour).or_default().add(usage);
        }
        if let Some(timestamp) = timestamp(value) {
            self.coverage_start_at = Some(
                self.coverage_start_at
                    .map_or(timestamp, |current| current.min(timestamp)),
            );
            self.coverage_end_at = Some(
                self.coverage_end_at
                    .map_or(timestamp, |current| current.max(timestamp)),
            );
        }
    }

    fn push_session(
        &mut self,
        session_id: String,
        title: Option<String>,
        project: String,
        totals: Totals,
    ) {
        if totals.records == 0 {
            return;
        }
        let base_key = format!("{project} · {session_id}");
        let count = self.session_key_counts.entry(base_key.clone()).or_default();
        *count += 1;
        let key = if *count == 1 {
            base_key
        } else {
            format!("{base_key} · {}", *count)
        };
        self.sessions.push(SessionTotals {
            key,
            title,
            project,
            session_id,
            totals,
        });
    }

    fn into_value(self, name: &str, files_scanned: usize) -> Value {
        let daily_by_model = self
            .daily_by_model
            .into_iter()
            .map(|(model, points)| (model, Value::Array(groups(points))))
            .collect::<Map<String, Value>>();
        json!({
            "source": name,
            "summary": self.total.value(),
            "models": groups(self.models),
            "projects": groups(self.projects),
            "sessions": session_groups(self.sessions),
            "daily": groups(self.daily),
            "dailyByModel": daily_by_model,
            "hours": groups(self.hours),
            "filesScanned": files_scanned,
            "parseErrors": self.parse_errors,
            "coverageStartAt": self.coverage_start_at,
            "coverageEndAt": self.coverage_end_at,
        })
    }
}

fn source(root: PathBuf, name: &str, cutoff: Option<i64>) -> Value {
    let mut paths = Vec::new();
    files(&root, &mut paths);
    let mut collector = SourceCollector::default();

    paths.sort();
    for path in &paths {
        let session_id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("未知会话")
            .to_string();
        let fallback_project = project_name(&root, path);
        let Ok(file) = std::fs::File::open(path) else {
            collector.note_parse_error();
            continue;
        };

        let mut session_totals = Totals::default();
        let mut session_project: Option<String> = None;
        let mut ai_title: Option<String> = None;
        let mut summary: Option<String> = None;

        for line in BufReader::new(file).lines() {
            let Ok(line) = line else {
                collector.note_parse_error();
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                collector.note_parse_error();
                continue;
            };
            // Title metadata belongs to the whole JSONL session file. Read it
            // before applying the usage cutoff so an older title can still
            // label usage that falls inside the selected range. aiTitle has
            // precedence over summary regardless of event order.
            if let Some(title) = non_empty_text(value.get("aiTitle")) {
                ai_title = Some(title);
            }
            if let Some(value) = non_empty_text(value.get("summary")) {
                summary = Some(value);
            }
            // Records with a missing timestamp are excluded from a bounded
            // range rather than guessed from file mtime or browser time.
            if cutoff.is_some_and(|minimum| timestamp(&value).is_none_or(|ts| ts < minimum)) {
                continue;
            }
            let Some(usage) = usage(&value) else {
                continue;
            };
            let project = record_project(&value, &fallback_project);
            if session_project.is_none() {
                session_project = Some(project.clone());
            }
            session_totals.add(usage);
            collector.add(usage, &value, &project);
        }

        collector.push_session(
            session_id,
            ai_title.or(summary),
            session_project.unwrap_or(fallback_project),
            session_totals,
        );
    }

    collector.into_value(name, paths.len())
}

fn codebuddy_extension_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CodeBuddyExtension")
        .join("Data")
}

fn is_ide_conversation_index(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("index.json")
        && path
            .parent()
            .and_then(|parent| parent.parent())
            .and_then(|parent| parent.parent())
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            == Some("history")
}

fn ide_index_files(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|name| name.to_str());
            // Message bodies contain chat content and must not be scanned.
            // Checkpoints and the shared Public bucket are unrelated to usage.
            if !matches!(name, Some("messages" | "check-point" | "backups" | "Public")) {
                ide_index_files(&path, output);
            }
        } else if is_ide_conversation_index(&path) {
            output.push(path);
        }
    }
}

fn decode_genie_workspace(name: &str) -> Option<String> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let try_decode = |value: &str| -> Option<String> {
        let padded = match value.len() % 4 {
            0 => value.to_string(),
            remainder => format!("{value}{}", "=".repeat(4 - remainder)),
        };
        let bytes = engine.decode(padded).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.contains('\0') {
            return None;
        }
        Some(trimmed.to_string())
    };
    try_decode(name)
        .or_else(|| try_decode(&name.replace('_', "/")))
        .or_else(|| try_decode(&name.replace('_', "+")))
}

fn ide_project_by_session() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(root) = crate::modules::vscode_cn_inject::codebuddy_cn_data_dir().map(|dir| {
        dir.join("User")
            .join("globalStorage")
            .join("tencent-cloud.coding-copilot")
            .join("genie-history")
    }) else {
        return map;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let project = decode_genie_workspace(folder)
            .as_deref()
            .map(Path::new)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= 120)
            .unwrap_or("未知项目")
            .to_string();
        let Ok(conversations) = std::fs::read_dir(path.join("conversations")) else {
            continue;
        };
        for conversation in conversations.flatten() {
            if let Some(id) = conversation.file_name().to_str() {
                if !id.is_empty() {
                    map.insert(id.to_string(), project.clone());
                }
            }
        }
    }
    map
}

/// 会话文件所在 workspace 的 `index.json` 路径。
///
/// 布局：`<workspace>/<会话目录>/<文件>` ⇒ `<workspace>/index.json`。
fn workspace_index_path(conv_index: &Path) -> Option<PathBuf> {
    Some(conv_index.parent()?.parent()?.join("index.json"))
}

/// 解析一份 workspace `index.json`，产出 `会话 id → (标题, 模型)` 查表。
///
/// 解析失败 / 无 `conversations` ⇒ 返回**空表**；调用方据此回落
/// `(None, "未知模型")`，与逐次调用时代的语义一致。
///
/// **同名 id 取首次出现**：改造前是「线性扫描、返回第一个匹配」，
/// 因此这里必须用 `entry().or_insert()` 而不是 `insert()` —— 后者会变成「最后一个赢」。
fn workspace_meta_index(ws_index: &Path) -> HashMap<String, (Option<String>, String)> {
    let mut table = HashMap::new();
    let Ok(text) = std::fs::read_to_string(ws_index) else {
        return table;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return table;
    };
    let Some(conversations) = value.get("conversations").and_then(Value::as_array) else {
        return table;
    };
    for conversation in conversations {
        // 无 `id` 或 `id` 为空串的条目**永远匹配不上**（调用方传的是会话**目录名**，
        // 空目录名会先被 `.filter(|name| !name.is_empty())` 换成 "未知会话"）⇒ 直接跳过。
        let Some(id) = conversation.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let title = non_empty_text(conversation.get("name"))
            .or_else(|| non_empty_text(conversation.get("title")));
        let model = non_empty_text(conversation.get("selectedModelId"))
            .or_else(|| non_empty_text(conversation.get("modelId")))
            .or_else(|| non_empty_text(conversation.get("model")))
            .unwrap_or_else(|| "未知模型".to_string());
        table.entry(id.to_string()).or_insert((title, model));
    }
    table
}

/// 按 workspace `index.json` 路径缓存查表，**同一份索引只读解析一次**。
///
/// ## 为什么需要（`docs/perf-audit-2026-09-24.md` 的 P0-4「无争议的那一半」）
///
/// 改造前 `ide_workspace_meta` 被调在**每个会话文件**的循环体内
/// （见 [`ide_source`]），而同一 workspace 下的 N 个会话指向**同一份** `index.json`
/// ⇒ 同一份索引被 `read_to_string` + `serde_json::from_str` **N 次**。
/// 这是**纯冗余**：与「是否缓存聚合结果」无关（后者才需要向作者确认）。
///
/// 本改动**不改变任何输出语义** —— 只是把「N 次解析」变成「1 次解析 + N 次查表」，
/// 故风险与收益不成比例的「全量结果缓存」不同，可以直接做。
#[derive(Default)]
struct WorkspaceMetaCache {
    tables: HashMap<PathBuf, HashMap<String, (Option<String>, String)>>,
    /// 只用于测试断言「同一份索引确实只解析一次」。
    ///
    /// 这是一个**可观测缝**：没有它就只能靠计时或读盘计数去间接推断，
    /// 两者都不可靠。`#[cfg(test)]` 保证发布构建零成本。
    #[cfg(test)]
    parses: usize,
}

impl WorkspaceMetaCache {
    /// 查某会话的 `(标题, 模型)`；查不到回落 `(None, "未知模型")`。
    fn lookup(&mut self, conv_index: &Path, conv_id: &str) -> (Option<String>, String) {
        let Some(ws_index) = workspace_index_path(conv_index) else {
            return (None, "未知模型".to_string());
        };
        let table = self.tables.entry(ws_index.clone()).or_insert_with(|| {
            #[cfg(test)]
            {
                self.parses += 1;
            }
            workspace_meta_index(&ws_index)
        });
        table
            .get(conv_id)
            .cloned()
            .unwrap_or_else(|| (None, "未知模型".to_string()))
    }
}


fn ide_request_usage(request: &Value) -> Option<Usage> {
    let object = request.get("usage")?.as_object()?;
    if field(object, &["inputTokens", "input_tokens", "prompt_tokens"]).is_none() {
        return None;
    }
    Some(Usage {
        input: field(object, &["inputTokens", "input_tokens", "prompt_tokens"]).unwrap_or(0),
        output: field(
            object,
            &["outputTokens", "output_tokens", "completion_tokens"],
        )
        .unwrap_or(0),
        read: field(
            object,
            &[
                "cacheTokens",
                "cacheReadInputTokens",
                "cache_read_input_tokens",
            ],
        )
        .unwrap_or(0),
        write: positive_field(
            object,
            &[
                "cachedWriteTokens",
                "cacheWriteInputTokens",
                "cache_write_input_tokens",
                "cache_creation_input_tokens",
            ],
        )
        .unwrap_or(0),
    })
}

fn ide_request_timestamp(request: &Value) -> Option<i64> {
    timestamp(request).or_else(|| {
        request.get("startedAt").and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
                .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
        })
    })
}

fn ide_source(
    root: PathBuf,
    name: &str,
    cutoff: Option<i64>,
    project_by_session: &HashMap<String, String>,
) -> Value {
    let mut paths = Vec::new();
    ide_index_files(&root, &mut paths);
    paths.sort();
    let mut collector = SourceCollector::default();
    // 同一 workspace 下的多个会话共用一份 `index.json`：提到循环外按路径解析一次。
    let mut workspace_meta = WorkspaceMetaCache::default();

    for path in &paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            collector.note_parse_error();
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            collector.note_parse_error();
            continue;
        };
        let Some(requests) = value.get("requests").and_then(Value::as_array) else {
            continue;
        };
        let session_id = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("未知会话")
            .to_string();
        let (title, model_name) = workspace_meta.lookup(path, &session_id);
        let fallback_project = project_by_session
            .get(&session_id)
            .cloned()
            .unwrap_or_else(|| "未知项目".to_string());
        let mut session_totals = Totals::default();
        let mut session_project: Option<String> = None;

        for request in requests {
            let ts = ide_request_timestamp(request);
            if cutoff.is_some_and(|minimum| ts.is_none_or(|value| value < minimum)) {
                continue;
            }
            let Some(usage) = ide_request_usage(request) else {
                continue;
            };
            let project = fallback_project.clone();
            if session_project.is_none() {
                session_project = Some(project.clone());
            }
            session_totals.add(usage);
            collector.add(
                usage,
                &json!({
                    "timestamp": ts,
                    "providerData": { "model": model_name },
                }),
                &project,
            );
        }

        collector.push_session(
            session_id,
            title,
            session_project.unwrap_or(fallback_project),
            session_totals,
        );
    }

    collector.into_value(name, paths.len())
}

/// 数值型 summary 字段：合并时**求和**。
///
/// `cacheHitRate` 刻意不在此列表——它是比例字段，必须按分子分母重算（见
/// [`merge_summary`]），直接相加会得到无意义的值（如 `0.5 + 0.75 = 1.25`）。
const SUMMED_SUMMARY_FIELDS: &[&str] = &[
    "total",
    "input",
    "output",
    "cacheRead",
    "cacheWrite",
    "uncachedInput",
    "records",
];

fn value_as_u64(value: Option<&Value>) -> u64 {
    value.and_then(number).unwrap_or(0)
}

/// 把同名分组项（`models`/`projects`/`daily`/`hours`/`sessions`）的数值字段求和。
///
/// `key_field` 为该字段的去重键名（`"key"` 或 `"sessionId"`）。非数值字段（`key` /
/// `title` / `project` / `sessionId`）保留首个非 null 值。
fn merge_group_item(base: &mut Value, other: &Value) {
    let Some(other_obj) = other.as_object() else {
        return;
    };
    let Some(base_obj) = base.as_object_mut() else {
        return;
    };
    for (field, other_value) in other_obj {
        if let Some(other_number) = number(other_value) {
            if base_obj.contains_key(field.as_str()) {
                let current = value_as_u64(base_obj.get(field.as_str()));
                base_obj.insert(field.clone(), json!(current.saturating_add(other_number)));
            } else {
                base_obj.insert(field.clone(), other_value.clone());
            }
        } else if !base_obj.contains_key(field.as_str())
            || base_obj.get(field.as_str()).is_some_and(Value::is_null)
        {
            base_obj.insert(field.clone(), other_value.clone());
        }
    }
    // summary 型分组项也可能带 cacheHitRate；重算以避免相加错误。
    recompute_cache_hit_rate(base_obj);
}

/// 若对象含有 `cacheRead` / `uncachedInput`，按 `cacheRead / (cacheRead + uncachedInput)`
/// 重算 `cacheHitRate`（分母为 0 → `null`）。
fn recompute_cache_hit_rate(object: &mut Map<String, Value>) {
    if !object.contains_key("cacheRead") && !object.contains_key("uncachedInput") {
        return;
    }
    let cache_read = value_as_u64(object.get("cacheRead"));
    let uncached = value_as_u64(object.get("uncachedInput"));
    let denominator = cache_read.saturating_add(uncached);
    let rate = (denominator > 0).then(|| cache_read as f64 / denominator as f64);
    object.insert("cacheHitRate".into(), json!(rate));
}

/// 合并 `summary`：数值字段求和；`cacheHitRate` 按合并后的 `cacheRead` / `uncachedInput`
/// **重算**（分母为 0 → `null`），绝不直接相加。
fn merge_summary(values: &[Value]) -> Value {
    let mut totals: Map<String, Value> = Map::new();
    for field in SUMMED_SUMMARY_FIELDS {
        let sum: u64 = values
            .iter()
            .map(|value| value_as_u64(value.get("summary").and_then(|s| s.get(*field))))
            .fold(0u64, u64::saturating_add);
        totals.insert((*field).to_string(), json!(sum));
    }
    // cacheHitRate 的分子/分母来自合并后的 cacheRead / uncachedInput。
    let cache_read = value_as_u64(totals.get("cacheRead"));
    let uncached = value_as_u64(totals.get("uncachedInput"));
    let denominator = cache_read.saturating_add(uncached);
    let rate = (denominator > 0).then(|| cache_read as f64 / denominator as f64);
    totals.insert("cacheHitRate".into(), json!(rate));
    Value::Object(totals)
}

/// 按去重键分组合并列表字段（`models` / `projects` / `daily` / `hours` / `sessions`）。
///
/// 同名 key 必须**合并为一条**（值为各版之和），不得出现两条同名条目；结果按 `total`
/// 降序排列，与 [`groups`] 的单版排序一致，保证前端渲染顺序稳定。
fn merge_grouped_list(field: &str, key_field: &str, values: &[Value]) -> Vec<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Value> = HashMap::new();
    for value in values {
        let Some(items) = value.get(field).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let key = item
                .get(key_field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match grouped.get_mut(&key) {
                Some(existing) => merge_group_item(existing, item),
                None => {
                    order.push(key.clone());
                    grouped.insert(key, item.clone());
                }
            }
        }
    }
    let mut values: Vec<Value> = order
        .into_iter()
        .filter_map(|key| grouped.remove(&key))
        .collect();
    values.sort_by(|left, right| total_value(right).cmp(&total_value(left)));
    values
}

/// 合并 `dailyByModel`：按 `model → date` 两维分组、同名 key 求和合并为一条。
fn merge_daily_by_model(values: &[Value]) -> Value {
    let mut models: Map<String, Value> = Map::new();
    let mut seen_models: Vec<String> = Vec::new();
    let mut collected: HashMap<String, Vec<Value>> = HashMap::new();
    for value in values {
        let Some(map) = value.get("dailyByModel").and_then(Value::as_object) else {
            continue;
        };
        for (model_name, points) in map {
            if !collected.contains_key(model_name) {
                seen_models.push(model_name.clone());
            }
            collected
                .entry(model_name.clone())
                .or_default()
                .push(points.clone());
        }
    }
    for model_name in seen_models {
        let points = collected.remove(&model_name).unwrap_or_default();
        let merged = merge_grouped_list_from_arrays("key", &points);
        models.insert(model_name, Value::Array(merged));
    }
    Value::Object(models)
}

/// 合并若干「已是数组」的分组序列（供 `dailyByModel` 的内部日期数组复用）。
fn merge_grouped_list_from_arrays(key_field: &str, arrays: &[Value]) -> Vec<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Value> = HashMap::new();
    for array in arrays {
        let Some(items) = array.as_array() else {
            continue;
        };
        for item in items {
            let key = item
                .get(key_field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match grouped.get_mut(&key) {
                Some(existing) => merge_group_item(existing, item),
                None => {
                    order.push(key.clone());
                    grouped.insert(key, item.clone());
                }
            }
        }
    }
    let mut result: Vec<Value> = order
        .into_iter()
        .filter_map(|key| grouped.remove(&key))
        .collect();
    result.sort_by(|left, right| total_value(right).cmp(&total_value(left)));
    result
}

/// 取 `coverageStartAt` 的最小值（缺失视为 `null`，若两版皆缺失则为 `null`）。
fn coverage_min(values: &[Value]) -> Value {
    let min = values
        .iter()
        .filter_map(|value| value.get("coverageStartAt").and_then(Value::as_i64))
        .min();
    json!(min)
}

/// 取 `coverageEndAt` 的最大值（缺失视为 `null`，若两版皆缺失则为 `null`）。
fn coverage_max(values: &[Value]) -> Value {
    let max = values
        .iter()
        .filter_map(|value| value.get("coverageEndAt").and_then(Value::as_i64))
        .max();
    json!(max)
}

/// 把同一 `source` 名下的多版（cn / global）聚合值合并为一个。
///
/// 合并语义：
/// - 数值字段（`summary.*`、`filesScanned`、`parseErrors`）**求和**；
/// - `cacheHitRate` **按合并后的 `cacheRead / (cacheRead + uncachedInput)` 重算**
///   （分母为 0 → `null`），**绝不直接相加**；
/// - `models` / `projects` / `sessions` / `daily` / `hours` 按各自去重键分组求和，
///   同名 key **合并为一条**（不得出现两条同名）；
/// - `dailyByModel` 按 `model → date` 两维分组求和；
/// - `coverageStartAt` 取 min、`coverageEndAt` 取 max（区间并集）。
///
/// 纯函数，可直接单测与变异注入验证。
pub fn merge_source_values(values: &[Value]) -> Value {
    let source_name = values
        .iter()
        .find_map(|value| value.get("source").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();

    let summary = merge_summary(values);
    let models = merge_grouped_list("models", "key", values);
    let projects = merge_grouped_list("projects", "key", values);
    let sessions = merge_grouped_list("sessions", "sessionId", values);
    let daily = merge_grouped_list("daily", "key", values);
    let hours = merge_grouped_list("hours", "key", values);
    let daily_by_model = merge_daily_by_model(values);

    let files_scanned: u64 = values
        .iter()
        .map(|value| value_as_u64(value.get("filesScanned")))
        .fold(0u64, u64::saturating_add);
    let parse_errors: u64 = values
        .iter()
        .map(|value| value_as_u64(value.get("parseErrors")))
        .fold(0u64, u64::saturating_add);

    json!({
        "source": source_name,
        "summary": summary,
        "models": models,
        "projects": projects,
        "sessions": sessions,
        "daily": daily,
        "dailyByModel": daily_by_model,
        "hours": hours,
        "filesScanned": files_scanned,
        "parseErrors": parse_errors,
        "coverageStartAt": coverage_min(values),
        "coverageEndAt": coverage_max(values),
    })
}

/// Return independent WorkBuddy, CodeBuddy CLI, and CodeBuddy IDE aggregates (CN).
/// `days` is interpreted in Rust using the same millisecond clock for every source.
pub fn get_statistics(days: Option<i64>) -> Value {
    get_statistics_for(crate::modules::region::Region::Cn, days)
}

/// Return the per-region token aggregates（旧签名薄包装，行为逐字节不变）。
///
/// CN keeps the three isolated sources (`.workbuddy/projects`, `.codebuddy/projects`,
/// CodeBuddy IDE). Global reads only its own client directory
/// (`.workbuddy-ai/projects`), so the two versions never mix (对照设计「统计按 region
/// 分别汇总」)。`days` is interpreted with the same millisecond clock for every source.
pub fn get_statistics_for(region: Region, days: Option<i64>) -> Value {
    get_statistics_for_filter(RegionFilter::from(region), days)
}

/// 采集某个具体 region 的原始 `sources[]`（不含顶层包装）。纯采集，供合并路径复用。
fn collect_sources_for(region: Region, cutoff: Option<i64>, home: &Path, ide_projects: &HashMap<String, String>) -> Vec<Value> {
    match region {
        Region::Cn => vec![
            source(home.join(".workbuddy/projects"), "workbuddy", cutoff),
            source(home.join(".codebuddy/projects"), "codebuddy-cli", cutoff),
            ide_source(
                codebuddy_extension_data_dir(),
                "codebuddy-ide",
                cutoff,
                ide_projects,
            ),
        ],
        Region::Global => vec![source(
            home.join(".workbuddy-ai/projects"),
            "workbuddy-ai",
            cutoff,
        )],
    }
}

/// `workbuddy-gateway` 源：本机网关（7863）逐请求用量桶（`modules::usage`，
/// 移植 panel 的 `internal/usage`：小时桶 90 天 → 折叠日桶长期保留）。
///
/// 客户端三源（`.workbuddy/projects` 等）只记录**IDE/CLI 内**的对话；
/// 经网关中转的调用（如外部 OpenAI 客户端）只存在于网关用量桶里。
/// 该源缺失时（网关从未跑过）返回「零值源」而非省略，前端来源切换条
/// 能如实展示「有来源、暂无数据」。
fn gateway_source(days: Option<i64>, filter: &RegionFilter) -> Value {
    let now_ms = crate::modules::config::now_ms();
    // 窗口口径与 panel 一致：`None` → 全部历史；`Some(d)` → 最近 d×24 个整点桶
    // （日桶不入小时窗口——90 天外的数据本就是日桶，days>90 时被排除属既有约定）。
    let hours = days.map(|d| d * 24).unwrap_or(0);
    let recorder = usage::UsageRecorder::load(Some(usage::usage_file()));
    let snap = usage::snapshot(&recorder, hours, filter, now_ms);

    let summary = gateway_group_value("gateway", snap.totals.clone());
    let group = |key: &str, agg: &usage::UsageAgg| gateway_group_value(key, agg.clone());
    let models = snap
        .by_model
        .iter()
        .map(|g| group(&g.key, &g.agg))
        .collect::<Vec<_>>();
    let daily_by_model: BTreeMap<String, Vec<Value>> = BTreeMap::new(); // 网关桶没有「按天×模型」交叉序列，前端缺省走 daily
    let _ = daily_by_model;
    // daily：按 scope 前缀 d: 的桶（长期）+ 把小时桶按日历日归并（近 90 天）。
    let mut daily: BTreeMap<String, usage::UsageAgg> = BTreeMap::new();
    for point in &snap.series {
        let date_key = if point.scope == "day" {
            point.t.clone()
        } else {
            point
                .t
                .split('T')
                .next()
                .unwrap_or(&point.t)
                .to_string()
        };
        merge_gateway_agg(&mut daily.entry(date_key).or_default(), &point.agg);
    }
    let daily_groups: Vec<Value> = daily
        .into_iter()
        .map(|(key, agg)| group(&key, &agg))
        .collect();
    let mut hours_map: BTreeMap<String, usage::UsageAgg> = BTreeMap::new();
    for point in &snap.series {
        if point.scope != "hour" {
            continue;
        }
        let key = gateway_hour_profile_key(&point.t);
        merge_gateway_agg(&mut hours_map.entry(key).or_default(), &point.agg);
    }
    let hours_groups: Vec<Value> = hours_map
        .into_iter()
        .map(|(key, agg)| group(&key, &agg))
        .collect();

    let (coverage_start, coverage_end) = gateway_coverage(&snap.series, &snap.since);

    json!({
        "source": "workbuddy-gateway",
        "summary": summary,
        "models": models,
        "projects": [],
        "sessions": [],
        "daily": daily_groups,
        "dailyByModel": {},
        "hours": hours_groups,
        "filesScanned": snap.buckets,
        "parseErrors": 0,
        "coverageStartAt": coverage_start,
        "coverageEndAt": coverage_end,
    })
}

/// 网关桶的分组值形状（与 `Totals::value` 对齐：cache 恒 0，命中率为 null——
/// 网关上游不回传 cache 字段，无口径可算）。
fn gateway_group_value(key: &str, agg: usage::UsageAgg) -> Value {
    json!({
        "total": agg.total_tokens,
        "input": agg.prompt_tokens,
        "output": agg.completion_tokens,
        "cacheRead": 0,
        "cacheWrite": 0,
        "uncachedInput": agg.prompt_tokens,
        "records": agg.requests,
        "cacheHitRate": None::<u64>,
        "key": key,
    })
}

fn merge_gateway_agg(target: &mut usage::UsageAgg, other: &usage::UsageAgg) {
    target.requests = target.requests.saturating_add(other.requests);
    target.errors = target.errors.saturating_add(other.errors);
    target.prompt_tokens = target.prompt_tokens.saturating_add(other.prompt_tokens);
    target.completion_tokens = target.completion_tokens.saturating_add(other.completion_tokens);
    target.total_tokens = target.total_tokens.saturating_add(other.total_tokens);
}

/// 小时桶 scope（"YYYY-MM-DDTHH"）→ 与客户端三源同形的「周几-小时」画像键。
fn gateway_hour_profile_key(scope_hour: &str) -> String {
    let (date, hour) = match scope_hour.split_once('T') {
        Some((d, h)) => (d, h),
        None => return scope_hour.to_string(),
    };
    let hour = hour.parse::<u32>().unwrap_or(0);
    let weekday = parse_hour_scope_weekday(date);
    format!("{weekday}-{hour}")
}

fn parse_hour_scope_weekday(date: &str) -> u32 {
    let (y, m, d) = match date.split('-').nth(0) {
        Some(y) => {
            let m = date.split('-').nth(1).unwrap_or("1");
            let d = date.split('-').nth(2).unwrap_or("1");
            (y, m, d)
        }
        None => return 1,
    };
    let (y, m, d) = match (y.parse::<i32>(), m.parse::<u32>(), d.parse::<u32>()) {
        (Ok(y), Ok(m), Ok(d)) => (y, m, d),
        _ => return 1,
    };
    match chrono::NaiveDate::from_ymd_opt(y, m, d) {
        Some(date) => date.weekday().num_days_from_monday(),
        None => 1,
    }
}

fn gateway_coverage(series: &[usage::UsagePoint], since: &Option<String>) -> (Option<i64>, Option<i64>) {
    let mut start: Option<i64> = None;
    let mut end: Option<i64> = None;
    for point in series {
        let ts = gateway_scope_start_ms(point.t.as_str(), point.scope);
        if let Some(ts) = ts {
            start = Some(start.map_or(ts, |v| v.min(ts)));
            end = Some(end.map_or(ts, |v| v.max(ts)));
        }
    }
    // 日桶窗口外的 since（最早小时桶）也纳入起点。
    if let Some(since) = since {
        let bare = since.strip_prefix('h').unwrap_or(since).strip_prefix(':').unwrap_or(since);
        if let Some(ts) = gateway_scope_start_ms(bare, "hour") {
            start = Some(start.map_or(ts, |v| v.min(ts)));
        }
    }
    (start, end)
}

/// scope 起始毫秒：hour → 该小时起点；day → 当日 00:00。解析失败返回 None。
fn gateway_scope_start_ms(bare: &str, scope: &str) -> Option<i64> {
    let local = match scope {
        "hour" => {
            let (date, hour) = bare.split_once('T')?;
            let (y, m, d) = split_date(date)?;
            let hour = hour.parse::<u32>().ok()?;
            let naive = chrono::NaiveDate::from_ymd_opt(y, m, d)?.and_hms_opt(hour, 0, 0)?;
            naive.and_local_timezone(chrono::Local).earliest()?
        }
        "day" => {
            let (y, m, d) = split_date(bare)?;
            let naive = chrono::NaiveDate::from_ymd_opt(y, m, d)?.and_hms_opt(0, 0, 0)?;
            naive.and_local_timezone(chrono::Local).earliest()?
        }
        _ => return None,
    };
    Some(local.timestamp_millis())
}

fn split_date(date: &str) -> Option<(i32, u32, u32)> {
    let mut parts = date.split('-');
    Some((
        parts.next()?.parse::<i32>().ok()?,
        parts.next()?.parse::<u32>().ok()?,
        parts.next()?.parse::<u32>().ok()?,
    ))
}

/// 按 region 过滤范围返回 Token 聚合。
///
/// - `Cn` -> 三源（`workbuddy` / `codebuddy-cli` / `codebuddy-ide`）；
/// - `Global` -> 单源（`workbuddy-ai`）；
/// - `All` -> cn 三源 + global 单源，按 `source` 分组后逐组调用 [`merge_source_values`]，
///   顶层 `region` 为 `"all"`。因两版源名天然不同，合并视图实际是**四源并集**
///   （PRD §6.1：不跨源合并 raw）；`merge_source_values` 保证未来出现同名源时仍正确聚合。
///
/// `region=all` **实时聚合，不缓存、不落库**。
pub fn get_statistics_for_filter(filter: RegionFilter, days: Option<i64>) -> Value {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let generated_at = crate::modules::config::now_ms();
    let range_days = match days {
        Some(7) => Some(7),
        Some(30) => Some(30),
        Some(90) => Some(90),
        _ => None,
    };
    let cutoff = range_days.map(|value| generated_at - value * 86_400_000);

    // 单版路径保持既有行为：直接返回该 region 的三源 / 单源。
    if let Some(region) = filter.single() {
        let ide_projects = ide_project_by_session();
        let mut sources = collect_sources_for(region, cutoff, &home, &ide_projects);
        sources.push(gateway_source(days, &region.into()));
        return json!({
            "region": region.as_str(),
            "generatedAt": generated_at,
            "rangeDays": range_days,
            "sources": sources,
        });
    }

    // 合并路径：采集两版全部源，按 source 名分组后逐组合并。
    let ide_projects = ide_project_by_session();
    let mut collected: Vec<Value> = Vec::new();
    for region in filter.regions() {
        collected.extend(collect_sources_for(region, cutoff, &home, &ide_projects));
    }
    // 网关源跨两版一份文件（桶自带 realm），只在合并视图追加一次，避免双重计数。
    collected.push(gateway_source(days, &RegionFilter::All));

    // 以 source 名为分组键，保持首次出现顺序（cn 三源在前，global 单源在后）。
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Vec<Value>> = HashMap::new();
    for value in collected {
        let name = value
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        if !grouped.contains_key(&name) {
            order.push(name.clone());
        }
        grouped.entry(name).or_default().push(value);
    }
    let sources: Vec<Value> = order
        .into_iter()
        .filter_map(|name| grouped.remove(&name))
        .map(|group| merge_source_values(&group))
        .collect();

    json!({
        "region": filter.as_str(),
        "generatedAt": generated_at,
        "rangeDays": range_days,
        "sources": sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn usage_priority_aliases_and_raw_cache_write() {
        let value = json!({
            "providerData": {
                "usage": { "inputTokens": 99, "outputTokens": 22 },
                "rawUsage": { "prompt_cache_write_tokens": 2 }
            },
            "message": { "usage": {
                "input_tokens": 10,
                "output_tokens": 3,
                "cache_read_input_tokens": 4
            }}
        });
        assert_eq!(usage(&value), Some(Usage { input: 10, output: 3, read: 4, write: 2 }));
    }

    #[test]
    fn cache_write_uses_explicit_aliases_but_never_cache_miss() {
        let provider_usage_write = json!({
            "providerData": {
                "usage": {
                    "inputTokens": 99,
                    "outputTokens": 22,
                    "cache_write_input_tokens": 0,
                    "cache_creation_input_tokens": 7
                },
                "rawUsage": {
                    "prompt_cache_miss_tokens": 91,
                    "prompt_cache_write_tokens": 0
                }
            },
            "message": { "usage": {
                "input_tokens": 10,
                "output_tokens": 3,
                "cache_read_input_tokens": 4
            }}
        });
        assert_eq!(
            usage(&provider_usage_write),
            Some(Usage {
                input: 10,
                output: 3,
                read: 4,
                write: 7,
            })
        );

        let cache_miss_only = json!({
            "providerData": {
                "rawUsage": { "prompt_cache_miss_tokens": 91 }
            },
            "message": { "usage": {
                "input_tokens": 10,
                "output_tokens": 3
            }}
        });
        assert_eq!(
            usage(&cache_miss_only),
            Some(Usage {
                input: 10,
                output: 3,
                read: 0,
                write: 0,
            })
        );
    }

    #[test]
    fn cache_read_accepts_nested_provider_details() {
        let value = json!({
            "providerData": {
                "usage": {
                    "inputTokens": 99,
                    "outputTokens": 3,
                    "inputTokensDetails": [{ "cached_tokens": 7 }]
                }
            }
        });
        assert_eq!(
            usage(&value),
            Some(Usage {
                input: 99,
                output: 3,
                read: 7,
                write: 0,
            })
        );

        let raw = json!({
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 2,
                "cache_read_input_tokens": 0,
                "prompt_cache_hit_tokens": 12
            }
        });
        assert_eq!(
            usage(&raw),
            Some(Usage {
                input: 20,
                output: 2,
                read: 12,
                write: 0,
            })
        );

    }

    #[test]
    fn source_excludes_subagents_and_counts_each_record_once() {
        let root = std::env::temp_dir().join(format!(
            "buddy-switch-token-stats-{}-{}",
            std::process::id(),
            crate::modules::config::now_ms()
        ));
        let project = root.join("fixture-project");
        let ignored = project.join("subagents");
        fs::create_dir_all(&ignored).expect("create fixture dirs");
        let record = json!({
            "timestamp": crate::modules::config::now_ms(),
            "providerData": {
                "model": "fixture-model",
                "usage": { "inputTokens": 20, "outputTokens": 5 }
            },
            "message": { "usage": {
                "input_tokens": 10,
                "output_tokens": 3,
                "cache_read_input_tokens": 4
            }}
        });
        fs::write(project.join("session.jsonl"), format!("{}\nnot-json\n", record))
            .expect("write fixture");
        fs::write(ignored.join("agent.jsonl"), format!("{}\n", record)).expect("write ignored fixture");

        let result = source(root.clone(), "fixture", None);
        assert_eq!(result["filesScanned"], 1);
        assert_eq!(result["parseErrors"], 1);
        assert_eq!(result["summary"]["input"], 10);
        assert_eq!(result["summary"]["output"], 3);
        assert_eq!(result["summary"]["cacheRead"], 4);
        assert_eq!(result["summary"]["total"], 13);
        assert_eq!(result["summary"]["records"], 1);
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(result["dailyByModel"]["fixture-model"][0]["key"], today);
        assert_eq!(result["projects"][0]["key"], "fixture-project");
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn bounded_source_excludes_records_before_cutoff() {
        let now = crate::modules::config::now_ms();
        let root = std::env::temp_dir().join(format!(
            "buddy-switch-token-stats-range-{}-{now}",
            std::process::id()
        ));
        let project = root.join("fixture-project");
        fs::create_dir_all(&project).expect("create fixture dirs");
        let record = |timestamp| {
            json!({
                "timestamp": timestamp,
                "cwd": "/fixture/example-project",
                "message": { "usage": { "input_tokens": 10, "output_tokens": 2 } }
            })
        };
        fs::write(
            project.join("session.jsonl"),
            format!("{}\n{}\n", record(now - 10_000), record(now - 100_000)),
        )
        .expect("write fixture");

        let result = source(root.clone(), "fixture", Some(now - 50_000));
        assert_eq!(result["summary"]["records"], 1);
        assert_eq!(result["summary"]["input"], 10);
        assert_eq!(result["projects"][0]["key"], "example-project");
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn session_titles_are_file_scoped_and_independent_from_usage_cutoff() {
        let now = crate::modules::config::now_ms();
        let root = std::env::temp_dir().join(format!(
            "buddy-switch-token-stats-titles-{}-{now}",
            std::process::id()
        ));
        let project = root.join("fixture-project");
        fs::create_dir_all(&project).expect("create fixture dirs");
        let usage_record = |input| {
            json!({
                "timestamp": now,
                "cwd": "/private/example-project",
                "message": { "usage": { "input_tokens": input, "output_tokens": 2 } }
            })
        };

        fs::write(
            project.join("session-a.jsonl"),
            format!(
                "{}\n{}\n{}\n{}\n",
                json!({ "type": "summary", "summary": "摘要不应覆盖 AI 标题" }),
                json!({ "type": "ai-title", "aiTitle": "旧标题" }),
                usage_record(10),
                json!({ "type": "ai-title", "aiTitle": "最新 AI 标题" }),
            ),
        )
        .expect("write ai title fixture");
        fs::write(
            project.join("session-b.jsonl"),
            format!(
                "{}\n{}\n",
                json!({
                    "type": "ai-title",
                    "timestamp": now - 100_000,
                    "aiTitle": "范围外保留标题"
                }),
                usage_record(20),
            ),
        )
        .expect("write cutoff title fixture");
        fs::write(
            project.join("session-c.jsonl"),
            format!(
                "{}\n{}\n",
                usage_record(30),
                json!({ "type": "summary", "summary": "摘要回退标题" }),
            ),
        )
        .expect("write summary fixture");
        fs::write(
            project.join("session-d.jsonl"),
            format!("{}\n", usage_record(40)),
        )
        .expect("write untitled fixture");
        fs::write(
            project.join("session-e.jsonl"),
            format!(
                "{}\n{}\n",
                json!({ "type": "ai-title", "aiTitle": "最新 AI 标题" }),
                usage_record(50),
            ),
        )
        .expect("write duplicate title fixture");

        let result = source(root.clone(), "fixture", Some(now - 50_000));
        let sessions = result["sessions"].as_array().expect("session groups");
        let by_id = |session_id: &str| {
            sessions
                .iter()
                .find(|session| session["sessionId"] == session_id)
                .expect("session group by id")
        };

        assert_eq!(result["summary"]["input"], 150);
        assert_eq!(result["summary"]["records"], 5);
        assert_eq!(sessions.len(), 5);
        assert_eq!(by_id("session-a")["title"], "最新 AI 标题");
        assert_eq!(by_id("session-b")["title"], "范围外保留标题");
        assert_eq!(by_id("session-c")["title"], "摘要回退标题");
        assert!(by_id("session-d")["title"].is_null());
        assert_eq!(by_id("session-a")["project"], "example-project");
        assert_ne!(by_id("session-a")["key"], by_id("session-e")["key"]);

        fs::remove_dir_all(root).expect("remove fixture");
    }

    /// P0-4「无争议的那一半」：同一 workspace 下的 N 个会话**只解析一次** `index.json`。
    ///
    /// 断言三件事：
    /// ① 每个会话拿到**自己**的标题/模型（证明查表按 conv id 分键，不是「第一条赢」）；
    /// ② 解析次数恰好 **1**（这是本改动的全部意义，且是**直接**断言、不靠计时推断）；
    /// ③ 改造前是 N 次 —— 该断言在改造前必红。
    #[test]
    fn workspace_meta_cache_parses_each_index_once() {
        let root = std::env::temp_dir().join(format!(
            "buddy-switch-wsmeta-{}-{}",
            std::process::id(),
            crate::modules::config::now_ms()
        ));
        let workspace = root.join("workspace-hash");
        for id in ["conv-a", "conv-b", "conv-c"] {
            fs::create_dir_all(workspace.join(id)).expect("create conv dir");
        }
        fs::write(
            workspace.join("index.json"),
            json!({
                "conversations": [
                    { "id": "conv-a", "name": "标题 A", "selectedModelId": "model-a" },
                    { "id": "conv-b", "title": "标题 B", "modelId": "model-b" },
                    { "id": "conv-c", "model": "model-c" }
                ]
            })
            .to_string(),
        )
        .expect("write workspace index");

        let mut cache = WorkspaceMetaCache::default();
        let a = cache.lookup(&workspace.join("conv-a").join("index.json"), "conv-a");
        let b = cache.lookup(&workspace.join("conv-b").join("index.json"), "conv-b");
        let c = cache.lookup(&workspace.join("conv-c").join("index.json"), "conv-c");

        assert_eq!(a, (Some("标题 A".to_string()), "model-a".to_string()));
        // `title` 是 `name` 缺失时的第二优先
        assert_eq!(b, (Some("标题 B".to_string()), "model-b".to_string()));
        // `model` 是第三优先；三个标题字段都缺 ⇒ 标题为 None，模型仍取自 model
        assert_eq!(c, (None, "model-c".to_string()));

        assert_eq!(
            cache.parses, 1,
            "同一份 index.json 三个会话只应解析一次（改造前是 3 次）"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// 查不到会话时的回落必须与逐次调用时代**逐字相同**：`(None, "未知模型")`。
    #[test]
    fn workspace_meta_cache_falls_back_exactly_like_the_linear_scan() {
        let root = std::env::temp_dir().join(format!(
            "buddy-switch-wsmeta-miss-{}-{}",
            std::process::id(),
            crate::modules::config::now_ms()
        ));
        let workspace = root.join("ws");
        fs::create_dir_all(workspace.join("conv-x")).expect("create conv dir");

        // ① index.json 不存在
        let mut cache = WorkspaceMetaCache::default();
        assert_eq!(
            cache.lookup(&workspace.join("conv-x").join("index.json"), "conv-x"),
            (None, "未知模型".to_string()),
            "索引缺失时必须回落"
        );

        // ② 索引存在但没有该会话
        fs::write(
            workspace.join("index.json"),
            json!({ "conversations": [{ "id": "other", "name": "别的会话" }] }).to_string(),
        )
        .expect("write index");
        let mut cache = WorkspaceMetaCache::default();
        assert_eq!(
            cache.lookup(&workspace.join("conv-x").join("index.json"), "conv-x"),
            (None, "未知模型".to_string()),
            "会话不在索引里时必须回落"
        );

        // ③ 索引是坏 JSON（解析失败也要回落，不得 panic）
        fs::write(workspace.join("index.json"), "{ not json").expect("write bad index");
        let mut cache = WorkspaceMetaCache::default();
        assert_eq!(
            cache.lookup(&workspace.join("conv-x").join("index.json"), "conv-x"),
            (None, "未知模型".to_string()),
            "索引损坏时必须回落"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// 同名 id **取首次出现** —— 改造前是「线性扫描返回第一个匹配」。
    ///
    /// 若把 `entry().or_insert()` 写成 `insert()`，本用例立刻变红（会拿到「后一条」）。
    #[test]
    fn workspace_meta_index_keeps_the_first_duplicate_id() {
        let root = std::env::temp_dir().join(format!(
            "buddy-switch-wsmeta-dup-{}-{}",
            std::process::id(),
            crate::modules::config::now_ms()
        ));
        let workspace = root.join("ws");
        fs::create_dir_all(workspace.join("conv-dup")).expect("create conv dir");
        fs::write(
            workspace.join("index.json"),
            json!({
                "conversations": [
                    { "id": "conv-dup", "name": "第一条", "selectedModelId": "first-model" },
                    { "id": "conv-dup", "name": "第二条", "selectedModelId": "second-model" }
                ]
            })
            .to_string(),
        )
        .expect("write index");

        let mut cache = WorkspaceMetaCache::default();
        assert_eq!(
            cache.lookup(&workspace.join("conv-dup").join("index.json"), "conv-dup"),
            (Some("第一条".to_string()), "first-model".to_string()),
            "同名 id 必须取首次出现"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ide_source_reads_request_usage_and_skips_message_bodies() {
        let now = crate::modules::config::now_ms();
        let root = std::env::temp_dir().join(format!(
            "buddy-switch-token-stats-ide-{}-{now}",
            std::process::id()
        ));
        let history = root
            .join("uid")
            .join("CodeBuddyIDE")
            .join("uid")
            .join("history")
            .join("workspace-hash");
        let conv_a = history.join("conv-a");
        let conv_b = history.join("conv-b");
        let messages = conv_a.join("messages");
        fs::create_dir_all(&messages).expect("create ide fixture dirs");
        fs::create_dir_all(&conv_b).expect("create second conversation");

        fs::write(
            history.join("index.json"),
            json!({
                "conversations": [
                    {
                        "id": "conv-a",
                        "name": "IDE 会话标题",
                        "selectedModelId": "deepseek-v4-flash"
                    },
                    {
                        "id": "conv-b",
                        "name": "范围外会话",
                        "selectedModelId": "hy4-preview"
                    }
                ]
            })
            .to_string(),
        )
        .expect("write workspace index");
        fs::write(
            conv_a.join("index.json"),
            json!({
                "messages": [{ "id": "m1", "role": "assistant", "isComplete": true }],
                "requests": [{
                    "id": "req-1",
                    "state": "complete",
                    "startedAt": now,
                    "usage": {
                        "inputTokens": 100,
                        "outputTokens": 20,
                        "cacheTokens": 40,
                        "cachedWriteTokens": 5
                    }
                }]
            })
            .to_string(),
        )
        .expect("write conversation index");
        fs::write(
            conv_b.join("index.json"),
            json!({
                "requests": [{
                    "id": "req-old",
                    "state": "complete",
                    "startedAt": now - 100_000,
                    "usage": {
                        "inputTokens": 999,
                        "outputTokens": 9,
                        "cacheTokens": 1,
                        "cachedWriteTokens": 0
                    }
                }]
            })
            .to_string(),
        )
        .expect("write out-of-range conversation");
        fs::write(
            messages.join("ignored.json"),
            json!({
                "role": "assistant",
                "usage": { "inputTokens": 10_000, "outputTokens": 10_000 }
            })
            .to_string(),
        )
        .expect("write ignored message body");

        let mut projects = HashMap::new();
        projects.insert("conv-a".to_string(), "example-project".to_string());
        let result = ide_source(root.clone(), "codebuddy-ide", Some(now - 50_000), &projects);

        assert_eq!(result["source"], "codebuddy-ide");
        assert_eq!(result["filesScanned"], 2);
        assert_eq!(result["summary"]["records"], 1);
        assert_eq!(result["summary"]["input"], 100);
        assert_eq!(result["summary"]["output"], 20);
        assert_eq!(result["summary"]["cacheRead"], 40);
        assert_eq!(result["summary"]["cacheWrite"], 5);
        assert_eq!(result["summary"]["total"], 125);
        assert_eq!(result["summary"]["uncachedInput"], 60);
        assert_eq!(result["models"][0]["key"], "deepseek-v4-flash");
        assert_eq!(result["projects"][0]["key"], "example-project");
        assert_eq!(result["sessions"][0]["sessionId"], "conv-a");
        assert_eq!(result["sessions"][0]["title"], "IDE 会话标题");
        assert_eq!(result["sessions"].as_array().map(Vec::len), Some(1));
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(result["dailyByModel"]["deepseek-v4-flash"][0]["key"], today);

        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn decode_genie_workspace_recovers_unix_project_path() {
        assert_eq!(
            decode_genie_workspace("L1VzZXJzL2FwcGxlL0RvY3VtZW50cy9Qcm9qZWN0L215LWFnZW50")
                .as_deref(),
            Some("/Users/apple/Documents/Project/my-agent")
        );
    }

    #[test]
    fn get_statistics_returns_three_isolated_sources() {
        let value = get_statistics(None);
        let sources = value["sources"].as_array().expect("sources");
        let names: Vec<_> = sources
            .iter()
            .map(|source| source["source"].as_str().unwrap_or_default())
            .collect();
        // 客户端三源 + 本机网关桶（`workbuddy-gateway`）一并返回。
        assert_eq!(
            names,
            ["workbuddy", "codebuddy-cli", "codebuddy-ide", "workbuddy-gateway"]
        );
        assert_eq!(value["region"], "cn");
    }

    #[test]
    fn get_statistics_for_global_reads_only_its_own_directory() {
        let value = get_statistics_for(crate::modules::region::Region::Global, None);
        let sources = value["sources"].as_array().expect("sources");
        let names: Vec<_> = sources
            .iter()
            .map(|source| source["source"].as_str().unwrap_or_default())
            .collect();
        // 国际版只读 `.workbuddy-ai/projects`，不混入 CN 的 CLI/IDE 目录；
        // 网关桶跨两版共用，单独追加一份（桶自带 realm，由 region 过滤裁剪）。
        assert_eq!(names, ["workbuddy-ai", "workbuddy-gateway"]);
        assert_eq!(value["region"], "global");
    }

    #[test]
    fn get_statistics_for_filter_all_returns_four_union_sources() {
        // 合并视图返回 cn 三源 + global 单源 + 网关桶（跨域一份）的并集，顶层 region = "all"。
        let value = get_statistics_for_filter(
            crate::modules::region::RegionFilter::All,
            None,
        );
        let sources = value["sources"].as_array().expect("sources");
        let names: Vec<_> = sources
            .iter()
            .map(|source| source["source"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            names,
            [
                "workbuddy",
                "codebuddy-cli",
                "codebuddy-ide",
                "workbuddy-ai",
                "workbuddy-gateway"
            ]
        );
        assert_eq!(value["region"], "all");
    }

    /// 构造一个最小可用的 source `Value`（只含合并需要关心的字段）。
    fn source_fixture(
        source: &str,
        input: u64,
        output: u64,
        cache_read: u64,
        uncached_input: u64,
        models: Vec<(&str, u64)>,
        sessions: Vec<(&str, u64)>,
        daily: Vec<(&str, u64)>,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Value {
        let models: Vec<Value> = models
            .into_iter()
            .map(|(key, total)| json!({ "key": key, "total": total }))
            .collect();
        let sessions: Vec<Value> = sessions
            .into_iter()
            .map(|(session_id, total)| json!({ "sessionId": session_id, "total": total }))
            .collect();
        let daily: Vec<Value> = daily
            .into_iter()
            .map(|(key, total)| json!({ "key": key, "total": total }))
            .collect();
        json!({
            "source": source,
            "summary": {
                "total": input + output + cache_read + uncached_input,
                "input": input,
                "output": output,
                "cacheRead": cache_read,
                "cacheWrite": 0,
                "uncachedInput": uncached_input,
                "records": 1,
                "cacheHitRate": (input > 0).then(|| cache_read as f64 / input as f64),
            },
            "models": models,
            "projects": [],
            "sessions": sessions,
            "daily": daily,
            "dailyByModel": { "gpt-x": daily_ref() },
            "hours": [],
            "filesScanned": 2,
            "parseErrors": 0,
            "coverageStartAt": start,
            "coverageEndAt": end,
        })
    }

    fn daily_ref() -> Value {
        json!([])
    }

    #[test]
    fn merge_source_values_sums_numeric_fields() {
        // 变异：把求和改成覆盖 / 取较大值 → 本断言失败。
        let cn = source_fixture(
            "workbuddy",
            10,
            5,
            10,
            10,
            vec![("gpt-x", 11)],
            vec![("s1", 11)],
            vec![("2024-01-01", 11)],
            Some(100),
            Some(200),
        );
        let global = source_fixture(
            "workbuddy",
            30,
            7,
            30,
            10,
            vec![("gpt-x", 33)],
            vec![("s1", 33)],
            vec![("2024-01-01", 33)],
            Some(150),
            Some(180),
        );
        let merged = merge_source_values(&[cn, global]);

        assert_eq!(merged["source"], "workbuddy");
        assert_eq!(merged["summary"]["input"], 40);
        assert_eq!(merged["summary"]["output"], 12);
        assert_eq!(merged["summary"]["cacheRead"], 40);
        assert_eq!(merged["summary"]["uncachedInput"], 20);
        assert_eq!(merged["summary"]["records"], 2);
        assert_eq!(merged["filesScanned"], 4);
        assert_eq!(merged["parseErrors"], 0);
        // coverage 取并集：min start / max end。
        assert_eq!(merged["coverageStartAt"], 100);
        assert_eq!(merged["coverageEndAt"], 200);
    }

    #[test]
    fn merge_source_values_recomputes_cache_hit_rate_instead_of_summing() {
        // cn: cacheRead=10, uncachedInput=10；global: cacheRead=30, uncachedInput=10。
        // 合并后 = 40 / (40 + 20) = 2/3。
        // 变异：把 cacheHitRate 改成相加（0.5 + 0.75 = 1.25）或朴素平均（0.625）→ 本断言失败。
        let cn = source_fixture(
            "workbuddy",
            20,
            0,
            10,
            10,
            vec![],
            vec![],
            vec![],
            None,
            None,
        );
        let global = source_fixture(
            "workbuddy",
            40,
            0,
            30,
            10,
            vec![],
            vec![],
            vec![],
            None,
            None,
        );
        assert_eq!(cn["summary"]["cacheHitRate"], 0.5);
        assert_eq!(global["summary"]["cacheHitRate"], 0.75);

        let merged = merge_source_values(&[cn, global]);
        let rate = merged["summary"]["cacheHitRate"]
            .as_f64()
            .expect("cacheHitRate is a finite number");
        assert!(
            (rate - (40.0f64 / 60.0f64)).abs() < 1e-9,
            "cacheHitRate must be recomputed as 40/60, got {rate}"
        );
        assert_ne!(rate, 1.25, "cacheHitRate must never be the naive sum");
        assert_ne!(rate, 0.625, "cacheHitRate must never be the naive average");
    }

    #[test]
    fn merge_source_values_recomputes_cache_hit_rate_to_null_when_denominator_zero() {
        let cn = source_fixture("workbuddy", 0, 0, 0, 0, vec![], vec![], vec![], None, None);
        let global = source_fixture("workbuddy", 0, 0, 0, 0, vec![], vec![], vec![], None, None);
        let merged = merge_source_values(&[cn, global]);
        assert!(merged["summary"]["cacheHitRate"].is_null());
    }

    #[test]
    fn merge_source_values_merges_same_named_keys_into_one_row() {
        // 变异：不做分组、直接 concat → 出现两条同名 key → 本断言失败。
        let cn = source_fixture(
            "workbuddy",
            0,
            0,
            0,
            0,
            vec![("gpt-x", 10), ("only-cn", 5)],
            vec![("s1", 10)],
            vec![("2024-01-01", 10)],
            None,
            None,
        );
        let global = source_fixture(
            "workbuddy",
            0,
            0,
            0,
            0,
            vec![("gpt-x", 30)],
            vec![("s1", 30)],
            vec![("2024-01-01", 30)],
            None,
            None,
        );
        let merged = merge_source_values(&[cn, global]);

        let models = merged["models"].as_array().expect("models");
        let gpt: Vec<&Value> = models
            .iter()
            .filter(|item| item["key"] == "gpt-x")
            .collect();
        assert_eq!(gpt.len(), 1, "同名 key 必须合并为一条，不得出现两条");
        assert_eq!(gpt[0]["total"], 40, "同名 key 的值必须是两版之和");

        let sessions = merged["sessions"].as_array().expect("sessions");
        let s1: Vec<&Value> = sessions
            .iter()
            .filter(|item| item["sessionId"] == "s1")
            .collect();
        assert_eq!(s1.len(), 1, "同 sessionId 必须合并为一条");
        assert_eq!(s1[0]["total"], 40);

        let daily = merged["daily"].as_array().expect("daily");
        let day: Vec<&Value> = daily
            .iter()
            .filter(|item| item["key"] == "2024-01-01")
            .collect();
        assert_eq!(day.len(), 1, "同日期必须合并为一条");
        assert_eq!(day[0]["total"], 40);
    }
}
