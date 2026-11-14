//! 外部 Agent hook 的**本地接收端** + **hook 命令**（P15 · S2）。
//!
//! ## 两件事，一个文件
//!
//! 1. [`serve`]：在 `127.0.0.1` 上收客户端 hook 的 POST，解析成 [`AgentEvent`]
//!    塞进 [`agent_watch::shared`]。
//! 2. [`try_run_hook_cli`]：**客户端 hook 调用的那条命令**的入口 —— 读 stdin、
//!    转发给上面的接收端、退出。
//!
//! 放一起是因为它们共用**同一个** [`HOOK_PORT`] 与**同一份** payload 契约：
//! 改了端口或字段名，两边必须同时改；分成两个文件只会让「改一半」更容易发生。
//!
//! ## ★ 零周期任务
//!
//! 接收端是**事件驱动**的常驻监听（`axum::serve` 等 accept），不是轮询循环 ——
//! 因此**不需要**在 `background.rs` 的 `TASKS` 注册表里登记任何东西
//! （`P15-SPIKE-RESULTS §2 Q2`）。
//!
//! ## ★ 安全基线（六条，逐条对齐外部参考实现的做法）
//!
//! | 条 | 做法 | 为什么 |
//! |:--|:---|:---|
//! | 1 | 只绑 **`127.0.0.1`** | 局域网内任何设备都能往里灌事件；本功能没有远程需求 |
//! | 2 | `Host` 必须是 `127.0.0.1` / `localhost` | **DNS rebinding**：网页可以把任意域名解析到 `127.0.0.1`，此时 `Origin` 是攻击者的域、但 `Host` 也是攻击者的域 ⇒ 这条能拦住它 |
//! | 3 | `Origin` 若存在，必须是 `http://<host>` | 浏览器发起的跨站请求一律拒绝（hook 客户端**不发** `Origin`） |
//! | 4 | 请求体上限 [`MAX_BODY_BYTES`] | 无上限的本地端口是内存放大器 |
//! | 5 | `Content-Type` 必须是 `application/json` | 让「浏览器表单 / 跨站简单请求」这类无预检的形态走不通 |
//! | 6 | **不落盘任何正文** | 会话标题就是用户的 prompt 前 240 字 —— 只进内存，进日志也不行 |
//!
//! 方法白名单（第 7 条）由路由本身保证：只注册 `POST`，其余方法由 axum 回 405。

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use serde_json::Value;
use tokio::net::TcpListener;

use super::agent_watch::{self, canonical_source, parse_event, AgentEvent, SOURCE_WORKBUDDY_CN};

/// 接收端监听的端口。
///
/// ⚠️ **登记在册**：本仓已占 `57890`（webui）/ `57891`（WB 网关）/ `57892`（统一网关）/
/// `57894`（Trae 网关）/ `57895`（Qoder 网关）/ `8899`（Trae MITM）—— 见
/// `docs/guides/GATEWAY-CLIENT-INTEGRATION.md` 附录 A。
/// 取 `57893` 是把 `5789x` 一段补齐后的唯一空位，改动此值必须**同步附录 A 与 [`HOOK_URL_NAME`]**。
pub const HOOK_PORT: u16 = 57893;

/// 接收路径。
const HOOK_PATH: &str = "/hook";

/// 请求体上限（64 KiB）。
///
/// 一条 hook payload 的正常大小在**几 KB** 以内（含 prompt 摘要）；
/// 64 KiB 给了充足余量，同时让「往这个端口灌大文件」立刻失败。
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// hook 命令连接接收端的超时（毫秒）。
///
/// ⚠️ 这个值必须**远小于**客户端的 hook 超时（外部参考实现用 3 s）——
/// 宿主没运行时我们要**尽快**失败退出，而不是拖住客户端的任务。
const CONNECT_TIMEOUT_MS: u64 = 500;
/// 读写超时（毫秒）。
const IO_TIMEOUT_MS: u64 = 1_000;

/// hook 目录里那个「接收端地址」文件名（与外部参考实现同款：hook 脚本读它拿到 base URL）。
///
/// 本仓的 hook 命令**不读它**（端口是编译期常量），保留这个名字只是为了让
/// 「两个实现装在同一台机器上」时能互相识别、不误判成对方装的东西。
pub const HOOK_URL_NAME: &str = "buddy-switch-monitor.url";

// ---------------------------------------------------------------------------
// 校验（纯逻辑，可单测）
// ---------------------------------------------------------------------------

