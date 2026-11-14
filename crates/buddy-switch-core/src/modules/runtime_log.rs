//! 本工具**自身**的运行日志（排查问题用）。
//!
//! ## 它解决的问题
//!
//! 桌面宿主是 `windows_subsystem = "windows"` 的 release GUI（见 `src-tauri/src/main.rs`）：
//! 没有控制台，`eprintln!` 在用户机器上**写进虚无**。于是「网关起不来」「切号失败」
//! 这类问题只有一句界面 toast，用户拿不到任何可粘贴给维护者的东西 —— 本模块把那些
//! 诊断输出落到**文件**里，并在设置页「日志」分区直接可读。
//!
//! ## 与其它日志的分工（三件不同的事，别混）
//!
//! | 日志 | 回答的问题 | 载体 |
//! |:--|:--|:--|
//! | **本模块** | 本工具**自己**做了什么、哪一步失败了 | `~/.buddy-switch/logs/runtime-*.log`（纯文本行） |
//! | 网关请求日志 | 客户端经网关发了什么、用了多少 | `gateway_logs.json`（JSON 数组，面向统计） |
//! | 上游客户端日志 | 被切换的客户端发生了什么 | 各客户端自己的数据目录 |
//!
//! 与 `trae::logs` 的关系：那个模块读**Trae 客户端的**日志；本模块写**本工具的**日志。
//! 两者可以并排出现在设置页，但数据源互不相干。
//!
//! ## 四条硬约束
//!
//! 1. **永不失败、永不 panic**：日志是旁路能力。任何 IO 错误都在内部吞掉（内部错误走
//!    `eprintln!`，**不得**回调本模块 —— 否则出错时自我递归）。
//! 2. **有界**：单文件 4 MiB × 单日最多 4 个分片（≤16 MiB/天），保留 7 天（≤112 MiB）。
//!    用户机器上的一个调试功能不能把磁盘写满。
//! 3. **出站脱敏**：写盘前过 [`redact`]（token / 密码 / JWT / 信封）。日志文件可能被
//!    贴进 issue，凭据不能跟着走。
//! 4. **本机全路径是允许的**：与「诊断包」不同（那份是**发给别人**的，见
//!    [`crate::modules::diagnostics`]，路径一律不出现），本文件留在用户自己机器上，
//!    数据目录路径正是排查时最需要的信息，因此照写。
//!
//! ## 级别怎么调
//!
//! 环境变量 `BUDDY_SWITCH_LOG_LEVEL`（`error` / `warn` / `info` / `debug`），缺省 `info`。
//! 进程启动时读一次并缓存：改它要重启应用。**刻意不做成设置页开关** —— 那需要
//! 一对命令 + 四处接线 + 首帧未读到的问题，而调日志级别是排查时的临时动作。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::{Local, TimeZone};
use serde_json::{json, Value};

use crate::modules::config;

/// 日志目录名（在 `store_dir()` 下）。
pub const LOGS_DIR_NAME: &str = "logs";

/// 文件名前缀（清空 / 保留裁剪 / 读取都按它筛，避免误删同目录的其它文件）。
const FILE_PREFIX: &str = "runtime-";

/// 单日保留天数。取 7：一次排查通常跨不过一周，再久的信息量已被新日志覆盖。
pub const KEEP_DAYS: i64 = 7;

/// 单个日志文件的上限（字节）。达到即滚到下一个分片。
pub const MAX_PART_BYTES: u64 = 4 * 1024 * 1024;

/// 单日最多几个分片。超出后**丢弃**后续行，但只写一条丢弃标记。
pub const MAX_PARTS_PER_DAY: usize = 4;

/// 读取时单次返回的最大条数（防止把整个文件塞进 HTTP 响应）。
pub const MAX_READ_LIMIT: usize = 2000;

/// 读取时的默认条数（取**尾部**：排查看的是最近发生了什么）。
pub const DEFAULT_READ_LIMIT: usize = 500;

/// 级别（数值越大越啰嗦；过滤用 `>=` 阈值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

impl Level {
    /// 稳定标识 —— 与前端筛选器的 `value` **逐字一致**（`error` / `warn` / `info` / `debug`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
        }
    }

    /// 定宽大写（落盘用，让纯文本日志按列对齐、`grep` 更好读）。
    fn tag(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
        }
    }

    /// 解析（大小写不敏感；非法值 `None`，由调用方决定回落）。
    pub fn parse(raw: &str) -> Option<Level> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Level::Error),
            "warn" | "warning" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" | "trace" => Some(Level::Debug),
            _ => None,
        }
    }
}

/// 环境变量名（级别覆盖）。
pub const LOG_LEVEL_ENV: &str = "BUDDY_SWITCH_LOG_LEVEL";

