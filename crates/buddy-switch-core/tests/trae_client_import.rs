//! `trae::client_import` 的集成测试：**自造明文库**，不依赖任何真实 Trae 数据。
//!
//! ## 为什么全部自建夹具
//!
//! 本模块的设计前提是「用户自己导出的明文副本」，其 schema **我们没见过**。测试若去读
//! 真机库，就会出现「夹具跟着真机漂移」和「CI 上没有真机就跳过」两个问题。因此这里
//! 只验证**契约**：给定某种可辨认形状，模块必须给出对应的结果或对应的错误码。
//!
//! 覆盖的形状刻意不完全相同（`message` / `chat` / `foo`），因为真实库的表名未知——
//! 若实现偷偷写死了 `message`，后两个用例就会红。

use std::path::PathBuf;

use buddy_switch_core::modules::trae::client_import;
use buddy_switch_core::modules::trae::token_stats::TraeTokenScope;
use rusqlite::Connection;
use tempfile::TempDir;

/// 固定时间戳：`2026-03-05T12:00:00Z`。取正午是为了让任意时区的本地日期都不跨日。
const SEC: i64 = 1_772_712_000;

fn db_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

/// 该时间戳的**本地**日期（期望值由 chrono 现算，避免测试机时区把断言写死）。
fn expected_date() -> String {
    chrono::DateTime::from_timestamp_millis(SEC * 1000)
        .expect("固定时间戳合法")
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d")
        .to_string()
}

fn text_of(path: &PathBuf) -> String {
    path.to_string_lossy().to_string()
}

/// 1) 正常明文库：能认出 `message` 表，并算出正确的 daily / models / summary。
#[test]
fn summarize_reads_plaintext_message_table() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = db_path(&dir, "database.db");
    {
        let conn = Connection::open(&path).expect("建库");
        // 诱饵表：也有时间列和「像 JSON」的文本列，但正文里没有用量标记。
        // 真机上这类表一定存在（`session_project` / `model_config_cache`），
        // 打分必须把 `message` 排在它前面，否则会按项目表算出一堆 0。
        conn.execute_batch(
            "CREATE TABLE session_project (
                 project_id TEXT,
                 created_at INTEGER,
                 data TEXT
             );
             CREATE TABLE message (
                 id INTEGER PRIMARY KEY,
                 session_id TEXT,
                 created_at INTEGER,
                 content TEXT
             );",
        )
        .expect("建表");
        conn.execute(
            "INSERT INTO session_project (project_id, created_at, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["p1", SEC, r#"{"name":"demo","path":"D:/demo"}"#],
        )
        .expect("插项目行");

        let rows = [
            // 新写法：input/output
            (
                "s1",
                r#"{"role":"assistant","model":"claude-4","usage":{"input_tokens":10,"output_tokens":2}}"#,
            ),
            // 旧写法：prompt/completion（必须能回落）
            (
                "s1",
                r#"{"role":"assistant","model":"claude-4","usage":{"prompt_tokens":5,"completion_tokens":1}}"#,
            ),
            // 嵌套位置不同：usage 藏在 data 里
            (
                "s2",
                r#"{"role":"assistant","data":{"model":"gpt-5","usage":{"input_tokens":3,"output_tokens":4}}}"#,
            ),
        ];
        for (session, content) in rows {
            conn.execute(
                "INSERT INTO message (session_id, created_at, content) VALUES (?1, ?2, ?3)",
                rusqlite::params![session, SEC, content],
            )
            .expect("插消息行");
        }
    }

    let target = text_of(&path);
    let stats = client_import::summarize(&target, None, TraeTokenScope::All);
    assert!(stats.get("error").is_none(), "明文库不该报错：{stats}");
    assert_eq!(stats["source"], "trae-client");
    assert_eq!(stats["label"], "Trae 客户端（导入）");
    assert_eq!(stats["summary"]["total"], 25, "10+2+5+1+3+4");
    assert_eq!(stats["summary"]["input"], 18);
    assert_eq!(stats["summary"]["output"], 7);
    assert_eq!(stats["summary"]["records"], 3, "每条消息都算一条记录");

    let models = stats["models"].as_array().expect("models 是数组");
    assert_eq!(models.len(), 2, "只有两个模型：{models:?}");
    assert_eq!(models[0]["key"], "claude-4", "按 total 降序（18 > 7）");
    assert_eq!(models[0]["total"], 18);
    assert_eq!(models[1]["key"], "gpt-5");

    let daily = stats["daily"].as_array().expect("daily 是数组");
    assert_eq!(daily.len(), 1);
    assert_eq!(daily[0]["key"], expected_date());
    assert_eq!(daily[0]["total"], 25);

    let hours = stats["hours"].as_array().expect("hours 是数组");
    assert_eq!(hours.len(), 1);
    let slot = hours[0]["key"].as_str().expect("槽位是字符串");
    let (weekday, hour) = slot.split_once('-').expect("槽位必须是 `{weekday}-{HH}`");
    assert!((0..=6).contains(&weekday.parse::<i64>().expect("星期是数字")));
    assert_eq!(hour.len(), 2, "小时必须两位补零：{slot}");

    assert_eq!(stats["variantCounts"]["cn"], 3, "导入库全量计入国内档");
    assert_eq!(stats["variantCounts"]["global"], 0);
    assert_eq!(stats["variantCounts"]["all"], 3);
    assert_eq!(stats["filesScanned"], 1);
    assert_eq!(stats["parseErrors"], 0);

    // 探测：给出「用哪张表、哪一列、哪种取用量方式」，用户能据此判断对不对。
    let probe = client_import::probe(&target);
    assert_eq!(probe["ok"], true, "{probe}");
    assert_eq!(
        probe["detected"]["messagesTable"], "message",
        "诱饵表不得胜出"
    );
    assert_eq!(probe["detected"]["textColumn"], "content");
    assert_eq!(probe["detected"]["timeColumn"], "created_at");
    assert_eq!(probe["detected"]["usageMode"], "json");
    assert_eq!(probe["detected"]["sessionColumn"], "session_id");
    assert_eq!(probe["detected"]["sessionsEstimated"], false);
    assert_eq!(probe["counts"]["messages"], 3);
    assert_eq!(probe["counts"]["withUsage"], 3);
    assert_eq!(probe["counts"]["sessions"], 2);
}

