//! Trae 客户端对话库导入：读**用户自行导出的明文 SQLite 副本**，聚合逐日 token 用量。
//!
//! ## 为什么不内置解密（这条是本模块的设计前提，不是偷懒）
//!
//! Trae IDE 的对话库在 `%APPDATA%\<变体>\ModularData\ai-agent\database.db`，实测是
//! **SQLCipher 加密**的：文件头 16 字节是 salt（不是 `SQLite format 3\0`），整文件熵 8.00。
//! 要读它就得拿到用户的库口令（在客户端自己的加密存储里），而
//!
//! 1. 那是**逆向客户端密钥派生**——本质是破解一个正在维护的商业产品的本地保护，
//!    上游每次改 KDF 都会让本工具静默失效；
//! 2. 一旦本工具能解，就等于在本工具里**复制了一份用户对话正文的解密路径**，
//!    而本模块真正需要的只有一行数字（token 用量）。
//!
//! 因此边界划在这里：**用户自己**用 sqlcipher 的 `sqlcipher_export` 导出一份明文副本
//! （官方支持的迁移手段，不需要任何破解），把路径交给本工具；本工具**只读**这份副本。
//! 代价是「多一步手工操作」，换来的是不用把破解逻辑与用户明文正文绑进进程。
//!
//! ## 为什么 schema 必须自适应
//!
//! 我们**没有解开过**那个库，表名/列名只能从 `ai_agent.dll` 的字符串里推测。已知的几个
//! 字面量（`chat_session_goal` / `fts_message_content` / `model_config_cache` / `project` /
//! `users` / `migration_sql_store`，字段 `token_usage` / `prompt_tokens` /
//! `completion_tokens` / `input_tokens` / `credits`）**只作为探测线索**，一处也不写死：
//! 主会话/消息表名完全未知（可能是 `session` / `message` / `chat_message` /
//! `conversation` / `item`……），版本差异还会让同一张表换列名。
//!
//! 所以流程是「**先看库、再决定怎么读**」：
//!
//! 1. 枚举全部表（跳过 `sqlite_%` 与 FTS 影子表，它们没有用量）；
//! 2. 每张表 `PRAGMA table_info` 取列，找出「文本列 + 时间列」都有的表当候选；
//! 3. 抽样 5 行，看文本里有没有 `"usage"` / `prompt_tokens` 之类的标记，有则加权；
//! 4. 得分最高的候选即「消息表」，其文本列/时间列/会话列/模型列一并确定。
//!
//! 判定与取值全部走**候选表 + 优先级**（见 `TEXT_COLUMNS` / `TIME_COLUMNS` /
//! `SESSION_COLUMNS`），新增一个变体只需要往常量里加名字。
//!
//! ## 三条必须写进返回体的边界
//!
//! - **缺省 ≠ 0**：认不出结构时返回 `error`（[`ImportError`]），**不**返回一份全 0 的
//!   统计冒充「这段时间没用」——「没导入」「文件还是加密的」「这版没记录用量」是
//!   三件不同的事，用户要据此知道下一步做什么。
//! - **区域不可分**：导入库是**单客户端**的私有库，不含区域标识（见
//!   [`TraeTokenScope`] 的过滤说明），因此 `Global` / `Unlabeled` 两档只能返回空统计。
//! - **会话数是估计**：库里没有可用的会话列时，按「一行一个会话」的保守口径计数，
//!   并在 `sessionsEstimated` 里如实标记。
//!
//! ## 与 [`super::token_stats`] 的关系
//!
//! 返回体字段与 `token_stats::get_statistics` **同名同形**（前端共用渲染），但聚合桶
//! [`Bucket`] 是**本模块自己的**：那边按网关日志（含延迟 / 状态码 / 流式）累加，这边
//! 只有 input/output/records，两者的字段集合会**各自演化**，共用一个结构只会让其中
//! 一边被另一边的字段拽着走。两处 `Bucket` 刻意互相独立，改动时请对照着看。

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use super::handlers;
use super::token_stats::TraeTokenScope;

/// 导入配置的落盘位置（相对 `~/.buddy-switch/`）。
pub const CLIENT_IMPORT_FILE: &str = "trae/client_import.json";

/// 单表扫描上限。
///
/// **为什么必须有上限**：用户导出的是**整库**副本，真机对话库动辄上百 MB、几十万行，
/// 而本页只需要「逐日用量」。无限扫描会让一次「切到 Trae 统计页」卡住整个 GUI 线程
/// （命令走 `spawn_blocking`，但等待仍是秒级到十秒级）。
///
/// 触到上限会在 `note` 里如实标注 `truncated`，**不静默截断**——统计值变成下界这件事
/// 必须让用户看见。
pub const MAX_ROWS_PER_TABLE: usize = 200_000;

/// 返回体里的固定标识（前端按 `source` 选数据源徽标）。
const SOURCE: &str = "trae-client";
const LABEL: &str = "Trae 客户端（导入）";

/// SQLite 明文文件头。命中它才可能有明文页；SQLCipher 的库这里是 salt。
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// 时间戳的「秒 / 毫秒」分界。`1e11` 毫秒 ≈ 1973 年，`1e11` 秒 ≈ 5138 年，
/// 真实数据两侧都离得很远，因此这个阈值不会误判。
const MILLISECOND_THRESHOLD: i64 = 100_000_000_000;

/// 判定可用时间列（按优先级；命中即用，顺序即优先级）。
const TIME_COLUMNS: [&str; 9] = [
    "created_at",
    "create_time",
    "createdat",
    "createat",
    "created",
    "timestamp",
    "updated_at",
    "ts",
    "time",
];

/// 判定文本列（按优先级）。`content` 最可能是消息正文，`data` / `json` 是兜底。
const TEXT_COLUMNS: [&str; 7] = [
    "content", "payload", "body", "message", "text", "data", "json",
];

/// 判定会话列（仅供会话数统计；拿不到就退化为「一行一会话」）。
const SESSION_COLUMNS: [&str; 6] = [
    "session_id",
    "conversation_id",
    "chat_id",
    "thread_id",
    "sessionid",
    "conversationid",
];

