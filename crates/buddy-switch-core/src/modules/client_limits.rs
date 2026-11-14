//! 客户端限流可视化：**账号 × 模型 × 恢复时刻**（纯只读观测）。
//!
//! 三张已有的本地表 join 即得，**不依赖网关、不代理客户端流量、不要求客户端配合**：
//!
//! | # | 数据源 | 内容 |
//! |---|---|---|
//! | ① | `{data_dir}/projects/**/*.jsonl` | 限流错误本体 + 发生时刻 + `sessionId` |
//! | ② | `{data_dir}/workbuddy.db` → `sessions`（**只读**） | `model` + `user_id` |
//! | ③ | 账号库 `accounts.json` / `accounts.global.json` | `uid` → `nickname` |
//!
//! `{data_dir}` = CN `~/.workbuddy`、Global `~/.workbuddy-ai`（见 [`session_data_dir`]）。
//!
//! 完整背景与验收口径见 `.trellis/tasks/09-22-client-rate-limit-visibility/prd.md`。
//!
//! ## 硬约束（PRD R1–R10，改动前先读）
//!
//! - **只读客户端数据**：`workbuddy.db` 以 `SQLITE_OPEN_READ_ONLY` 打开，**绝不写库**
//!   （它是 WAL 库，客户端运行时也在写；本功能不得要求用户关闭客户端）。
//!   ⚠️ 本模块**唯一**的写操作是 [`store_dir`] 下自己的扫描缓存
//!   （`client-limits-scan-cache.json`，PRD S5 明确许可），**不碰客户端任何文件**。
//! - **以 jsonl 为主表**：join 失败（会话已删除 / playground）**不得丢弃**记录，
//!   落「未知账号 / 未知模型」兜底。
//! - **uid 走 `auth_raw.account.uid` / `profile_raw.uid`**，**绝不用**账号库本地 `id`。
//! - **恢复时刻固定按 UTC+8 解释**（复用 [`cst::cst_offset`]），**不按系统时区**。
//! - 解析失败保留原始 `code` / `message`，**不静默丢弃**。
//!
//! ## ★★ 「仍受限」必须有**实证**（2026-10-12 修正 · 本轮最关键的一条）
//!
//! 上游 `6004` 文案里的 `will reset at <T> UTC+8` **不是解锁时刻，是配额窗口的重置时刻**。
//! 实测（Global，会话 `9b9f45ca`）：17:20:11 报 429「reset at 2026-10-13 09:04:43」，
//! 而 **17:36:50 起同一会话就恢复了正常产出**（`reasoning` / `function_call` /
//! `function_call_result completed`，17:37:07 产出完整 assistant 消息）。
//! ⇒ 只按 `reset_at > now` 判「仍受限」会把**正在正常使用**的账号报成受限（实测 6/6 全错）。
//!
//! 现在的判据是**反证优先**：只有当「该限流事件**之后**确实有成功的模型活动」这一
//! 反证**不存在**时，才敢说「仍受限」。实证来源见 [`is_success_record`]，
//! 出口字段 `evidenceAt`。
//!
//! ## ★★ 扫描性能（2026-10-12 起，2026-10-13 再优化）
//!
//! 最初是**逐字读完整份文件**：实测 Global「近 7 天」单线程读一遍 **103 s**，
//! 界面基本处于假死状态。现在的四道手段：
//!
//! 1. **目录遍历**：`entry.file_type()` 取代 `path.is_dir()`（同一棵树 **0.69 s → 0.03 s**），
//!    一次遍历同时完成「判类型 + 取 mtime + 应用预筛」。
//! 2. **尾部有界读取**（[`jsonl_tail`]，与「今日 Token」共用同一实现）：会话 jsonl 是
//!    **只追加**的 ⇒ 窗口内的记录一定在文件**尾部**。起始块 32 KB、逐次翻倍，
//!    每次只读新增的那一段。实测实读字节从 5.28 GB 降到 **1.25 GB**（预筛文件总大小
//!    49.9 GB 的 2.5%）。
//! 3. **解析前廉价预筛**（[`line_may_matter`]）：窗口里绝大多数行与台账无关，先按子串跳过。
//! 4. **并行解析 + 持久化缓存**：`worker_count`（上限 32）+ `client-limits-scan-cache.json`
//!    （键 = 绝对路径，命中 = `(mtime, size)` 未变 **且** 缓存覆盖 ⊇ 本次窗口）。
//!
//! **实测**（Global，近 7 天，`examples/client_limits_bench.rs`）：
//!
//! | 场景 | 优化前 | 现在 |
//! |---|---:|---:|
//! | 冷启（缓存空 + OS 缓存冷） | ≈20.4 s | **≈4.5 s** |
//! | 热刷（全部命中缓存） | ≈1.4 s | **≈0.05 s** |
//!
//! ⚠️ 缓存的 `covered_from_ms` 语义是「**逻辑上保证完整**的那段区间」，不是「读了多少字节」；
//! 且**命中 ≠ 结果可直接用** —— 缓存是本次窗口的**超集**，命中路径必须再过一遍
//! [`FileScan::narrowed_to`]（见该函数与 `cache_covers` 的注释）。
//!
//! 缓存的设计与 `modules::token_stats` 的同名机制**同源同思路**（那边为 P0-4 已落地）；
//! 这里**刻意不抽公共模块**：两边缓存载荷完全不同（那边是几十个 `Totals` 的聚合，
//! 这边是「几条错误 + 一个时间戳」）。**改动任一侧时请对照另一侧。**
//! （⚠️ 尾部**读取**则相反：语义完全相同，已抽成 [`jsonl_tail`] 唯一实现。）
//!
//! ## 统计口径（2026-10-12 早先一轮修正）
//!
//! 1. **窗口按「记录自身的时间」判定**，文件 `mtime` 只是「跳过明显过期文件」的**性能预筛**。
//! 2. **只有限流类错误进台账**（[`is_rate_limit_error`]）；被排除的条数在
//!    `counts.skipped` 里**如实报出**。
//! 3. **去重键含错误码**，每条带 `occurrences`（合并了多少条原始记录）。
//! 4. **`reset_at` 解析不出时状态是「未知」，不是「已恢复」**（[`LimitState`]）。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::modules::account;
use crate::modules::config::{atomic_write, now_ms, store_dir};
use crate::modules::cst;
use crate::modules::jsonl_tail;
use crate::modules::region::Region;
use crate::modules::session::{session_data_dir, workbuddy_db_path_for};

/// 默认扫描窗口（近 7 天，PRD S5）。
pub const DEFAULT_SINCE_DAYS: i64 = 7;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// 尾部残行重读前的等待。客户端通常毫秒级就写完一行，30ms 足够且不阻塞界面。
const TORN_TAIL_RETRY_DELAY: Duration = Duration::from_millis(30);

/// 已知的**限流类**错误码（实测口径，见 PRD「错误码语义分布」）。
///
/// `6004` = 配额/频率限流（占绝对多数）；`6020` / `6022` = 上游排队块（语义文案待拍板，R7）。
/// ⚠️ 这是**加分项不是判据**：上游换码时靠 [`is_rate_limit_error`] 的 `status == 429`
/// 与文案兜底，**不得**把「不在表里」等同于「不是限流」。
/// （实测 Global 今日还有 `14003`「429 too many requests」与 `14018`「429 Credits exhausted」，
/// 都靠 `status == 429` 正确收下；而 `11155`(400) / `502` / `3002`(502) / `ECONNRESET`
/// 都是网络或客户端缺陷，正确排除。）
const RATE_LIMIT_CODES: [i64; 3] = [6004, 6020, 6022];

/// 文案里的限流标记（小写比较）。用于「状态码/错误码都变了，但语义仍是限流」的兜底。
const RATE_LIMIT_MARKERS: [&str; 6] = [
    "frequency limit",
    "usage limit",
    "rate limit",
    "频率限制",
    "使用上限",
    "queue.",
];

/// 一条从 jsonl 提取的**原始**限流错误（未 join、未聚合）。
///
/// `Serialize` / `Deserialize` 只服务于扫描缓存（[`ScanCacheEntry`]）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawClientError {
    session_id: String,
    timestamp: i64,
    /// 数值错误码（`6004`）；字符串码为 `None`。
    code: Option<i64>,
    /// 字符串错误码原文（实测 `"ECONNRESET"`）。⚠️ 只认数值会把这类记录整条丢掉（违反 R6）。
    code_text: Option<String>,
    status: Option<i64>,
    message: String,
    agent: Option<String>,
    cwd: Option<String>,
    reset_at: Option<i64>,
    /// 本条错误**所在文件**中，「该错误之后仍有过成功模型活动」的最早时刻（`0` = 无）。
    ///
    /// 派生字段：缓存里按文件存一份，装载时回填（[`ScanCacheEntry::to_scan`]）。
    #[serde(skip)]
    file_success_ms: i64,
    /// 本条错误**所在文件**中，「用户/客户端在它之后**又试过**」的最晚时刻（`0` = 没再试）。
    ///
    /// 「又试过」= 又出现一条用户消息或又一次错误。见 [`is_attempt_record`]。
    #[serde(skip)]
    file_attempt_ms: i64,
}

/// 条目的**三态**状态。
///
/// ⚠️ 二值 `active` 是错的：`reset_at` 解析不出时「不知道」既不等于「仍受限」也不等于
/// 「已恢复」。界面必须能区分，否则就是把「未知」谎报成结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitState {
    /// 该事件之后**又试过**（用户消息 / 又一次错误）却仍无成功产出 —— 此刻仍受限。
    ///
    /// ⚠️ 只有 `reset_at > now` **不构成**「仍受限」：那个时刻是**配额窗口**的重置时刻，
    /// 实测报 429 后 16 分钟就恢复了产出（见模块头）。
    Active,
    /// 既无成功实证、也无「又试过」实证 —— **状态未知**（含「错误就是该会话最后一条」、
    /// 文案漂移解析不出 `reset_at`、排队类错误本就没有恢复时刻）。
    Unknown,
    /// 已过恢复时刻，**或**该事件之后确实有成功的模型活动。
    Recovered,
}

impl LimitState {
    /// 排序权重：仍受限 > 未知 > 已恢复（未知比「确定已恢复」更需要被看见）。
    fn rank(self) -> u8 {
        match self {
            LimitState::Active => 0,
            LimitState::Unknown => 1,
            LimitState::Recovered => 2,
        }
    }
}

/// 一条聚合后的「限流台账」条目（下发前端的形状）。
///
/// `account` / `model` 为 `None` 表示 **join 失败**（未知账号 / 未知模型）—— 由前端渲染
/// 「未知」而**不是**丢弃该条（PRD R1）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientLimitEntry {
    /// 账号昵称（join 失败为 `null`）。
    pub account: Option<String>,
    /// 原始 `user_id`（= 云端 uid），便于排障；join 失败为 `null`。
    pub account_uid: Option<String>,
    /// 会话所选模型；join 失败为 `null`。⚠️ `hy4-preview` 与 `hy4-preview-f` 是**两个**模型，不可前缀合并。
    pub model: Option<String>,
    /// 上游错误码（`6004` 限流 / `6020` / `6022` 排队 / `14003` / …）；字符串码为 `null`。
    pub code: Option<i64>,
    /// 字符串错误码原文（实测 `"ECONNRESET"`）；数值码为 `null`。
    pub code_text: Option<String>,
    /// HTTP 状态码（限流多为 `429`）。
    pub status: Option<i64>,
    /// 上游文案里写的恢复时刻（毫秒）；文案漂移解析不出为 `null`。
    ///
    /// ⚠️ **这是配额窗口的重置时刻，不等于解锁时刻** —— 见模块头「仍受限必须有实证」。
    pub reset_at: Option<i64>,
    /// 本条目内**最近一次**发生时刻（毫秒）。
    pub occurred_at: i64,
    /// **实测可用时刻**：`occurred_at` 之后、该账号在该模型上确实成功产出过的最早时刻
    /// （毫秒）；无实证为 `null`。有值时 `state` 必为 `recovered`。
    pub evidence_at: Option<i64>,
    /// 三态状态（见 [`LimitState`]）。
    pub state: LimitState,
    /// 合并进本条目的**原始记录数**（`1` = 未去重）。让「条数」可被解释，而不是黑箱。
    pub occurrences: u32,
    /// **原始**上游文案（PRD R6：解析失败时保留原文，不静默丢弃）。
    pub message: String,
    /// 会话 ID（便于跳转定位）。
    pub session_id: String,
    /// 会话工作目录（可能为真实项目路径，展示层只给必要信息）。
    pub cwd: Option<String>,
    /// `providerData.agent`（`cli` / 子代理名）。
    pub agent: Option<String>,
    /// 所属 region（`cn` / `global`）。
    pub region: String,
    /// 聚合内部用：桶内「该错误之后又试过」的最晚时刻。**不下发**。
    #[serde(skip)]
    attempt_ms: i64,
}

