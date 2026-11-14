//! 对话转发抓取（诊断旁路）。
//!
//! ## 用途
//!
//! 每次出站尝试落盘一组文件（目录 `~/.buddy-switch/trae/capture/<程序位>/`）：
//!
//! | 后缀 | 内容 | 何时写入 |
//! |:---|:---|:---|
//! | `_entry.json` | 入口请求体（见下方「入口体的确切含义」） | 出站前 |
//! | `_upstream.json` | 改写后发往上游的 `llm_utils_chat` 请求体 | 出站前 |
//! | `_upstream_resp.ndjson` | 上游返回的**原始 SSE 流**（逐块追加） | 流式消费时 |
//! | `_upstream_http_error.txt` | 上游非 2xx 的错误响应体 | 连接期失败时 |
//!
//! 编号（文件名前缀）是「一次出站尝试」的关联键：换号重试时每次尝试各自
//! 一组文件，同一次尝试的请求与响应流靠编号配对。
//!
//! ## 入口体的确切含义（别当成「客户端原文」）
//!
//! `_entry.json` 是**进入中继那一层**的请求体：
//!
//! - `/v1/chat/completions`：就是客户端发来的原文；
//! - `/v1/messages`：是 Anthropic → OpenAI 转换**之后**、改写**之前**的形态
//!   （转换在 [`super::routes::messages_for_variant`] 里完成，中继只见到转换结果）。
//!   想看真正的 Anthropic 原文，得在入口端点另抓 —— 本旁路不做那件事。
//!
//! 这条区别写在这里而不是留给人猜：把它当成「客户端原文」去比对形状，
//! 会得出「客户端发的是 OpenAI 协议」这种错误结论。
//!
//! ## 目录与生命周期
//!
//! 按**程序位**分目录（`capture/<变体>/`）：网关同时服务多条产品线，
//! 混在一个目录里排查时无法区分是哪条线的流量。
//!
//! 目录在 [`Capture::new`] 里解析并创建**一次**，之后每次落盘只是纯 IO ——
//! 不是每块 SSE 都 `create_dir_all`（长流会变成每块一次 mkdir 系统调用）。
//!
//! ⚠️ **无轮转、无上限**：抓取默认关闭（[`super::TraeGatewayConfig::capture_enabled`]），
//! 只在人工排查时打开，排查完请**手工清理整个目录**。
//!
//! ## 为什么失败也静默
//!
//! 抓取是诊断旁路：落盘失败（磁盘满 / 权限 / 目录被删）绝不能影响转发主链路，
//! 顶多少一份诊断材料。因此所有 IO 错误一律忽略，不记日志、不上抛。

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use buddy_switch_core::modules::trae::paths;
use buddy_switch_core::modules::trae::variant::TraeVariant;

/// 尝试序号（进程内自增，避免同毫秒内编号碰撞）。
static SEQ: AtomicU64 = AtomicU64::new(0);

/// 测试专用抓取根目录覆盖：`Some(path)` 时全部抓取落到该目录。
///
/// 为什么不用环境变量：并发测试对 home 的覆盖是**进程级**环境变量，
/// 多线程下互相污染（A 线程写入 B 线程的临时目录）——真实发生过
/// （apikey 系测试把 home 指到各自 `Temp\trae-apikey-*`）。
/// 模块内覆盖与 home 解析彻底解耦，产品代码永不设置，因此零行为影响。
static TEST_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// 抓取根目录：`~/.buddy-switch/trae/capture/`（**不创建**，由调用方按需创建）。
///
/// 复用 Trae 数据目录机制（[`paths::trae_dir`]）而不是相对工作目录的 `temp/`：
/// 网关进程的工作目录不确定（Tauri 桌面端 / webui / 测试各不相同），
/// 只有数据目录的解析是稳定可预期的。
fn capture_root() -> PathBuf {
    TEST_ROOT
        .read()
        .unwrap()
        .clone()
        .unwrap_or_else(|| paths::trae_dir().join("capture"))
}

/// 仅供测试：设置/清除抓取根目录覆盖（`None` 恢复默认数据目录）。
#[cfg(test)]
pub fn set_test_root(dir: Option<PathBuf>) {
    *TEST_ROOT.write().unwrap() = dir;
}

/// 一次出站尝试的抓取句柄：目录在构造时解析并创建一次，编号同时是文件名前缀。
#[derive(Debug, Clone)]
pub struct Capture {
    dir: PathBuf,
    id: String,
}