/// 请求被拒的原因（每一条对应安全基线里的一条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// `Host` 缺失或不是回环地址（基线 2）。
    BadHost,
    /// `Origin` 存在且不是本机（基线 3）。
    BadOrigin,
    /// `Content-Type` 不是 JSON（基线 5）。
    BadContentType,
    /// 请求体超限（基线 4）。
    TooLarge,
}

impl Reject {
    /// 对外的 HTTP 状态码。
    pub fn status(self) -> StatusCode {
        match self {
            Reject::BadHost | Reject::BadOrigin => StatusCode::FORBIDDEN,
            Reject::BadContentType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Reject::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        }
    }
}

/// `Host` 头是否指向本机回环（基线 2）。
///
/// 只接受 `127.0.0.1` 与 `localhost`：本仓只绑 IPv4 回环，
/// 一个正常的 hook 客户端不可能送别的值。`[::1]` 也放行（有客户端会规范化成 IPv6 字面量）。
fn host_is_loopback(host: &str) -> bool {
    let host = host.trim();
    // IPv6 字面量：`[::1]` 或 `[::1]:port`
    if let Some(rest) = host.strip_prefix('[') {
        return rest.starts_with("::1]");
    }
    let hostname = host.split(':').next().unwrap_or_default();
    hostname == "127.0.0.1" || hostname.eq_ignore_ascii_case("localhost")
}

/// `Origin` 是否可接受（基线 3）。
///
/// **缺失即可接受**（hook 客户端不发 `Origin`）；
/// 存在时必须与 `Host` 同源 —— 这拦掉浏览器发起的跨站请求。
fn origin_allowed(origin: Option<&str>, host: &str) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let origin = origin.trim();
    if origin.is_empty() {
        return true;
    }
    origin.eq_ignore_ascii_case(&format!("http://{host}"))
        || origin.eq_ignore_ascii_case(&format!("https://{host}"))
}

/// `Content-Type` 是否是 JSON（基线 5）。
///
/// 只看 MIME 主体：客户端会带 `application/json; charset=utf-8`。
fn content_type_is_json(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let mime = content_type.split(';').next().unwrap_or_default().trim();
    mime.eq_ignore_ascii_case("application/json")
}

/// 四条请求校验的**唯一**收敛点（基线 2–5）。
///
/// 抽成纯函数是为了让「哪一条该回什么码」可以被单测钉死 —— 安全校验最容易在
/// 「加一条新规则」时被顺手改坏，而它的症状是**静默放开**（不报错、只是不再拦）。
pub fn validate(
    host: Option<&str>,
    origin: Option<&str>,
    content_type: Option<&str>,
    body_len: usize,
) -> Result<(), Reject> {
    let Some(host) = host else {
        return Err(Reject::BadHost);
    };
    if !host_is_loopback(host) {
        return Err(Reject::BadHost);
    }
    if !origin_allowed(origin, host.trim()) {
        return Err(Reject::BadOrigin);
    }
    if !content_type_is_json(content_type) {
        return Err(Reject::BadContentType);
    }
    if body_len > MAX_BODY_BYTES {
        return Err(Reject::TooLarge);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 接收端
// ---------------------------------------------------------------------------

/// `POST /hook` 的 handler。
///
/// 响应码约定（**客户端据此决定要不要重试**）：
/// - `204` —— 收下了（**包括「payload 里没有可用的会话标识」**：那不是错误，
///   回非 2xx 只会让客户端反复重试一条永远解析不出东西的事件）；
/// - `403` / `413` / `415` —— 见 [`Reject`]。
async fn hook_handler(headers: HeaderMap, body: Bytes) -> StatusCode {
    let header_str = |name: header::HeaderName| -> Option<&str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    };
    if let Err(reject) = validate(
        header_str(header::HOST),
        header_str(header::ORIGIN),
        header_str(header::CONTENT_TYPE),
        body.len(),
    ) {
        return reject.status();
    }

    // 解析失败**不是**错误：客户端重试没有意义，只有 2xx 能止住它。
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::NO_CONTENT;
    };
    let now = crate::modules::config::now_ms();
    if let Some(event) = parse_event(SOURCE_WORKBUDDY_CN, &payload, now) {
        ingest(event);
    }
    StatusCode::NO_CONTENT
}

/// 把一条事件塞进共享会话表。
///
/// 锁中毒（panic 在持锁期间）**不**向上传播：会话表是纯内存缓存，
/// 拿着中毒的锁继续用不会损坏任何持久化数据，而让 handler panic
/// 会把「客户端 hook 收到 500」变成用户可见的故障。
fn ingest(event: AgentEvent) {
    let mut watch = agent_watch::shared()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    watch.ingest(event);
}