/// 2) 加密库：文件头是 16 字节 salt ⇒ 必须是 `NotPlaintext`，**不能**被当成空库。
#[test]
fn encrypted_database_is_reported_as_not_plaintext() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = db_path(&dir, "database.db");
    // 与真实 SQLCipher 库同构：16 字节随机 salt + 高熵页。关键点是文件头不是
    // `SQLite format 3\0`（这正是「能不能读」的分界线）。
    let bytes: Vec<u8> = (0..4096u32)
        .map(|index| ((index.wrapping_mul(37).wrapping_add(11)) % 256) as u8)
        .collect();
    assert_ne!(
        &bytes[..16],
        b"SQLite format 3\0",
        "夹具本身不能撞成合法文件头"
    );
    std::fs::write(&path, &bytes).expect("写加密库");

    let target = text_of(&path);
    let probe = client_import::probe(&target);
    assert_eq!(probe["ok"], false);
    assert_eq!(
        probe["error"]["code"], "not_plaintext",
        "加密库必须单独一档：{probe}"
    );

    let stats = client_import::summarize(&target, None, TraeTokenScope::All);
    assert_eq!(stats["error"]["code"], "not_plaintext");
    assert!(
        stats["summary"].is_null(),
        "错误时不得下发一份全 0 的统计冒充「这段没用」"
    );
}

/// 3) 无关库：`foo(a, b)` 既没有文本列也没有时间列 ⇒ `Unrecognized`。
#[test]
fn unrelated_database_is_unrecognized() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = db_path(&dir, "other.db");
    {
        let conn = Connection::open(&path).expect("建库");
        conn.execute_batch("CREATE TABLE foo (a INTEGER, b TEXT);")
            .expect("建表");
        conn.execute(
            "INSERT INTO foo (a, b) VALUES (?1, ?2)",
            rusqlite::params![1, "x"],
        )
        .expect("插行");
    }

    let target = text_of(&path);
    let probe = client_import::probe(&target);
    assert_eq!(probe["ok"], false);
    assert_eq!(probe["error"]["code"], "unrecognized", "{probe}");
    assert_eq!(
        probe["tables"],
        serde_json::json!(["foo"]),
        "认不出也要把表名给用户（便于反馈）"
    );

    let stats = client_import::summarize(&target, None, TraeTokenScope::All);
    assert_eq!(stats["error"]["code"], "unrecognized");
    assert!(stats["summary"].is_null());
}