/// 聚合键：`(uid, model, 错误码标识, reset_at)`。
///
/// ⚠️ **`code` 必须在键里**。只有 `(uid, model, reset_at)` 时，`reset_at` 解析不出的记录
/// 会退化到 `(uid, model, None)`，把同一账号同模型下**不同错误码、不同时刻**的事件并成一条。
type LimitKey = (Option<String>, Option<String>, Option<String>, Option<i64>);

/// 当前限流台账（按 region + 时间窗口）。
///
/// `since_days`：`None` ⇒ 默认近 7 天；`Some(0)` ⇒ 全量（无时间下界）；
/// `Some(n>0)` ⇒ 近 n 天；其余（负数）⇒ 默认近 7 天。
///
/// 返回 `{ region, since, now, counts: { scanned, entries, active, unknown, skipped }, limits: [...] }`。
pub fn client_limits_for(region: Region, since_days: Option<i64>) -> Value {
    // `now` **只取一次**：窗口下界、`state` 判定、`active` 计数共用同一个基准，
    // 否则同一次请求内部就可能自相矛盾（扫描耗时跨越了某个 reset 时刻）。
    let now = now_ms();
    let since_ms = resolve_since_ms(since_days, now);
    let scans = scan_client_errors_for(region, since_ms);
    let (errors, skipped) = collect_rate_limit_errors(scans);
    let scanned = errors.len();
    let meta = load_session_meta_for(region);
    let names = load_uid_nicknames_for(region);
    let limits = build_limits(region, errors, &meta, &names, now);
    let active = limits
        .iter()
        .filter(|entry| entry.state == LimitState::Active)
        .count();
    let unknown = limits
        .iter()
        .filter(|entry| entry.state == LimitState::Unknown)
        .count();
    // 有新解析才写盘；热刷新（全部命中缓存）一个字节都不写。
    persist_scan_cache_if_dirty();
    json!({
        "region": region.as_str(),
        "since": if since_ms > 0 { json!(since_ms) } else { Value::Null },
        "now": now,
        "counts": {
            // 窗口内**限流类**原始记录数（已过记录级时间过滤）。
            "scanned": scanned,
            // 聚合后的台账条数。
            "entries": limits.len(),
            "active": active,
            "unknown": unknown,
            // 窗口内被判定为**非限流**（网络类等）而排除的原始记录数。报出来 = 不静默丢弃。
            "skipped": skipped,
        },
        "limits": limits,
    })
}

/// 扫描窗口的毫秒下界；`0` 表示全量（无下界）。
fn resolve_since_ms(since_days: Option<i64>, now: i64) -> i64 {
    match since_days {
        Some(0) => 0,
        Some(days) if days > 0 => now - days * DAY_MS,
        _ => now - DEFAULT_SINCE_DAYS * DAY_MS,
    }
}

/// 记录是否落在窗口内。`since_ms <= 0` = 全量 ⇒ 恒真。
///
/// ⚠️ 判据是**记录自身的 `timestamp`**，不是文件 `mtime`。只按 `mtime` 会让「最近仍在写的
/// 会话文件」把窗口外的旧错误一起带进来（实测「近 7 天」混进 6 条窗口外记录）。
fn in_window(timestamp: i64, since_ms: i64) -> bool {
    since_ms <= 0 || timestamp >= since_ms
}

/// 是否为**限流类**错误 —— 只有限流类才进台账。
///
/// 判据（任一命中）：
/// 1. `status == 429`（限流的规范状态码，覆盖面最广）；
/// 2. `code` 命中 [`RATE_LIMIT_CODES`]；
/// 3. 文案命中 [`RATE_LIMIT_MARKERS`]。
///
/// 反面（实测语料）：`3002` / `502` 是「网络不可达 / 代理 TLS 失败」，
/// `ECONNRESET`（文案 `aborted`）是断连，`11155`(400) 是客户端 reasoning 回传缺陷
/// —— **都不是限流**。
fn is_rate_limit_error(error: &RawClientError) -> bool {
    if error.status == Some(429) {
        return true;
    }
    if error
        .code
        .is_some_and(|code| RATE_LIMIT_CODES.contains(&code))
    {
        return true;
    }
    let lowered = error.message.to_lowercase();
    RATE_LIMIT_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

/// 一条记录是否代表「**用户/客户端又试了一次**」。
///
/// 判据：用户消息（`type == "message" && role == "user"`）或**任意上游错误记录**。
/// ⚠️ **不取** `file-history-snapshot` / `session-meta` 之类的客户端内务记录：
/// 它们在会话闲置时也会周期性落盘，拿它们当「又试过」会把闲置会话误判成「仍在尝试」。
fn is_attempt_record(value: &Value) -> bool {
    if value.get("type").and_then(Value::as_str) == Some("message")
        && value.get("role").and_then(Value::as_str) == Some("user")
    {
        return true;
    }
    value
        .get("providerData")
        .and_then(|provider| provider.get("error"))
        .is_some_and(Value::is_object)
}

/// 一条记录是否证明「**模型确实工作过**」（= 该账号在该模型上没被挡住）。
///
/// 判据（且**无** `providerData.error`）：
/// - `type == "reasoning"` —— 模型产出了推理；
/// - `type == "function_call"` —— 模型产出了工具调用；
/// - `type == "message" && role == "assistant" && status == "completed"` —— 模型产出了完整回复。
///
/// ⚠️ **不取** `function_call_result`：那是工具执行结果，模型未必参与了这一次往返。
fn is_success_record(value: &Value) -> bool {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "reasoning" | "function_call" => true,
        "message" => {
            value.get("role").and_then(Value::as_str) == Some("assistant")
                && value.get("status").and_then(Value::as_str) == Some("completed")
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// ① jsonl 扫描
// ---------------------------------------------------------------------------

/// 单文件的扫描结果（= 扫描缓存的载荷）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FileScan {
    /// 文件所属会话（取文件内任意记录的 `sessionId`，回落文件名 stem）。
    session_id: String,
    /// 窗口内错误记录。
    errors: Vec<RawClientError>,
    /// 窗口内「用户/客户端**又试过**」的最晚时刻（`0` = 没再试）。
    last_attempt_ms: i64,
    /// 窗口内「模型确实工作过」的**最晚**时刻（`0` = 无）。
    ///
    /// ⚠️ **必须是最晚，不是最早**：判据要回答的是「该错误之后**是否存在**成功产出」，
    /// 而 `max > occurred_at` 恰好等价于「存在一个晚于它的成功」。存最早值会让
    /// 判据几乎恒假（会话里最早的成功几乎总在错误**之前**）—— 实测踩到：6 条本该
    /// 判「已恢复」的条目全部仍报「仍受限」。
    last_success_ms: i64,
}

/// 扫描 + 分类：返回 `(限流类错误, 被排除的原始记录数)`。
///
/// 被排除的条数**必须报出去**（进 `counts.skipped`），不是静默丢弃。
fn collect_rate_limit_errors(scans: Vec<FileScan>) -> (Vec<RawClientError>, usize) {
    let mut kept = Vec::new();
    let mut skipped = 0usize;
    for scan in scans {
        let success = scan.last_success_ms;
        let attempt = scan.last_attempt_ms;
        for mut error in scan.errors {
            error.file_success_ms = success;
            error.file_attempt_ms = attempt;
            if is_rate_limit_error(&error) {
                kept.push(error);
            } else {
                skipped += 1;
            }
        }
    }
    (kept, skipped)
}

/// 扫 `{data_dir}/projects/**/*.jsonl`，返回窗口内的逐文件扫描结果。
fn scan_client_errors_for(region: Region, since_ms: i64) -> Vec<FileScan> {
    scan_client_errors_in(&session_data_dir(region).join("projects"), since_ms)
}

/// [`scan_client_errors_for`] 的可注入目录版本（单测用临时目录驱动整条扫描链）。
fn scan_client_errors_in(projects: &Path, since_ms: i64) -> Vec<FileScan> {
    let mut files: Vec<FileEntry> = Vec::new();
    collect_jsonl_files(projects, since_ms, &mut files);
    // `read_dir` 的返回顺序**不保证稳定** ⇒ 先按路径排序，让缓存与结果都可复现。
    files.sort_by(|a, b| a.path.cmp(&b.path));
    scan_entries(&files, since_ms)
}

/// 文件条目：路径 + 一次 `stat` 拿到的指纹（`mtime` 同时用于窗口预筛与缓存校验）。
struct FileEntry {
    path: PathBuf,
    mtime_ms: i64,
    size: u64,
}

/// 递归收集 `.jsonl` 文件；按 `mtime` 预筛（`since_ms > 0` 时）。
///
/// 取不到 `mtime` 时**不排除**（宁可多扫一个文件，也不漏一条记录）——与 R1 的
/// 「join 失败不丢弃」同一取向：观测功能的默认倾向是**保守保留**。
///
/// 预筛是**保守安全**的：会话 jsonl 只追加 ⇒ 文件 `mtime` 不早于文件内任何记录的写入时刻
/// ⇒ 整体跳过过旧文件不会漏掉窗口内记录。
fn collect_jsonl_files(dir: &Path, since_ms: i64, out: &mut Vec<FileEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // ⚠️ 必须用 `entry.file_type()`，**不要**用 `path.is_dir()`：
        // 后者每个条目都要一次 `stat` 系统调用。实测同一棵树（Global，26,932 个文件）
        // 遍历 **0.43s → 0.03s**。
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_jsonl_files(&entry.path(), since_ms, out);
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        // 一次 `stat` 同时喂给「窗口预筛」和「缓存校验」，避免二次系统调用。
        let stamp = entry.metadata().ok().and_then(|meta| file_stamp(&meta));
        let (mtime_ms, size) = stamp.unwrap_or((i64::MAX, 0));
        if since_ms > 0 && mtime_ms < since_ms {
            continue;
        }
        out.push(FileEntry {
            path,
            mtime_ms,
            size,
        });
    }
}

fn file_stamp(meta: &std::fs::Metadata) -> Option<(i64, u64)> {
    let millis = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some((i64::try_from(millis).ok()?, meta.len()))
}

// ---------------------------------------------------------------------------
// 扫描缓存（进程内 + 落盘）
// ---------------------------------------------------------------------------

/// 落盘格式版本。**任何字段语义变化都必须 +1** —— 不匹配时整体丢弃
/// （宁可重扫一次，不可读出错值）。
///
/// v2（2026-10-13）：尾部窗口的**停止判据**修掉了一个会截断窗口的缺陷
/// （半行里的嵌套秒级时间戳，见 [`jsonl_tail::complete_lines_reach_before`]）。
/// 旧缓存里的条目是**按被截断的窗口**解析出来的，却声称「对 `ts >= covered` 完整」——
/// 那正是本字段要挡住的「读出错值」。不 +1 的话，旧条目会一直被命中，漏掉的记录永远补不回来。
const SCAN_CACHE_VERSION: u32 = 2;

/// 单文件的缓存条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScanCacheEntry {
    mtime_ms: i64,
    size: u64,
    /// 解析这份 `scan` 时**实际生效的窗口下界**：
    /// - `None` ⇒ 无有效覆盖（**永不命中**，同时是 serde 缺字段时的默认值 ⇒ 安全）；
    /// - `Some(0)` ⇒ 按**全量**解析过（覆盖一切窗口）；
    /// - `Some(since)` ⇒ 已完整收录 `ts >= since` 的记录。
    ///
    /// ⚠️ 这里存的**不是「读了多少字节」**，而是「逻辑上保证完整的那段区间」。
    /// 尾部读取只是 I/O 优化：无论读了多少字节，只要解析时用的是窗口 `since`，
    /// 结果对 `ts >= since` 就是完整的。存成「读到的最早字节」会让**窗口变宽时命中
    /// 一份被过滤过的结果**（实测踩到：全量扫描拿到 1 条而非 2 条）。
    covered_from_ms: Option<i64>,
    #[serde(default)]
    scan: FileScan,
}

impl ScanCacheEntry {
    /// 缓存里的错误记录**不含** `file_success_ms`（那是派生字段），装载时回填。
    fn to_scan(&self) -> FileScan {
        let mut scan = self.scan.clone();
        let success = scan.last_success_ms;
        let attempt = scan.last_attempt_ms;
        for error in &mut scan.errors {
            error.file_success_ms = success;
            error.file_attempt_ms = attempt;
        }
        scan
    }
}

