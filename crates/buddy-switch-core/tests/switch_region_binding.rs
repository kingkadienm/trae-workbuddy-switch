//! `switch_account` region 绑定的**可证伪**集成测试。
//!
//! ## 为什么需要它
//!
//! `switch.rs` 里旧签名 `switch_account()` 是 `switch_account_for(Region::Cn, …)`
//! 的薄包装。底层 `switch_account_for` 已被充分覆盖，但「CN 包装是否真的把
//! `Region::Cn` 传了下去」这个绑定本身是覆盖盲区：要证明它，需要 CN / Global
//! 两套**可观测的**不同账号库，而一旦账号能被查到，就会写**真实的**认证文件。
//!
//! ## 隔离方式
//!
//! 全部 fixture 写入一个临时目录，并把进程级环境变量 `BUDDY_SWITCH_HOME` 指向它
//! （`config::home_dir()` 的覆盖缝）。测试结束时：
//! - 断言**真实**用户目录 `~/.buddy-switch/` 与真实认证文件仍与开始时完全一致；
//! - 恢复 `BUDDY_SWITCH_HOME` 并清理临时目录。
//!
//! ## 可证伪性
//!
//! 环境变量是进程级全局状态，Rust 同二进制内测试并行执行会 race，故用
//! [`ENV_LOCK`] 串行化所有会修改它的测试，并在结束时恢复原值，避免污染他人。
//!
//! 若把生产代码里的 `Region::Cn` 写成 `Region::Global`：
//! - `cn_wrapper_binds_cn_region_and_leaves_global_untouched` 的断言 (a) 会因
//!   `cn-only` 在 Global 库里查不到而 `unwrap_err` panic；
//! - 断言 (b) 会因 `global-only` 被错误找到而成功，`unwrap_err` panic。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use buddy_switch_core::modules::config::BUDDY_SWITCH_HOME_ENV;
use buddy_switch_core::modules::export_import;
use buddy_switch_core::modules::region::{self, Region, RegionFilter};
use buddy_switch_core::modules::{account, auth_file, config, credit_usage, switch};

/// 串行化所有会修改 `BUDDY_SWITCH_HOME` 的测试（该变量是进程级全局状态）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

const CN_ACCOUNTS: &str = r#"[
  {
    "id": "cn-only",
    "uid": "uid-cn",
    "nickname": "CN User",
    "email": "cn@example.com",
    "access_token": "CN_TOKEN",
    "refresh_token": "CN_REFRESH",
    "token_type": "Bearer",
    "domain": "www.codebuddy.cn"
  }
]"#;

const GLOBAL_ACCOUNTS: &str = r#"[
  {
    "id": "global-only",
    "uid": "uid-global",
    "nickname": "Global User",
    "email": "global@example.com",
    "access_token": "GLOBAL_TOKEN",
    "refresh_token": "GLOBAL_REFRESH",
    "token_type": "Bearer",
    "domain": "www.workbuddy.ai"
  }
]"#;

const CN_AUTH_BEFORE: &str = r#"{"account":{"uid":"preexisting-cn"},"auth":{"accessToken":"CN_PRE","domain":"www.codebuddy.cn"},"marker":"CN_BEFORE"}"#;

const GLOBAL_AUTH_BEFORE: &str = r#"{"account":{"uid":"preexisting-global"},"auth":{"accessToken":"GLOBAL_PRE","domain":"www.workbuddy.ai"},"marker":"GLOBAL_BEFORE"}"#;

/// 认证文件相对 home 的平台结构（与 `auth_file::auth_candidates_for` 独立编码，
/// 从而不依赖被测函数自身，保证断言可证伪）。
fn auth_rel(filename: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    let dir = "AppData/Local/CodeBuddyExtension/Data/Public/auth";
    #[cfg(target_os = "macos")]
    let dir = "Library/Application Support/CodeBuddyExtension/Data/Public/auth";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let dir = ".local/share/CodeBuddyExtension/Data/Public/auth";

    Path::new(dir).join(filename)
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("buddy-switch-{label}-{}-{nanos}", std::process::id()))
}