/// 4) 有会话无 usage ⇒ `NoUsage`。
///
/// ## 取舍：为什么是显式错误，而不是 `ok` + `withUsage == 0`
///
/// 「这个版本的 Trae 不记录用量」与「这段时间你真的没用」是两件事，后者返回 0 才对，
/// 前者返回 0 就是把「测不到」说成了「没用」（本仓既有铁律：缺席 ≠ 0）。
/// 因此结构认得出来、但一行用量都没有时，统一走 `NoUsage`，前端据此提示「版本差异」。
#[test]
fn table_without_usage_fields_is_reported_as_no_usage() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = db_path(&dir, "nou.db");
    {
        let conn = Connection::open(&path).expect("建库");
        conn.execute_batch(
            "CREATE TABLE message (
                 id INTEGER PRIMARY KEY,
                 session_id TEXT,
                 created_at INTEGER,
                 content TEXT
             );",
        )
        .expect("建表");
        conn.execute(
            "INSERT INTO message (session_id, created_at, content) VALUES (?1, ?2, ?3)",
            rusqlite::params!["s1", SEC, r#"{"role":"user","text":"你好"}"#],
        )
        .expect("插行");
    }

    let target = text_of(&path);
    let probe = client_import::probe(&target);
    assert_eq!(probe["ok"], false);
    assert_eq!(probe["error"]["code"], "no_usage", "{probe}");
    // 认得出的部分仍然要给：用户据此才能确认「库选对了，只是没用量」。
    assert_eq!(probe["detected"]["messagesTable"], "message");
    assert_eq!(probe["counts"]["messages"], 1);
    assert_eq!(probe["counts"]["withUsage"], 0);

    let stats = client_import::summarize(&target, None, TraeTokenScope::All);
    assert_eq!(stats["error"]["code"], "no_usage");
    assert!(stats["summary"].is_null());
}

/// 5) 时间戳混用：同一张表里一行秒、一行毫秒 ⇒ 必须归到**同一个本地日**。
#[test]
fn second_and_millisecond_timestamps_normalize_to_the_same_day() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = db_path(&dir, "mixed.db");
    {
        let conn = Connection::open(&path).expect("建库");
        conn.execute_batch(
            "CREATE TABLE message (
                 id INTEGER PRIMARY KEY,
                 session_id TEXT,
                 created_at INTEGER,
                 content TEXT
             );",
        )
        .expect("建表");
        let rows = [
            // 秒
            (
                SEC,
                r#"{"model":"m","usage":{"input_tokens":1,"output_tokens":0}}"#,
            ),
            // 毫秒（同一时刻）
            (
                SEC * 1000,
                r#"{"model":"m","usage":{"input_tokens":2,"output_tokens":0}}"#,
            ),
        ];
        for (ts, content) in rows {
            conn.execute(
                "INSERT INTO message (session_id, created_at, content) VALUES (?1, ?2, ?3)",
                rusqlite::params!["s1", ts, content],
            )
            .expect("插行");
        }
    }

    let target = text_of(&path);
    let stats = client_import::summarize(&target, None, TraeTokenScope::All);
    assert!(stats.get("error").is_none(), "{stats}");
    let daily = stats["daily"].as_array().expect("daily 是数组");
    assert_eq!(
        daily.len(),
        1,
        "秒与毫秒必须归一到同一天，否则会幻影出两天：{daily:?}"
    );
    assert_eq!(daily[0]["key"], expected_date());
    assert_eq!(daily[0]["total"], 3);
    assert_eq!(stats["summary"]["records"], 2);
}