impl FileScan {
    /// 把缓存里（可能按**更宽**窗口解析出来的）结果**收敛到本次窗口**。
    ///
    /// 缓存命中条件是「缓存覆盖 ⊇ 本次窗口」（见 [`cache_covers`]），因此命中时缓存里
    /// 可能带着窗口**之外**的旧记录 —— 直接用会把它们带进台账。
    /// 实测踩到：先用 30 天窗口刷过一次、再切回「近 7 天」，09-16 / 09-23 的记录仍在列表里
    /// （`entries` 94 → 27、`scanned` 9673 → 119，其中「仍受限」那条完全是窗口外的旧记录）。
    ///
    /// `last_attempt_ms` / `last_success_ms` 取自更宽的窗口**不会**改变判据结果：
    /// 两者只有**严格晚于** `occurred_at` 才起作用，而窗口外的记录必然更早
    /// （窗口内错误的 `timestamp >= since_ms > 任何窗口外记录的时刻`）。
    fn narrowed_to(&self, since_ms: i64) -> FileScan {
        let mut scan = self.clone();
        scan.errors
            .retain(|error| in_window(error.timestamp, since_ms));
        scan
    }
}

/// 按**绝对路径**缓存单文件解析结果（进程内）。
fn scan_cache() -> &'static Mutex<HashMap<PathBuf, ScanCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, ScanCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 仅供测试：移除**某一个路径**的缓存条目。
///
/// 刻意按路径清除而不是整表清空：`cargo test` 并行跑用例，整表清空会把别的用例
/// 正在做的「冷 → 热」序列打断，让它们的「命中不再解析」断言随机变红。
#[cfg(test)]
fn clear_scan_cache_entry(path: &Path) {
    scan_cache().lock().unwrap().remove(path);
}

fn scan_cache_path() -> PathBuf {
    store_dir().join("client-limits-scan-cache.json")
}

/// `ensure_cache_loaded` 的可复位句柄（`OnceLock` 无法复位，故用 `AtomicBool`）。
static CACHE_LOADED_FLAG: AtomicBool = AtomicBool::new(false);

/// 进程内**有过新解析** —— 只有这时才值得写盘。
static CACHE_DIRTY: AtomicBool = AtomicBool::new(false);

/// 落盘表示（`files` 用 `BTreeMap` ⇒ 输出稳定，便于人工 diff）。
#[derive(Serialize, Deserialize)]
struct PersistedScanCache {
    version: u32,
    files: std::collections::BTreeMap<String, ScanCacheEntry>,
}

/// 从指定路径读缓存。`None` = **没有可用缓存**（文件不存在 / 坏 JSON / 版本不符）。
///
/// 抽成显式路径的纯函数，一是让 `ensure_cache_loaded` 保持单行语义，
/// 二是让用例能直接喂临时文件测试，**不必依赖进程级一次性标记**。
fn read_persisted_cache(path: &Path) -> Option<HashMap<PathBuf, ScanCacheEntry>> {
    let text = std::fs::read_to_string(path).ok()?;
    let persisted = serde_json::from_str::<PersistedScanCache>(&text).ok()?;
    if persisted.version != SCAN_CACHE_VERSION {
        return None;
    }
    Some(
        persisted
            .files
            .into_iter()
            .map(|(file, entry)| (PathBuf::from(file), entry))
            .collect(),
    )
}

/// 进程内只从盘上装载一次。**任何失败都静默回落成空缓存**：缓存是纯加速，
/// 读不到就重扫，绝不能因为缓存坏了让统计失败。
fn ensure_cache_loaded() {
    if CACHE_LOADED_FLAG.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(loaded) = read_persisted_cache(&scan_cache_path()) else {
        return;
    };
    let mut cache = scan_cache().lock().unwrap();
    for (path, entry) in loaded {
        cache.insert(path, entry);
    }
}

fn write_persisted_cache(path: &Path, cache: &HashMap<PathBuf, ScanCacheEntry>) {
    // 丢掉文件已不存在的条目：否则缓存只增不减（用户删掉旧会话后条目永远留着）。
    // 这是**纯清理**，不影响正确性 —— 不存在的文件本来也不会被查询。
    let files: std::collections::BTreeMap<String, ScanCacheEntry> = cache
        .iter()
        .filter(|(file, _)| file.exists())
        .map(|(file, entry)| (file.to_string_lossy().into_owned(), entry.clone()))
        .collect();
    let payload = PersistedScanCache {
        version: SCAN_CACHE_VERSION,
        files,
    };
    let Ok(text) = serde_json::to_string(&payload) else {
        return;
    };
    let _ = std::fs::create_dir_all(store_dir());
    let _ = atomic_write(path, &text);
}

/// 有新解析时把缓存落盘。**没有新解析就一个字节都不写** —— 热路径零成本。
fn persist_scan_cache_if_dirty() {
    if !CACHE_DIRTY.swap(false, Ordering::AcqRel) {
        return;
    }
    let cache = scan_cache().lock().unwrap();
    write_persisted_cache(&scan_cache_path(), &cache);
}

/// 缓存条目是否覆盖本次窗口。
///
/// ⚠️ 命中**不等于**结果可直接用：命中只说明「缓存里的记录集 ⊇ 本次窗口所需」，
/// 缓存里**可能带着窗口之外的旧记录** ⇒ 命中路径必须再走 [`FileScan::narrowed_to`]。
fn cache_covers(entry: &ScanCacheEntry, since_ms: i64) -> bool {
    match entry.covered_from_ms {
        // 无有效覆盖 ⇒ 永不命中（兜住坏缓存 / 旧格式）。
        None => false,
        // 按全量解析过 ⇒ 覆盖一切窗口。
        Some(0) => true,
        // 按某个窗口解析过 ⇒ 只要本次窗口不比它更宽就命中。
        Some(covered) => since_ms > 0 && since_ms >= covered,
    }
}

/// 并行度：够用即可。上限 32 是刻意的 —— 再多只会在同一条磁盘队列上排队。
fn worker_count(pending: usize) -> usize {
    let cores = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4);
    cores.clamp(1, 32).min(pending.max(1))
}

/// **先查缓存，未命中的并行解析**，按 `files` 顺序返回（顺序保证输出可复现）。
fn scan_entries(files: &[FileEntry], since_ms: i64) -> Vec<FileScan> {
    ensure_cache_loaded();

    let mut slots: Vec<Option<FileScan>> = (0..files.len()).map(|_| None).collect();
    let mut pending: Vec<usize> = Vec::new();

    {
        let cache = scan_cache().lock().unwrap();
        for (index, file) in files.iter().enumerate() {
            let hit = cache
                .get(&file.path)
                .filter(|entry| {
                    entry.mtime_ms == file.mtime_ms
                        && entry.size == file.size
                        && cache_covers(entry, since_ms)
                })
                .map(|entry| ScanCacheEntry::to_scan(entry).narrowed_to(since_ms));
            match hit {
                Some(scan) => slots[index] = Some(scan),
                None => pending.push(index),
            }
        }
    }

    if pending.is_empty() {
        return slots.into_iter().map(Option::unwrap_or_default).collect();
    }
    CACHE_DIRTY.store(true, Ordering::Release);

    let next = AtomicUsize::new(0);
    let parsed: Mutex<Vec<(usize, FileScan, Option<i64>)>> =
        Mutex::new(Vec::with_capacity(pending.len()));
    std::thread::scope(|scope| {
        for _ in 0..worker_count(pending.len()) {
            scope.spawn(|| loop {
                let cursor = next.fetch_add(1, Ordering::Relaxed);
                let Some(&index) = pending.get(cursor) else {
                    break;
                };
                let file = &files[index];
                let (scan, covered_from_ms) = scan_one_file(&file.path, since_ms);
                parsed.lock().unwrap().push((index, scan, covered_from_ms));
            });
        }
    });

    let mut cache = scan_cache().lock().unwrap();
    for (index, scan, covered_from_ms) in parsed.into_inner().unwrap() {
        let file = &files[index];
        // 读失败（`None`）⇒ 不写缓存，下次重试；结果仍返回（退化为空扫描）。
        if covered_from_ms.is_some() {
            cache.insert(
                file.path.clone(),
                ScanCacheEntry {
                    mtime_ms: file.mtime_ms,
                    size: file.size,
                    covered_from_ms,
                    scan: scan.clone(),
                },
            );
        }
        slots[index] = Some(scan);
    }
    drop(cache);
    slots.into_iter().map(Option::unwrap_or_default).collect()
}

// ---------------------------------------------------------------------------
// 单文件：尾部有界读取 + 解析
// ---------------------------------------------------------------------------

/// 读一份 jsonl 的**窗口内尾部**；读不到（打不开 / I/O 错）返回 `None`（⇒ 不进缓存）。
///
/// ## 为什么可以只读尾部
///
/// 会话 jsonl 是**只追加**的（客户端一次写入即落盘），记录按时间顺序追加 ⇒
/// 窗口内的记录必然在文件**尾部**。块大小、停止判据与边界用例都在
/// [`jsonl_tail`] —— 与「今日 Token」的扫描**共用同一实现**。
///
/// ⚠️ 这是**纯粹的 I/O 优化**：逻辑覆盖范围恒等于窗口 `since_ms`（见 [`ScanCacheEntry`]），
/// 所以「读得少」不会让缓存声称自己覆盖了没读过的区间。
fn read_window_text(path: &Path, since_ms: i64) -> Option<String> {
    let first = read_window_text_once(path, since_ms)?;
    // 尾部残行（客户端正在追加写）：该行必然解析失败，而**最新那条恰恰是决定
    // 「仍受限」的那条**。短等后重读一次，取更长的一份（只追加 ⇒ 更长 = 更全）。
    let torn = !first.is_empty() && !first.ends_with('\n') && !tail_is_complete_json(&first);
    if !torn {
        return Some(first);
    }
    std::thread::sleep(TORN_TAIL_RETRY_DELAY);
    match read_window_text_once(path, since_ms) {
        Some(second) if second.len() > first.len() => Some(second),
        _ => Some(first),
    }
}

fn read_window_text_once(path: &Path, since_ms: i64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size == 0 {
        return Some(String::new());
    }
    // 全量：没有窗口可用来提前停，只能整份读。
    if since_ms <= 0 {
        let mut text = String::new();
        return file.read_to_string(&mut text).ok().map(|_| text);
    }

    // 尾部窗口读取（块大小 / 停止判据 / 边界用例见 `jsonl_tail`）——
    // 与「今日 Token」的扫描**共用同一实现**，两边不再各写一份。
    let (mut buffer, start) = jsonl_tail::read_window_from(&mut file, size, since_ms)?;
    // `start > 0` 时首行可能被切断 ⇒ 丢掉它（半行本来也解析不出来）。
    if start > 0 {
        match buffer.iter().position(|byte| *byte == b'\n') {
            Some(cut) => {
                buffer.drain(..=cut);
            }
            None => buffer.clear(),
        }
    }
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// 尾部（最后一个换行符之后）那段是否已是**合法 JSON**。
///
/// 合法 ⇒ 文件只是「末行没写换行」，不是半行，无需重读；不合法 ⇒ 疑似半行。
fn tail_is_complete_json(text: &str) -> bool {
    let tail = text.rsplit('\n').next().unwrap_or_default().trim();
    tail.is_empty() || serde_json::from_str::<Value>(tail).is_ok()
}

/// 解析单个文件（读 + 解析），返回 `(扫描结果, 缓存覆盖标记)`。
///
/// 覆盖标记恒为 `Some(since_ms)`：只要这次解析用了窗口 `since_ms`，结果对
/// `ts >= since_ms` 就是完整的（尾部读取只是少读了窗口外的前缀）。
/// 读失败 ⇒ `None` ⇒ **不写缓存**，下次重试。
fn scan_one_file(path: &Path, since_ms: i64) -> (FileScan, Option<i64>) {
    let Some(text) = read_window_text(path, since_ms) else {
        return (FileScan::default(), None);
    };
    let fallback_session = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();
    (
        parse_records(&text, since_ms, &fallback_session),
        Some(since_ms),
    )
}

/// 一行是否**可能**影响限流台账（廉价子串预筛，**宁可多留，不可漏**）。
///
/// 逐条对应 [`parse_records`] 关心的三类行：
///
/// | 关心什么 | 判据 | 该行必然含 |
/// |---|---|---|
/// | 用户「又试过」 | [`is_attempt_record`]：`type=="message" && role=="user"` | `"user"` |
/// | 错误记录 / 又试过 | `providerData.error` 是对象 | `"providerData"` |
/// | 成功产出 | [`is_success_record`]：`type=="reasoning"` | `"reasoning"` |
/// | 成功产出 | [`is_success_record`]：`type=="function_call"` | `"function_call"` |
/// | 成功产出 | [`is_success_record`]：`message`+`assistant`+`completed` | `"completed"` |
///
/// ⚠️ 用**裸子串**而不是精确键值：客户端与夹具的序列化风格可能不同
/// （`"role": "user"` / `"role":"user"` 都要命中），与既有的 `"timestamp"` 预筛同一取向。
/// 多命中只是白解析一行；**少命中才会丢数据** —— 所以宁可多留。
fn line_may_matter(line: &str) -> bool {
    line.contains("\"providerData\"")
        || line.contains("\"user\"")
        || line.contains("\"reasoning\"")
        || line.contains("\"function_call\"")
        || line.contains("\"completed\"")
}

/// 从一份 jsonl 文本里抽出：窗口内的错误记录 + 窗口内最早的成功时刻 + 会话 ID。
fn parse_records(text: &str, since_ms: i64, fallback_session: &str) -> FileScan {
    let mut scan = FileScan {
        session_id: fallback_session.to_string(),
        ..FileScan::default()
    };
    let mut session_seen = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || !line.contains("\"timestamp\"") {
            continue;
        }
        // ★ 第二道廉价预筛（2026-10-13）：窗口里绝大多数行是普通消息 / 内务记录，
        // 对台账毫无贡献。先按子串跳过它们，省掉 JSON 解析（冷启扫描的主要 CPU 成本）。
        if !line_may_matter(line) {
            continue;
        }
        // 半行 / 坏行：跳过。该行本来就没有可用的业务信息，不构成「静默丢弃」。
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let timestamp = value.get("timestamp").and_then(Value::as_i64).unwrap_or(0);
        if !session_seen {
            if let Some(session) = value.get("sessionId").and_then(Value::as_str) {
                if !session.is_empty() {
                    scan.session_id = session.to_string();
                    session_seen = true;
                }
            }
        }
        if !in_window(timestamp, since_ms) {
            continue;
        }
        if is_attempt_record(&value) && timestamp > scan.last_attempt_ms {
            scan.last_attempt_ms = timestamp;
        }
        if let Some(error) = value
            .get("providerData")
            .and_then(|provider| provider.get("error"))
            .filter(|error| error.is_object())
        {
            if let Some(record) = parse_error_record(&value, error, timestamp) {
                scan.errors.push(record);
            }
            continue;
        }
        if is_success_record(&value) && timestamp > scan.last_success_ms {
            scan.last_success_ms = timestamp;
        }
    }
    scan
}