/// 接收端的路由（只注册 `POST` —— 这就是方法白名单那条）。
pub fn router() -> Router {
    Router::new()
        .route(HOOK_PATH, post(hook_handler))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

/// 接收端地址。
pub fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], HOOK_PORT))
}

/// 起接收端（**永不返回**，由调用方 spawn；绑定失败只返回 `Err`，不 panic）。
///
/// 调用方应当在**已有 tokio 运行时**里 spawn 它 —— 本函数不自己起 runtime。
pub async fn serve() -> Result<(), String> {
    let addr = addr();
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|error| format!("hook 接收端无法监听 {addr}：{error}"))?;
    axum::serve(listener, router())
        .await
        .map_err(|error| format!("hook 接收端异常退出：{error}"))
}

// ---------------------------------------------------------------------------
// hook 命令（客户端 hook 调用的入口）
// ---------------------------------------------------------------------------

/// 如果 `args` 是本工具的 `hook` 子命令，执行它并返回退出码；否则返回 `None`。
///
/// ⚠️ **必须在宿主初始化之前调用**（`main` 的第一件事）：hook 是客户端在**每次工具调用**
/// 时同步执行的命令，让 GUI 初始化吃掉几百毫秒既无必要，也会撞上客户端的 3 s 超时。
pub fn try_run_hook_cli(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("hook") {
        return None;
    }
    Some(run_hook_cli(source_from_args(args)))
}

/// 从 hook 命令的参数里解析来源 id —— **纯函数**，白名单行为由单测钉死。
///
/// ## ★ 白名单 = [`agent_watch::HOOK_SOURCES`]（唯一事实来源）
///
/// 认不出的取值**一律回落国内版**：不能把用户传的任意串当成来源，
/// 那会让会话表里出现一个谁都不认识的 `source`，窗口里显示成裸 id。
///
/// ⚠️ **早先这里是硬编码的 `match`（只有 `codex` / `workbuddy-global` 两条）** ——
/// 新客户端（trae / qoder / claude / codebuddy）会被**静默**回落成 `workbuddy-cn`，
/// 于是「Trae 里正在跑的会话」在悬浮窗里显示成「WorkBuddy 国内版」。
/// 这类错标没有任何报错，只能靠注册表 + 单测防住（见
/// [`hook_sources_are_all_accepted_by_source_arg`]）。
pub fn source_from_args(args: &[String]) -> &'static str {
    let mut source = SOURCE_WORKBUDDY_CN;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                if let Some(value) = args.get(index + 1) {
                    source = canonical_source(value).unwrap_or(SOURCE_WORKBUDDY_CN);
                }
                index += 2;
            }
            _ => index += 1,
        }
    }
    source
}

/// hook 命令本体：stdin → 接收端。**恒返回 0**。
///
/// ## 为什么恒返回 0
///
/// 客户端把 hook 的非零退出码当成「这个 hook 失败了」，可能重试、可能中断它自己的流程。
/// 本命令是**旁路观测**：宿主没运行、payload 不合法、接收端没起来 —— 都不该影响
/// 用户在客户端里的正常工作。所有失败路径静默吞掉。
fn run_hook_cli(source: &str) -> i32 {
    // ★ 客户端会把 hook 的 stdout 当 JSON 解析 ⇒ 先吐一个空对象。
    // ★ 用 `let _ =` 而不是 `println!`：Windows 桌面版是 **GUI 子系统**
    //   （`windows_subsystem = "windows"`），stdout 可能是无效句柄，
    //   `println!` 在写入失败时会 panic —— 那会让 hook 以非零码退出。
    let _ = writeln!(io::stdout(), "{{}}");

    let mut raw = String::new();
    if io::stdin().read_to_string(&mut raw).is_err() {
        return 0;
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return 0;
    }
    let Ok(mut payload) = serde_json::from_str::<Value>(raw) else {
        return 0;
    };
    // 注入来源；payload 自带 `agent_source` 时不覆盖（一个接收端服务多个客户端）。
    if let Value::Object(map) = &mut payload {
        map.entry("agent_source".to_string())
            .or_insert_with(|| Value::String(source.to_string()));
    }
    let Ok(body) = serde_json::to_string(&payload) else {
        return 0;
    };
    let _ = post_to_receiver(&body);
    0
}