/// 隔离 home 守卫：设置 `BUDDY_SWITCH_HOME`、持有 [`ENV_LOCK`]，Drop 时恢复环境变量
/// 并删除临时目录。字段按声明顺序析构，`_lock` 在 `drop` 体之后释放，故恢复环境
/// 变量期间仍持锁。
struct IsolatedHome {
    _lock: MutexGuard<'static, ()>,
    previous: Option<String>,
    dir: PathBuf,
}

impl IsolatedHome {
    fn new(label: &str) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var(BUDDY_SWITCH_HOME_ENV).ok();
        let dir = unique_temp_dir(label);
        fs::create_dir_all(&dir).expect("create isolated home");
        std::env::set_var(BUDDY_SWITCH_HOME_ENV, &dir);
        Self {
            _lock: lock,
            previous,
            dir,
        }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for IsolatedHome {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(BUDDY_SWITCH_HOME_ENV, value),
            None => std::env::remove_var(BUDDY_SWITCH_HOME_ENV),
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// 写入 CN / Global 两套账号库与两份认证文件。
fn write_fixtures(home: &Path) {
    let store = home.join(".buddy-switch");
    fs::create_dir_all(&store).expect("create store dir");
    fs::write(store.join("accounts.json"), CN_ACCOUNTS).expect("write CN accounts");
    fs::write(store.join("accounts.global.json"), GLOBAL_ACCOUNTS).expect("write Global accounts");

    for (filename, content) in [
        ("workbuddy-desktop.info", CN_AUTH_BEFORE),
        ("workbuddy-desktop-ai.info", GLOBAL_AUTH_BEFORE),
    ] {
        let path = home.join(auth_rel(filename));
        fs::create_dir_all(path.parent().expect("auth parent")).expect("create auth dir");
        fs::write(path, content).expect("write auth file");
    }
}

/// 真实用户目录关键路径的快照，用于证明隔离测试未触碰真实数据。
#[derive(Debug, PartialEq, Eq)]
struct RealHomeSnapshot {
    store_dir_exists: bool,
    accounts: Option<Vec<u8>>,
    accounts_global: Option<Vec<u8>>,
    cn_auth: Option<Vec<u8>>,
    global_auth: Option<Vec<u8>>,
}

fn file_bytes(path: &Path) -> Option<Vec<u8>> {
    if path.is_file() {
        fs::read(path).ok()
    } else {
        None
    }
}

fn real_home_snapshot() -> RealHomeSnapshot {
    let home = dirs::home_dir().expect("real home dir");
    let store = home.join(".buddy-switch");
    RealHomeSnapshot {
        store_dir_exists: store.is_dir(),
        accounts: file_bytes(&store.join("accounts.json")),
        accounts_global: file_bytes(&store.join("accounts.global.json")),
        cn_auth: file_bytes(&home.join(auth_rel("workbuddy-desktop.info"))),
        global_auth: file_bytes(&home.join(auth_rel("workbuddy-desktop-ai.info"))),
    }
}

// ---------------------------------------------------------------------------
// 统计 Region 化：region=all 契约 + 按账号归属过滤（可证伪）
// ---------------------------------------------------------------------------

/// 写入 CN / Global 两套账号库与两版积分快照（按账号 id 归档的全局存储）。
///
/// 快照文件路径与 core 的 `credit_usage_snapshots_file()` 对齐：`<home>/.buddy-switch/` 下的
/// `credit_usage_snapshots.json`（由 core 常量决定，此处独立复制以免依赖被测函数自身）。
fn write_credit_fixtures(home: &Path) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let store = home.join(".buddy-switch");
    fs::create_dir_all(&store).expect("create store dir");
    fs::write(store.join("accounts.json"), CN_ACCOUNTS).expect("write CN accounts");
    fs::write(store.join("accounts.global.json"), GLOBAL_ACCOUNTS).expect("write Global accounts");

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(1_000_000);
    let snapshots = format!(
        r#"[
  {{ "ts": {}, "accountId": "cn-only", "accountName": "cn@example.com", "total": 100.0, "remaining": 70.0 }},
  {{ "ts": {}, "accountId": "global-only", "accountName": "global@example.com", "total": 200.0, "remaining": 150.0 }},
  {{ "ts": {}, "accountId": "cn-only", "accountName": "cn@example.com", "total": 100.0, "remaining": 40.0 }},
  {{ "ts": {}, "accountId": "global-only", "accountName": "global@example.com", "total": 200.0, "remaining": 100.0 }}
]"#,
        now - 120_000,
        now - 120_000,
        now - 60_000,
        now - 60_000,
    );
    fs::write(store.join("credit_usage_snapshots.json"), snapshots).expect("write snapshots");
}