/// 从一行里构造 [`RawClientError`]；`code` 缺失或为 `null` 返回 `None`。
///
/// ⚠️ `code` 既可能是数值（`6004`）也可能是字符串（实测 `"ECONNRESET"`）。只认数值会把
/// 字符串码的记录整条丢掉（违反 R6「不静默丢弃」），故两者都收。
fn parse_error_record(value: &Value, error: &Value, timestamp: i64) -> Option<RawClientError> {
    let code_value = error.get("code")?;
    let code = code_value.as_i64();
    let code_text = match code_value {
        Value::String(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        _ => None,
    };
    if code.is_none() && code_text.is_none() {
        return None;
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(RawClientError {
        session_id: value
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        timestamp,
        code,
        code_text,
        status: error.get("status").and_then(Value::as_i64),
        reset_at: parse_reset_at_ms(&message),
        message,
        agent: value
            .get("providerData")
            .and_then(|provider| provider.get("agent"))
            .and_then(Value::as_str)
            .map(str::to_string),
        cwd: value
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.is_empty())
            .map(str::to_string),
        file_success_ms: 0,
        file_attempt_ms: 0,
    })
}

// ---------------------------------------------------------------------------
// ② sessions 表（只读）
// ---------------------------------------------------------------------------

/// 读 `sessions` 表取 `sessionId → (model, user_id)` 映射；**只读**打开（PRD R4）。
///
/// 列缺失时（老客户端 schema）用 `NULL` 占位而非整表失败 —— join 退化为「未知」，
/// 但**不丢记录**（R1）。表不存在 / 库不可读返回空表。
fn load_session_meta_for(region: Region) -> HashMap<String, (Option<String>, Option<String>)> {
    let mut map = HashMap::new();
    let db = workbuddy_db_path_for(region);
    if !db.is_file() {
        return map;
    }
    let Ok(conn) = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return map;
    };
    let _ = conn.busy_timeout(Duration::from_secs(5));
    let columns = table_columns(&conn, "sessions");
    if !columns.iter().any(|column| column == "id") {
        return map;
    }
    let model_expr = if columns.iter().any(|column| column == "model") {
        "model"
    } else {
        "NULL"
    };
    let uid_expr = if columns.iter().any(|column| column == "user_id") {
        "user_id"
    } else {
        "NULL"
    };
    let sql = format!("SELECT id, {model_expr}, {uid_expr} FROM sessions");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return map;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    }) else {
        return map;
    };
    for (id, model, uid) in rows.flatten() {
        let Some(id) = nonempty(id) else {
            continue;
        };
        map.insert(id, (nonempty(model), nonempty(uid)));
    }
    map
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return Vec::new();
    };
    let Ok(iter) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return Vec::new();
    };
    iter.flatten().collect()
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

// ---------------------------------------------------------------------------
// ③ 账号库 uid → nickname
// ---------------------------------------------------------------------------

/// 读账号库构造 `uid → nickname` 映射（PRD R2：uid 走 `auth_raw.account.uid` / `profile_raw.uid`）。
fn load_uid_nicknames_for(region: Region) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for acc in account::load_accounts_for(region) {
        let Some(uid) = account_uid(&acc) else {
            continue;
        };
        map.entry(uid)
            .or_insert_with(|| account::account_display_name(&acc));
    }
    map
}

/// 从账号记录提取 uid：`auth_raw.account.uid`（CN）→ `profile_raw.uid`（Global）→ 顶层 `uid`。
///
/// ⚠️ **绝不**用本地 `id`（PRD R2）。Global 的 `auth_raw` 顶层键与 CN 不同（实测只有
/// `accessToken` 等，**没有** `account`），故需要第二条路径 `profile_raw.uid`；顶层 `uid`
/// 是两者都有的兜底（实测与上述两处逐字相等）。
fn account_uid(acc: &Value) -> Option<String> {
    let pick = |value: &Value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(String::from)
    };
    acc.get("auth_raw")
        .and_then(|auth| auth.get("account"))
        .and_then(|account| account.get("uid"))
        .and_then(pick)
        .or_else(|| {
            acc.get("profile_raw")
                .and_then(|profile| profile.get("uid"))
                .and_then(pick)
        })
        .or_else(|| acc.get("uid").and_then(pick))
}

// ---------------------------------------------------------------------------
// ④ 聚合与状态判定
// ---------------------------------------------------------------------------

/// 错误码的**去重标识**：数值码优先，字符串码兜底（两者都无 ⇒ `None`）。
fn code_identity(error: &RawClientError) -> Option<String> {
    error
        .code
        .map(|code| code.to_string())
        .or_else(|| error.code_text.clone())
}

/// 聚合键（见 [`LimitKey`] 的说明）。
fn limit_key(uid: &Option<String>, model: &Option<String>, error: &RawClientError) -> LimitKey {
    (
        uid.clone(),
        model.clone(),
        code_identity(error),
        error.reset_at,
    )
}

/// 「实测可用」的证据来源（毫秒，`0` = 无实证）。
///
/// 三路取最早：
/// 1. **同文件**的后续成功 —— 最直接（同一会话之后确实跑通了）；
/// 2. **同会话**的后续成功 —— 同上，覆盖文件被切分的情况；
/// 3. **同账号同模型**的后续成功 —— 覆盖「这个会话被弃用、用户在新会话里继续」的情况。
///
/// ⚠️ 第 3 路**只在 uid 存在时**成立：join 失败时 `(None, None)` 会把所有未知会话并成
/// 一个桶，那等于「任何一个未知会话成功 ⇒ 全部未知条目都算恢复」。
fn evidence_ms(
    error: &RawClientError,
    uid: &Option<String>,
    model: &Option<String>,
    success_by_session: &HashMap<String, i64>,
    success_by_account_model: &HashMap<(Option<String>, Option<String>), i64>,
) -> i64 {
    let mut best = error.file_success_ms;
    if let Some(value) = success_by_session.get(&error.session_id) {
        best = best.max(*value);
    }
    if uid.is_some() {
        if let Some(value) = success_by_account_model.get(&(uid.clone(), model.clone())) {
            best = best.max(*value);
        }
    }
    best
}

/// 状态判定（**每一条都要有实证，没有实证就不下结论**）。
///
/// | 条件 | 状态 | 依据 |
/// |---|---|---|
/// | 该事件之后**确实有成功产出**（`success > occurred_at`） | [`LimitState::Recovered`] | 实测 |
/// | 已过文案里的 `reset_at` | [`LimitState::Recovered`] | 上游口径 |
/// | 该事件之后**又试过**（`attempt > occurred_at`）却仍无成功产出 | [`LimitState::Active`] | 实测 |
/// | 其余（含「错误就是该会话最后一条」） | [`LimitState::Unknown`] | 无从判断 |
///
/// ## 为什么必须这么严
///
/// 上游文案的 `reset at <T>` 是**配额窗口**重置时刻，**不是解锁时刻**：
/// 实测 17:20:11 报 429 并声称「10-13 09:04 才重置」，而 17:36:50 该会话就恢复了产出。
/// 只看 `reset_at > now` 会把正在正常使用的账号报成「仍受限」（实测 6/6 全错）。
///
/// 反过来，「错误之后没有任何记录」同样**推不出**「仍受限」：实测 `d49456c2` 与
/// `8fe7b8b0` 里那条 429 就是文件的最后一条记录 —— 那只是**这个会话停了**，
/// 不是「账号还被限着」。⇒ 判 `Active` 必须有「之后又试过」的实证。
fn limit_state(
    reset_at: Option<i64>,
    success: i64,
    attempt: i64,
    occurred_at: i64,
    now: i64,
) -> LimitState {
    // ① 反证优先：该错误之后确实有成功产出 ⇒ 确定已恢复（与文案里的时刻无关）。
    if success > occurred_at {
        return LimitState::Recovered;
    }
    // ② 已过文案里的 `reset_at` ⇒ 按上游口径算已恢复。
    if reset_at.is_some_and(|reset| reset <= now) {
        return LimitState::Recovered;
    }
    // ③ 只有「之后**又试过**、却仍无成功产出」才敢说「仍受限」。
    //    ⚠️ 必须**严格晚于**：错误就是该会话最后一条时 `attempt == occurred_at`，
    //    那只是「这个会话停了」，不是「账号还被限着」（实测 `d49456c2` / `8fe7b8b0`）。
    if attempt > occurred_at {
        return LimitState::Active;
    }
    // ④ 其余一律「未知」—— 没有实证就不下结论。
    LimitState::Unknown
}

/// 按 [`limit_key`] 聚合去重，桶内取**最近一条**；`occurrences` 记录合并了多少条原始记录。
///
/// **纯函数**（显式入参，不碰磁盘）—— 便于单测覆盖 join / 去重 / 兜底三条链。
///
/// 「最近一条」的判据是 `(timestamp, session_id)` 的**字典序**，不是 `timestamp` 单键：
/// 时间戳相同的两条若按「先到先得」，结果就取决于 `read_dir` 顺序 ⇒ 同一次数据两次刷新
/// 可能给出不同结果。加 `session_id` 做第二键后，输出与输入顺序**无关**。
fn build_limits(
    region: Region,
    errors: Vec<RawClientError>,
    meta: &HashMap<String, (Option<String>, Option<String>)>,
    names: &HashMap<String, String>,
    now: i64,
) -> Vec<ClientLimitEntry> {
    // 先按会话 / 账号×模型汇总「成功活动的最早时刻」，供 `evidence_ms` 取反证。
    let mut success_by_session: HashMap<String, i64> = HashMap::new();
    let mut success_by_account_model: HashMap<(Option<String>, Option<String>), i64> =
        HashMap::new();
    for error in &errors {
        if error.file_success_ms <= 0 {
            continue;
        }
        let (model, uid) = meta.get(&error.session_id).cloned().unwrap_or((None, None));
        record_success(
            &mut success_by_session,
            error.session_id.clone(),
            error.file_success_ms,
        );
        if uid.is_some() {
            record_success(
                &mut success_by_account_model,
                (uid, model),
                error.file_success_ms,
            );
        }
    }

    let mut map: HashMap<LimitKey, ClientLimitEntry> = HashMap::new();
    for error in errors {
        let (model, uid) = meta.get(&error.session_id).cloned().unwrap_or((None, None));
        let key = limit_key(&uid, &model, &error);
        let evidence = evidence_ms(
            &error,
            &uid,
            &model,
            &success_by_session,
            &success_by_account_model,
        );
        match map.get_mut(&key) {
            None => {
                let entry = ClientLimitEntry {
                    account: uid.as_ref().and_then(|value| names.get(value).cloned()),
                    account_uid: uid,
                    model,
                    code: error.code,
                    code_text: error.code_text,
                    status: error.status,
                    reset_at: error.reset_at,
                    occurred_at: error.timestamp,
                    evidence_at: (evidence > 0).then_some(evidence),
                    attempt_ms: error.file_attempt_ms,
                    // 占位：桶收口后由末尾的统一回填覆盖。
                    state: LimitState::Unknown,
                    occurrences: 1,
                    message: error.message,
                    session_id: error.session_id,
                    cwd: error.cwd,
                    agent: error.agent,
                    region: region.as_str().to_string(),
                };
                map.insert(key, entry);
            }
            Some(entry) => {
                entry.occurrences += 1;
                entry.attempt_ms = entry.attempt_ms.max(error.file_attempt_ms);
                if evidence > 0 {
                    entry.evidence_at = Some(match entry.evidence_at {
                        Some(current) => current.min(evidence),
                        None => evidence,
                    });
                }
                if is_newer_than(&error, entry) {
                    entry.occurred_at = error.timestamp;
                    entry.status = error.status;
                    entry.code_text = error.code_text;
                    entry.message = error.message;
                    entry.session_id = error.session_id;
                    entry.cwd = error.cwd;
                    entry.agent = error.agent;
                }
            }
        }
    }

    // 状态必须在桶**收口之后**才能定：它同时依赖桶内的 `occurred_at` 与 `evidence_at`。
    let mut limits: Vec<ClientLimitEntry> = map
        .into_values()
        .map(|mut entry| {
            let evidence = entry.evidence_at.unwrap_or(0);
            entry.state = limit_state(
                entry.reset_at,
                evidence,
                entry.attempt_ms,
                entry.occurred_at,
                now,
            );
            // 「实测可用」只在它确实**晚于**最近一次错误时才构成恢复实证；
            // 否则（成功发生在错误之前）不展示，避免误导。
            entry.evidence_at = (evidence > entry.occurred_at).then_some(evidence);
            entry
        })
        .collect();
    // **完全排序**（含并列 tie-break）⇒ 输出与 `HashMap` 遍历顺序无关，刷新结果稳定。
    limits.sort_by(|a, b| {
        a.state
            .rank()
            .cmp(&b.state.rank())
            .then(b.occurred_at.cmp(&a.occurred_at))
            .then(b.reset_at.unwrap_or(0).cmp(&a.reset_at.unwrap_or(0)))
            .then(a.account_uid.cmp(&b.account_uid))
            .then(a.model.cmp(&b.model))
            .then(a.code.cmp(&b.code))
            .then(a.code_text.cmp(&b.code_text))
            .then(a.session_id.cmp(&b.session_id))
    });
    limits
}

