//! Trae 运行日志读取与聚合（`logs/app.log` / `checkin.log` / `switcher.log`）。
//!
//! ## 为什么是「读文件」而不是「查索引」
//!
//! 这三个文件由 [`crate::modules::trae::store::append_log`] 以
//! `[YYYY-MM-DD HH:MM:SS] message` 的纯文本行持续追加，是**唯一真相**。
//! 再维护一份结构化索引只会引入第二份状态，并在两者不一致时让人无从判断谁对。
//! 日志量级是本机单用户工具，直接读文件完全够用。
//!
//! ## 与 `api_gateway_logs.json` 的分工
//!
//! 本模块**只读纯文本运行日志**。网关的请求日志是 JSON 数组、字段化、面向统计
//! （见 [`crate::modules::trae::token_stats`]），两者用途不同：前者是「发生了什么」的
//! 时间线，后者是「用了多少」的度量。系统日志页把两者放在不同 Tab，不要混在一起。
//!
//! ## 三条边界（与页面提示一致）
//!
//! 1. **只读，不裁剪**。裁剪由 `store::trim_log` 按保留天数负责，读侧不得顺手删行。
//! 2. **无日期前缀的行照样返回**（外部脚本往日志里追加的内容）。这类行 `time` 为空，
//!    只在未按日期筛选时出现——否则「筛了某天却冒出无日期行」会让人困惑。
//! 3. **文件不存在不是错误**。Trae 模块没被使用过时三个文件都不存在，
//!    此时返回空列表 + `sources` 里标 `exists: false`，而不是报错。

use std::path::Path;

use serde_json::{json, Value};

use crate::modules::trae::paths;
use crate::modules::trae::variant::TraeVariant;

/// 单次返回的最大条数（防止把整个日志文件塞进响应）。
pub const MAX_LIMIT: usize = 2000;

/// 默认返回条数。
pub const DEFAULT_LIMIT: usize = 500;

/// 日期下拉里最多列出多少天。
const MAX_DATES: usize = 60;

/// 一条运行日志。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// `app` / `checkin` / `switch`。
    pub kind: &'static str,
    /// `YYYY-MM-DD HH:MM:SS`；无前缀的行为空串。
    pub time: String,
    /// 正文（已去掉 `[时间] ` 前缀）。
    pub message: String,
}

impl LogEntry {
    /// 行内的日期部分；无前缀时为空。
    pub fn date(&self) -> &str {
        self.time.get(..10).unwrap_or("")
    }
}

/// 解析一行。
///
/// 前缀缺失或格式不符时按「无日期行」处理并**保留正文**——日志的价值在于内容，
/// 因为前缀不合规就丢掉整行，恰恰会丢掉最需要看的那行。
pub fn parse_line(kind: &'static str, line: &str) -> Option<LogEntry> {
    let line = line.trim_end_matches('\r');
    if line.trim().is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix('[') {
        if let Some((stamp, message)) = rest.split_once(']') {
            if is_timestamp(stamp) {
                return Some(LogEntry {
                    kind,
                    time: stamp.to_string(),
                    message: message.trim_start().to_string(),
                });
            }
        }
    }
    Some(LogEntry {
        kind,
        time: String::new(),
        message: line.to_string(),
    })
}

/// `YYYY-MM-DD HH:MM:SS` 形状校验（不做时区/闰秒等语义校验，只保证可当日期用）。
fn is_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 19 {
        return false;
    }
    let digit = |index: usize| bytes[index].is_ascii_digit();
    let layout_ok = bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b' '
        && bytes[13] == b':'
        && bytes[16] == b':';
    layout_ok
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .into_iter()
            .all(digit)
}

/// 读取单个日志文件。
pub fn read_file(kind: &'static str, path: &Path) -> Vec<LogEntry> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .filter_map(|line| parse_line(kind, line))
            .collect(),
        // 文件缺失/不可读一律视为「没有日志」，不是错误。
        Err(_) => Vec::new(),
    }
}