/// 解析级别：入参显式（测试用）> 缺省 `Info`。
///
/// 取入参而非直接读环境变量：本仓约定「可并行单测的纯决策函数一律显式入参」，
/// 否则测试之间会互相踩 `std::env`。
fn resolve_level(raw: Option<&str>) -> Level {
    raw.and_then(Level::parse).unwrap_or(Level::Info)
}

/// 进程内生效级别（启动后首次写入时读一次环境变量并缓存）。
fn effective_level() -> Level {
    static LEVEL: OnceLock<Level> = OnceLock::new();
    *LEVEL.get_or_init(|| resolve_level(std::env::var(LOG_LEVEL_ENV).ok().as_deref()))
}

/// 路径 → 展示字符串：**统一成正斜杠**。
///
/// Windows 的 `Path::display()` 会产出 `C:/Users/x\Temp\.buddy-switch` 这种**混用**形式
/// （前缀用 `/`、分隔用 `\`），贴进 issue 或复制到命令行时容易出错。日志与 UI 里
/// 统一成 `/` —— Windows 的文件管理器与 `cd` 都接受正斜杠，macOS/Linux 本来就是 `/`。
fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// 日志目录。**不主动创建**（读侧不该因为「看一眼」就在磁盘上建目录）。
pub fn logs_dir() -> PathBuf {
    config::store_dir().join(LOGS_DIR_NAME)
}

/// 某一天的**第 `part` 个分片**路径。
///
/// `part == 0` 时文件名不带序号（`runtime-2026-10-23.log`）—— 让 `ls` 出来的主文件
/// 一眼可认，也兼容用户手工 `tail -f` 的习惯。
pub fn log_file_for(date: &str, part: usize) -> PathBuf {
    let name = if part == 0 {
        format!("{FILE_PREFIX}{date}.log")
    } else {
        format!("{FILE_PREFIX}{date}.{part}.log")
    };
    logs_dir().join(name)
}

/// 从文件名解出 `(日期, 分片序号)`；不是本模块的文件 ⇒ `None`。
///
/// 这是清空 / 裁剪 / 列日期的**唯一解析点**：三处各写一遍正则迟早漂移
/// （真实风险：`.1.log` 被当成日期、或 `runtime-2026-10-23.bak` 被当成合法分片）。
fn parse_log_name(name: &str) -> Option<(String, usize)> {
    let rest = name.strip_prefix(FILE_PREFIX)?.strip_suffix(".log")?;
    // 形如 `2026-10-23` 或 `2026-10-23.2`
    let (date, part) = match rest.split_once('.') {
        Some((date, part)) => (date, part.parse::<usize>().ok()?),
        None => (rest, 0),
    };
    // 日期必须是 `YYYY-MM-DD`：位数与分隔位置都卡死，顺带挡掉路径穿越与怪名字。
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return None;
    }
    Some((date.to_string(), part))
}