/// 汇总「模型确实工作过」的时刻：**取最晚**（见 [`FileScan::last_success_ms`] 的说明）。
fn record_success<K: std::hash::Hash + Eq>(map: &mut HashMap<K, i64>, key: K, value: i64) {
    map.entry(key)
        .and_modify(|current| *current = (*current).max(value))
        .or_insert(value);
}

/// `error` 是否比当前桶内已保留的那条更新（字典序 `(timestamp, session_id)`）。
fn is_newer_than(error: &RawClientError, entry: &ClientLimitEntry) -> bool {
    (error.timestamp, error.session_id.as_str()) > (entry.occurred_at, entry.session_id.as_str())
}

// ---------------------------------------------------------------------------
// 时间解析
// ---------------------------------------------------------------------------

/// 从上游错误文案解析「恢复时刻」（毫秒）。
///
/// ⚠️ **锚时间戳形态，不锚周边文案**（PRD 附录 C3）：中英文模板都含
/// `<YYYY-MM-DD HH:MM:SS> UTC+8`；文案会漂移，形态稳定。
///
/// ⚠️⚠️ 解出来的值**只是配额窗口的重置时刻**，不是解锁时刻 —— 见模块头。
///
/// 解析失败返回 `None`（R6：调用方保留原文，**不静默丢弃**）。
fn parse_reset_at_ms(message: &str) -> Option<i64> {
    for (index, _) in message.match_indices("UTC+8") {
        let prefix = message.get(..index)?.trim_end();
        let start = prefix.len().checked_sub(19)?;
        // `start` 可能落在多字节字符中间 ⇒ `get` 返回 `None`，跳过该命中点。
        let Some(candidate) = prefix.get(start..) else {
            continue;
        };
        if let Some(ms) = parse_cst_datetime(candidate) {
            return Some(ms);
        }
    }
    None
}

