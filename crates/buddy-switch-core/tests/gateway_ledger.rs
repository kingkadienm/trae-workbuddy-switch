//! 网关账本的**落盘往返**（`record` → `usage_dim`）。
//!
//! 为什么放在集成测试而不是单元测试：本文件会改**进程级** `BUDDY_SWITCH_HOME`，
//! 与并行运行的其他单元测试共享进程 ⇒ 必须独占（[`ENV_LOCK`] + [`IsolatedHome`]），
//! 与 `cross_region_migration.rs` 同一套守卫。
//!
//! 覆盖三件单元测试覆盖不到的事：
//! 1. 落盘 JSON 的键名 / 形状（前端 `GatewayUsageDim` 直接读它）；
//! 2. **同一 scope 一轮又一轮**不双计（水位线真的落到了盘上）；
//! 3. `all` 的合并口径（cn + global + unlabeled 相加，不是取大）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use buddy_switch_core::modules::config::BUDDY_SWITCH_HOME_ENV;
use buddy_switch_core::modules::gateway_ledger::{
    record, record_all, usage_dim, LedgerProduct, LedgerScope,
};

/// 串行化所有会修改 `BUDDY_SWITCH_HOME` 的测试（进程级全局状态）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct IsolatedHome {
    _lock: MutexGuard<'static, ()>,
    previous: Option<String>,
    dir: PathBuf,
}

impl IsolatedHome {
    fn new(label: &str) -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var(BUDDY_SWITCH_HOME_ENV).ok();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-ledger-{label}-{}-{nanos}",
            std::process::id()
        ));
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

/// 今日某一时刻（毫秒，本地时区）。
fn today_at(hour: u32) -> i64 {
    use chrono::{Datelike, Local, TimeZone};
    let today = Local::now().date_naive();
    Local
        .with_ymd_and_hms(today.year(), today.month(), today.day(), hour, 0, 0)
        .single()
        .expect("valid local time")
        .timestamp_millis()
}

fn write_qoder_log(home: &Path, entries: &Value) {
    let dir = home.join(".buddy-switch").join("qoder").join("logs");
    fs::create_dir_all(&dir).expect("create qoder logs dir");
    fs::write(
        dir.join("gateway_requests.jsonl"),
        serde_json::to_string_pretty(entries).expect("serialize log"),
    )
    .expect("write log");
}