/// 列出所有日志文件 `(日期, 分片, 路径)`，按日期、分片升序。
fn list_log_files() -> Vec<(String, usize, PathBuf)> {
    let dir = logs_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out: Vec<(String, usize, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?;
            let (date, part) = parse_log_name(name)?;
            Some((date, part, path))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    out
}

// ---------------------------------------------------------------------------
// 脱敏
// ---------------------------------------------------------------------------

/// 需要脱敏的 `name=value` 参数名（**小写比较**）。
///
/// 只收**明确的凭据参数名**，不收 `key` / `code` 这类泛词 —— 后者在日志里大量合法出现
/// （`error_code=...`、`key=pool`），一并抹掉等于把排查线索也删了。
const SECRET_PARAMS: &[&str] = &[
    "access_token",
    "refresh_token",
    "id_token",
    "token",
    "password",
    "passwd",
    "secret",
    "client_secret",
    "api_key",
    "apikey",
    "authorization",
    "cookie",
    "set-cookie",
];

/// 脱敏标记（唯一写法，测试按它断言）。
pub const REDACTED: &str = "<redacted>";

/// 写盘前的**出站脱敏**。
///
/// 处理三类（其余原样保留 —— 日志的价值在于信息量，宁可少抹不可乱抹）：
/// 1. `name=value` / `name: value` 形式，`name` 命中 [`SECRET_PARAMS`]；
/// 2. `Bearer <token>`；
/// 3. JWT 形状（`eyJ…` 三段 base64url）—— 它会以「纯 token」出现在 header 转储里，
///    没有参数名可依附。
pub fn redact(input: &str) -> String {
    let mut text = redact_bearer(input);
    text = redact_secret_params(&text);
    redact_jwt(&text)
}

/// `Bearer xxx` → `Bearer <redacted>`（大小写不敏感）。
fn redact_bearer(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find("bearer ") {
        let start = cursor + found;
        let value_start = start + "bearer ".len();
        out.push_str(&input[cursor..value_start]);
        // token 到下一个空白 / 引号 / 逗号 / 分号为止。
        let end = input[value_start..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == ';')
            .map(|offset| value_start + offset)
            .unwrap_or(input.len());
        if end > value_start {
            out.push_str(REDACTED);
        }
        cursor = end;
    }
    out.push_str(&input[cursor..]);
    out
}

/// `name=value` / `name: value` 形式脱敏。
fn redact_secret_params(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    // 逐个候选名找，取**最左**命中继续，保证输出顺序与原串一致。
    loop {
        // (命中起点, 参数名结束处 —— 即分隔符之后、值之前)
        let mut best: Option<(usize, usize)> = None;
        for name in SECRET_PARAMS {
            for separator in ['=', ':'] {
                let needle = format!("{name}{separator}");
                let Some(found) = lower[cursor..].find(&needle) else {
                    continue;
                };
                let start = cursor + found;
                // 左侧必须是边界（行首 / 非标识符字符），否则 `my_token=` 会被当成 `token=`。
                let boundary = start == 0
                    || !lower.as_bytes()[start - 1].is_ascii_alphanumeric()
                        && lower.as_bytes()[start - 1] != b'_';
                if !boundary {
                    continue;
                }
                if best.map(|(b, _)| start < b).unwrap_or(true) {
                    best = Some((start, start + needle.len()));
                }
            }
        }
        let Some((start, name_end)) = best else {
            out.push_str(&input[cursor..]);
            return out;
        };
        out.push_str(&input[cursor..start]);
        // 保留 `name=` / `name: ` 原样（含分隔符后的空格），只换掉值。
        let mut keep_end = name_end;
        while keep_end < input.len() && input.as_bytes()[keep_end] == b' ' {
            keep_end += 1;
        }
        out.push_str(&input[start..keep_end]);
        // 值到分隔符为止（& 空格 " ' , ; } ] 换行）。
        let value_end = input[keep_end..]
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '&' | '"' | '\'' | ',' | ';' | '}' | ']' | ')')
            })
            .map(|offset| keep_end + offset)
            .unwrap_or(input.len());
        if value_end > keep_end {
            out.push_str(REDACTED);
        }
        cursor = value_end;
    }
}