/// `YYYY-MM-DD[ T]HH:MM:SS` → 按**固定 UTC+8** 解释的毫秒时间戳。
fn parse_cst_datetime(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 19 {
        return None;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    if bytes[10] != b' ' && bytes[10] != b'T' {
        return None;
    }
    let num =
        |range: std::ops::Range<usize>| -> Option<i32> { text.get(range)?.parse::<i32>().ok() };
    let year = num(0..4)?;
    let month = num(5..7)? as u32;
    let day = num(8..10)? as u32;
    let hour = num(11..13)? as u32;
    let minute = num(14..16)? as u32;
    let second = num(17..19)? as u32;
    let naive =
        chrono::NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second)?;
    use chrono::TimeZone;
    cst::cst_offset()
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn cst_millis(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        cst::cst_ms(year, month, day, hour, minute)
    }

    /// 2026-10-12 18:05（CST）—— 复现问题截图那一刻。
    fn snapshot_now() -> i64 {
        cst_millis(2026, 10, 12, 18, 5)
    }

    fn raw(
        session_id: &str,
        timestamp: i64,
        code: i64,
        message: &str,
        reset_at: Option<i64>,
    ) -> RawClientError {
        RawClientError {
            session_id: session_id.to_string(),
            timestamp,
            code: Some(code),
            code_text: None,
            status: Some(429),
            message: message.to_string(),
            agent: Some("cli".to_string()),
            cwd: Some("c:\\proj".to_string()),
            reset_at,
            file_success_ms: 0,
            file_attempt_ms: 0,
        }
    }

    // -----------------------------------------------------------------------
    // 时间解析（原有断言，未改）
    // -----------------------------------------------------------------------

    #[test]
    fn parse_reset_at_uses_fixed_utc8_for_chinese_template() {
        let message = "429 您的使用量已超出频率限制，将在 2026-09-16 16:34:24 UTC+8 重置，您也可以切换其他模型继续使用。 (req/abc)";
        // 2026-09-16 16:34:24 UTC+8 == 2026-09-16 08:34:24 UTC。
        // 断言**固定 UTC 瞬时**（而非本机本地时刻）：若把基准从 UTC+8 改成系统本地时区，
        // 在非 UTC+8 机器上这里即变红（PRD 验收 5 的可证伪点）。
        let expected = Utc
            .with_ymd_and_hms(2026, 9, 16, 8, 34, 24)
            .single()
            .expect("合法 UTC")
            .timestamp_millis();
        assert_eq!(parse_reset_at_ms(message), Some(expected));
    }

    #[test]
    fn parse_reset_at_handles_english_template_and_t_separator() {
        let expected = Utc
            .with_ymd_and_hms(2026, 9, 16, 8, 34, 24)
            .single()
            .expect("合法 UTC")
            .timestamp_millis();
        let english = "429 usage exceeds frequency limit, your usage will reset at 2026-09-16 16:34:24 UTC+8, alternatively...";
        assert_eq!(parse_reset_at_ms(english), Some(expected));
        let t_form = "reset at 2026-09-16T16:34:24 UTC+8";
        assert_eq!(parse_reset_at_ms(t_form), Some(expected));
    }

    #[test]
    fn parse_reset_at_returns_none_when_text_drifts() {
        assert_eq!(parse_reset_at_ms("429 queue.waiting.title"), None);
        assert_eq!(parse_reset_at_ms("reset at 2026-09-16 16:34:24"), None);
        assert_eq!(parse_reset_at_ms("将在稍后重置"), None);
        assert_eq!(
            parse_reset_at_ms("reset at 2026-13-40 99:99:99 UTC+8"),
            None
        );
    }

    #[test]
    fn parse_reset_at_survives_multibyte_prefix() {
        let message = "重置将在 2026-09-16 16:34:24 UTC+8 生效";
        let expected = Utc
            .with_ymd_and_hms(2026, 9, 16, 8, 34, 24)
            .single()
            .expect("合法 UTC")
            .timestamp_millis();
        assert_eq!(parse_reset_at_ms(message), Some(expected));
    }

    #[test]
    fn account_uid_prefers_auth_then_profile_and_never_uses_local_id() {
        let cn = json!({
            "id": "local-id-cn",
            "uid": "uid-top",
            "auth_raw": { "account": { "uid": "uid-auth" } },
            "profile_raw": { "uid": "uid-profile" },
        });
        assert_eq!(account_uid(&cn).as_deref(), Some("uid-auth"));

        let global = json!({
            "id": "local-id-global",
            "uid": "uid-top",
            "auth_raw": { "accessToken": "x" },
            "profile_raw": { "uid": "uid-profile" },
        });
        assert_eq!(account_uid(&global).as_deref(), Some("uid-profile"));

        let bare = json!({ "id": "local-id-only", "uid": "uid-top" });
        assert_eq!(account_uid(&bare).as_deref(), Some("uid-top"));

        let no_uid = json!({ "id": "local-id-only" });
        assert_eq!(account_uid(&no_uid), None);
    }

    #[test]
    fn resolve_since_ms_default_all_and_days() {
        let now = 1_000_000_000_000;
        assert_eq!(
            resolve_since_ms(None, now),
            now - DEFAULT_SINCE_DAYS * DAY_MS
        );
        assert_eq!(resolve_since_ms(Some(0), now), 0);
        assert_eq!(resolve_since_ms(Some(30), now), now - 30 * DAY_MS);
        assert_eq!(
            resolve_since_ms(Some(-5), now),
            now - DEFAULT_SINCE_DAYS * DAY_MS
        );
    }

    // -----------------------------------------------------------------------
    // ★ 恢复判定：文案里的恢复时刻 ≠ 解锁时刻
    // -----------------------------------------------------------------------

    #[test]
    fn is_success_record_accepts_model_activity_and_rejects_tool_results() {
        assert!(is_success_record(&json!({ "type": "reasoning" })));
        assert!(is_success_record(&json!({ "type": "function_call" })));
        assert!(is_success_record(
            &json!({ "type": "message", "role": "assistant", "status": "completed" })
        ));
        // 工具执行结果**不算**：模型未必参与了这一次往返。
        assert!(!is_success_record(
            &json!({ "type": "function_call_result", "status": "completed" })
        ));
        assert!(!is_success_record(
            &json!({ "type": "message", "role": "assistant", "status": "incomplete" })
        ));
        assert!(!is_success_record(
            &json!({ "type": "message", "role": "user" })
        ));
        assert!(!is_success_record(
            &json!({ "type": "file-history-snapshot" })
        ));
        assert!(!is_success_record(&json!({ "type": "session-meta" })));
    }

    #[test]
    fn limit_state_prefers_recovery_evidence_over_the_quoted_reset_time() {
        let now = snapshot_now();
        let occurred = now - 3_600_000;
        // 复现实测：报 429 时文案说「明天 09:04 重置」，但 16 分钟后模型已恢复产出。
        let quoted_reset = now + 15 * 3_600_000;
        let evidence = occurred + 16 * 60_000;
        let retried = occurred + 300_000;
        assert_eq!(
            limit_state(Some(quoted_reset), evidence, retried, occurred, now),
            LimitState::Recovered,
            "有恢复实证时不得报「仍受限」—— 文案里的时刻是配额窗口，不是解锁时刻"
        );
        // 有恢复实证时**优先**于「reset_at 缺失」：排队类错误（6020/6022）本就没有恢复时刻，
        // 但之后模型确实跑通了 ⇒ 是**确定**的已恢复，不该退化成「未知」。
        assert_eq!(
            limit_state(None, evidence, retried, occurred, now),
            LimitState::Recovered
        );
        // 没有成功、但之后**又试过** ⇒ 这才敢说「仍受限」。
        assert_eq!(
            limit_state(Some(quoted_reset), 0, retried, occurred, now),
            LimitState::Active
        );
        // ⚠️ 没有成功、也**没再试过**（错误就是该会话最后一条）⇒ 无从判断，不是「仍受限」。
        assert_eq!(
            limit_state(Some(quoted_reset), 0, 0, occurred, now),
            LimitState::Unknown
        );
        assert_eq!(
            limit_state(Some(quoted_reset), 0, occurred, occurred, now),
            LimitState::Unknown,
            "「又试过」的时刻必须严格晚于该错误"
        );
        // 成功实证发生在错误**之前** ⇒ 不构成恢复证据。
        assert_eq!(
            limit_state(Some(quoted_reset), occurred - 1, retried, occurred, now),
            LimitState::Active
        );
        // 已过恢复时刻 ⇒ 已恢复。
        assert_eq!(
            limit_state(Some(now - 1), 0, 0, occurred, now),
            LimitState::Recovered
        );
        assert_eq!(limit_state(None, 0, 0, occurred, now), LimitState::Unknown);
    }

    #[test]
    fn is_attempt_record_ignores_client_housekeeping() {
        assert!(is_attempt_record(
            &json!({ "type": "message", "role": "user" })
        ));
        assert!(is_attempt_record(
            &json!({ "providerData": { "error": { "code": 6004 } } })
        ));
        // 客户端内务记录在会话闲置时也会周期性落盘 ⇒ 不能当「又试过」。
        assert!(!is_attempt_record(
            &json!({ "type": "file-history-snapshot" })
        ));
        assert!(!is_attempt_record(&json!({ "type": "session-meta" })));
        assert!(!is_attempt_record(
            &json!({ "type": "message", "role": "assistant", "status": "completed" })
        ));
    }

    #[test]
    fn still_limited_requires_evidence_of_a_later_attempt() {
        // ⚠️ 本轮的第二条关键修正：**「之后没有任何记录」推不出「仍受限」**。
        // 实测 `d49456c2` / `8fe7b8b0` 里那条 429 就是文件的最后一条记录 ——
        // 那只是「这个会话停了」，不是「账号还被限着」。
        let now = snapshot_now();
        let occurred = now - 3_600_000;
        let reset = now + 3_600_000;
        let mut meta = HashMap::new();
        meta.insert(
            "s1".to_string(),
            (Some("m".to_string()), Some("uid-a".to_string())),
        );
        let names: HashMap<String, String> = HashMap::new();

        // ① 错误就是最后一条 ⇒ 未知。
        let ended = raw("s1", occurred, 6004, "429 x", Some(reset));
        let limits = build_limits(Region::Cn, vec![ended], &meta, &names, now);
        assert_eq!(limits[0].state, LimitState::Unknown);

        // ② 之后又试过、仍无成功产出 ⇒ 仍受限。
        let mut retried = raw("s1", occurred, 6004, "429 x", Some(reset));
        retried.file_attempt_ms = occurred + 300_000;
        let limits = build_limits(Region::Cn, vec![retried], &meta, &names, now);
        assert_eq!(limits[0].state, LimitState::Active);

        // ③ 之后又试过**并且成功了** ⇒ 已恢复（成功优先于尝试）。
        let mut both = raw("s1", occurred, 6004, "429 x", Some(reset));
        both.file_attempt_ms = occurred + 300_000;
        both.file_success_ms = occurred + 600_000;
        let limits = build_limits(Region::Cn, vec![both], &meta, &names, now);
        assert_eq!(limits[0].state, LimitState::Recovered);
    }

    #[test]
    fn entries_report_evidence_at_only_when_it_postdates_the_error() {
        let now = snapshot_now();
        let occurred = now - 3_600_000;
        let reset = now + 3_600_000;
        let mut meta = HashMap::new();
        meta.insert(
            "s1".to_string(),
            (Some("m".to_string()), Some("uid-a".to_string())),
        );
        let names: HashMap<String, String> = HashMap::new();

        let mut with_evidence = raw("s1", occurred, 6004, "429 x", Some(reset));
        with_evidence.file_success_ms = occurred + 600_000;
        let limits = build_limits(Region::Cn, vec![with_evidence], &meta, &names, now);
        assert_eq!(limits[0].state, LimitState::Recovered);
        assert_eq!(limits[0].evidence_at, Some(occurred + 600_000));
        assert_eq!(limits[0].reset_at, Some(reset), "配额时刻仍如实带出");

        // 成功发生在错误之前 ⇒ 不报「实测可用」；之后又试过 ⇒ 仍受限。
        let mut stale = raw("s1", occurred, 6004, "429 x", Some(reset));
        stale.file_success_ms = occurred - 600_000;
        stale.file_attempt_ms = occurred + 300_000;
        let limits = build_limits(Region::Cn, vec![stale], &meta, &names, now);
        assert_eq!(limits[0].state, LimitState::Active);
        assert_eq!(limits[0].evidence_at, None);
    }

    #[test]
    fn later_success_in_another_session_of_the_same_account_counts_as_recovery() {
        let now = snapshot_now();
        let occurred = now - 3_600_000;
        let reset = now + 3_600_000;
        let mut meta = HashMap::new();
        for session in ["s-limited", "s-new"] {
            meta.insert(
                session.to_string(),
                (Some("m".to_string()), Some("uid-a".to_string())),
            );
        }
        let names: HashMap<String, String> = HashMap::new();
        // 限流发生在 s-limited；用户随即在**另一个会话** s-new 里继续干活并成功。
        let limited = raw("s-limited", occurred, 6004, "429 x", Some(reset));
        let mut continued = raw("s-new", occurred + 60_000, 6004, "429 x", Some(reset));
        continued.file_success_ms = occurred + 120_000;
        let limits = build_limits(Region::Cn, vec![limited, continued], &meta, &names, now);
        assert_eq!(limits.len(), 1, "同 (uid, model, code, reset) 聚合为一条");
        assert_eq!(limits[0].occurrences, 2);
        assert_eq!(
            limits[0].state,
            LimitState::Recovered,
            "同账号同模型的后续成功也是恢复实证"
        );
        assert_eq!(limits[0].evidence_at, Some(occurred + 120_000));
    }

    #[test]
    fn join_failure_evidence_does_not_leak_across_unknown_sessions() {
        // ⚠️ join 失败时 uid/model 都是 None：若把「账号×模型」证据也套上去，
        // 任何一个未知会话成功都会让**全部**未知条目变成「已恢复」。
        let now = snapshot_now();
        let occurred = now - 3_600_000;
        let reset = now + 3_600_000;
        let meta: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        let names: HashMap<String, String> = HashMap::new();
        let mut succeeded = raw("ghost-ok", occurred, 6004, "429 x", Some(reset));
        succeeded.file_success_ms = occurred + 60_000;
        let still_stuck = raw("ghost-stuck", occurred, 6004, "429 y", Some(reset));
        let limits = build_limits(Region::Cn, vec![succeeded, still_stuck], &meta, &names, now);
        assert_eq!(limits.len(), 1, "join 失败时两者同桶（uid/model 均 None）");
        // 桶内任一条有实证 ⇒ 该桶不再报「仍受限」（保守地少报一次「受限」）。
        assert_eq!(limits[0].state, LimitState::Recovered);
        // 阳性对照：都没有实证、但之后又试过 ⇒ 仍是 Active。
        let mut plain = raw("ghost-1", occurred, 6004, "429 x", Some(reset));
        plain.file_attempt_ms = occurred + 60_000;
        let limits = build_limits(Region::Cn, vec![plain], &meta, &names, now);
        assert_eq!(limits[0].state, LimitState::Active);
    }

    // -----------------------------------------------------------------------
    // 窗口 / 分类 / 去重 / 三态（早先一轮修正，保持）
    // -----------------------------------------------------------------------

    #[test]
    fn in_window_uses_record_time_and_full_scan_accepts_everything() {
        let since = 1_000_000;
        assert!(in_window(since, since), "边界（恰好等于下界）算窗口内");
        assert!(!in_window(since - 1, since));
        assert!(in_window(since + 1, since));
        assert!(in_window(0, 0));
        assert!(in_window(0, -1));
    }

    #[test]
    fn scan_window_filters_by_record_timestamp_not_only_file_mtime() {
        let dir = temp_projects_dir("window");
        let ws = dir.join("ws");
        let since = snapshot_now() - DEFAULT_SINCE_DAYS * DAY_MS;
        let body = [
            jsonl_line(
                "s-old-a",
                cst_millis(2026, 10, 4, 13, 42),
                json!(6004),
                json!(429),
                "429 您的使用量已超出频率限制，将在 2026-10-05 08:20:43 UTC+8 重置",
            ),
            jsonl_line(
                "s-old-b",
                cst_millis(2026, 10, 5, 13, 8),
                json!(3002),
                json!(502),
                "502 网络不可达：当前设备未连接到网络",
            ),
            jsonl_line(
                "s-new",
                cst_millis(2026, 10, 7, 11, 17),
                json!(6004),
                json!(429),
                "429 您的使用量已超出频率限制，将在 2026-10-08 09:00:00 UTC+8 重置",
            ),
        ]
        .join("\n");
        std::fs::write(ws.join("sess.jsonl"), format!("{body}\n")).expect("写 jsonl");

        let (errors, skipped) = collect_rate_limit_errors(scan_client_errors_in(&dir, since));
        assert_eq!(errors.len(), 1, "窗口外的记录必须被记录级时间过滤挡掉");
        assert_eq!(errors[0].session_id, "s-new");
        assert_eq!(skipped, 0, "窗口外那条网络错误也被窗口挡掉，不进 skipped");

        let (all, skipped_all) = collect_rate_limit_errors(scan_client_errors_in(&dir, 0));
        assert_eq!(all.len(), 2, "全量扫描：两条限流记录都在");
        assert_eq!(skipped_all, 1, "全量扫描：网络错误如实报数（不静默丢弃）");

        clear_scan_cache_entry(&ws.join("sess.jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_rate_limit_error_excludes_network_failures_and_keeps_429s() {
        let unreachable = raw_with(
            Some(3002),
            None,
            Some(502),
            "502 网络不可达：当前设备未连接到网络（connect EHOSTUNREACH 162.14.3.241:443）",
        );
        assert!(!is_rate_limit_error(&unreachable));
        let aborted = raw_with(None, Some("ECONNRESET"), None, "aborted");
        assert!(!is_rate_limit_error(&aborted));
        // 实测还有一类 400：客户端 reasoning 回传缺陷，同样不是限流。
        let reasoning_bug = raw_with(
            Some(11155),
            None,
            Some(400),
            "400 the reasoning content from the previous turn must be passed back in thinking mode",
        );
        assert!(!is_rate_limit_error(&reasoning_bug));

        assert!(is_rate_limit_error(&raw_with(
            Some(6004),
            None,
            Some(429),
            "429 usage exceeds frequency limit"
        )));
        assert!(is_rate_limit_error(&raw_with(
            Some(6020),
            None,
            Some(429),
            "429 queue.waiting.title"
        )));
        // 实测新增的两种 429：靠 status 收下（不在已知码表里）。
        assert!(is_rate_limit_error(&raw_with(
            Some(14003),
            None,
            Some(429),
            "429 too many requests"
        )));
        assert!(is_rate_limit_error(&raw_with(
            Some(14018),
            None,
            Some(429),
            "429 Credits exhausted. Please visit the link below..."
        )));
        assert!(
            is_rate_limit_error(&raw_with(Some(7000), None, Some(429), "429 换了个新码")),
            "status=429 单独成立 —— 上游换码不得漏"
        );
        assert!(
            is_rate_limit_error(&raw_with(None, None, None, "usage exceeds frequency limit")),
            "文案命中限流标记单独成立"
        );
    }

    #[test]
    fn parse_error_record_keeps_string_codes_instead_of_dropping_the_record() {
        let line = jsonl_line("s1", 1, json!("ECONNRESET"), Value::Null, "aborted");
        let value: Value = serde_json::from_str(&line).expect("合法 JSON");
        let error = value
            .get("providerData")
            .and_then(|provider| provider.get("error"))
            .expect("有 error 对象");
        let record = parse_error_record(&value, error, 1).expect("字符串码不得被静默丢弃（R6）");
        assert_eq!(record.code, None);
        assert_eq!(record.code_text.as_deref(), Some("ECONNRESET"));
        assert_eq!(record.status, None);

        // `code` 缺失 / 为 `null` 的行仍不取（它们不是上游错误事件）。
        let null_code = jsonl_line("s1", 1, Value::Null, json!(429), "x");
        let value: Value = serde_json::from_str(&null_code).expect("合法 JSON");
        let error = value
            .get("providerData")
            .and_then(|provider| provider.get("error"))
            .expect("有 error 对象");
        assert!(parse_error_record(&value, error, 1).is_none());
    }

    #[test]
    fn distinct_unknown_reset_events_are_not_collapsed_into_one_entry() {
        let now = snapshot_now();
        let mut meta = HashMap::new();
        meta.insert(
            "s1".to_string(),
            (
                Some("deepseek-v4.1-flash".to_string()),
                Some("uid-a".to_string()),
            ),
        );
        meta.insert(
            "s2".to_string(),
            (
                Some("deepseek-v4.1-flash".to_string()),
                Some("uid-a".to_string()),
            ),
        );
        let names = HashMap::new();
        let errors = vec![
            raw("s1", now - 3_000, 6020, "429 queue.waiting.title", None),
            raw("s2", now - 2_000, 6022, "429 queue.userLimit.title", None),
            raw("s1", now - 1_000, 6020, "429 queue.waiting.title", None),
        ];
        let limits = build_limits(Region::Cn, errors, &meta, &names, now);
        assert_eq!(limits.len(), 2, "不同错误码必须分桶");
        let queue_6020 = limits
            .iter()
            .find(|entry| entry.code == Some(6020))
            .expect("6020 桶存在");
        assert_eq!(queue_6020.occurrences, 2, "原始记录数要如实带出");
        assert_eq!(queue_6020.occurred_at, now - 1_000, "桶内取最近一条");
    }

    #[test]
    fn join_failures_with_different_codes_do_not_share_a_bucket() {
        let now = snapshot_now();
        let meta: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        let names: HashMap<String, String> = HashMap::new();
        let errors = vec![
            raw("ghost-1", now - 5_000, 6004, "429 ...", None),
            raw(
                "ghost-2",
                now - 4_000,
                6020,
                "429 queue.waiting.title",
                None,
            ),
            raw(
                "ghost-3",
                now - 3_000,
                6020,
                "429 queue.waiting.title",
                None,
            ),
            raw(
                "ghost-4",
                now - 2_000,
                6022,
                "429 queue.userLimit.title",
                None,
            ),
        ];
        let limits = build_limits(Region::Cn, errors, &meta, &names, now);
        assert_eq!(limits.len(), 3, "join 失败也要按错误码分开，不得塌缩成一条");
        assert_eq!(limits.iter().map(|entry| entry.occurrences).sum::<u32>(), 4);
        assert!(limits
            .iter()
            .all(|entry| entry.account.is_none() && entry.model.is_none()));
    }

    #[test]
    fn build_limits_dedupes_same_uid_model_code_reset_and_keeps_latest() {
        let now = cst_millis(2026, 9, 16, 10, 0);
        let reset = cst_millis(2026, 9, 16, 11, 30);
        let mut meta = HashMap::new();
        meta.insert(
            "s1".to_string(),
            (
                Some("deepseek-v4.1-flash".to_string()),
                Some("uid-a".to_string()),
            ),
        );
        meta.insert(
            "s2".to_string(),
            (
                Some("deepseek-v4.1-flash".to_string()),
                Some("uid-a".to_string()),
            ),
        );
        let mut names = HashMap::new();
        names.insert("uid-a".to_string(), "Jackey".to_string());

        // 三条都要带「之后又试过」，否则会落到 `unknown`（见 `limit_state` 的判据表）。
        let mut first = raw(
            "s1",
            cst_millis(2026, 9, 16, 9, 0),
            6004,
            "第一条",
            Some(reset),
        );
        let mut second = raw(
            "s2",
            cst_millis(2026, 9, 16, 9, 30),
            6004,
            "第二条(最新)",
            Some(reset),
        );
        let mut third = raw(
            "s1",
            cst_millis(2026, 9, 16, 9, 15),
            6004,
            "中间",
            Some(reset),
        );
        for entry in [&mut first, &mut second, &mut third] {
            entry.file_attempt_ms = cst_millis(2026, 9, 16, 9, 45);
        }
        let errors = vec![first, second, third];
        let limits = build_limits(Region::Cn, errors, &meta, &names, now);
        assert_eq!(
            limits.len(),
            1,
            "同 (uid, model, code, reset_at) 必须聚合为一条"
        );
        let entry = &limits[0];
        assert_eq!(entry.account.as_deref(), Some("Jackey"));
        assert_eq!(entry.model.as_deref(), Some("deepseek-v4.1-flash"));
        assert_eq!(entry.occurred_at, cst_millis(2026, 9, 16, 9, 30));
        assert_eq!(entry.message, "第二条(最新)");
        assert_eq!(entry.occurrences, 3);
        assert_eq!(
            entry.state,
            LimitState::Active,
            "无实证 + reset 在未来 ⇒ 仍受限"
        );
    }

    #[test]
    fn build_limits_keeps_join_failures_as_unknown() {
        let now = cst_millis(2026, 9, 16, 10, 0);
        let meta: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        let names: HashMap<String, String> = HashMap::new();
        let errors = vec![raw("ghost", now - 1000, 6004, "429 ...", None)];
        let limits = build_limits(Region::Cn, errors, &meta, &names, now);
        assert_eq!(limits.len(), 1, "join 失败不得丢弃（R1）");
        assert_eq!(limits[0].account, None);
        assert_eq!(limits[0].account_uid, None);
        assert_eq!(limits[0].model, None);
        assert_eq!(limits[0].session_id, "ghost");
        assert_eq!(
            limits[0].state,
            LimitState::Unknown,
            "reset_at 未知 ⇒ 未知，**不是**「已恢复」"
        );
    }

    #[test]
    fn build_limits_orders_active_then_unknown_then_recovered() {
        let now = snapshot_now();
        let mut meta = HashMap::new();
        for (session, uid) in [("a", "uid-a"), ("b", "uid-b"), ("c", "uid-c")] {
            meta.insert(
                session.to_string(),
                (Some("m".to_string()), Some(uid.to_string())),
            );
        }
        let names: HashMap<String, String> = HashMap::new();
        let mut live = raw("b", now - 60_000, 6004, "live", Some(now + 3_600_000));
        live.file_attempt_ms = now - 30_000;
        let errors = vec![
            raw("a", now - 60_000, 6004, "recovered", Some(now - 60_000)),
            raw("c", now - 3_600_000, 6020, "unknown", None),
            live,
        ];
        let limits = build_limits(Region::Cn, errors, &meta, &names, now);
        assert_eq!(limits.len(), 3);
        assert_eq!(limits[0].state, LimitState::Active, "仍受限排最前");
        assert_eq!(limits[0].account_uid.as_deref(), Some("uid-b"));
        assert_eq!(limits[1].state, LimitState::Unknown, "未知排在已恢复之前");
        assert_eq!(limits[2].state, LimitState::Recovered);
    }

    #[test]
    fn output_order_is_deterministic_regardless_of_input_order() {
        let now = snapshot_now();
        let mut meta = HashMap::new();
        for session in ["s1", "s2", "s3"] {
            meta.insert(
                session.to_string(),
                (
                    Some("deepseek-v4.1-flash".to_string()),
                    Some("uid-a".to_string()),
                ),
            );
        }
        let names: HashMap<String, String> = HashMap::new();
        let base = vec![
            raw("s1", now - 1_000, 6020, "A", None),
            raw("s2", now - 1_000, 6020, "B", None),
            raw("s3", now - 5_000, 6004, "C", Some(now + 600_000)),
            raw("s3", now - 9_000, 6004, "D", Some(now + 600_000)),
        ];
        let forward = build_limits(Region::Cn, base.clone(), &meta, &names, now);
        let mut reversed_input = base;
        reversed_input.reverse();
        let backward = build_limits(Region::Cn, reversed_input, &meta, &names, now);
        assert_eq!(forward, backward, "输出必须与输入顺序无关");
        let unknown = forward
            .iter()
            .find(|entry| entry.state == LimitState::Unknown)
            .expect("未知桶存在");
        assert_eq!(unknown.session_id, "s2");
    }

    // -----------------------------------------------------------------------
    // ★ 性能：尾部有界读取 + 缓存
    //
    // 尾部读取的**算法与边界用例**已抽到 `modules::jsonl_tail`（与「今日 Token」
    // 的扫描共用同一实现）—— 那边覆盖块大小 / 停止判据 / 转义形态 / 窗口覆盖整份文件。
    // 这里只留**本模块自己的契约**：读出来的文本必须只含窗口内记录，且能喂给 `parse_records`。
    // -----------------------------------------------------------------------

    #[test]
    fn tail_read_stops_at_the_window_boundary_and_covers_only_what_it_read() {
        let dir = temp_projects_dir("tail");
        let ws = dir.join("ws");
        let path = ws.join("sess.jsonl");
        let since = cst_millis(2026, 10, 5, 0, 0);
        // 老记录（窗口外）+ 填充 + 新记录（窗口内）。填充把老记录推到首块之外。
        let mut body = String::new();
        body.push_str(&jsonl_line(
            "s1",
            cst_millis(2026, 9, 1, 0, 0),
            json!(6004),
            json!(429),
            "429 老记录",
        ));
        body.push('\n');
        let filler = jsonl_line(
            "s1",
            cst_millis(2026, 9, 2, 0, 0),
            json!(6004),
            json!(429),
            &"填".repeat(1200),
        );
        while body.len() < jsonl_tail::TAIL_CHUNK_BYTES as usize + 64 * 1024 {
            body.push_str(&filler);
            body.push('\n');
        }
        body.push_str(&jsonl_line(
            "s1",
            cst_millis(2026, 10, 6, 12, 0),
            json!(6004),
            json!(429),
            "429 新记录",
        ));
        body.push('\n');
        std::fs::write(&path, &body).expect("写 jsonl");

        let text = read_window_text(&path, since).expect("读得到");
        assert!(
            text.len() < body.len(),
            "尾部读取必须比整份文件短（实测中位只需读 19% 的字节）"
        );
        assert!(
            !text.contains("429 老记录"),
            "停在窗口边界 ⇒ 窗口外的前缀根本没被读进来"
        );
        let scan = parse_records(&text, since, "s1");
        assert_eq!(scan.errors.len(), 1, "只留下窗口内那条");
        assert_eq!(scan.errors[0].message, "429 新记录");

        // 全量窗口 ⇒ 整份读（老记录也在）。
        let full = read_window_text(&path, 0).expect("读得到");
        assert_eq!(full.len(), body.len());
        assert!(full.contains("429 老记录"));

        clear_scan_cache_entry(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_covers_only_when_the_window_is_no_wider_than_what_was_parsed() {
        let entry = |covered_from_ms| ScanCacheEntry {
            mtime_ms: 1,
            size: 1,
            covered_from_ms,
            scan: FileScan::default(),
        };
        let seven_days = cst_millis(2026, 10, 5, 0, 0);
        let cached = entry(Some(seven_days));
        // 同一窗口 / 更窄的窗口 ⇒ 命中。
        assert!(cache_covers(&cached, seven_days));
        assert!(cache_covers(&cached, cst_millis(2026, 10, 6, 0, 0)));
        // 更宽的窗口 ⇒ 必须重扫（否则会拿到被过滤过的结果）。
        assert!(!cache_covers(&cached, cst_millis(2026, 9, 20, 0, 0)));
        assert!(!cache_covers(&cached, 0));
        // 全量解析过 ⇒ 覆盖一切。
        let full = entry(Some(0));
        assert!(cache_covers(&full, 0));
        assert!(cache_covers(&full, seven_days));
        // 无有效覆盖（缺字段 / 旧格式）⇒ 永不命中。
        assert!(!cache_covers(&entry(None), seven_days));
        assert!(!cache_covers(&entry(None), 0));
    }

    #[test]
    fn persisted_cache_round_trips_and_rejects_a_version_mismatch() {
        let dir = temp_projects_dir("cache");
        let path = dir.join("cache.json");
        let mut cache: HashMap<PathBuf, ScanCacheEntry> = HashMap::new();
        cache.insert(
            PathBuf::from("C:/x/a.jsonl"),
            ScanCacheEntry {
                mtime_ms: 111,
                size: 222,
                covered_from_ms: Some(333),
                scan: FileScan {
                    session_id: "s1".to_string(),
                    last_success_ms: 444,
                    last_attempt_ms: 666,
                    errors: vec![raw("s1", 555, 6004, "429 x", None)],
                },
            },
        );
        // `write_persisted_cache` 会丢掉不存在的路径 ⇒ 先造一个真文件。
        std::fs::write("C:/x/a.jsonl", "").ok();
        let probe = std::env::temp_dir().join("buddy-switch-cache-probe.jsonl");
        std::fs::write(&probe, "").expect("造真文件");
        let mut cache2: HashMap<PathBuf, ScanCacheEntry> = HashMap::new();
        cache2.insert(
            probe.clone(),
            cache.values().next().expect("有条目").clone(),
        );
        write_persisted_cache(&path, &cache2);
        let loaded = read_persisted_cache(&path).expect("读得回来");
        let entry = loaded.get(&probe).expect("条目在");
        assert_eq!(entry.mtime_ms, 111);
        assert_eq!(entry.size, 222);
        assert_eq!(entry.covered_from_ms, Some(333));
        let scan = entry.to_scan();
        assert_eq!(scan.last_success_ms, 444);
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(
            scan.errors[0].file_success_ms, 444,
            "派生字段必须在装载时回填，否则「恢复实证」全丢"
        );

        // 版本不符 ⇒ 整体丢弃（宁可重扫，不可读出错值）。
        // ⚠️ 锚点必须由 `SCAN_CACHE_VERSION` 拼出来：写死数字的话，版本号一升这条用例
        // 就会「替换不到 ⇒ 文件仍是合法版本 ⇒ 断言失败」，看起来像功能坏了。
        let broken = std::fs::read_to_string(&path).expect("读得到").replace(
            &format!("\"version\":{SCAN_CACHE_VERSION}"),
            "\"version\":999",
        );
        std::fs::write(&path, broken).expect("写坏版本");
        assert!(read_persisted_cache(&path).is_none());

        let _ = std::fs::remove_file(&probe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_uses_the_cache_and_reparses_after_the_file_changes() {
        let dir = temp_projects_dir("cachehit");
        let ws = dir.join("ws");
        let path = ws.join("sess.jsonl");
        let now = snapshot_now();
        std::fs::write(
            &path,
            format!(
                "{}\n",
                jsonl_line("s1", now - 60_000, json!(6004), json!(429), "429 x")
            ),
        )
        .expect("写 jsonl");
        clear_scan_cache_entry(&path);

        let cold = scan_client_errors_in(&dir, now - DAY_MS);
        assert_eq!(cold.len(), 1);
        assert_eq!(cold[0].errors.len(), 1);

        // 热：文件没变 ⇒ 命中缓存，结果一致。
        let warm = scan_client_errors_in(&dir, now - DAY_MS);
        assert_eq!(warm.len(), 1);
        assert_eq!(warm[0].errors.len(), 1);
        assert_eq!(warm[0].errors[0].message, cold[0].errors[0].message);

        // 文件追加 ⇒ (mtime, size) 变了 ⇒ 必须重扫，新记录要出来。
        let mut body = std::fs::read_to_string(&path).expect("读得到");
        body.push_str(&format!(
            "{}\n",
            jsonl_line(
                "s1",
                now - 30_000,
                json!(6020),
                json!(429),
                "429 queue.waiting.title"
            )
        ));
        std::fs::write(&path, body).expect("追加");
        let refreshed = scan_client_errors_in(&dir, now - DAY_MS);
        assert_eq!(
            refreshed[0].errors.len(),
            2,
            "文件变了必须重扫，不得吃旧缓存"
        );

        clear_scan_cache_entry(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn narrower_window_reuses_the_cache_without_leaking_older_records() {
        // ⚠️ 命中缓存的**前提**是「缓存覆盖 ⊇ 本次窗口」⇒ 缓存里可能带着窗口外的旧记录。
        // 实测踩到：先用 30 天窗口刷过一次，再切回「近 7 天」，09-16 / 09-23 的记录
        // 仍在列表里（entries 94 → 27、scanned 9673 → 119，连「仍受限」那条都是窗口外的）。
        let dir = temp_projects_dir("cache-window");
        let ws = dir.join("ws");
        let path = ws.join("sess.jsonl");
        let now = snapshot_now();
        let old = now - 30 * DAY_MS;
        let recent = now - 3_600_000;
        let body = [
            jsonl_line("s1", old, json!(6004), json!(429), "429 老记录"),
            jsonl_line("s1", recent, json!(6004), json!(429), "429 新记录"),
        ]
        .join("\n");
        std::fs::write(&path, format!("{body}\n")).expect("写 jsonl");
        clear_scan_cache_entry(&path);

        // ① 先用**全量**窗口解析一次（缓存 `covered_from_ms = Some(0)`，覆盖一切窗口）。
        let (wide, _) = collect_rate_limit_errors(scan_client_errors_in(&dir, 0));
        assert_eq!(wide.len(), 2);

        // ② 再按「近 7 天」请求 ⇒ **命中缓存**，但必须只剩窗口内那一条。
        let (narrow, _) = collect_rate_limit_errors(scan_client_errors_in(&dir, now - 7 * DAY_MS));
        assert_eq!(
            narrow.len(),
            1,
            "命中缓存也必须按本次窗口收敛，不得把窗口外的旧记录带出来"
        );
        assert_eq!(narrow[0].timestamp, recent);

        // ③ 反方向：更宽的窗口不得吃窄缓存（否则会丢记录）。
        clear_scan_cache_entry(&path);
        let (narrow_first, _) =
            collect_rate_limit_errors(scan_client_errors_in(&dir, now - 7 * DAY_MS));
        assert_eq!(narrow_first.len(), 1);
        let (wide_again, _) = collect_rate_limit_errors(scan_client_errors_in(&dir, 0));
        assert_eq!(
            wide_again.len(),
            2,
            "更宽的窗口必须重扫，不得吃窄窗口的缓存"
        );

        clear_scan_cache_entry(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★ 预筛（[`line_may_matter`]）**不得**漏掉「用户又试过」这条实证。
    ///
    /// 预筛把不相关的行直接跳过（省 JSON 解析），一旦子串写漏，`active` 会静默退化。
    /// 这里用**真实形状**的 `role=user` 消息行 + 一条无关的内务行驱动整条扫描链。
    #[test]
    fn prefilter_keeps_a_user_message_as_attempt_evidence() {
        let dir = temp_projects_dir("prefilter");
        let ws = dir.join("ws");
        let now = snapshot_now();
        let limited_at = now - 3_600_000;
        let retried_at = limited_at + 300_000;
        let body = [
            jsonl_line("s1", limited_at, json!(6004), json!(429), "429 x"),
            // 与台账无关的行（内务记录）：预筛会跳过它。
            json!({
                "id": "h1",
                "timestamp": limited_at + 100_000,
                "type": "file-history-snapshot",
                "sessionId": "s1",
            })
            .to_string(),
            // 用户消息 = 「又试过」的唯一实证来源。
            json!({
                "id": "u1",
                "timestamp": retried_at,
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "继续" }],
                "sessionId": "s1",
            })
            .to_string(),
        ]
        .join("\n");
        std::fs::write(ws.join("sess.jsonl"), format!("{body}\n")).expect("写 jsonl");

        let (errors, _) = collect_rate_limit_errors(scan_client_errors_in(&dir, 0));
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].file_attempt_ms, retried_at,
            "用户消息必须被算作「又试过」—— 预筛把它漏掉的话这里会停在错误自身"
        );
        let meta = HashMap::new();
        let names = HashMap::new();
        let limits = build_limits(Region::Cn, errors, &meta, &names, now);
        assert_eq!(
            limits[0].state,
            LimitState::Active,
            "之后又试过、却无成功产出 ⇒ 仍受限"
        );

        clear_scan_cache_entry(&ws.join("sess.jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_detects_a_success_that_proves_the_limit_is_over() {
        let dir = temp_projects_dir("evidence");
        let ws = dir.join("ws");
        let now = snapshot_now();
        // 复现实测形状：17:20 报 429，17:36 起模型恢复正常产出。
        let limited_at = now - 3_600_000;
        let body = [
            // ⚠️ 关键：错误**之前**也有成功产出。若把「成功时刻」存成文件内**最早**值，
            // 判据 `evidence > occurred_at` 就恒假 ⇒ 6 条本该「已恢复」的全部误报「仍受限」。
            jsonl_success("s1", limited_at - 600_000, "reasoning"),
            jsonl_line("s1", limited_at, json!(6004), json!(429), "429 x"),
            jsonl_line("s1", limited_at + 960_000, Value::Null, json!(429), "x"),
            jsonl_success("s1", limited_at + 990_000, "reasoning"),
        ]
        .join("\n");
        std::fs::write(ws.join("sess.jsonl"), format!("{body}\n")).expect("写 jsonl");
        let (errors, _) = collect_rate_limit_errors(scan_client_errors_in(&dir, 0));
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].file_success_ms,
            limited_at + 990_000,
            "必须记下**最晚**的成功时刻（最早值会让「错误之后是否有成功」恒假）"
        );
        let meta = HashMap::new();
        let names = HashMap::new();
        let limits = build_limits(Region::Cn, errors, &meta, &names, now);
        assert_eq!(limits[0].state, LimitState::Recovered);
        assert_eq!(limits[0].evidence_at, Some(limited_at + 990_000));

        clear_scan_cache_entry(&ws.join("sess.jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_reports_unknown_when_the_error_ends_the_session() {
        // 实测形状：429 就是该会话的**最后一条**记录（用户没再试）⇒ 状态未知，
        // **不得**报「仍受限」（复现 `d49456c2` / `8fe7b8b0`）。
        let dir = temp_projects_dir("ended");
        let ws = dir.join("ws");
        let now = snapshot_now();
        let limited_at = now - 3_600_000;
        let body = [
            jsonl_line("s1", limited_at - 600_000, Value::Null, json!(429), "x"),
            jsonl_success("s1", limited_at - 300_000, "reasoning"),
            jsonl_line(
                "s1",
                limited_at,
                json!(6004),
                json!(429),
                "429 usage exceeds frequency limit, your usage will reset at 2026-10-13 09:04:43 UTC+8",
            ),
        ]
        .join("\n");
        std::fs::write(ws.join("sess.jsonl"), format!("{body}\n")).expect("写 jsonl");

        let (errors, _) = collect_rate_limit_errors(scan_client_errors_in(&dir, 0));
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].file_attempt_ms, limited_at,
            "错误就是最后一条 ⇒ 「又试过」的时刻就等于它自己，不严格晚于"
        );
        assert!(
            errors[0].file_success_ms < limited_at,
            "成功发生在错误之前，不构成恢复实证"
        );
        let meta = HashMap::new();
        let names = HashMap::new();
        let limits = build_limits(Region::Cn, errors, &meta, &names, now);
        assert_eq!(limits[0].state, LimitState::Unknown);
        assert_eq!(limits[0].evidence_at, None);

        clear_scan_cache_entry(&ws.join("sess.jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // 并发：尾部半行
    // -----------------------------------------------------------------------

    #[test]
    fn tail_is_complete_json_detects_torn_last_line() {
        assert!(tail_is_complete_json(r#"{"a":1}"#));
        assert!(tail_is_complete_json("{\"a\":1}\n"));
        assert!(tail_is_complete_json("{\"a\":1}\n\n"));
        assert!(!tail_is_complete_json("{\"a\":1}\n{\"a\""));
        assert!(tail_is_complete_json("{\"a\":1}\n  "), "纯空白尾行不算半行");
        assert!(tail_is_complete_json(""));
    }

    #[test]
    fn scan_keeps_complete_lines_when_the_tail_is_torn() {
        let dir = temp_projects_dir("torn");
        let ws = dir.join("ws");
        let path = ws.join("sess.jsonl");
        let complete = jsonl_line(
            "s1",
            cst_millis(2026, 10, 7, 11, 17),
            json!(6004),
            json!(429),
            "429 您的使用量已超出频率限制",
        );
        let torn = torn_prefix(&complete);
        std::fs::write(&path, format!("{complete}\n{torn}")).expect("写 jsonl");

        let (errors, _) = collect_rate_limit_errors(scan_client_errors_in(&dir, 0));
        assert_eq!(errors.len(), 1, "完整行留下、半行不产生记录");
        assert_eq!(errors[0].session_id, "s1");

        clear_scan_cache_entry(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_window_text_rereads_when_the_tail_is_torn() {
        let dir = temp_projects_dir("reread");
        let path = dir.join("ws").join("sess.jsonl");
        let complete = jsonl_line("s1", 1, json!(6004), json!(429), "429 x");
        let half = torn_prefix(&complete).to_string();
        std::fs::write(&path, &half).expect("写半行");
        // ⚠️ 追加而不是 `fs::write`：后者会先 truncate ⇒ 文件会短暂变成 **0 字节**，
        // 此时 `size == 0` 会立刻返回空串（不进重读分支），用例随机变红。
        // 真实客户端也只追加，不会截断。
        let rest = complete[half.len()..].to_string();
        let path_for_writer = path.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path_for_writer)
                .expect("打开追加");
            file.write_all(rest.as_bytes()).expect("补完");
        });
        let text = read_window_text(&path, 0).expect("读得到");
        writer.join().expect("写线程不 panic");
        assert_eq!(text, complete, "尾部半行必须触发一次重读");

        clear_scan_cache_entry(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // 测试夹具
    // -----------------------------------------------------------------------

    fn raw_with(
        code: Option<i64>,
        code_text: Option<&str>,
        status: Option<i64>,
        message: &str,
    ) -> RawClientError {
        RawClientError {
            session_id: "s".to_string(),
            timestamp: 0,
            code,
            code_text: code_text.map(str::to_string),
            status,
            message: message.to_string(),
            agent: None,
            cwd: None,
            reset_at: None,
            file_success_ms: 0,
            file_attempt_ms: 0,
        }
    }

    /// 一行 jsonl（字段形态与客户端实测一致）。
    fn jsonl_line(
        session_id: &str,
        timestamp: i64,
        code: Value,
        status: Value,
        message: &str,
    ) -> String {
        json!({
            "id": "rec-1",
            "timestamp": timestamp,
            "type": "message",
            "role": "assistant",
            "providerData": {
                "skipRun": true,
                "error": { "code": code, "status": status, "message": message },
                "agent": "cli",
            },
            "sessionId": session_id,
            "cwd": "c:\\proj",
        })
        .to_string()
    }

    /// 一行「模型确实工作过」的 jsonl（无 error）。
    fn jsonl_success(session_id: &str, timestamp: i64, kind: &str) -> String {
        json!({
            "id": "ok-1",
            "timestamp": timestamp,
            "type": kind,
            "sessionId": session_id,
        })
        .to_string()
    }

    /// 取字符串前半段作为「半行」——**必须落在字符边界上**（夹具含中文，
    /// 直接按字节切会 panic）。
    fn torn_prefix(text: &str) -> &str {
        let half = text.len() / 2;
        let end = (0..=half)
            .rev()
            .find(|index| text.is_char_boundary(*index))
            .unwrap_or(0);
        &text[..end]
    }

    /// 建一个临时 `projects/` 目录（含一个 workspace 子目录）。
    fn temp_projects_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("buddy-switch-client-limits-{tag}-{nanos}"));
        std::fs::create_dir_all(dir.join("ws")).expect("建临时目录");
        dir
    }
}