/// 把 payload POST 给接收端。
///
/// ## 为什么手写 HTTP 而不是拉一个客户端库
///
/// 只有一个固定目标（`127.0.0.1:<常量端口>`）、一个固定方法、一个固定头集合。
/// 手写 20 行换掉「引入 HTTP 客户端 + 起 tokio runtime」这两件事 ——
/// hook 命令跑在**每次工具调用**的路径上，启动开销直接进用户的等待时间。
fn post_to_receiver(body: &str) -> io::Result<()> {
    let addr = addr();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(CONNECT_TIMEOUT_MS))?;
    stream.set_write_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS)))?;
    stream.set_read_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS)))?;
    let request = format!(
        "POST {HOOK_PATH} HTTP/1.1\r\n\
         Host: 127.0.0.1:{HOOK_PORT}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\r\n{body}",
        len = body.len(),
    );
    stream.write_all(request.as_bytes())?;
    // 读一口响应就够 —— 我们只关心「服务端把连接处理完了」，不解析状态码
    // （解析它也没用：无论 204 还是 403，本命令都返回 0）。
    let mut sink = [0u8; 64];
    let _ = stream.read(&mut sink);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ 回环判定：只认本机名。**这条是 DNS rebinding 的唯一防线**。
    ///
    /// 可证伪性：把 `host_is_loopback` 改成恒 `true` ⇒ 前四条断言变红。
    #[test]
    fn host_must_be_loopback() {
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("127.0.0.1:57893"));
        assert!(host_is_loopback("localhost:57893"));
        assert!(host_is_loopback("LOCALHOST"));
        assert!(host_is_loopback("[::1]:57893"));

        assert!(!host_is_loopback("evil.example.com"));
        assert!(!host_is_loopback("evil.example.com:57893"));
        assert!(!host_is_loopback("127.0.0.1.evil.com"));
        assert!(!host_is_loopback(""));
    }

    /// ★ 缺 `Origin` 可以（hook 客户端不发）；有 `Origin` 必须同源。
    #[test]
    fn origin_must_match_host_when_present() {
        assert!(origin_allowed(None, "127.0.0.1:57893"));
        assert!(origin_allowed(Some(""), "127.0.0.1:57893"));
        assert!(origin_allowed(
            Some("http://127.0.0.1:57893"),
            "127.0.0.1:57893"
        ));
        assert!(!origin_allowed(
            Some("http://evil.example.com"),
            "127.0.0.1:57893"
        ));
    }

    /// `Content-Type` 只看 MIME 主体（客户端会带 `charset`）。
    #[test]
    fn content_type_must_be_json() {
        assert!(content_type_is_json(Some("application/json")));
        assert!(content_type_is_json(Some(
            "application/json; charset=utf-8"
        )));
        assert!(!content_type_is_json(Some("text/plain")));
        assert!(!content_type_is_json(Some(
            "application/x-www-form-urlencoded"
        )));
        assert!(!content_type_is_json(None));
    }

    /// ★ 四条校验各回各的码，且**顺序**是 host → origin → content-type → size。
    #[test]
    fn validate_reports_the_right_reject() {
        let ok = ("127.0.0.1:57893", None, "application/json", 128);
        assert_eq!(validate(Some(ok.0), ok.1, Some(ok.2), ok.3), Ok(()));

        assert_eq!(
            validate(None, None, Some("application/json"), 1),
            Err(Reject::BadHost)
        );
        assert_eq!(
            validate(Some("evil.com"), None, Some("application/json"), 1),
            Err(Reject::BadHost)
        );
        assert_eq!(
            validate(
                Some("127.0.0.1:1"),
                Some("http://evil.com"),
                Some("application/json"),
                1
            ),
            Err(Reject::BadOrigin)
        );
        assert_eq!(
            validate(Some("127.0.0.1:1"), None, Some("text/plain"), 1),
            Err(Reject::BadContentType)
        );
        assert_eq!(
            validate(
                Some("127.0.0.1:1"),
                None,
                Some("application/json"),
                MAX_BODY_BYTES + 1
            ),
            Err(Reject::TooLarge)
        );
        // 边界：正好等于上限**放行**（`>` 而不是 `>=`）
        assert_eq!(
            validate(
                Some("127.0.0.1:1"),
                None,
                Some("application/json"),
                MAX_BODY_BYTES
            ),
            Ok(())
        );
    }

    /// 拒绝原因 → HTTP 状态码（客户端据此区分「配置错了」与「太大了」）。
    #[test]
    fn reject_maps_to_status_codes() {
        assert_eq!(Reject::BadHost.status(), StatusCode::FORBIDDEN);
        assert_eq!(Reject::BadOrigin.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            Reject::BadContentType.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(Reject::TooLarge.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// 非 `hook` 子命令 ⇒ `None`（宿主必须照常启动 GUI）。
    ///
    /// 可证伪性：把 `args.get(1) != "hook"` 的判断删掉 ⇒ `None` 那两条变红
    /// （桌面版会在**每一次正常启动**时被当成 hook 命令吃掉）。
    #[test]
    fn non_hook_argv_is_not_claimed() {
        assert_eq!(try_run_hook_cli(&[]), None);
        assert_eq!(try_run_hook_cli(&["app.exe".to_string()]), None);
        assert_eq!(
            try_run_hook_cli(&["app.exe".to_string(), "serve".to_string()]),
            None
        );
        assert_eq!(
            try_run_hook_cli(&["app.exe".to_string(), "--version".to_string()]),
            None
        );
    }

    /// 接收端只绑回环地址（基线 1）。
    ///
    /// 可证伪性：把 [`addr`] 改成 `0.0.0.0` ⇒ 变红。
    #[test]
    fn receiver_binds_loopback_only() {
        let addr = addr();
        assert!(
            addr.ip().is_loopback(),
            "接收端绝不能绑到非回环地址：{addr}"
        );
        assert_eq!(addr.port(), HOOK_PORT);
    }

    /// hook 命令参数 → 来源 id（`["app.exe", "hook", "--source", <id>]`）。
    fn hook_args(source: Option<&str>) -> Vec<String> {
        let mut args = vec!["app.exe".to_string(), "hook".to_string()];
        if let Some(source) = source {
            args.push("--source".to_string());
            args.push(source.to_string());
        }
        args
    }

    /// ★★ **注册表里的每个来源都必须被 `--source` 认出来**。
    ///
    /// 这是本文件最重要的一条断言：早先 `--source` 是硬编码 `match`（只有两条），
    /// 新客户端会被**静默**回落成 `workbuddy-cn` —— 悬浮窗里显示成别人的会话。
    ///
    /// 可证伪性：把 [`source_from_args`] 换回「只认 `codex` / `workbuddy-global` 的 match」
    /// ⇒ 下面那个遍历断言立刻变红，并逐字报出是哪个来源没被认出来。
    #[test]
    fn hook_sources_are_all_accepted_by_source_arg() {
        for id in agent_watch::HOOK_SOURCES {
            assert_eq!(
                source_from_args(&hook_args(Some(id))),
                *id,
                "来源 {id} 没有被 --source 认出来 —— 它会被静默错标成 {SOURCE_WORKBUDDY_CN}"
            );
        }
    }

    /// 缺 `--source` ⇒ 回落国内版（保持既有行为，不引入新的默认值）。
    #[test]
    fn missing_source_arg_falls_back_to_workbuddy_cn() {
        assert_eq!(source_from_args(&hook_args(None)), SOURCE_WORKBUDDY_CN);
    }

    /// ★ 认不出的取值**回落**而不是原样透传（会话表里不得出现野生 source）。
    ///
    /// 可证伪性：把 `canonical_source(value).unwrap_or(...)` 改成 `value`（原样透传）
    /// ⇒ 前两条断言变红。
    #[test]
    fn unknown_source_arg_falls_back_instead_of_passing_through() {
        for raw in ["evil", "", "   ", "codex-desktop", "trae-cn-x", "wb-cn-x"] {
            assert_eq!(
                source_from_args(&hook_args(Some(raw))),
                SOURCE_WORKBUDDY_CN,
                "{raw:?} 不得被当成来源原样透传（前缀形态尤其不行）"
            );
        }
    }

    /// 无关参数不干扰解析（客户端可能给命令加别的开关）。
    #[test]
    fn unrelated_args_do_not_break_source_parsing() {
        let args = vec![
            "app.exe".to_string(),
            "hook".to_string(),
            "--verbose".to_string(),
            "--source".to_string(),
            "trae-global".to_string(),
            "--extra".to_string(),
        ];
        assert_eq!(source_from_args(&args), agent_watch::SOURCE_TRAE_GLOBAL);
    }

    /// `--source` 是**最后一个**参数（缺值）⇒ 不得 panic，回落国内版。
    #[test]
    fn dangling_source_flag_is_harmless() {
        let args = vec![
            "app.exe".to_string(),
            "hook".to_string(),
            "--source".to_string(),
        ];
        assert_eq!(source_from_args(&args), SOURCE_WORKBUDDY_CN);
    }
}