/// JWT（`eyJ` 开头的三段 base64url）脱敏。
///
/// 判据：以 `eyJ` 起、只含 base64url 字符与 `.`、至少两个 `.`、总长 > 40。
/// 这三个条件一起才动手 —— 单看 `eyJ` 会误伤普通标识符。
///
/// ⚠️ 逐 `char` 扫描而不是按字节切片：日志里混着中文（`[gateway] 启动失败`），
/// 按字节下标切会在多字节字符中间落刀而 **panic**（真实踩过：一次 panic 就把
/// 「日志模块」变成新的故障源，比没有日志更糟）。
fn redact_jwt(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while !rest.is_empty() {
        if rest.starts_with("eyJ") {
            // 往后吃 base64url / `.` / `=` 的**字符**，遇到其它字符就停（天然对齐边界）。
            let mut consumed = 0;
            let mut dots = 0;
            for ch in rest.chars() {
                let accepted = ch == '.'
                    || ch.is_ascii_alphanumeric()
                    || matches!(ch, '-' | '_' | '+' | '/' | '=');
                if !accepted {
                    break;
                }
                if ch == '.' {
                    dots += 1;
                }
                consumed += ch.len_utf8();
            }
            if dots >= 2 && consumed > 40 {
                out.push_str(REDACTED);
                rest = &rest[consumed..];
                continue;
            }
        }
        let mut chars = rest.chars();
        if let Some(ch) = chars.next() {
            out.push(ch);
            rest = chars.as_str();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 写入
// ---------------------------------------------------------------------------

/// 写入器的进程内状态（**单锁**：日期 / 分片 / 已写字节 / 丢弃标记）。
struct WriterState {
    date: String,
    part: usize,
    size: u64,
    dropped_marked: bool,
}

static WRITER: Mutex<Option<WriterState>> = Mutex::new(None);

/// 上次执行保留裁剪的时刻（毫秒）；0 = 从未。
static LAST_PRUNE_MS: AtomicI64 = AtomicI64::new(0);

/// 保留裁剪的检查间隔（1 小时）。裁剪是 `read_dir` + 若干 `remove_file`，
/// 每行日志都跑没必要；一小时足够让「跨天」在下次写入时被发现。
const PRUNE_INTERVAL_MS: i64 = 60 * 60 * 1000;

/// 写一条日志（**唯一写入入口**）。
///
/// `scope` 是短标签（`gateway` / `switch` / `trae` …），对应日志行的方括号段。
pub fn log(level: Level, scope: &str, message: &str) {
    if level > effective_level() {
        return;
    }
    // 保留裁剪挂在写入路径上（节流到每小时一次）：桌面宿主可能连续跑几周不重启，
    // 只在启动时裁一次会让「7 天上限」在长跑进程里失效。
    prune_throttled();
    let now = Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    // 单行化：日志是**逐行**格式，消息里的换行会把一条记录拆成两条（`grep` 时更明显）。
    let message = message.replace(['\r', '\n'], " ");
    let line = format!(
        "{} {} [{}] {}\n",
        now.format("%Y-%m-%d %H:%M:%S%.3f"),
        level.tag(),
        scope,
        redact(&message)
    );

    write_line(&date, &line, scope);
}

/// 内部：把一行落到当日的分片里。
///
/// 任何失败都在这里**吞掉**（`eprintln!` 只给开发态看，且**不回调** [`log`]）。
fn write_line(date: &str, line: &str, scope: &str) {
    let Ok(mut guard) = WRITER.lock() else {
        eprintln!("[runtime-log] writer lock poisoned");
        return;
    };
    if guard.as_ref().map(|state| state.date.as_str()) != Some(date) {
        let size = std::fs::metadata(log_file_for(date, 0))
            .map(|meta| meta.len())
            .unwrap_or(0);
        *guard = Some(WriterState {
            date: date.to_string(),
            part: 0,
            size,
            dropped_marked: false,
        });
    }
    let Some(state) = guard.as_mut() else {
        return;
    };

    // 分片满了 ⇒ 往后找一个还有空间的；全都满了 ⇒ 只写一次丢弃标记。
    while state.size + line.len() as u64 > MAX_PART_BYTES {
        if state.part + 1 >= MAX_PARTS_PER_DAY {
            if !state.dropped_marked {
                state.dropped_marked = true;
                let marker = format!(
                    "{} {} [{}] 当日日志已达 {} 个分片上限，后续日志被丢弃\n",
                    Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                    Level::Warn.tag(),
                    scope,
                    MAX_PARTS_PER_DAY
                );
                let path = log_file_for(date, state.part);
                let _ = append(&path, &marker);
            }
            return;
        }
        state.part += 1;
        state.size = std::fs::metadata(log_file_for(date, state.part))
            .map(|meta| meta.len())
            .unwrap_or(0);
    }

    let path = log_file_for(date, state.part);
    if let Err(error) = append(&path, line) {
        // 只报一次（避免每行一条 stderr）；不动 state，下次重试。
        if !state.dropped_marked {
            state.dropped_marked = true;
            eprintln!("[runtime-log] 写入失败（{}）：{error}", path.display());
        }
        return;
    }
    state.size += line.len() as u64;
}

/// 追加写（`create_dir_all` + `OpenOptions::append`）。
fn append(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())
}

/// 便捷入口。
pub fn error(scope: &str, message: impl AsRef<str>) {
    log(Level::Error, scope, message.as_ref());
}

pub fn warn(scope: &str, message: impl AsRef<str>) {
    log(Level::Warn, scope, message.as_ref());
}

pub fn info(scope: &str, message: impl AsRef<str>) {
    log(Level::Info, scope, message.as_ref());
}

pub fn debug(scope: &str, message: impl AsRef<str>) {
    log(Level::Debug, scope, message.as_ref());
}

// ---------------------------------------------------------------------------
// 保留裁剪
// ---------------------------------------------------------------------------

/// 删除超过 [`KEEP_DAYS`] 的日志文件，返回删除数量。
///
/// 判据用**文件名里的日期**而不是 mtime：mtime 会被「补写历史」「拷贝备份」改变，
/// 而文件名里的日期是写入时钉死的，与本模块的语义（一天一份）严格一致。
pub fn prune(at_ms: i64) -> usize {
    let cutoff = at_ms.saturating_sub(KEEP_DAYS * 24 * 3600 * 1000);
    let cutoff_date = match Local.timestamp_millis_opt(cutoff).single() {
        Some(time) => time.format("%Y-%m-%d").to_string(),
        None => return 0,
    };
    let mut removed = 0;
    for (date, _part, path) in list_log_files() {
        // 字符串比较对 `YYYY-MM-DD` 是等价于时间比较的（定宽、高位在前）。
        if date.as_str() < cutoff_date.as_str() && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 节流版裁剪：至多每小时一次。
fn prune_throttled() {
    let now = config::now_ms();
    let last = LAST_PRUNE_MS.load(Ordering::Relaxed);
    if now - last < PRUNE_INTERVAL_MS {
        return;
    }
    // CAS 抢占：多线程同时到达时只有一个真正去删（`read_dir` 不必重复）。
    if LAST_PRUNE_MS
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        prune(now);
    }
}

/// 供宿主在**启动时**调用：装 panic hook + 写启动横幅 + 触发一次裁剪。
///
/// `app_label` 用于横幅（`BuddySwitch desktop` / `buddy-switch server`）。
/// 挂 hook 而不是让每个宿主自己写一遍：panic 是**最需要落盘**的一类事件，
/// 漏装一处就等于「某端崩了但日志里什么都没有」。
pub fn init(app_label: &str) {
    install_panic_hook();
    let now = config::now_ms();
    info(
        "startup",
        &format!(
            "{app_label} 启动 · 版本 {} · {} {} · pid {} · 数据目录 {} · 日志级别 {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::process::id(),
            display_path(&config::store_dir()),
            effective_level().as_str()
        ),
    );
    prune(now);
    LAST_PRUNE_MS.store(now, Ordering::Relaxed);
}

/// 装 panic hook：把 panic 位置与载荷写进日志，再交给默认 hook（保留原有 stderr 行为）。
///
/// 幂等性说明：`set_hook` 会**替换**上一个 hook，两个宿主各调一次没问题（同一进程只
/// 有一个宿主）。重复调用只会让「链式调用」多套一层 —— 故本函数只在启动时调用一次。
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| panic_info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "（载荷非字符串）".to_string());
        let location = panic_info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "未知位置".to_string());
        // 脱敏在 `log` 内部完成；这里不再预处理。
        error("panic", &format!("{location} · {payload}"));
        previous(panic_info);
    }));
}