#[test]
fn credit_stats_split_view_filters_by_account_ownership() {
    let home = IsolatedHome::new("credit-region-split");

    assert_eq!(config::home_dir(), home.path().to_path_buf());
    write_credit_fixtures(home.path());

    // global 视图：accounts[].accountId 必须全部属于 global 账号库。
    let global = futures_block_on(credit_usage::get_statistics_for_filter(
        RegionFilter::Global,
        false,
    ));
    assert_eq!(global["region"], "global");
    let accounts = global["accounts"].as_array().expect("accounts");
    assert!(!accounts.is_empty(), "global 视图应含 global-only 账号");
    for account in accounts {
        assert_eq!(
            account["accountId"], "global-only",
            "global 视图不得串入 cn 账号: {account}"
        );
        assert_eq!(account["region"], "global", "region 徽标必须为 global");
    }

    // ⚠️ 顺序是**故意的**，别把 global 视图那段挪到后面或与之合并：上面那次 global
    // 查询会触发官方用量采集，而采集链路在 token 陈旧时会按 region 刷新并把账号
    // **写回账号库**。曾经 `official_usage` 固定走 CN 的 `authenticated_post`，于是
    // 这次 global 查询会把 `global-only`（连 `needs_relogin` 标记）写进 CN 的
    // `accounts.json`，下面的 cn 视图随即串入 global 账号。现在采集链路带 region，
    // 写回的是 `accounts.global.json`。
    //
    // 先钉住**存储层**（比投影层更贴近根因）：CN 账号库文件内容必须与夹具逐字节一致。
    // 即使将来有人改坏 `build_statistics` 的过滤逻辑、让下面那圈投影层断言失效，
    // 这一条仍会报警。
    let cn_store = home.path().join(".buddy-switch/accounts.json");
    assert_eq!(
        fs::read_to_string(&cn_store).expect("read cn accounts"),
        CN_ACCOUNTS,
        "CN 账号库被写脏：global 账号被写进了 accounts.json"
    );

    // cn 视图：accounts[].accountId 必须全部属于 cn 账号库。
    let cn = futures_block_on(credit_usage::get_statistics_for_filter(RegionFilter::Cn, false));
    assert_eq!(cn["region"], "cn");
    for account in cn["accounts"].as_array().expect("accounts") {
        assert_eq!(account["accountId"], "cn-only", "cn 视图不得串入 global 账号");
        assert_eq!(account["region"], "cn");
    }
}

#[test]
fn credit_stats_merged_view_returns_union_with_region_badges() {
    let home = IsolatedHome::new("credit-region-all");

    assert_eq!(config::home_dir(), home.path().to_path_buf());
    write_credit_fixtures(home.path());

    let merged = futures_block_on(credit_usage::get_statistics_for_filter(RegionFilter::All, false));
    assert_eq!(merged["region"], "all");
    let accounts = merged["accounts"].as_array().expect("accounts");
    let ids: Vec<&str> = accounts
        .iter()
        .filter_map(|account| account["accountId"].as_str())
        .collect();
    assert!(ids.contains(&"cn-only"), "合并视图应含 cn 账号");
    assert!(ids.contains(&"global-only"), "合并视图应含 global 账号");
    for account in accounts {
        let id = account["accountId"].as_str().unwrap_or_default();
        let expected = if ids.contains(&id) {
            account["region"].as_str().unwrap_or_default()
        } else {
            ""
        };
        assert!(
            expected == "cn" || expected == "global",
            "每个账号必须有 cn/global 徽标: {account}"
        );
    }
}