impl Capture {
    /// 为一次出站尝试建立抓取句柄：解析并创建 `capture/<程序位>/`，生成编号。
    ///
    /// 目录创建失败不报错（写盘时自然失败并静默），与「旁路不影响主链路」一致。
    pub fn new(variant: TraeVariant) -> Self {
        let dir = capture_root().join(variant.as_str());
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            id: new_capture_id(),
        }
    }

    /// 本次尝试的抓取编号（文件名前缀 / 请求与响应的配对键）。
    pub fn id(&self) -> &str {
        &self.id
    }

    fn path(&self, suffix: &str) -> PathBuf {
        self.dir.join(format!("{}{suffix}", self.id))
    }

    /// 整文件覆盖写（用于请求体 / 错误体）。失败静默。
    pub fn write(&self, suffix: &str, content: &[u8]) {
        let _ = std::fs::write(self.path(suffix), content);
    }

    /// 追加写（用于响应流逐块落盘）。失败静默、不重试。
    pub fn append(&self, suffix: &str, content: &[u8]) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path(suffix))
        {
            let _ = file.write_all(content);
        }
    }

    /// 抓一次出站尝试的**请求对**：入口体 + 改写后上游体。
    ///
    /// 调用点在出站前一刻（改写已完成、尚未发出），两者形状可比对。
    pub fn request_pair(&self, entry_body: &[u8], upstream_body: &[u8]) {
        self.write("_entry.json", entry_body);
        self.write("_upstream.json", upstream_body);
    }
}

/// 生成本次出站尝试的抓取编号：`<yyyyMMdd_HHmmss>_<毫秒>_<序号3位>`。
///
/// 编号同时是文件名前缀与请求/响应的配对键；序号取模 1000 只用于
/// 同毫秒去重，不承担别的语义。
fn new_capture_id() -> String {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) % 1000;
    let now = chrono::Local::now();
    format!(
        "{}_{:03}_{:03}",
        now.format("%Y%m%d_%H%M%S"),
        now.timestamp_subsec_millis(),
        seq
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 抓取编号必须形如 `<日期8位>_<时间6位>_<毫秒3位>_<序号3位>`，且同进程内不重复。
    #[test]
    fn capture_id_shape_and_uniqueness() {
        let first = new_capture_id();
        let second = new_capture_id();
        // 形态：日期_时间_毫秒_序号（下划线分隔，全数字段）。
        for id in [&first, &second] {
            let parts: Vec<&str> = id.split('_').collect();
            assert_eq!(parts.len(), 4, "编号应为四段：{id}");
            assert_eq!(parts[0].len(), 8, "日期段应为 8 位：{id}");
            assert_eq!(parts[1].len(), 6, "时间段应为 6 位：{id}");
            assert_eq!(parts[2].len(), 3, "毫秒段应为 3 位：{id}");
            assert_eq!(parts[3].len(), 3, "序号段应为 3 位：{id}");
            assert!(id.chars().all(|c| c.is_ascii_digit() || c == '_'));
        }
        assert_ne!(first, second, "同进程内编号不得重复");
    }

    /// 写入 + 追加 + 请求对：文件名按编号落盘且内容逐字一致；
    /// 目录按**程序位**分家（两条产品线的抓取不混）。
    ///
    /// 用模块内覆盖重定向到独立临时目录：并发测试对 home 的覆盖是进程级
    /// 环境变量，多线程互相污染（写进 A 线程的临时 home、读时变成 B 的），
    /// 模块内覆盖与 home 彻底解耦，可并发安全地反复运行。
    #[test]
    fn write_append_pair_roundtrip() {
        let isolated = std::env::temp_dir().join(format!(
            "capture-roundtrip-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        set_test_root(Some(isolated.clone()));
        let capture = Capture::new(TraeVariant::TraeWork);
        let id = capture.id().to_string();
        capture.request_pair(b"{\"entry\":1}", b"{\"upstream\":2}");
        capture.append("_upstream_resp.ndjson", b"event:a\n\n");
        capture.append("_upstream_resp.ndjson", b"event:b\n\n");

        let read = |suffix: &str| {
            let path = capture.path(suffix);
            std::fs::read(&path).unwrap_or_else(|error| {
                panic!(
                    "读取 {path:?} 失败：{error}；目录内容：{:?}",
                    std::fs::read_dir(capture.dir.clone())
                        .map(|entries| {
                            entries
                                .filter_map(|entry| entry.ok())
                                .map(|entry| entry.file_name().to_string_lossy().to_string())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                )
            })
        };
        assert_eq!(read("_entry.json"), b"{\"entry\":1}");
        assert_eq!(read("_upstream.json"), b"{\"upstream\":2}");
        assert_eq!(read("_upstream_resp.ndjson"), b"event:a\n\nevent:b\n\n");

        // 编号必须是文件名前缀（配对键）。
        assert!(
            capture
                .path("_upstream.json")
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&id),
            "文件名必须由编号打头"
        );

        // 程序位分目录：同一根目录下另一条产品线落在**不同**子目录。
        let other = Capture::new(TraeVariant::Trae);
        assert_ne!(capture.dir, other.dir, "不同程序位的抓取目录必须分开");
        assert_eq!(
            other.dir.parent(),
            Some(isolated.as_path()),
            "抓取目录应为 <根目录>/<程序位>"
        );
        assert_eq!(
            other.dir.file_name().unwrap().to_string_lossy(),
            TraeVariant::Trae.as_str(),
            "子目录名必须是程序位标识"
        );

        // 覆盖恢复 + 测试产物自清理（临时目录整个移除，不留垃圾）。
        set_test_root(None);
        let _ = std::fs::remove_dir_all(&isolated);
    }
}