// ---------------------------------------------------------------------------
// 读取 / 清空（设置页用）
// ---------------------------------------------------------------------------

/// 一条解析后的日志。
struct Entry {
    ts: i64,
    time: String,
    level: Level,
    scope: String,
    message: String,
}

/// 解析一行；不是本模块格式的行（用户手写的、旧版本的）⇒ `None`。
///
/// 宽松处：`level` 段大小写不敏感、`scope` 允许缺失（只认「时间 + 级别 + 方括号」骨架）。
fn parse_line(line: &str) -> Option<Entry> {
    // `YYYY-MM-DD HH:MM:SS.mmm LEVEL [scope] message`
    if line.len() < 20 {
        return None;
    }
    let (date_part, rest) = line.split_once(' ')?;
    if date_part.len() != 10 {
        return None;
    }
    let (time_part, rest) = rest.split_once(' ')?;
    if time_part.len() < 8 {
        return None;
    }
    let (level_part, rest) = rest.split_once(' ')?;
    let level = Level::parse(level_part)?;
    let rest = rest.trim_start();
    let (scope, message) = match rest.strip_prefix('[') {
        Some(after) => match after.split_once("] ") {
            Some((scope, message)) => (scope.to_string(), message.to_string()),
            None => (String::new(), rest.to_string()),
        },
        None => (String::new(), rest.to_string()),
    };
    // 毫秒时间戳：解析失败仍保留该行（只是排序退化为按行序），不因一行坏时间丢整条。
    let ts = chrono::NaiveDateTime::parse_from_str(
        &format!("{date_part} {time_part}"),
        "%Y-%m-%d %H:%M:%S%.3f",
    )
    .or_else(|_| {
        chrono::NaiveDateTime::parse_from_str(
            &format!("{date_part} {time_part}"),
            "%Y-%m-%d %H:%M:%S",
        )
    })
    .ok()
    .and_then(|naive| Local.from_local_datetime(&naive).single())
    .map(|time| time.timestamp_millis())
    .unwrap_or(0);
    Some(Entry {
        ts,
        time: format!("{date_part} {time_part}"),
        level,
        scope,
        message,
    })
}