#[test]
fn token_stats_merged_view_contains_all_sources_and_region_all() {
    let home = IsolatedHome::new("token-region-all");
    assert_eq!(config::home_dir(), home.path().to_path_buf());

    // 合并视图顶层 region=all，且 sources 含 global 专属的 workbuddy-ai 源。
    let value = buddy_switch_core::modules::token_stats::get_statistics_for_filter(
        RegionFilter::All,
        None,
    );
    assert_eq!(value["region"], "all");
    let names: Vec<&str> = value["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .filter_map(|source| source["source"].as_str())
        .collect();
    assert!(names.contains(&"workbuddy-ai"), "合并视图应含 global 源");
    assert!(names.contains(&"workbuddy"), "合并视图应含 cn 源");

    // global 单版只含 workbuddy-ai。
    let global = buddy_switch_core::modules::token_stats::get_statistics_for_filter(
        RegionFilter::Global,
        None,
    );
    let global_names: Vec<&str> = global["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .filter_map(|source| source["source"].as_str())
        .collect();
    assert_eq!(global_names, ["workbuddy-ai"]);
}

/// 写入含**孤儿快照**的积分 fixture：`cn-only` / `global-only` / `orphan-42`。
///
/// `orphan-42` **不属于任何一版账号库**（模拟账号被删除或轮换后残留的历史快照）。
/// 消耗：cn-only 30、global-only 50、orphan-42 99。
fn write_credit_fixtures_with_orphan(home: &Path) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let store = home.join(".buddy-switch");
    fs::create_dir_all(&store).expect("create store dir");
    fs::write(store.join("accounts.json"), CN_ACCOUNTS).expect("write CN accounts");
    fs::write(store.join("accounts.global.json"), GLOBAL_ACCOUNTS).expect("write Global accounts");

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(1_000_000);
    let snapshots = format!(
        r#"[
  {{ "ts": {}, "accountId": "cn-only", "accountName": "cn@example.com", "total": 1000.0, "remaining": 70.0 }},
  {{ "ts": {}, "accountId": "cn-only", "accountName": "cn@example.com", "total": 1000.0, "remaining": 40.0 }},
  {{ "ts": {}, "accountId": "global-only", "accountName": "global@example.com", "total": 1000.0, "remaining": 150.0 }},
  {{ "ts": {}, "accountId": "global-only", "accountName": "global@example.com", "total": 1000.0, "remaining": 100.0 }},
  {{ "ts": {}, "accountId": "orphan-42", "accountName": "orphan@example.com", "total": 1000.0, "remaining": 999.0 }},
  {{ "ts": {}, "accountId": "orphan-42", "accountName": "orphan@example.com", "total": 1000.0, "remaining": 900.0 }}
]"#,
        now - 120_000,
        now - 60_000,
        now - 120_000,
        now - 60_000,
        now - 120_000,
        now - 60_000,
    );
    fs::write(store.join("credit_usage_snapshots.json"), snapshots).expect("write snapshots");
}