/// 日志来源定义（kind / 路径 / 展示名；按变体分家）。
///
/// ## 只有两条来源分家，`app` 刻意不分
///
/// `checkin.log` / `switcher.log` 记的是**某条产品线的账号**发生了什么
/// （哪个账号签到了、哪个账号被切了），因此必须按变体分开 ——
/// 否则用户在 Trae CN 分区会看到 Trae Work 的签到记录，且两边互相盖写。
///
/// `app.log` 记的是**本工具自身**的启动/异常，与管哪条产品线无关，
/// 共用一份才对（出问题时用户想看的正是「全局发生了什么」）。
fn sources_for(variant: TraeVariant) -> [(&'static str, std::path::PathBuf, &'static str); 3] {
    [
        ("app", paths::app_log_file(), "运行"),
        ("checkin", paths::checkin_log_file_for(variant), "签到"),
        ("switch", paths::switcher_log_file_for(variant), "切换"),
    ]
}

/// 读取全部运行日志（未过滤；默认变体，兼容壳）。
pub fn read_all() -> Vec<LogEntry> {
    read_all_for(TraeVariant::default())
}

/// 读取全部运行日志（未过滤；按变体分家）。
pub fn read_all_for(variant: TraeVariant) -> Vec<LogEntry> {
    let mut entries: Vec<LogEntry> = sources_for(variant)
        .into_iter()
        .flat_map(|(kind, path, _)| read_file(kind, &path))
        .collect();
    sort_newest_first(&mut entries);
    entries
}

/// 按时间倒序；无时间行排最后（它们无法参与时间比较）。
pub fn sort_newest_first(entries: &mut [LogEntry]) {
    entries.sort_by(|left, right| {
        let left_known = !left.time.is_empty();
        let right_known = !right.time.is_empty();
        match (left_known, right_known) {
            (true, true) => right.time.cmp(&left.time),
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => std::cmp::Ordering::Equal,
        }
    });
}

/// 过滤条件。
#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    /// `all` / `app` / `checkin` / `switch`；未知值按 `all` 处理。
    pub kind: String,
    /// `YYYY-MM-DD`；空表示不限。
    pub date: String,
    /// 关键字（大小写不敏感，匹配正文）。
    pub keyword: String,
    /// 返回条数上限。
    pub limit: usize,
}

/// 已知的日志类型（`all` 与空串视为「不限」）。
pub const KNOWN_KINDS: [&str; 3] = ["app", "checkin", "switch"];

/// 把类型入参归一化为空串（不限）或一个已知类型。
pub fn normalize_kind(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return String::new();
    }
    KNOWN_KINDS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(trimmed))
        .map(|known| (*known).to_string())
        // 未知类型 → 不限，而不是「谁都不匹配」。
        .unwrap_or_default()
}

impl LogQuery {
    /// 从 JSON 入参解析（两条通道共用同一套字段名与默认值）。
    pub fn from_value(params: &Value) -> Self {
        let text = |key: &str| {
            params
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_LIMIT)
            .min(MAX_LIMIT);
        Self {
            // 未知类型归一化为 `all`：若放任未知值参与比较，`matches` 会「谁都不匹配」，
            // 页面直接白屏。前端传错枚举值时应当退化成「显示全部」，而不是显示空白。
            kind: normalize_kind(&text("kind")),
            date: text("date"),
            keyword: text("keyword"),
            limit,
        }
    }

    /// 该条目是否通过过滤。
    fn matches(&self, entry: &LogEntry) -> bool {
        if !self.kind.is_empty() && self.kind != "all" && self.kind != entry.kind {
            return false;
        }
        // 按日期筛选时，无日期行一律排除——否则「筛了某天却冒出无日期行」。
        if !self.date.is_empty() && entry.date() != self.date {
            return false;
        }
        if !self.keyword.is_empty() {
            let needle = self.keyword.to_lowercase();
            if !entry.message.to_lowercase().contains(&needle) {
                return false;
            }
        }
        true
    }
}

/// 条目 → 线上形态（camelCase）。
pub fn entry_json(entry: &LogEntry) -> Value {
    json!({
        "kind": entry.kind,
        "time": entry.time,
        "date": entry.date(),
        "message": entry.message,
    })
}

/// 系统日志页所需的全部数据。
///
/// 返回 `entries`（已过滤、倒序、截断）/ `total`（过滤后总数）/ `dates`（可选日期）/
/// `sources`（三个文件的状态）/ `counts`（各类型条数）/ `note`。
pub fn query_logs(params: &Value) -> Value {
    query_logs_for(TraeVariant::default(), params)
}