/// 判定模型列（用于 `models` / `modelDaily`）。
const MODEL_COLUMNS: [&str; 4] = ["model", "model_name", "modelid", "model_id"];

/// 输入 token 的列名候选（`columns` 模式下用）。
const INPUT_COLUMNS: [&str; 4] = [
    "input_tokens",
    "prompt_tokens",
    "inputtokens",
    "prompttokens",
];

/// 输出 token 的列名候选（`columns` 模式下用）。
const OUTPUT_COLUMNS: [&str; 4] = [
    "output_tokens",
    "completion_tokens",
    "outputtokens",
    "completiontokens",
];

/// 积分列（若客户端把用量记成 credits，一并累加进 `counts.credits`）。
const CREDITS_COLUMNS: [&str; 2] = ["credits", "credit"];

/// 文本里出现任一标记 ⇒ 这张表很可能带用量（探测打分用，**不做解析**）。
const USAGE_MARKERS: [&str; 5] = [
    "\"usage\"",
    "token_usage",
    "\"input_tokens\"",
    "\"prompt_tokens\"",
    "\"completion_tokens\"",
];

/// 抽样行数（探测阶段每表只读这么多行，避免为了「认结构」把整库读一遍）。
const SAMPLE_ROWS: usize = 5;

/// 导入失败的分类。
///
/// 五档**必须分开**：用户看到的下一步动作完全不同（换路径 / 先导出明文 / 报不兼容 /
/// 换新版本客户端 / 查磁盘权限）。前端按 [`ImportError::code`] 选文案。
#[derive(Debug, Clone, PartialEq)]
pub enum ImportError {
    /// 路径为空，或文件不存在。
    Missing,
    /// 能打开但不是明文 SQLite：还是加密的，或根本不是 SQLite。
    NotPlaintext,
    /// 能打开，但认不出会话/消息结构。
    Unrecognized,
    /// 认出了结构，但没有任何 usage 字段（客户端版本差异）。
    NoUsage,
    /// 其他 IO / SQL 失败。
    Io(String),
}

impl ImportError {
    /// 稳定错误码（前端按它选文案）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::NotPlaintext => "not_plaintext",
            Self::Unrecognized => "unrecognized",
            Self::NoUsage => "no_usage",
            Self::Io(_) => "io",
        }
    }

    /// 面向用户的一句话（含下一步该做什么）。
    pub fn message(&self) -> String {
        match self {
            Self::Missing => {
                "没有指定明文库文件，或该文件已经不见了。请重新选择导出的副本。".into()
            }
            Self::NotPlaintext => {
                "这个文件还是加密的（或不是 SQLite）：请先用 sqlcipher 的 sqlcipher_export \
                 导出一份明文副本，再选那份。"
                    .into()
            }
            Self::Unrecognized => {
                "认不出这个库的会话/消息结构，可能不是 Trae 对话库的导出副本。".into()
            }
            Self::NoUsage => {
                "库结构认出来了，但里面没有任何 token 用量字段：这个版本的 Trae 可能还没记录用量。"
                    .into()
            }
            Self::Io(detail) => format!("读取失败：{detail}"),
        }
    }

    /// 错误对象（`{ code, message }`）。
    fn to_value(&self) -> Value {
        json!({ "code": self.code(), "message": self.message() })
    }
}

// ---------------------------------------------------------------------------
// 配置（~/.buddy-switch/trae/client_import.json = `{ path, savedAt }`）
// ---------------------------------------------------------------------------

fn config_path() -> PathBuf {
    crate::modules::config::store_dir().join(CLIENT_IMPORT_FILE)
}

/// 内置解密产出的**明文副本**固定落点：`~/.buddy-switch/trae/client_plain/database.db`。
///
/// 为什么固定而不是让用户选：解密产物是**派生数据**，用户不需要也不应该为它挑位置；
/// 固定一处还能保证「重新解锁就覆盖旧副本」（否则旧副本会被当成新数据读，见
/// `export_plaintext` 里删目标的注释）。
pub fn plaintext_path() -> PathBuf {
    crate::modules::config::store_dir()
        .join("trae")
        .join("client_plain")
        .join("database.db")
}