/// **可证伪的不变量**：合并视图必须排除「孤儿快照」，使 `all == cn + global`。
///
/// `load_snapshots()` 读的是**全局**快照库（按账号 id 归档），其中可能残留已删除 /
/// 已轮换账号的历史快照——它们既不属于 CN 也不属于 Global 账号库。分开视图按账号
/// 归属预过滤会排除它们；若合并视图不过滤，`all` 就会**大于** `cn + global`，破坏
/// 「合并值 = 两版之和」。
///
/// 变异（把合并视图的快照过滤去掉）→ `used_all` 变为 179.0（= 30 + 50 + 99）→
/// 本测试的两条断言同时失败。
#[test]
fn credit_stats_merged_view_excludes_orphan_snapshots_and_equals_split_sum() {
    let home = IsolatedHome::new("credit-region-orphan");

    assert_eq!(config::home_dir(), home.path().to_path_buf());
    write_credit_fixtures_with_orphan(home.path());

    let cn = futures_block_on(credit_usage::get_statistics_for_filter(RegionFilter::Cn, false));
    let global = futures_block_on(credit_usage::get_statistics_for_filter(
        RegionFilter::Global,
        false,
    ));
    let all = futures_block_on(credit_usage::get_statistics_for_filter(RegionFilter::All, false));

    // 分开视图：各自只含本版账号的消耗。
    let used_cn = cn["summary"]["usage7Days"].as_f64().unwrap_or(-1.0);
    let used_global = global["summary"]["usage7Days"].as_f64().unwrap_or(-1.0);
    let used_all = all["summary"]["usage7Days"].as_f64().unwrap_or(-1.0);

    assert!(
        (used_cn - 30.0).abs() < 1e-9,
        "cn 视图应只含 cn-only 的消耗 30，got {used_cn}"
    );
    assert!(
        (used_global - 50.0).abs() < 1e-9,
        "global 视图应只含 global-only 的消耗 50，got {used_global}"
    );

    // 核心不变量：合并 = 两版之和（孤儿快照 99 必须被排除）。
    assert!(
        (used_all - 80.0).abs() < 1e-9,
        "合并视图必须排除孤儿快照，期望 30 + 50 = 80，got {used_all}"
    );
    assert_ne!(
        used_all, 179.0,
        "合并视图不得计入孤儿快照（30 + 50 + 99 = 179 是错误值）"
    );
    assert!(
        (used_all - (used_cn + used_global)).abs() < 1e-9,
        "不变量被破坏：all({used_all}) != cn({used_cn}) + global({used_global})"
    );

    // 孤儿账号不得出现在任何视图的账号列表中。
    let merged_ids: Vec<&str> = all["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .filter_map(|account| account["accountId"].as_str())
        .collect();
    assert!(
        !merged_ids.contains(&"orphan-42"),
        "合并视图不得含孤儿账号: {merged_ids:?}"
    );
    assert!(
        merged_ids.contains(&"cn-only") && merged_ids.contains(&"global-only"),
        "合并视图应含两版真实账号: {merged_ids:?}"
    );

    // 签到事件维度：合并视图保留无归属事件（有意设计），故为 `>=` 而非 `==`。
    let events_all = all["events"].as_array().map_or(0, Vec::len);
    let events_split = cn["events"].as_array().map_or(0, Vec::len)
        + global["events"].as_array().map_or(0, Vec::len);
    assert!(
        events_all >= events_split,
        "合并视图 events 应 >= 两版之和（无归属事件中性保留），got {events_all} vs {events_split}"
    );
}

/// 在同步测试中运行 async 函数的最小运行时（不引入额外依赖）。
fn futures_block_on<F: std::future::Future>(future: F) -> F::Output {
    // core 依赖 tokio（见 Cargo.toml），此处复用其多线程运行时。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(future)
}

#[test]
fn cn_wrapper_binds_cn_region_and_leaves_global_untouched() {
    let home = IsolatedHome::new("switch-cn-binding");
    let real_before = real_home_snapshot();

    // 隔离缝必须生效：否则下面的 fixture 会写进真实 home。
    assert_eq!(config::home_dir(), home.path().to_path_buf());
    write_fixtures(home.path());

    let cn_auth = home.path().join(auth_rel("workbuddy-desktop.info"));
    let global_auth = home.path().join(auth_rel("workbuddy-desktop-ai.info"));
    let global_before = fs::read_to_string(&global_auth).expect("read global auth");

    // (a) CN 薄包装必须能切换到 CN 库里的 `cn-only`。
    let ok = switch::switch_account(None, "cn-only", false, &[])
        .expect("CN-only account must switch through the CN wrapper");
    assert!(
        ok.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
        "switch result must report ok: {ok}"
    );

    let cn_after = fs::read_to_string(&cn_auth).expect("read CN auth after switch");
    assert!(
        cn_after.contains("CN_TOKEN"),
        "CN auth file must carry the switched account token, got: {cn_after}"
    );
    assert!(
        !cn_after.contains("CN_PRE"),
        "CN auth file must be rewritten (previous token gone), got: {cn_after}"
    );
    assert_eq!(
        fs::read_to_string(&global_auth).expect("read global auth"),
        global_before,
        "switching a CN account must not modify the Global auth file by a single byte"
    );

    // (b) 证伪关键：`global-only` 只存在于 Global 库，CN 包装必须报「账号不存在」。
    //     若薄包装错误地传了 `Region::Global`，这里会成功 → unwrap_err panic。
    let err = switch::switch_account(None, "global-only", false, &[])
        .expect_err("CN wrapper must not resolve Global-only accounts");
    assert!(err.contains("账号不存在"), "unexpected error: {err}");

    // (e) 跨区失败路径必须零副作用。
    assert_eq!(
        fs::read_to_string(&cn_auth).expect("read CN auth"),
        cn_after,
        "a failed CN switch must not touch the CN auth file"
    );
    assert_eq!(
        fs::read_to_string(&global_auth).expect("read global auth"),
        global_before,
        "a failed CN switch must not touch the Global auth file"
    );

    assert_eq!(
        real_home_snapshot(),
        real_before,
        "the real user home must be untouched by the isolated test"
    );
}

#[test]
fn switch_account_for_global_binds_global_region_and_leaves_cn_untouched() {
    let home = IsolatedHome::new("switch-global-binding");
    let real_before = real_home_snapshot();

    assert_eq!(config::home_dir(), home.path().to_path_buf());
    write_fixtures(home.path());

    let cn_auth = home.path().join(auth_rel("workbuddy-desktop.info"));
    let global_auth = home.path().join(auth_rel("workbuddy-desktop-ai.info"));
    let cn_before = fs::read_to_string(&cn_auth).expect("read CN auth");

    // (c) `Region::Global` 必须能切换到 Global 库里的 `global-only`。
    let ok = switch::switch_account_for(Region::Global, None, "global-only", false, &[])
        .expect("global-only account must switch under Region::Global");
    assert!(
        ok.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
        "switch result must report ok: {ok}"
    );

    let global_after = fs::read_to_string(&global_auth).expect("read Global auth after switch");
    assert!(
        global_after.contains("GLOBAL_TOKEN"),
        "Global auth file must carry the switched account token, got: {global_after}"
    );
    assert!(
        !global_after.contains("GLOBAL_PRE"),
        "Global auth file must be rewritten, got: {global_after}"
    );
    assert_eq!(
        fs::read_to_string(&cn_auth).expect("read CN auth"),
        cn_before,
        "switching a Global account must not modify the CN auth file"
    );

    // (d) 证伪关键：`cn-only` 只存在于 CN 库，Global 目标必须报「账号不存在」。
    let err = switch::switch_account_for(Region::Global, None, "cn-only", false, &[])
        .expect_err("Region::Global must not resolve CN-only accounts");
    assert!(err.contains("账号不存在"), "unexpected error: {err}");

    assert_eq!(
        fs::read_to_string(&global_auth).expect("read Global auth"),
        global_after,
        "a failed Global switch must not touch the Global auth file"
    );
    assert_eq!(
        fs::read_to_string(&cn_auth).expect("read CN auth"),
        cn_before,
        "a failed Global switch must not touch the CN auth file"
    );

    assert_eq!(
        real_home_snapshot(),
        real_before,
        "the real user home must be untouched by the isolated test"
    );
}

#[test]
fn home_dir_honours_override_and_store_dir_derives_from_it() {
    let home = IsolatedHome::new("home-override");

    assert_eq!(
        config::home_dir(),
        home.path().to_path_buf(),
        "BUDDY_SWITCH_HOME must override the home dir"
    );
    assert_eq!(
        config::store_dir(),
        home.path().join(".buddy-switch"),
        "store_dir must derive from the overridden home"
    );
    assert_eq!(
        config::accounts_file(),
        home.path().join(".buddy-switch").join("accounts.json")
    );
    // region 化账号库同样落在覆盖根目录下（CN 与 Global 文件名不同）。
    assert_eq!(
        region::accounts_file_for(Region::Global),
        home.path().join(".buddy-switch").join("accounts.global.json")
    );
    assert_ne!(
        region::accounts_file_for(Region::Cn),
        region::accounts_file_for(Region::Global)
    );
    // 认证文件路径也必须基于覆盖后的 home。
    assert!(
        auth_file::auth_file_path_for(Region::Cn).starts_with(home.path()),
        "auth file path must live under the overridden home"
    );
}

#[test]
fn home_dir_falls_back_to_real_home_when_unset() {
    let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::env::var(BUDDY_SWITCH_HOME_ENV).ok();
    std::env::remove_var(BUDDY_SWITCH_HOME_ENV);

    let expected = dirs::home_dir().expect("real home dir");
    assert_eq!(
        config::home_dir(),
        expected,
        "an unset BUDDY_SWITCH_HOME must fall back to the real home dir"
    );

    match previous {
        Some(value) => std::env::set_var(BUDDY_SWITCH_HOME_ENV, value),
        None => std::env::remove_var(BUDDY_SWITCH_HOME_ENV),
    }
    drop(lock);
}

#[test]
fn blank_override_is_ignored_and_falls_back_to_real_home() {
    let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::env::var(BUDDY_SWITCH_HOME_ENV).ok();
    std::env::set_var(BUDDY_SWITCH_HOME_ENV, "   ");

    let expected = dirs::home_dir().expect("real home dir");
    assert_eq!(
        config::home_dir(),
        expected,
        "a blank BUDDY_SWITCH_HOME must be treated as unset"
    );

    match previous {
        Some(value) => std::env::set_var(BUDDY_SWITCH_HOME_ENV, value),
        None => std::env::remove_var(BUDDY_SWITCH_HOME_ENV),
    }
    drop(lock);
}

// ---------------------------------------------------------------------------
// F2：`BUDDY_SWITCH_HOME` 护栏——覆盖值必须是一个**已存在的绝对目录**
//
// 复用与上面用例**同一把** [`ENV_LOCK`]（该变量是进程级全局状态），避免与之 race。
// ---------------------------------------------------------------------------

/// 在持有 [`ENV_LOCK`] 的前提下，把 `BUDDY_SWITCH_HOME` 临时设为 `value` 运行 `f`，
/// 结束后恢复原值。只用于**不改动**环境变量以外状态的断言。
fn with_override<T>(value: &str, f: impl FnOnce() -> T) -> T {
    let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::env::var(BUDDY_SWITCH_HOME_ENV).ok();
    std::env::set_var(BUDDY_SWITCH_HOME_ENV, value);
    let result = f();
    match previous {
        Some(value) => std::env::set_var(BUDDY_SWITCH_HOME_ENV, value),
        None => std::env::remove_var(BUDDY_SWITCH_HOME_ENV),
    }
    drop(lock);
    result
}

/// F2：覆盖值指向一个**不存在的绝对路径**时必须被忽略并回落真实 home，
/// 且**不得**顺手创建该目录。
#[test]
fn non_existent_absolute_override_is_ignored_and_never_created() {
    let missing = unique_temp_dir("missing-override");
    assert!(!missing.exists(), "precondition: the path must not exist");
    let raw = missing.to_str().expect("utf8 path").to_string();

    let expected = dirs::home_dir().expect("real home dir");
    let actual = with_override(&raw, config::home_dir);
    assert_eq!(
        actual, expected,
        "a non-existent BUDDY_SWITCH_HOME must be ignored (fall back to the real home)"
    );
    assert!(
        !missing.exists(),
        "the guard must never create the rejected directory"
    );
}

/// F2：覆盖值是**相对路径**时必须被忽略（否则 store 会落到进程 CWD）。
#[test]
fn relative_override_is_ignored() {
    let expected = dirs::home_dir().expect("real home dir");
    // 无论该相对名是否恰好存在，都不是绝对路径 → 一律忽略。
    let actual = with_override("relative-buddy-switch-home", config::home_dir);
    assert_eq!(
        actual, expected,
        "a relative BUDDY_SWITCH_HOME must be ignored (fall back to the real home)"
    );
}

/// F2：覆盖值指向一个**已存在的普通文件**时必须被忽略（只接受目录）。
#[test]
fn existing_file_override_is_ignored() {
    let file = unique_temp_dir("existing-file-override");
    fs::write(&file, b"i am a file, not a directory").expect("write temp file");
    assert!(file.is_file(), "precondition: path must be a regular file");
    let raw = file.to_str().expect("utf8 path").to_string();

    let expected = dirs::home_dir().expect("real home dir");
    let actual = with_override(&raw, config::home_dir);
    assert_eq!(
        actual, expected,
        "a BUDDY_SWITCH_HOME pointing at a file must be ignored"
    );

    let _ = fs::remove_file(&file);
}

// ---------------------------------------------------------------------------
// panel `auths/` 目录导入（`import_auths_dir`）：隔离 home 下的**写库全链路**
// ---------------------------------------------------------------------------

/// panel `auths/*.json` 的落盘形态：嵌套双对象、`expiresAt` 为 Unix **秒**。
fn write_auths_fixture(home: &Path, cn: usize, global: usize) -> PathBuf {
    let dir = home.join("auths");
    fs::create_dir_all(&dir).expect("create auths fixture dir");
    for i in 0..cn {
        let text = format!(
            r#"{{"account":{{"enterpriseId":"","nickname":"cn-{i}","uid":"cn-uid-{i}"}},"auth":{{"accessToken":"tok-cn-{i}","refreshToken":"ref-cn-{i}","expiresAt":1793155025,"domain":"www.codebuddy.cn","realm":"cn"}}}}"#
        );
        fs::write(dir.join(format!("workbuddy-cn-{i}.json")), text).expect("write cn fixture");
    }
    for i in 0..global {
        let text = format!(
            r#"{{"account":{{"enterpriseId":"","nickname":"global-{i}","uid":"g-uid-{i}"}},"auth":{{"accessToken":"tok-g-{i}","refreshToken":"ref-g-{i}","expiresAt":1793155026,"domain":"www.workbuddy.ai","realm":"global"}}}}"#
        );
        fs::write(dir.join(format!("workbuddy-global-{i}.json")), text).expect("write global fixture");
    }
    dir
}

/// 可证伪：把 3 个 CN + 1 个 Global 的 panel 文件按真实形态落进隔离 home 下的
/// `auths/`，分别对 CN / Global 库执行目录导入，钉住两条红线：
/// - **域分流**：CN 库里只能出现 cn 账号、Global 库只能出现 global 账号，
///   跨域文件计入 `mismatch` 且**不落任何库**；
/// - **秒 → 毫秒**：panel 落盘是 Unix 秒（10 位），导入后账号库必须存 13 位毫秒
///   （`norm_ts` 口径），否则 UI 的过期判定会按「已过期 5 万年后」处理。
#[test]
fn import_auths_dir_splits_by_realm_and_normalizes_seconds_to_ms() {
    let home = IsolatedHome::new("auths-dir-import");
    let dir = write_auths_fixture(home.path(), 3, 1);

    // CN 库：收 3 个、mismatch 1（那个 global 文件不得进 CN 库）。
    let cn = export_import::import_auths_dir(&dir, Region::Cn).expect("导入 CN 库");
    assert_eq!(cn.imported, 3, "3 个 CN 文件应全部入库");
    assert_eq!(cn.mismatch, 1, "Global 域文件应计 mismatch 跳过");
    assert_eq!(cn.skipped, 0);
    let accounts = account::load_accounts_for(Region::Cn);
    assert_eq!(accounts.len(), 3);
    assert!(accounts.iter().all(|a| a["uid"].as_str().is_some_and(|u| u.starts_with("cn-uid-"))));
    assert!(accounts
        .iter()
        .all(|a| a["expiresAt"].as_i64() == Some(1_793_155_025_000)),
        "秒 1793155025 必须归一成毫秒 1793155025000: {accounts:?}");

    // Global 库：收 1 个、mismatch 3；Global 库文件此时才首次出现。
    let g = export_import::import_auths_dir(&dir, Region::Global).expect("导入 Global 库");
    assert_eq!(g.imported, 1);
    assert_eq!(g.mismatch, 3);
    let global_accounts = account::load_accounts_for(Region::Global);
    assert_eq!(global_accounts.len(), 1);
    assert_eq!(global_accounts[0]["uid"], "g-uid-0");
    assert_eq!(global_accounts[0]["expiresAt"], 1_793_155_026_000_i64);

    // 幂等：同目录再导一次，全部按 uid 覆盖，不产生重复账号。
    let again = export_import::import_auths_dir(&dir, Region::Cn).expect("重复导入");
    assert_eq!(again.imported, 3);
    assert_eq!(again.overwritten, 3, "同 uid 再导入必须走覆盖而不是追加");
    assert_eq!(account::load_accounts_for(Region::Cn).len(), 3);

    // 隔离护栏：真实账号库未被触碰。
    let real = real_home_snapshot();
    drop(home);
    assert_eq!(real, real_home_snapshot(), "隔离导入不得写真实 ~/.buddy-switch");
}