/// 6) 没有会话列 / 列名与 `message` 完全不同（`chat.ts` + `chat.payload`）：
/// 仍然要认得出，且会话数按「一行一个会话」保守估计并**如实标记**。
#[test]
fn missing_session_column_falls_back_to_row_count() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = db_path(&dir, "chat.db");
    {
        let conn = Connection::open(&path).expect("建库");
        conn.execute_batch(
            "CREATE TABLE chat (
                 id INTEGER PRIMARY KEY,
                 ts INTEGER,
                 payload TEXT
             );",
        )
        .expect("建表");
        for _ in 0..3 {
            conn.execute(
                "INSERT INTO chat (ts, payload) VALUES (?1, ?2)",
                rusqlite::params![SEC, r#"{"model":"m","usage":{"input_tokens":1}}"#],
            )
            .expect("插行");
        }
    }

    let target = text_of(&path);
    let probe = client_import::probe(&target);
    assert_eq!(probe["ok"], true, "{probe}");
    assert_eq!(probe["detected"]["messagesTable"], "chat");
    assert_eq!(probe["detected"]["timeColumn"], "ts");
    assert_eq!(probe["detected"]["textColumn"], "payload");
    assert!(probe["detected"]["sessionColumn"].is_null());
    assert_eq!(probe["detected"]["sessionsEstimated"], true);
    assert_eq!(probe["counts"]["sessions"], 3);
    assert_eq!(probe["counts"]["messages"], 3);

    let stats = client_import::summarize(&target, None, TraeTokenScope::All);
    assert_eq!(stats["summary"]["total"], 3);
    assert!(
        stats["note"]
            .as_str()
            .unwrap_or_default()
            .contains("保守估计"),
        "会话数是估的，必须在 note 里说明：{}",
        stats["note"]
    );
}

/// 7) 结构认得出来但**一行都没有**（导早了）：那是「空副本」，返回 0 才是诚实；
/// 报 `unrecognized` / `no_usage` 都会把「还没聊过」说成「读不出」。
#[test]
fn recognized_but_empty_table_returns_zero_with_a_note() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = db_path(&dir, "empty.db");
    {
        let conn = Connection::open(&path).expect("建库");
        conn.execute_batch(
            "CREATE TABLE message (
                 id INTEGER PRIMARY KEY,
                 session_id TEXT,
                 created_at INTEGER,
                 content TEXT
             );",
        )
        .expect("建表");
    }

    let target = text_of(&path);
    let probe = client_import::probe(&target);
    assert_eq!(probe["ok"], true, "空库不是错误：{probe}");
    assert!(probe.get("error").is_none(), "{probe}");
    assert_eq!(probe["detected"]["messagesTable"], "message");
    assert_eq!(probe["counts"]["messages"], 0);

    let stats = client_import::summarize(&target, None, TraeTokenScope::All);
    assert!(stats.get("error").is_none(), "{stats}");
    assert_eq!(stats["summary"]["total"], 0);
    assert_eq!(stats["summary"]["records"], 0);
    assert_eq!(stats["filesScanned"], 0);
    assert!(
        stats["note"]
            .as_str()
            .unwrap_or_default()
            .contains("没有任何对话记录"),
        "空副本必须说明原因：{}",
        stats["note"]
    );
}

/// 一行里含**多份** `usage` 时必须被统计出来，并在 note 里披露「可能偏大」。
///
/// 这是本模块最可能不适配真实库的地方：如果客户端把「整段对话快照」存在一行里，
/// 累加会把历史轮次重复计数。我们不做猜测性去重（无从判断），但**不能让一个看起来
/// 精确的错数字悄悄上屏** —— 所以既有 `multiUsageRows` 计数，也有这句 note。
#[test]
fn multiple_usage_objects_per_row_is_disclosed() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = db_path(&dir, "snapshot.db");
    {
        let conn = Connection::open(&path).expect("建库");
        conn.execute_batch(
            "CREATE TABLE message (
                 id INTEGER PRIMARY KEY,
                 session_id TEXT,
                 created_at INTEGER,
                 content TEXT
             );",
        )
        .expect("建表");
        // 一行 = 整段对话快照：两份 usage（历史轮次 + 本轮）。
        conn.execute(
            "INSERT INTO message (session_id, created_at, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                "s1",
                SEC,
                r#"{"messages":[{"usage":{"input_tokens":10,"output_tokens":1}},{"usage":{"input_tokens":20,"output_tokens":2}}]}"#
            ],
        )
        .expect("插快照行");
    }

    let target = text_of(&path);
    let stats = client_import::summarize(&target, None, TraeTokenScope::All);
    assert!(stats.get("error").is_none(), "{stats}");
    assert_eq!(stats["summary"]["total"], 33, "累加语义：10+1+20+2");

    let note = stats["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("含多份用量"),
        "必须披露可能出现重复计数：{note}"
    );

    let probe = client_import::probe(&target);
    assert_eq!(probe["counts"]["multiUsageRows"], 1, "{probe}");
}