/// 已保存的导入路径（读不到 / 空 / 坏 JSON ⇒ `None`）。
pub fn load_config() -> Option<String> {
    let text = std::fs::read_to_string(config_path()).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let path = value.get("path")?.as_str()?.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// 保存导入路径；空路径视为非法（清除请用 [`clear_config`]）。
pub fn save_config(path: &str) -> bool {
    let target = path.trim();
    if target.is_empty() {
        return false;
    }
    let file = config_path();
    if let Some(parent) = file.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    let payload = json!({ "path": target, "savedAt": chrono::Local::now().timestamp_millis() });
    std::fs::write(&file, payload.to_string()).is_ok()
}

/// 清除导入路径（文件本来就不存在也算成功——目标状态已达成）。
pub fn clear_config() -> bool {
    match std::fs::remove_file(config_path()) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

/// 已保存的路径（**不做存在性过滤**）。
///
/// ## 为什么不过滤
///
/// 过滤会把「上次选的文件被删了」静默变成「你从来没配过」——用户于是重新走一遍选择框，
/// 却不知道自己上一份配置去哪了。保留原值、由前端/调用方比对 `is_file()`（见
/// `commands.rs` 的 `get_trae_client_db_config`），才能显示「上次的文件不见了」。
/// 路径失效时 [`summarize`] 会如实返回 [`ImportError::Missing`]，不会被当成空统计。
pub fn configured_path() -> Option<String> {
    load_config()
}

// ---------------------------------------------------------------------------
// 探测
// ---------------------------------------------------------------------------

/// 探测结果（内部形状；对外由 [`probe`] 摊平成 JSON）。
#[derive(Debug, Clone)]
struct Detected {
    /// 消息表名。
    table: String,
    /// 正文列（JSON 或纯文本）。
    text_column: String,
    /// 时间列（秒或毫秒，运行时判定）。
    time_column: String,
    /// 会话列（可能没有）。
    session_column: Option<String>,
    /// 模型列（可能没有；没有时退回从 JSON 里捞）。
    model_column: Option<String>,
    /// 用量列 `(输入列, 输出列?)`；`None` ⇒ 只能从 JSON 里取。
    usage_columns: Option<(String, Option<String>)>,
    /// 积分列（可能没有）。
    credits_column: Option<String>,
    /// 抽样到的非空文本行数。
    sample_rows: usize,
    /// 抽样文本里是否出现用量标记。
    has_usage_marker: bool,
}

impl Detected {
    /// 实际生效的取用量方式：`columns` 优先于 `json`（显式列比正文里的 JSON 更可靠）。
    fn usage_mode(&self) -> &'static str {
        if self.usage_columns.is_some() {
            "columns"
        } else {
            "json"
        }
    }
}

/// 探测：给「校验」按钮用，**不写配置**、不改库。
///
/// 返回 `{ ok, path, error?, tables, detected?, counts? }`。认不出结构时 `ok=false` 且
/// `tables` 仍然给出（用户能据此反馈「我的库里有哪些表」）；`no_usage` 时
/// `detected` / `counts` 也会给出——「认得出但没用量」本身就说明了原因。
pub fn probe(path: &str) -> Value {
    let target = path.trim().to_string();
    let mut tables: Vec<String> = Vec::new();
    match inspect(&target, &mut tables, None) {
        Ok((detected, scan)) => {
            let (sessions, sessions_estimated) = session_stats(&scan);
            let with_usage = scan.with_usage;
            let mut value = json!({
                // 结构认出来就算通过；空库（0 行）也是通过——「没有记录」与「读不出」不同。
                "ok": with_usage > 0 || scan.messages == 0,
                "path": target,
                "tables": tables,
                "detected": {
                    "messagesTable": detected.table,
                    "textColumn": detected.text_column,
                    "timeColumn": detected.time_column,
                    "usageMode": scan.usage_mode(&detected),
                    "sessionColumn": detected.session_column,
                    "modelColumn": detected.model_column,
                    "sampleRows": detected.sample_rows,
                    "usageHint": detected.has_usage_marker,
                    "sessionsEstimated": sessions_estimated,
                    "truncated": scan.truncated,
                },
                "counts": {
                    "sessions": sessions,
                    "messages": scan.messages,
                    "withUsage": with_usage,
                    "credits": scan.credits,
                    // 一行里出现多份 usage 的行数（> 0 ⇒ 统计值可能含历史轮次，见 note）。
                    "multiUsageRows": scan.multi_usage_rows,
                },
            });
            if with_usage == 0 && scan.messages > 0 {
                if let Some(object) = value.as_object_mut() {
                    object.insert("error".into(), ImportError::NoUsage.to_value());
                }
            }
            value
        }
        Err(error) => {
            let mut value = json!({
                "ok": false,
                "path": target,
                "error": error.to_value(),
                "tables": tables,
            });
            if let Some(object) = value.as_object_mut() {
                object.insert("note".into(), json!(error.message()));
            }
            value
        }
    }
}

// ---------------------------------------------------------------------------
// 聚合
// ---------------------------------------------------------------------------

/// 聚合：与 `trae::token_stats::get_statistics` **字段同名同形**的 JSON。
///
/// `days` 为 `None` 或 `<= 0` 表示不限窗口。失败时返回
/// `{ source, error: { code, message }, ... }`，**不**返回空统计冒充 0。
pub fn summarize(path: &str, days: Option<i64>, scope: TraeTokenScope) -> Value {
    let target = path.trim().to_string();
    // 导入库是单客户端的私有库，不含区域标识：`Cn` 与 `All` 等价（客户端不分区域，
    // 全量都是「这一台机器上这个客户端」的用量）；`Global` / `Unlabeled` 无法表达，
    // 只能返回空统计并在 note 里说明——**宁可空着，也不要把它算进某一区域冒充精确**。
    let scope_applies = matches!(scope, TraeTokenScope::All | TraeTokenScope::Cn);
    let mut tables: Vec<String> = Vec::new();
    let (_, scan) = match inspect(&target, &mut tables, cutoff_ms(days)) {
        Ok(found) => found,
        Err(error) => return failure(&error, &target, days),
    };
    // 有消息、但一条都没取到用量 ⇒ 版本差异（`NoUsage`）。
    // 一条消息都没有则是**空副本**（新装客户端 / 导出时机不对），那不是错误，
    // 如实返回 0 并在 note 里说明——这两件事必须分开（缺席 ≠ 0）。
    if scan.with_usage == 0 && scan.messages > 0 {
        return failure(&ImportError::NoUsage, &target, days);
    }

    // 会话数只在 `probe` 里有字段（`counts.sessions`）；`get_statistics` 同形的返回体
    // 没有这个键，因此这里只需要「是不是估的」这一个布尔去写 note。
    let (_sessions, sessions_estimated) = session_stats(&scan);
    let (summary, models, daily, hours, model_daily) = if scope_applies {
        build_aggregates(&scan)
    } else {
        (
            Bucket::default().summary_value(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    };

    let mut note = String::from(
        "数据来自你自行用 sqlcipher_export 导出的明文副本（本工具不解密 Trae 的原库）；\
         只统计该副本里记录的对话用量，与网关统计互不重叠。",
    );
    if !scope_applies {
        note.push_str(" 导入库不含区域信息，无法按「国际版 / 未标注」筛选，本次按空统计返回。");
    }
    if sessions_estimated {
        note.push_str(" 库里没有可用的会话列，会话数按「一行一个会话」保守估计。");
    }
    if scan.truncated {
        note.push_str(" 已到单表扫描上限，统计值可能只是下界。");
    }
    if scan.messages == 0 {
        note.push_str(" 这份副本里没有任何对话记录（可能导早了，或该客户端的记录在别的库里）。");
    }
    /* ★ 这一条是**诚实性**要求，不是可选提示：如果库里存在「一行含多份 usage」，
    我们把它们累加了 —— 当那些 usage 属于同一段对话的历史轮次时，统计值会成倍虚高。
    我们无法在不了解真实 schema 的前提下判定，所以既不猜着去重、也不假装精确：
    数字照给，但必须说清它可能偏大，以及有多少行是这种形状。 */
    if scan.multi_usage_rows > 0 {
        note.push_str(&format!(
            " 有 {} 行记录里含多份用量（可能是整段对话快照），这些行按累加计算，统计值可能偏大。",
            scan.multi_usage_rows
        ));
    }

    json!({
        "source": SOURCE,
        "label": LABEL,
        "generatedAt": chrono::Local::now().timestamp_millis(),
        "rangeDays": days.filter(|value| *value > 0),
        // 与网关侧同名字段：这里填的是**导入的库文件**（前端「数据来源」里要显示它）。
        "logFile": target,
        "summary": summary,
        "models": models,
        "accounts": Vec::<Value>::new(),
        "daily": daily,
        "hours": hours,
        "statuses": Vec::<Value>::new(),
        "modelDaily": model_daily,
        "variantCounts": {
            // 导入库全量只有一档：没有区域标识 ⇒ 全部计入国内档（见 `summarize` 说明）。
            "cn": scan.messages,
            "global": 0,
            "unlabeled": 0,
            "all": scan.messages,
        },
        "unsupported": unsupported_notes(),
        "filesScanned": if scan.messages > 0 { 1 } else { 0 },
        "parseErrors": scan.parse_errors,
        "coverageStartAt": scan.coverage_start,
        "coverageEndAt": scan.coverage_end,
        "note": note,
    })
}

/// 失败体：形状与成功体**同键**（前端不必写两套取值路径），但 `summary` 显式为
/// `null`、`error` 一定有——避免「空统计」被渲染成 0。
fn failure(error: &ImportError, path: &str, days: Option<i64>) -> Value {
    json!({
        "source": SOURCE,
        "label": LABEL,
        "generatedAt": chrono::Local::now().timestamp_millis(),
        "rangeDays": days.filter(|value| *value > 0),
        "logFile": path,
        "error": error.to_value(),
        "summary": Value::Null,
        "models": Vec::<Value>::new(),
        "accounts": Vec::<Value>::new(),
        "daily": Vec::<Value>::new(),
        "hours": Vec::<Value>::new(),
        "statuses": Vec::<Value>::new(),
        "modelDaily": Vec::<Value>::new(),
        "variantCounts": { "cn": 0, "global": 0, "unlabeled": 0, "all": 0 },
        "unsupported": unsupported_notes(),
        "filesScanned": 0,
        "parseErrors": 0,
        "coverageStartAt": Value::Null,
        "coverageEndAt": Value::Null,
        "note": error.message(),
    })
}

/// 导入这一路做不到的维度（形状由 [`handlers::unsupported_note`] 唯一产出，
/// 与网关侧的置灰卡共用同一组件）。
fn unsupported_notes() -> Vec<Value> {
    vec![
        handlers::unsupported_note(
            "cache_metrics",
            "缓存读取 / 写入 / 命中率",
            "客户端对话库只记录 input / output 计数，没有 cache 字段",
        ),
        handlers::unsupported_note(
            "account_dimension",
            "按账号维度统计",
            "客户端库属于本机当前登录的那个账号，库里不含账号标识",
        ),
        handlers::unsupported_note(
            "status_dimension",
            "成功 / 失败请求分布",
            "对话库记录的是已完成的消息，没有请求状态码",
        ),
    ]
}

/// 一次聚合的累加器。
///
/// **刻意不复用** `token_stats::Bucket`：那个桶为网关日志而生（延迟样本、状态码、
/// 流式标记），这里只有 token 与条数。共用一个结构会让两边互相绑死——网关加字段，
/// 这里就得凭空造一个恒 0 的值；导入侧要加「按会话去重」，那边也用不上。
/// 两处各自的字段集合应当**各自演化**，对照着看即可。
#[derive(Debug, Default, Clone)]
struct Bucket {
    input: u64,
    output: u64,
    /// 落入本桶的行数（含没有 usage 的行——它们仍是「消息」）。
    records: u64,
}

impl Bucket {
    fn add(&mut self, input: u64, output: u64) {
        self.input = self.input.saturating_add(input);
        self.output = self.output.saturating_add(output);
        self.records += 1;
    }

    fn total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }

    /// 与 `token_stats::Bucket::summary_value` **同键**：导入侧没有延迟/错误/流式，
    /// 这些键恒 0，只为让前端复用同一套渲染（缺键会让表格取到 `undefined`）。
    fn summary_value(&self) -> Value {
        json!({
            "total": self.total(),
            "input": self.input,
            "output": self.output,
            "records": self.records,
            "errors": 0,
            "streamRequests": 0,
            "avgLatencyMs": 0,
            "p95LatencyMs": 0,
        })
    }

    fn group_value(&self, key: &str) -> Value {
        let mut value = self.summary_value();
        if let Some(object) = value.as_object_mut() {
            object.insert("key".into(), json!(key));
        }
        value
    }
}

/// 扫描的中间结果（探测与聚合共用一次扫描，避免读两遍大库）。
#[derive(Debug, Default)]
struct Scan {
    /// 落在窗口内的消息行数。
    messages: u64,
    /// 其中带 usage 的行数。
    with_usage: u64,
    /// 取用量来自 JSON 正文 / 表列 的行数（用于报告实际生效的 `usageMode`）。
    json_rows: u64,
    column_rows: u64,
    /// 积分累加（列缺失 ⇒ 0）。
    credits: u64,
    /// 时间戳不可用的行数。
    parse_errors: u64,
    /// **一行里出现多份 `usage`** 的行数（见聚合处的长注释：这是最可能虚高的来源）。
    multi_usage_rows: u64,
    /// 见过的行数（含被窗口过滤掉的）——用于判定是否触到扫描上限。
    rows_seen: u64,
    truncated: bool,
    sessions: BTreeSet<String>,
    coverage_start: Option<i64>,
    coverage_end: Option<i64>,
    summary: Bucket,
    by_model: BTreeMap<String, Bucket>,
    by_model_daily: BTreeMap<(String, String), Bucket>,
    by_date: BTreeMap<String, Bucket>,
    by_hour: BTreeMap<String, Bucket>,
}

/// 会话数的两个口径：有会话列 ⇒ 去重计数；没有 ⇒ 一行一个会话（保守，且如实标记）。
fn session_stats(scan: &Scan) -> (u64, bool) {
    if !scan.sessions.is_empty() {
        (scan.sessions.len() as u64, false)
    } else if scan.messages > 0 {
        (scan.messages, true)
    } else {
        (0, false)
    }
}

impl Scan {
    /// 实际生效的取用量方式。
    ///
    /// 声明的是「表里有 token 列」，实际生效的取决于哪条通路真的读到了数：老库把用量
    /// 记成列、新库塞进正文 JSON，同一个库甚至可能两种都有。这里报**观测值**，
    /// 声明值只在没有任何一行取到用量时兜底。
    fn usage_mode(&self, detected: &Detected) -> &'static str {
        if self.column_rows > self.json_rows {
            "columns"
        } else if self.json_rows > 0 {
            "json"
        } else {
            detected.usage_mode()
        }
    }
}

/// 把聚合结果摊成前端要的五块（保持与 `token_stats` 相同的排序约定）。
fn build_aggregates(scan: &Scan) -> (Value, Vec<Value>, Vec<Value>, Vec<Value>, Vec<Value>) {
    let models = descending(scan.by_model.iter().map(|(k, v)| (k.as_str(), v)));
    let daily = scan
        .by_date
        .iter()
        .map(|(key, bucket)| bucket.group_value(key))
        .collect();
    let hours = scan
        .by_hour
        .iter()
        .map(|(key, bucket)| bucket.group_value(key))
        .collect();
    let model_daily = scan
        .by_model_daily
        .iter()
        .map(|((date, model), bucket)| {
            json!({
                "date": date,
                "model": model,
                "total": bucket.total(),
                "input": bucket.input,
                "output": bucket.output,
                "records": bucket.records,
            })
        })
        .collect();
    (
        scan.summary.summary_value(),
        models,
        daily,
        hours,
        model_daily,
    )
}

/// 按 `total` 降序（同名按键升序，保证渲染顺序稳定）。
fn descending<'a, I>(items: I) -> Vec<Value>
where
    I: Iterator<Item = (&'a str, &'a Bucket)>,
{
    let mut items: Vec<(&str, &Bucket)> = items.collect();
    items.sort_by(|left, right| {
        right
            .1
            .total()
            .cmp(&left.1.total())
            .then_with(|| left.0.cmp(right.0))
    });
    items
        .into_iter()
        .map(|(key, bucket)| bucket.group_value(key))
        .collect()
}

// ---------------------------------------------------------------------------
// 读库：探测 + 扫描
// ---------------------------------------------------------------------------

/// 打开并探测，然后扫一遍（探测与聚合共用的唯一入口）。
fn inspect(
    path: &str,
    tables: &mut Vec<String>,
    cutoff: Option<i64>,
) -> Result<(Detected, Scan), ImportError> {
    let conn = open_readonly(path)?;
    let detected = detect(&conn, tables)?;
    let scan = scan_table(&conn, &detected, cutoff)?;
    Ok((detected, scan))
}

/// 只读打开，并**先验证明文**。
///
/// 顺序很重要：`Connection::open` 对加密文件可能是「打开成功、第一条查询才报
/// `file is not a database`」，所以先用文件头判定——那是唯一能稳定区分
/// 「加密」与「空库 / 损坏库」的地方（空库有合法文件头，加密库没有）。
fn open_readonly(path: &str) -> Result<Connection, ImportError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ImportError::Missing);
    }
    let file = Path::new(trimmed);
    if !file.is_file() {
        return Err(ImportError::Missing);
    }
    ensure_plaintext(file)?;
    let conn = Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(map_sql_error)?;
    // 与 `session::open_db` 同口径：客户端可能正开着库，读不到锁时等 5 秒再说。
    let _ = conn.busy_timeout(Duration::from_secs(5));
    // 文件头像 SQLite、但页被加密工具改过时，只有第一条查询会暴露。
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })
    .map_err(map_sql_error)?;
    Ok(conn)
}