/// 读取运行日志。
///
/// 入参（全部可选，形状与 `trae::handlers::logs` 对齐，前端可复用同一套筛选器）：
/// - `level`：最低级别（`error` / `warn` / `info` / `debug`），缺省不过滤；
/// - `date`：`YYYY-MM-DD`，缺省读**全部保留日期**；
/// - `keyword`：大小写不敏感的子串匹配（在 scope + message 上）；
/// - `limit`：返回条数上限，缺省 [`DEFAULT_READ_LIMIT`]，上限 [`MAX_READ_LIMIT`]。
///
/// 返回：`{ entries, dates, counts, logDir, level }`。`entries` 按时间**降序**
/// （最新在前）—— 排查时第一眼要的是「刚才发生了什么」。
pub fn read(params: &Value) -> Value {
    let level_filter = params
        .get("level")
        .and_then(Value::as_str)
        .and_then(Level::parse);
    let date_filter = params
        .get("date")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "all")
        .map(str::to_string);
    let keyword = params
        .get("keyword")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_lowercase());
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_READ_LIMIT)
        .clamp(1, MAX_READ_LIMIT);

    let files = list_log_files();
    let mut dates: Vec<String> = files.iter().map(|(date, _, _)| date.clone()).collect();
    dates.dedup();
    dates.reverse(); // 最新日期在前（下拉框直接可用）

    let mut counts = json!({ "error": 0, "warn": 0, "info": 0, "debug": 0 });
    let mut entries: Vec<Entry> = Vec::new();
    let mut total = 0usize;

    for (date, _part, path) in &files {
        if let Some(want) = date_filter.as_deref() {
            if date.as_str() != want {
                continue;
            }
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let Some(entry) = parse_line(line) else {
                continue;
            };
            // counts 统计**过滤前**的当日总量：让用户知道「有多少是被筛掉的」。
            if counts.get(entry.level.as_str()).is_some() {
                counts[entry.level.as_str()] = json!(
                    counts
                        .get(entry.level.as_str())
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        + 1
                );
            }
            if let Some(min) = level_filter {
                if entry.level > min {
                    continue;
                }
            }
            if let Some(needle) = keyword.as_deref() {
                let haystack = format!("{} {}", entry.scope, entry.message).to_lowercase();
                if !haystack.contains(needle) {
                    continue;
                }
            }
            total += 1;
            entries.push(entry);
        }
    }

    // 稳定排序：ts 相同的行保持文件内顺序；整体按时间降序。
    entries.sort_by(|a, b| b.ts.cmp(&a.ts));
    let truncated = entries.len() > limit;
    entries.truncate(limit);

    let items: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            json!({
                "ts": entry.ts,
                "time": entry.time,
                "level": entry.level.as_str(),
                "scope": entry.scope,
                "message": entry.message,
            })
        })
        .collect();

    json!({
        "entries": items,
        "dates": dates,
        "counts": counts,
        "total": total,
        "truncated": truncated,
        "limit": limit,
        "level": effective_level().as_str(),
        "logDir": display_path(&logs_dir()),
        "keepDays": KEEP_DAYS,
        "maxPartBytes": MAX_PART_BYTES,
        "maxPartsPerDay": MAX_PARTS_PER_DAY,
    })
}