/// Qoder：三档分流 + 水位线不双计 + `all` 相加。
#[test]
fn qoder_ledger_splits_scopes_and_never_double_counts() {
    let home = IsolatedHome::new("qoder");
    let ts = today_at(10);
    write_qoder_log(
        home.path(),
        &json!([
            {
                "ts": ts,
                "region": "cn",
                "model": "dfmodel",
                "account": "uid-cn",
                "accountUid": "uid-cn",
                "endpoint": "chat",
                "status": 200,
                "promptTokens": 100,
                "completionTokens": 20,
                "credits": 0.75,
                "keyId": "k-cn",
            },
            {
                "ts": ts + 1,
                "region": "global",
                "model": "claude",
                "account": "uid-global",
                "accountUid": "uid-global",
                "endpoint": "chat",
                "status": 200,
                "promptTokens": 40,
                "completionTokens": 10,
                "credits": 0.25,
                "keyId": null,
            },
            {
                // 旧日志：没有 `region` ⇒ 未标注一档（不许塞进 cn）
                "ts": ts + 2,
                "model": "legacy",
                "account": "uid-legacy",
                "accountUid": "uid-legacy",
                "endpoint": "chat",
                "status": 200,
                "promptTokens": 5,
                "completionTokens": 5,
            }
        ]),
    );

    record_all(LedgerProduct::Qoder);
    // 再跑一轮：文件没变 ⇒ 一条都不该重复计入（水位线已落盘）。
    record_all(LedgerProduct::Qoder);

    let cn = usage_dim(LedgerProduct::Qoder, "cn");
    let cn_days = cn["days"].as_array().expect("days array");
    assert_eq!(cn_days.len(), 1);
    assert_eq!(cn_days[0]["models"][0]["model"], "dfmodel");
    assert_eq!(cn_days[0]["models"][0]["total"], 120, "第二轮不许双计");
    assert_eq!(cn_days[0]["models"][0]["credit"], 0.75);

    let unlabeled = usage_dim(LedgerProduct::Qoder, "unlabeled");
    assert_eq!(
        unlabeled["days"][0]["models"][0]["model"], "legacy",
        "无 region 的旧日志必须落未标注档"
    );

    let all = usage_dim(LedgerProduct::Qoder, "all");
    let total: u64 = all["days"][0]["models"]
        .as_array()
        .expect("models array")
        .iter()
        .map(|row| row["total"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(total, 120 + 50 + 10, "all = 三档相加（不是取大）");

    // 三条维度的合计必须一致（页面「按模型 = 按账号 = 网关总量」的自洽前提）。
    let sum_of = |key: &str| -> u64 {
        all["days"][0][key]
            .as_array()
            .expect("dim array")
            .iter()
            .map(|row| row["total"].as_u64().unwrap_or(0))
            .sum()
    };
    assert_eq!(sum_of("models"), sum_of("accounts"));
    assert_eq!(sum_of("models"), sum_of("endpoints"));
    assert_eq!(
        sum_of("keys"),
        sum_of("models"),
        "密钥维度也必须自洽（用量页「按密钥看用量」直接读它）"
    );
    assert_eq!(
        all["days"][0]["keysPartial"].as_bool(),
        Some(false),
        "全新账本不存在升级播种，不该挂下界标记"
    );
}

/// Trae：`variant` 决定区域档，且 `record` 会真的把账本文件写到 trae 目录下。
#[test]
fn trae_ledger_maps_variant_to_region() {
    let home = IsolatedHome::new("trae");
    let ts = today_at(11);
    let dir = home.path().join(".buddy-switch").join("trae");
    fs::create_dir_all(&dir).expect("create trae dir");
    fs::write(
        dir.join("api_gateway_logs.json"),
        serde_json::to_string_pretty(&json!([
            {
                "ts": ts,
                "variant": "trae_merged",
                "model": "deepseek-v4.1-flash",
                "account": "uid-cn",
                "accountUid": "uid-cn",
                "endpoint": "/v1/chat/completions",
                "status": 200,
                "promptTokens": 30,
                "completionTokens": 6,
            },
            {
                "ts": ts + 1,
                "variant": "global_trae_merged",
                "model": "claude",
                "account": "uid-global",
                "accountUid": "uid-global",
                "endpoint": "/v1/chat/completions",
                "status": 502,
                "promptTokens": 0,
                "completionTokens": 0,
            }
        ]))
        .expect("serialize"),
    )
    .expect("write trae log");

    record_all(LedgerProduct::Trae);

    let cn = usage_dim(LedgerProduct::Trae, "cn");
    assert_eq!(cn["days"][0]["models"][0]["total"], 36);
    assert_eq!(cn["days"][0]["models"][0]["failed"], 0);

    let global = usage_dim(LedgerProduct::Trae, "global");
    assert_eq!(
        global["days"][0]["models"][0]["failed"], 1,
        "502 必须进 failed（网关账本要能给失败列）"
    );

    assert!(
        home.path()
            .join(".buddy-switch")
            .join("trae")
            .join("gateway_ledger.cn.json")
            .exists(),
        "账本必须落盘到 trae 目录（否则重启即丢）"
    );
}

/// Trae 网关日志目前不记 credit：账本的 credit 必须是 0（不知道），不是编造值。
#[test]
fn trae_ledger_credit_stays_zero_when_log_has_no_credit() {
    let home = IsolatedHome::new("trae-credit");
    let dir = home.path().join(".buddy-switch").join("trae");
    fs::create_dir_all(&dir).expect("create trae dir");
    fs::write(
        dir.join("api_gateway_logs.json"),
        serde_json::to_string_pretty(&json!([{
            "ts": today_at(12),
            "variant": "trae_merged",
            "model": "m",
            "account": "uid",
            "accountUid": "uid",
            "endpoint": "/v1/chat/completions",
            "status": 200,
            "promptTokens": 1,
            "completionTokens": 1,
        }]))
        .expect("serialize"),
    )
    .expect("write log");
    record(
        LedgerProduct::Trae,
        buddy_switch_core::modules::gateway_ledger::LedgerScope::Cn,
    );
    let dim = usage_dim(LedgerProduct::Trae, "cn");
    assert_eq!(dim["days"][0]["models"][0]["credit"], 0.0);
}

/// ★ v1 → v2 升级：当天**已有**用量但账本没有 `keys` 维度 ⇒ 必须整窗播种并标
/// `keysPartial`（否则当天此前的请求全部漏记，用户看到的是「这把 Key 只用了这么点」）。
#[test]
fn ledger_seeds_the_key_dimension_for_days_recorded_before_it_existed() {
    let home = IsolatedHome::new("trae-keys");
    let ts = today_at(12);
    let dir = home.path().join(".buddy-switch").join("trae");
    fs::create_dir_all(&dir).expect("create trae dir");

    // 今天那天的旧记录（日期键运行时才定）；升级前形态（v1）：有 total / models /
    // 水位线，**没有** keys。
    let day = {
        use chrono::Local;
        Local::now().date_naive().format("%Y-%m-%d").to_string()
    };
    fs::write(
        dir.join("gateway_ledger.cn.json"),
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "days": {
                day.clone(): {
                    "watermarkMs": ts - 1,
                    "truncated": false,
                    "total": { "input": 100, "output": 10, "requests": 2, "failed": 0, "credit": 0.0 },
                    "models": {
                        "m-old": { "input": 100, "output": 10, "requests": 2, "failed": 0, "credit": 0.0 }
                    }
                }
            }
        }))
        .expect("serialize prior"),
    )
    .expect("write prior");

    fs::write(
        dir.join("api_gateway_logs.json"),
        serde_json::to_string_pretty(&json!([
            {
                "ts": ts,
                "variant": "trae_merged",
                "model": "deepseek-v4.1-flash",
                "account": "uid-cn",
                "accountUid": "uid-cn",
                "endpoint": "/v1/chat/completions",
                "status": 200,
                "promptTokens": 30,
                "completionTokens": 6,
                "keyId": "k-1",
            },
            {
                "ts": ts + 1,
                "variant": "trae_merged",
                "model": "deepseek-v4.1-flash",
                "account": "uid-cn",
                "accountUid": "uid-cn",
                "endpoint": "/v1/chat/completions",
                "status": 200,
                "promptTokens": 4,
                "completionTokens": 0,
                "keyId": null,
            }
        ]))
        .expect("serialize log"),
    )
    .expect("write log");

    record(LedgerProduct::Trae, LedgerScope::Cn);

    let dim = usage_dim(LedgerProduct::Trae, "cn");
    let today = dim["days"]
        .as_array()
        .expect("days")
        .iter()
        .find(|entry| entry["key"] == day.as_str())
        .cloned()
        .expect("今日必须有记录");
    let keys = today["keys"].as_array().expect("keys 维度必须存在");
    let total_of = |name: &str| -> Option<u64> {
        keys.iter()
            .find(|entry| entry["key"] == name)
            .and_then(|entry| entry["total"].as_u64())
    };
    assert_eq!(total_of("k-1"), Some(36), "播种轮取整窗读数（30+6）");
    assert_eq!(
        total_of("__none__"),
        Some(4),
        "keyId: null 落「无密钥调用」"
    );
    assert_eq!(
        today["keysPartial"].as_bool(),
        Some(true),
        "v1 → v2 当天只是下界，UI 要能披露"
    );
}