/// 文件头必须是 `SQLite format 3\0`。
fn ensure_plaintext(path: &Path) -> Result<(), ImportError> {
    let mut file = std::fs::File::open(path).map_err(|error| ImportError::Io(error.to_string()))?;
    let mut header = [0u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => {}
        // 连 16 字节都读不满 ⇒ 不可能是合法 SQLite（最小页 512 字节）。
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ImportError::NotPlaintext)
        }
        Err(error) => return Err(ImportError::Io(error.to_string())),
    }
    if &header == SQLITE_MAGIC {
        Ok(())
    } else {
        Err(ImportError::NotPlaintext)
    }
}

fn map_sql_error(error: rusqlite::Error) -> ImportError {
    let text = error.to_string();
    if text.contains("not a database") {
        ImportError::NotPlaintext
    } else {
        ImportError::Io(text)
    }
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// 表名清单（跳过 `sqlite_%` 内部表与 `fts_*` 影子表）。
///
/// FTS5 的 `<x>_data` / `<x>_idx` / `<x>_content` / `<x>_docsize` / `<x>_config` 是索引
/// 实现的产物，没有时间列也读不出可用文本，留着只会干扰打分。
fn list_tables(conn: &Connection) -> Result<Vec<String>, ImportError> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(map_sql_error)?;
    let mut rows = stmt.query([]).map_err(map_sql_error)?;
    let mut names = Vec::new();
    while let Some(row) = rows.next().map_err(map_sql_error)? {
        let name: String = row.get(0).map_err(map_sql_error)?;
        if name.starts_with("sqlite_") || name.starts_with("fts_") {
            continue;
        }
        names.push(name);
    }
    Ok(names)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, ImportError> {
    let sql = format!("PRAGMA table_info({})", quote_ident(table));
    let mut stmt = conn.prepare(&sql).map_err(map_sql_error)?;
    let mut rows = stmt.query([]).map_err(map_sql_error)?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next().map_err(map_sql_error)? {
        let name: String = row.get(1).map_err(map_sql_error)?;
        columns.push(name);
    }
    Ok(columns)
}

/// 在列名里按**候选优先级**找第一个命中（大小写不敏感）。
fn find_column(columns: &[String], candidates: &[&str]) -> Option<String> {
    for candidate in candidates {
        if let Some(found) = columns
            .iter()
            .find(|column| column.eq_ignore_ascii_case(candidate))
        {
            return Some(found.clone());
        }
    }
    None
}

fn usage_columns_for(columns: &[String]) -> Option<(String, Option<String>)> {
    // 输入列是必需的：只有输出列的库无法判断「这一行有没有用量」。
    let input = find_column(columns, &INPUT_COLUMNS)?;
    Some((input, find_column(columns, &OUTPUT_COLUMNS)))
}

/// 文本列的权重：越像「消息正文」越优先（同分时取先出现的表）。
fn text_rank(column: &str) -> i32 {
    match column.to_ascii_lowercase().as_str() {
        "content" => 3,
        "payload" | "body" | "message" | "text" => 2,
        "data" | "json" => 1,
        _ => 0,
    }
}

/// 探测消息表。
///
/// 打分而不是「命中就取第一张」：真机上必然同时存在 `session_project`、
/// `model_config_cache` 这类**也有时间列**的表，只有「正文里出现用量标记」那条
/// 才能把它们排掉。
///
/// `tables` 在**进入打分前**就回填：认不出结构（`Unrecognized`）时也要把表名清单交给
/// 用户，那是他反馈「我的库里到底有什么」的唯一线索。
fn detect(conn: &Connection, tables: &mut Vec<String>) -> Result<Detected, ImportError> {
    let names = list_tables(conn)?;
    *tables = names.clone();
    let mut best: Option<(i32, Detected)> = None;
    for name in names.iter() {
        let columns = table_columns(conn, name)?;
        // 「文本列 + 时间列」是硬门槛：没有时间的表无法归日，不算消息表。
        let (Some(text_column), Some(time_column)) = (
            find_column(&columns, &TEXT_COLUMNS),
            find_column(&columns, &TIME_COLUMNS),
        ) else {
            continue;
        };
        let session_column = find_column(&columns, &SESSION_COLUMNS);
        let model_column = find_column(&columns, &MODEL_COLUMNS);
        let usage_columns = usage_columns_for(&columns);
        let credits_column = find_column(&columns, &CREDITS_COLUMNS);
        let samples = sample_texts(conn, name, &text_column)?;
        let has_usage_marker = samples
            .iter()
            .any(|text| USAGE_MARKERS.iter().any(|marker| text.contains(marker)));

        let mut score = text_rank(&text_column);
        if has_usage_marker {
            score += 8;
        }
        if usage_columns.is_some() {
            score += 4;
        }
        if session_column.is_some() {
            score += 2;
        }
        if !samples.is_empty() {
            score += 1;
        }
        let candidate = Detected {
            table: name.clone(),
            text_column,
            time_column,
            session_column,
            model_column,
            usage_columns,
            credits_column,
            sample_rows: samples.len(),
            has_usage_marker,
        };
        let better = best
            .as_ref()
            .map(|(best_score, _)| score > *best_score)
            .unwrap_or(true);
        if better {
            best = Some((score, candidate));
        }
    }
    let detected = best
        .map(|(_, candidate)| candidate)
        .ok_or(ImportError::Unrecognized)?;
    Ok(detected)
}

/// 抽样正文（只用于探测打分，最多 [`SAMPLE_ROWS`] 行）。
fn sample_texts(conn: &Connection, table: &str, column: &str) -> Result<Vec<String>, ImportError> {
    let sql = format!(
        "SELECT {} FROM {} LIMIT {}",
        quote_ident(column),
        quote_ident(table),
        SAMPLE_ROWS
    );
    let mut stmt = conn.prepare(&sql).map_err(map_sql_error)?;
    let mut rows = stmt.query([]).map_err(map_sql_error)?;
    let mut texts = Vec::new();
    while let Some(row) = rows.next().map_err(map_sql_error)? {
        // 单行取不到就当空行——探测阶段不该因为一行坏数据整体失败。
        if let Some(text) = row.get::<_, SqlValue>(0).ok().and_then(|v| value_text(&v)) {
            texts.push(text);
        }
    }
    Ok(texts)
}

/// 扫一遍消息表（最多 [`MAX_ROWS_PER_TABLE`] 行），窗口过滤与聚合都在这里完成。
///
/// `cutoff` 为窗口下界（毫秒，`None` 表示不限）——探测走 `None`（要看全库），
/// 聚合走 [`cutoff_ms`] 的结果。
fn scan_table(
    conn: &Connection,
    detected: &Detected,
    cutoff: Option<i64>,
) -> Result<Scan, ImportError> {
    let sql = format!(
        "SELECT * FROM {} LIMIT {}",
        quote_ident(&detected.table),
        MAX_ROWS_PER_TABLE
    );
    let mut stmt = conn.prepare(&sql).map_err(map_sql_error)?;
    let headers: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let index = |column: &str| -> Option<usize> {
        headers
            .iter()
            .position(|name| name.eq_ignore_ascii_case(column))
    };
    let time_index = index(&detected.time_column).ok_or(ImportError::Unrecognized)?;
    let text_index = index(&detected.text_column).ok_or(ImportError::Unrecognized)?;
    let session_index = detected.session_column.as_deref().and_then(index);
    let model_index = detected.model_column.as_deref().and_then(index);
    let credits_index = detected.credits_column.as_deref().and_then(index);
    let (input_index, output_index) = match &detected.usage_columns {
        Some((input, output)) => (index(input), output.as_deref().and_then(index)),
        None => (None, None),
    };

    let mut scan = Scan::default();
    let mut rows = stmt.query([]).map_err(map_sql_error)?;
    while let Some(row) = rows.next().map_err(map_sql_error)? {
        scan.rows_seen += 1;
        let Some(ts) = row
            .get::<_, SqlValue>(time_index)
            .ok()
            .and_then(|value| normalize_ts(&value))
        else {
            scan.parse_errors += 1;
            continue;
        };
        // 窗口过滤（覆盖区间也只看窗口内，与 `token_stats::get_statistics` 同口径）。
        if cutoff.map(|minimum| ts < minimum).unwrap_or(false) {
            continue;
        }
        scan.coverage_start = Some(scan.coverage_start.map_or(ts, |current| current.min(ts)));
        scan.coverage_end = Some(scan.coverage_end.map_or(ts, |current| current.max(ts)));

        let text = row
            .get::<_, SqlValue>(text_index)
            .ok()
            .and_then(|value| value_text(&value))
            .unwrap_or_default();
        let parsed: Option<Value> = serde_json::from_str(&text).ok();

        let mut input = 0u64;
        let mut output = 0u64;
        let mut has_usage = false;
        if let Some(usage) = parsed.as_ref().and_then(json_usage) {
            input = usage.input;
            output = usage.output;
            has_usage = true;
            scan.json_rows += 1;
            /* ★ 一行里出现多份 `usage` 就记一笔。
            这是本模块**最可能不适配真实库**的地方：若客户端把「整段对话快照」存在
            一行里，累加会把历史轮次重复计数（放大 N 倍）。我们无法在不了解真实
            schema 的前提下判定是「多轮快照」还是「一条消息的多段」，所以不做猜测性
            去重，而是**把这件事数出来**并在 note 里披露 —— 让用户能看出数字可能虚高，
            而不是让一个看起来精确的错数字悄悄上屏。 */
            if usage.hits > 1 {
                scan.multi_usage_rows += 1;
            }
        }
        if !has_usage {
            // 回落：表里若有显式 token 列就直接读（老版本把用量记成列而不是 JSON）。
            let column_input = input_index
                .and_then(|idx| row.get::<_, SqlValue>(idx).ok())
                .and_then(|value| value_u64(&value));
            let column_output = output_index
                .and_then(|idx| row.get::<_, SqlValue>(idx).ok())
                .and_then(|value| value_u64(&value));
            if column_input.is_some() || column_output.is_some() {
                input = column_input.unwrap_or(0);
                output = column_output.unwrap_or(0);
                has_usage = true;
                scan.column_rows += 1;
            }
        }

        scan.messages += 1;
        if has_usage {
            scan.with_usage += 1;
        }
        if let Some(amount) = credits_index
            .and_then(|idx| row.get::<_, SqlValue>(idx).ok())
            .and_then(|value| value_u64(&value))
        {
            scan.credits = scan.credits.saturating_add(amount);
        }
        if let Some(id) = session_index
            .and_then(|idx| row.get::<_, SqlValue>(idx).ok())
            .and_then(|value| value_text(&value))
        {
            scan.sessions.insert(id.trim().to_string());
        }

        let model = model_index
            .and_then(|idx| row.get::<_, SqlValue>(idx).ok())
            .and_then(|value| value_text(&value))
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .or_else(|| parsed.as_ref().and_then(json_model))
            .unwrap_or_else(|| "（未指定）".to_string());

        scan.summary.add(input, output);
        let date = local_date(ts);
        scan.by_model
            .entry(model.clone())
            .or_default()
            .add(input, output);
        scan.by_model_daily
            .entry((date.clone(), model))
            .or_default()
            .add(input, output);
        scan.by_date.entry(date).or_default().add(input, output);
        scan.by_hour
            .entry(local_slot(ts))
            .or_default()
            .add(input, output);
    }
    scan.truncated = scan.rows_seen >= MAX_ROWS_PER_TABLE as u64;
    Ok(scan)
}

// ---------------------------------------------------------------------------
// 取值：SQL 列 / JSON 正文
// ---------------------------------------------------------------------------

/// 单元格 → 文本（`TEXT` / `BLOB` 两种存放方式都要认，空值返回 `None`）。
fn value_text(value: &SqlValue) -> Option<String> {
    let text = match value {
        SqlValue::Text(text) => text.clone(),
        // 有的版本把 JSON 直接存成 BLOB；非 UTF-8 的字节按替换字符处理，不让整行读失败。
        SqlValue::Blob(bytes) => String::from_utf8_lossy(bytes).to_string(),
        _ => return None,
    };
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 单元格 → 非负整数（`TEXT` 也尝试解析：有些库把计数存成字符串）。
fn value_u64(value: &SqlValue) -> Option<u64> {
    match value {
        SqlValue::Integer(number) if *number >= 0 => Some(*number as u64),
        SqlValue::Real(number) if *number >= 0.0 => Some(*number as u64),
        SqlValue::Text(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// 时间单元格 → 毫秒时间戳（**秒 / 毫秒**在这里归一）。
///
/// 判定用 [`MILLISECOND_THRESHOLD`]：客户端不同版本分别写秒和毫秒，同一个库里也可能
/// 混着两种（迁移未回填），所以逐行判定而**不是**按列判定。
fn normalize_ts(value: &SqlValue) -> Option<i64> {
    let raw = match value {
        SqlValue::Integer(number) => *number,
        SqlValue::Real(number) => *number as i64,
        SqlValue::Text(text) => parse_text_ts(text)?,
        _ => return None,
    };
    if raw <= 0 {
        return None;
    }
    Some(if raw > MILLISECOND_THRESHOLD {
        raw
    } else {
        raw.saturating_mul(1000)
    })
}

/// 文本时间 → 毫秒（纯数字按时间戳处理；其余按 RFC3339 / `YYYY-MM-DD HH:MM:SS` 解析）。
///
/// 无时区后缀的字符串按 **UTC** 解释：这是 SQLite 里 `CURRENT_TIMESTAMP` 的口径，
/// 也是唯一不会随本机时区漂移的选择（漂移会让同一份数据在不同机器上落到不同日）。
fn parse_text_ts(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(number) = trimmed.parse::<i64>() {
        return Some(number);
    }
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(parsed.timestamp_millis());
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(trimmed, format) {
            return Some(naive.and_utc().timestamp_millis());
        }
    }
    None
}

/// 从正文 JSON 里取 `(input, output)`。
///
/// ## 为什么要递归找 `usage`
///
/// 消息行不是一个固定形状：可能整行就是 `{"usage":{…}}`，也可能是
/// `{"messages":[{…,"usage":{…}}]}` 或 `{"data":{"usage":{…}}}`。写死「顶层取
/// `usage.input_tokens`」在真机上大概率取不到，因此按**任意层级的 `usage` 对象**取。
///
/// 多个 `usage` 对象（一行里含历史快照）时**累加**：一行通常对应一条消息，一份用量；
/// 出现多份说明这一行是「一整段对话」的快照，逐份累加才能覆盖全部用量。反过来的
/// 风险（重复快照导致重复计数）无法在不了解真实 schema 的前提下排除，已在模块文档与
/// 交付说明里登记为最可能的不适配点。
fn json_usage(value: &Value) -> Option<JsonUsage> {
    let mut total = (0u64, 0u64);
    let mut hits = 0usize;
    collect_usage(value, &mut total, &mut hits);
    if hits == 0 {
        None
    } else {
        Some(JsonUsage {
            input: total.0,
            output: total.1,
            hits,
        })
    }
}

/// 一行里提取得的用量。
#[derive(Debug, Clone, Copy)]
struct JsonUsage {
    input: u64,
    output: u64,
    /// 本行命中了几个 `usage` 对象。
    ///
    /// `> 1` = 这一行可能存的是「整段对话快照」而非「一条消息」⇒ 累加值可能含历史轮次。
    /// 我们不做猜测性去重（无从判断），只把它数出来让用户看见。
    hits: usize,
}

fn collect_usage(value: &Value, total: &mut (u64, u64), hits: &mut usize) {
    match value {
        Value::Object(map) => {
            // `token_usage` / `tokenUsage` 是老版本与驼峰命名的两种写法。
            for key in ["usage", "token_usage", "tokenUsage"] {
                if let Some(Value::Object(usage)) = map.get(key) {
                    if let Some(pair) = read_usage_object(usage) {
                        total.0 = total.0.saturating_add(pair.0);
                        total.1 = total.1.saturating_add(pair.1);
                        *hits += 1;
                    }
                }
            }
            for (_, child) in map {
                collect_usage(child, total, hits);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_usage(item, total, hits);
            }
        }
        _ => {}
    }
}

/// 一个 `usage` 对象 → `(input, output)`。
///
/// 优先级按任务书：`input_tokens` → `prompt_tokens`，`output_tokens` →
/// `completion_tokens`；驼峰变体一并接受，避免换个命名就整库读不出。
fn read_usage_object(usage: &Map<String, Value>) -> Option<(u64, u64)> {
    let input = pick_number(
        usage,
        &[
            "input_tokens",
            "inputTokens",
            "prompt_tokens",
            "promptTokens",
        ],
    );
    let output = pick_number(
        usage,
        &[
            "output_tokens",
            "outputTokens",
            "completion_tokens",
            "completionTokens",
        ],
    );
    if input.is_none() && output.is_none() {
        None
    } else {
        Some((input.unwrap_or(0), output.unwrap_or(0)))
    }
}

fn pick_number(map: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(value) = map.get(*key) {
            if let Some(number) = value.as_u64() {
                return Some(number);
            }
            // 浮点（`12.0`）与字符串计数都接受：不同客户端写法不一。
            if let Some(number) = value.as_f64() {
                if number >= 0.0 {
                    return Some(number as u64);
                }
            }
            if let Some(text) = value.as_str() {
                if let Ok(number) = text.trim().parse::<u64>() {
                    return Some(number);
                }
            }
        }
    }
    None
}

/// 从正文 JSON 里捞模型名（取第一个命中的字符串，深度优先）。
fn json_model(value: &Value) -> Option<String> {
    find_string_key(value, &["model", "model_name", "modelName"])
}

fn find_string_key(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(Value::String(text)) = map.get(*key) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
            for (_, child) in map {
                if let Some(found) = find_string_key(child, keys) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_string_key(item, keys)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 时间格式化（与 `token_stats::local_*` **同一口径**）
// ---------------------------------------------------------------------------

/// 窗口下界（毫秒）。`None` / `<= 0` ⇒ 不限。
fn cutoff_ms(days: Option<i64>) -> Option<i64> {
    let days = days?;
    if days <= 0 {
        return None;
    }
    let now = chrono::Local::now().timestamp_millis();
    Some(now - days * 24 * 60 * 60 * 1000)
}

/// 毫秒时间戳 → 本地日期 `YYYY-MM-DD`。
fn local_date(ts_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts_ms)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| "未知".to_string())
}

/// 毫秒时间戳 → 本地小时 `00`..`23`（两位补零）。
fn local_hour(ts_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts_ms)
        .map(|utc| utc.with_timezone(&chrono::Local).format("%H").to_string())
        .unwrap_or_else(|| "--".to_string())
}

/// 毫秒时间戳 → 本地星期 `0`..`6`（**周一 = 0**，与 `token_stats::local_weekday` 同口径：
/// 前端 `HeatmapMatrix.cells[weekday][hour]` 与原型 `WEEKDAYS = ["周一"…]` 都按它排）。
fn local_weekday(ts_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts_ms)
        .map(|utc| {
            use chrono::Datelike;
            utc.with_timezone(&chrono::Local)
                .weekday()
                .num_days_from_monday()
                .to_string()
        })
        .unwrap_or_else(|| "-".to_string())
}

/// 毫秒时间戳 → 热力槽位 `"{weekday}-{HH}"`（如 `"0-09"`；键的字符串升序即周→小时）。
fn local_slot(ts_ms: i64) -> String {
    format!("{}-{}", local_weekday(ts_ms), local_hour(ts_ms))
}