/// 清空**全部**运行日志，返回删除数量。
///
/// 只删本模块命名的文件（[`parse_log_name`] 判据），同目录其它东西一律不碰。
pub fn clear() -> Result<Value, String> {
    let mut removed = 0;
    for (_date, _part, path) in list_log_files() {
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    // 写入器缓存里还记着旧字节数 ⇒ 复位，否则下一次写会以为分片已满而白滚一格。
    if let Ok(mut guard) = WRITER.lock() {
        *guard = None;
    }
    Ok(json!({ "ok": true, "removed": removed }))
}

/// 在系统文件管理器中打开日志目录，返回 `{ok, path}`。
///
/// 先 `create_dir_all`：日志目录**只在写入后才存在**，新装用户点「打开目录」不该看到
/// 「目录不存在」。
///
/// ⚠️ **测试不得调用本函数**：三个平台分支都会真的起一个 GUI 进程
/// （Windows `explorer` / macOS `open` / Linux `xdg-open`），CI 上会弹窗或挂住。
/// 与 `capability::open_workbuddy_data_dir` 是同一条纪律（见其文档）。
///
/// 平台分支刻意放在 core 而不是两个宿主各写一遍：`reveal_dir` 在
/// `buddy-switch-server/src/api.rs` 已经有一份（服务端打开**账号库**目录用），
/// 本函数是**日志目录**版本；两者用途不同但平台判据必须一致，
/// 故不合并、也不让宿主再抄第三份 `cfg` 分支。
pub fn open_logs_dir() -> Result<Value, String> {
    let dir = logs_dir();
    std::fs::create_dir_all(&dir).map_err(|error| format!("创建日志目录失败: {error}"))?;

    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";

    #[cfg(any(target_os = "windows", target_os = "macos", unix))]
    {
        std::process::Command::new(program)
            .arg(&dir)
            .spawn()
            .map_err(|error| format!("打开日志目录失败: {error}"))?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        return Err("当前平台不支持打开目录".to_string());
    }

    Ok(json!({ "ok": true, "path": display_path(&dir) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文件名解析：主文件、分片、以及**所有**不该被认的形态。
    ///
    /// 可证伪性：把 `bytes[4] != b'-'` 去掉 ⇒ 第一组「非法」用例里 `2026_10_23` 会变红。
    #[test]
    fn 文件名解析只认本模块格式() {
        assert_eq!(
            parse_log_name("runtime-2026-10-23.log"),
            Some(("2026-10-23".to_string(), 0))
        );
        assert_eq!(
            parse_log_name("runtime-2026-10-23.2.log"),
            Some(("2026-10-23".to_string(), 2))
        );
        // 非法：前缀不符 / 后缀不符 / 日期不是 YYYY-MM-DD / 分片不是数字 / 空
        assert_eq!(parse_log_name("runtime-2026_10_23.log"), None);
        assert_eq!(parse_log_name("runtime-2026-10-23.bak"), None);
        assert_eq!(parse_log_name("runtime-2026-10-23.log.gz"), None);
        assert_eq!(parse_log_name("runtime-2026-10-23.x.log"), None);
        assert_eq!(parse_log_name("runtime-26-10-23.log"), None);
        assert_eq!(parse_log_name("other-2026-10-23.log"), None);
        assert_eq!(parse_log_name("runtime-.log"), None);
    }

    /// 级别解析：合法值、别名、非法回落。
    #[test]
    fn 级别解析与阈值() {
        assert_eq!(Level::parse("ERROR"), Some(Level::Error));
        assert_eq!(Level::parse(" warn "), Some(Level::Warn));
        assert_eq!(Level::parse("warning"), Some(Level::Warn));
        assert_eq!(Level::parse("Info"), Some(Level::Info));
        assert_eq!(Level::parse("trace"), Some(Level::Debug));
        assert_eq!(Level::parse("verbose"), None);
        // 缺省 info：error/warn/info 落盘，debug 不落盘。
        assert_eq!(resolve_level(None), Level::Info);
        assert_eq!(resolve_level(Some("nope")), Level::Info);
        assert_eq!(resolve_level(Some("debug")), Level::Debug);
        assert!(
            Level::Debug > Level::Info && Level::Info > Level::Warn && Level::Warn > Level::Error
        );
    }

    /// 行解析：标准行、无 scope 行、坏行。
    #[test]
    fn 行解析() {
        let entry = parse_line("2026-10-23 12:00:00.123 ERROR [gateway] 启动失败: 端口被占")
            .expect("标准行应可解析");
        assert_eq!(entry.level, Level::Error);
        assert_eq!(entry.scope, "gateway");
        assert_eq!(entry.message, "启动失败: 端口被占");
        assert!(entry.ts > 0);

        // 无 scope（用户手写的行）：整段当 message，不丢行。
        let plain = parse_line("2026-10-23 12:00:00.123 INFO 裸消息").expect("应容错");
        assert_eq!(plain.scope, "");
        assert_eq!(plain.message, "裸消息");

        // 坏行：日期位数不对 / 级别不认识 / 太短
        assert!(parse_line("2026-1-3 12:00:00.123 INFO [x] y").is_none());
        assert!(parse_line("2026-10-23 12:00:00.123 LOUD [x] y").is_none());
        assert!(parse_line("garbage").is_none());
    }

    /// 脱敏：三类都要命中，且**不能**误伤合法字段。
    #[test]
    fn 脱敏命中凭据且不误伤() {
        // ① 参数形式
        let out = redact("url?access_token=abc123&region=cn");
        assert!(!out.contains("abc123"), "access_token 值必须抹掉：{out}");
        assert!(out.contains("region=cn"), "无关参数必须保留：{out}");
        assert!(out.contains(REDACTED));
        // ② Bearer
        let out = redact("Authorization: Bearer sk-live-xyz");
        assert!(!out.contains("sk-live-xyz"), "Bearer token 必须抹掉：{out}");
        // ③ JWT（纯 token 形态，没有参数名）
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let out = redact(&format!("token dump: {jwt}"));
        assert!(!out.contains("dozjgNryP4J3"), "JWT 必须抹掉：{out}");
        // 不误伤：`error_code=` 含 code 但不是凭据名；`key=pool` 同理。
        let keep = "failed error_code=auth_expired key=pool refresh=ok";
        assert_eq!(redact(keep), keep, "合法字段不得被抹：{keep}");
        // 前缀不构成命中（`my_token=` 不是 `token=`）
        let prefixed = "my_token=keepme";
        assert_eq!(redact(prefixed), prefixed);
        // 短 `eyJ` 开头但不是 JWT ⇒ 原样
        let short = "eyJabc";
        assert_eq!(redact(short), short);
    }

    /// 脱敏后仍保留 `name=` 的形状（排查要看「哪个参数漏了」）。
    #[test]
    fn 脱敏保留参数名() {
        let out = redact("refresh_token=SECRET&next=1");
        assert!(out.starts_with("refresh_token="), "参数名必须留下：{out}");
        assert!(out.contains("next=1"));
    }

    /// 分片与保留：写入超限后滚片；超天数被裁掉。
    ///
    /// 用临时 `BUDDY_SWITCH_HOME` 隔离，避免碰到真实 `~/.buddy-switch`；
    /// 改该变量必须走 [`config::HomeOverrideGuard`]（持锁 + drop 还原）——
    /// 裸 `set_var` 会与并行测试竞态，症状是自相矛盾的断言。
    #[test]
    fn 滚动与保留裁剪() {
        let temp = tempfile::tempdir().expect("临时目录");
        let _guard = config::HomeOverrideGuard::set(temp.path());
        let dir = logs_dir();
        std::fs::create_dir_all(&dir).expect("建日志目录");

        // 造两份「旧日期」+ 一份「今天」。
        std::fs::write(dir.join("runtime-2026-01-01.log"), "x\n").unwrap();
        std::fs::write(dir.join("runtime-2026-01-02.1.log"), "x\n").unwrap();
        std::fs::write(dir.join("runtime-2026-01-03.log"), "x\n").unwrap();
        // 干扰项：同目录下不属于本模块的文件，裁剪/清空都不得碰。
        std::fs::write(dir.join("keep-me.txt"), "important").unwrap();

        let removed = prune(config::now_ms());
        assert_eq!(removed, 3, "三份超期文件都应删除");
        assert!(!dir.join("runtime-2026-01-01.log").exists());
        assert!(dir.join("keep-me.txt").exists(), "非本模块文件不得删除");

        // clear 只删本模块文件
        std::fs::write(dir.join("runtime-2026-10-23.log"), "y\n").unwrap();
        let result = clear().expect("清空应成功");
        assert_eq!(result["removed"], json!(1));
        assert!(
            dir.join("keep-me.txt").exists(),
            "clear 不得删除非本模块文件"
        );
    }

    /// 端到端：写 → 读，级别 / 关键字 / limit 过滤都对。
    #[test]
    fn 写入后可读且过滤生效() {
        let temp = tempfile::tempdir().expect("临时目录");
        let _guard = config::HomeOverrideGuard::set(temp.path());
        let _ = clear();

        // 直接调 write_line 绕开级别阈值（阈值来自环境变量，测试不该依赖它）。
        let date = Local::now().format("%Y-%m-%d").to_string();
        write_line(
            &date,
            "2026-10-23 10:00:00.000 ERROR [gateway] 端口被占\n",
            "gateway",
        );
        write_line(
            &date,
            "2026-10-23 10:00:01.000 INFO  [switch] 切换成功\n",
            "switch",
        );
        write_line(
            &date,
            "2026-10-23 10:00:02.000 DEBUG [pool] 选号明细\n",
            "pool",
        );

        let all = read(&json!({}));
        assert_eq!(all["entries"].as_array().unwrap().len(), 3);
        // 最新在前
        assert_eq!(all["entries"][0]["level"], json!("debug"));

        let errors = read(&json!({ "level": "error" }));
        assert_eq!(errors["entries"].as_array().unwrap().len(), 1);
        assert_eq!(errors["entries"][0]["scope"], json!("gateway"));

        let hits = read(&json!({ "keyword": "切换" }));
        assert_eq!(hits["entries"].as_array().unwrap().len(), 1);
        assert_eq!(hits["entries"][0]["scope"], json!("switch"));

        let limited = read(&json!({ "limit": 2 }));
        assert_eq!(limited["entries"].as_array().unwrap().len(), 2);
        assert_eq!(limited["truncated"], json!(true));
        assert_eq!(limited["total"], json!(3));

        let _ = clear();
    }

    /// 写入必须**单行化**并**脱敏**（两条最容易在真实崩溃现场被破坏的性质）。
    ///
    /// ⚠️ 级别阈值来自进程级 `OnceLock`（其它测试可能已初始化），故这里不 `set_var`
    /// 调级别 —— 用 `error` 级别必然低于任何阈值，保证本行一定会落盘。
    #[test]
    fn 写入单行化且脱敏() {
        let temp = tempfile::tempdir().expect("临时目录");
        let _guard = config::HomeOverrideGuard::set(temp.path());
        let _ = clear();

        log(Level::Error, "test", "第一行\n第二行 access_token=leak-me");

        let text = std::fs::read_to_string(log_file_for(
            &Local::now().format("%Y-%m-%d").to_string(),
            0,
        ))
        .unwrap_or_default();
        assert!(!text.is_empty(), "error 级别必须落盘");
        assert_eq!(text.lines().count(), 1, "换行必须被单行化：{text}");
        assert!(!text.contains("leak-me"), "写盘前必须脱敏：{text}");
        assert!(text.contains(REDACTED), "应留下脱敏标记：{text}");

        let _ = clear();
    }

    /// 中文消息不能把脱敏器打崩（按字节切片会 panic —— 真实踩过）。
    #[test]
    fn 脱敏遇多字节字符不panic() {
        let samples = [
            "第一行\n第二行",
            "网关启动失败：端口 57891 被占用",
            "eyJ短",
            "中文 eyJ 混合 a.b.c 结束",
            "切换账号 成功 access_token=中文值 后续",
        ];
        for sample in samples {
            let out = redact(sample);
            assert!(!out.is_empty() || sample.is_empty());
        }
    }
}