/// 查询运行日志（按变体分家；`params.variant` 或显式传入的变体决定读哪条线）。
///
/// 变体优先用**显式参数**（HTTP/Tauri 的查询串或请求体里明确传的那条线）；
/// 未提供时回落 `params.variant` 里的值（有的调用点把变体塞在 params 里）。
/// 两者都没有 → 默认变体（向后兼容）。
pub fn query_logs_for(variant: TraeVariant, params: &Value) -> Value {
    let query = LogQuery::from_value(params);
    let all = read_all_for(variant);

    let dates = collect_dates(&all);
    let counts = collect_counts(&all);

    let filtered: Vec<&LogEntry> = all.iter().filter(|entry| query.matches(entry)).collect();
    let total = filtered.len();
    let entries: Vec<Value> = filtered
        .into_iter()
        .take(query.limit)
        .map(entry_json)
        .collect();

    let source_views: Vec<Value> = sources_for(variant)
        .into_iter()
        .map(|(kind, path, label)| {
            json!({
                "kind": kind,
                "label": label,
                "path": path.to_string_lossy(),
                "exists": path.exists(),
            })
        })
        .collect();

    json!({
        "entries": entries,
        "total": total,
        "limit": query.limit,
        "dates": dates,
        "counts": counts,
        "sources": source_views,
        "logDir": paths::logs_dir().to_string_lossy(),
        "note": "只读取本机纯文本运行日志（app / checkin / switcher）；\
                 网关请求日志在「网关请求日志」标签页。文件不存在时返回空列表而非报错。",
    })
}

/// 可用日期列表（倒序，最多 [`MAX_DATES`] 天）。
fn collect_dates(entries: &[LogEntry]) -> Vec<String> {
    let mut dates: Vec<String> = entries
        .iter()
        .map(|entry| entry.date().to_string())
        .filter(|date| !date.is_empty())
        .collect();
    dates.sort();
    dates.dedup();
    dates.reverse();
    dates.truncate(MAX_DATES);
    dates
}

/// 各类型条数。
fn collect_counts(entries: &[LogEntry]) -> Value {
    let count = |kind: &str| entries.iter().filter(|entry| entry.kind == kind).count();
    json!({
        "all": entries.len(),
        "app": count("app"),
        "checkin": count("checkin"),
        "switch": count("switch"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &'static str, time: &str, message: &str) -> LogEntry {
        LogEntry {
            kind,
            time: time.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn parse_line_reads_prefixed_lines() {
        let parsed = parse_line("checkin", "[2026-09-17 08:06:12] 账号 A 签到成功").unwrap();
        assert_eq!(parsed.kind, "checkin");
        assert_eq!(parsed.time, "2026-09-17 08:06:12");
        assert_eq!(parsed.date(), "2026-09-17");
        assert_eq!(parsed.message, "账号 A 签到成功");
    }

    #[test]
    fn parse_line_keeps_content_when_prefix_is_malformed() {
        // 前缀不合规时**保留整行**：丢掉最需要看的那行是本末倒置。
        for line in [
            "无前缀行",
            "[2026-09-17] 只有日期",
            "[not-a-time] 坏前缀",
            "[2026-09-17 08:06] 缺秒",
        ] {
            let parsed = parse_line("app", line).unwrap_or_else(|| panic!("{line} 应被保留"));
            assert_eq!(parsed.time, "", "{line}");
            assert!(!parsed.message.is_empty(), "{line}");
        }
    }

    #[test]
    fn parse_line_skips_blank_lines() {
        assert!(parse_line("app", "").is_none());
        assert!(parse_line("app", "   ").is_none());
        assert!(parse_line("app", "\r").is_none());
    }

    #[test]
    fn parse_line_tolerates_crlf() {
        let parsed = parse_line("app", "[2026-09-17 08:06:12] 正文\r").unwrap();
        assert_eq!(parsed.message, "正文");
    }

    #[test]
    fn timestamp_shape_check_is_strict_about_layout() {
        assert!(is_timestamp("2026-09-17 08:06:12"));
        assert!(!is_timestamp("2026-09-17 08:06:1"));
        assert!(!is_timestamp("2026-09-17T08:06:12"));
        assert!(!is_timestamp("2026/09/17 08:06:12"));
        assert!(!is_timestamp("2026-09-17 08:06:12 extra"));
    }

    #[test]
    fn sorting_is_newest_first_and_undated_go_last() {
        let mut entries = vec![
            entry("app", "2026-09-15 10:00:00", "旧"),
            entry("app", "", "无日期"),
            entry("app", "2026-09-17 10:00:00", "新"),
            entry("app", "2026-09-16 10:00:00", "中"),
        ];
        sort_newest_first(&mut entries);
        let order: Vec<&str> = entries.iter().map(|item| item.message.as_str()).collect();
        assert_eq!(order, vec!["新", "中", "旧", "无日期"]);
    }

    #[test]
    fn kind_filter_accepts_all_and_empty() {
        let entries = [
            entry("app", "2026-09-17 10:00:00", "a"),
            entry("checkin", "2026-09-17 10:00:00", "b"),
        ];
        for kind in ["", "all"] {
            let query = LogQuery {
                kind: kind.to_string(),
                ..LogQuery::default()
            };
            assert_eq!(entries.iter().filter(|e| query.matches(e)).count(), 2);
        }
        let query = LogQuery {
            kind: "checkin".to_string(),
            ..LogQuery::default()
        };
        assert_eq!(entries.iter().filter(|e| query.matches(e)).count(), 1);
    }

    #[test]
    fn unknown_kind_falls_back_to_all_instead_of_empty() {
        // 未知类型若被判成「谁都不匹配」，页面会白屏；归一化成「不限」更安全。
        let query = LogQuery::from_value(&json!({ "kind": "unknown" }));
        assert_eq!(query.kind, "");
        let entries = [entry("app", "2026-09-17 10:00:00", "a")];
        assert!(query.matches(&entries[0]), "未知类型必须退化为显示全部");
    }

    #[test]
    fn normalize_kind_accepts_known_values_case_insensitively() {
        assert_eq!(normalize_kind(""), "");
        assert_eq!(normalize_kind("   "), "");
        assert_eq!(normalize_kind("all"), "");
        assert_eq!(normalize_kind("ALL"), "");
        for known in KNOWN_KINDS {
            assert_eq!(normalize_kind(known), known);
            assert_eq!(normalize_kind(&known.to_uppercase()), known);
        }
        // 大小写不敏感，但归一化后必须是规范写法（否则 `matches` 会失配）。
        assert_eq!(normalize_kind("Checkin"), "checkin");
        assert_eq!(normalize_kind("nope"), "");
    }

    #[test]
    fn date_filter_excludes_undated_lines() {
        let query = LogQuery {
            date: "2026-09-17".to_string(),
            ..LogQuery::default()
        };
        assert!(query.matches(&entry("app", "2026-09-17 10:00:00", "x")));
        assert!(!query.matches(&entry("app", "2026-09-16 10:00:00", "x")));
        // 无日期行不得在按日期筛选时出现。
        assert!(!query.matches(&entry("app", "", "无日期")));
    }

    #[test]
    fn keyword_filter_is_case_insensitive() {
        let query = LogQuery {
            keyword: "PLANLIMIT".to_string(),
            ..LogQuery::default()
        };
        assert!(query.matches(&entry(
            "app",
            "2026-09-17 10:00:00",
            "错误 PlanLimit 已冷却"
        )));
        assert!(!query.matches(&entry("app", "2026-09-17 10:00:00", "签到成功")));
    }

    #[test]
    fn query_defaults_are_bounded() {
        let query = LogQuery::from_value(&json!({}));
        assert_eq!(query.limit, DEFAULT_LIMIT);
        assert_eq!(query.kind, "");
        assert_eq!(query.date, "");
        assert_eq!(query.keyword, "");

        // 超上限必须被夹住，0 / 负数回落默认值。
        let huge = LogQuery::from_value(&json!({ "limit": 999_999 }));
        assert_eq!(huge.limit, MAX_LIMIT);
        assert_eq!(
            LogQuery::from_value(&json!({ "limit": 0 })).limit,
            DEFAULT_LIMIT
        );
    }

    #[test]
    fn query_logs_shape_is_stable_and_never_errors() {
        // 隔离到临时 home，避免读到真实日志（结果会随本机使用而变化）。
        // 走 `HomeOverrideGuard` 而不是裸 `set_var`：它同时持有进程级 env 锁
        // （lib 单元测试共享同一进程，并行跑）并在 drop 时恢复原值，
        // 否则临时 home 会泄漏给后续测试。详见 `config::HomeOverrideGuard`。
        let dir = std::env::temp_dir().join(format!("trae-logs-shape-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let _guard = crate::modules::config::HomeOverrideGuard::set(&dir);

        let value = query_logs(&json!({}));
        for key in [
            "entries", "total", "limit", "dates", "counts", "sources", "logDir", "note",
        ] {
            assert!(value.get(key).is_some(), "缺少字段 {key}");
        }
        assert!(value.get("entries").unwrap().is_array());
        assert!(value.get("dates").unwrap().is_array());
        assert!(value.get("sources").unwrap().as_array().unwrap().len() == 3);
        // snake_case 不得泄漏到响应里。
        assert!(value.get("log_dir").is_none());
    }

    #[test]
    fn collect_counts_counts_each_kind() {
        let entries = vec![
            entry("app", "2026-09-17 10:00:00", "a"),
            entry("app", "2026-09-17 10:00:01", "b"),
            entry("checkin", "2026-09-17 10:00:02", "c"),
        ];
        let counts = collect_counts(&entries);
        assert_eq!(counts.get("all").unwrap().as_u64(), Some(3));
        assert_eq!(counts.get("app").unwrap().as_u64(), Some(2));
        assert_eq!(counts.get("checkin").unwrap().as_u64(), Some(1));
        assert_eq!(counts.get("switch").unwrap().as_u64(), Some(0));
    }

    #[test]
    fn collect_dates_is_descending_and_deduped() {
        let entries = vec![
            entry("app", "2026-09-15 10:00:00", "a"),
            entry("app", "2026-09-17 10:00:00", "b"),
            entry("app", "2026-09-17 11:00:00", "c"),
            entry("app", "", "无日期"),
        ];
        assert_eq!(collect_dates(&entries), vec!["2026-09-17", "2026-09-15"]);
    }

    /// 日志来源必须按变体分家：`checkin` / `switch` 各读各的，`app` 刻意共用。
    ///
    /// 反例（改坏会红）：若 `sources_for` 忽略传入变体、一律取默认路径，
    /// 则用户在 Trae CN 分区会看到 Trae Work 的签到记录，且两条线互相盖写。
    ///
    /// **只断言 basename，不断言绝对路径**：这些路径函数是无参全局函数，每次调用
    /// 都重读进程级 `BUDDY_SWITCH_HOME`；lib 单测共享同一进程并行跑，只要同组的
    /// `query_logs_shape_...` 中途换过 home，同一条 `assert_eq!` 左右两侧就会取到
    /// 不同的根目录（症状：左侧是本机真实 home、右侧是临时 home），**看起来像"串味"，
    /// 其实是被别的用例改了环境**。只比 basename 天然免疫。
    #[test]
    fn sources_split_by_variant_but_app_log_is_shared() {
        let work = sources_for(TraeVariant::TraeWork);
        let cn = sources_for(TraeVariant::Global);

        let pick = |list: &[(&'static str, std::path::PathBuf, &'static str)], kind: &str| {
            list.iter()
                .find(|(k, _, _)| *k == kind)
                .map(|(_, path, _)| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_string()
                })
                .unwrap_or_else(|| panic!("缺少来源 {kind}"))
        };

        // 分家的两条：文件名必须不同。
        assert_ne!(
            pick(&work, "checkin"),
            pick(&cn, "checkin"),
            "签到日志按变体分家，绝不该指向同一个文件"
        );
        assert_ne!(
            pick(&work, "switch"),
            pick(&cn, "switch"),
            "切换日志按变体分家"
        );
        // 刻意共用的那条：本工具自身日志与管哪条产品线无关。
        assert_eq!(
            pick(&work, "app"),
            pick(&cn, "app"),
            "app.log 是本工具自身日志，刻意共用一份"
        );
        // 顺带钉住两个分家文件的具体名字（改名字是契约变更，会红是提醒）。
        assert_eq!(pick(&work, "checkin"), "checkin.log");
        assert_eq!(pick(&cn, "checkin"), "checkin.global.log");
    }
}
