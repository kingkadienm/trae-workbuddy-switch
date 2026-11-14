//! 端到端：**加密库 → 内置解密 → 客户端账本统计**。
//!
//! 这是「内置解密」这条功能唯一的真实验证方式：真实 Trae 库需要口令（CI 上不可能有），
//! 所以用同一份 SQLCipher 引擎造一个「形状与 Trae 一致」的加密库，走完整链路：
//!
//! ```text
//! 明文库(message 表) --encrypt_fixture--> 加密库 --export_plaintext--> 明文副本
//!                                                      --client_import::summarize--> 统计
//! ```
//!
//! 覆盖的是**接线**而不是 SQLCipher 本身（后者由 `buddy-switch-vault` 自己的测试负责）：
//! 如果哪天有人把 `plaintext_path()` 与命令层的落点接歪、或导出产物没被 `client_import`
//! 认出，这条就会红。

use std::path::PathBuf;

use buddy_switch_core::modules::trae::client_import;
use buddy_switch_core::modules::trae::token_stats::TraeTokenScope;
use buddy_switch_vault::{encrypt_fixture, export_plaintext};
use rusqlite::Connection;

const PASS: &str = "trae-vault-e2e";

/// 固定时间戳（`2026-03-05T12:00:00Z`，正午避免时区跨日）。
const SEC: i64 = 1_772_712_000;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "buddy-vault-e2e-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 造一份「像 Trae 客户端库」的明文：表名与字段都按 `client_import` 的探测候选来。
fn make_client_plaintext(path: &PathBuf) {
    let conn = Connection::open(path).expect("建库");
    conn.execute_batch(
        "CREATE TABLE message (
             id INTEGER PRIMARY KEY,
             session_id TEXT,
             created_at INTEGER,
             content TEXT
         );",
    )
    .expect("建表");
    for (session, content) in [
        (
            "s1",
            r#"{"role":"assistant","model":"claude-4","usage":{"input_tokens":100,"output_tokens":20}}"#,
        ),
        (
            "s1",
            r#"{"role":"assistant","model":"claude-4","usage":{"input_tokens":50,"output_tokens":5}}"#,
        ),
        (
            "s2",
            r#"{"role":"assistant","model":"gpt-5","usage":{"input_tokens":30,"output_tokens":7}}"#,
        ),
    ] {
        conn.execute(
            "INSERT INTO message (session_id, created_at, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![session, SEC, content],
        )
        .expect("插行");
    }
}

#[test]
fn encrypted_database_becomes_readable_client_ledger() {
    let dir = temp_dir();
    let plain_source = dir.join("plain_source.db");
    let encrypted = dir.join("database.db");
    let exported = dir.join("exported.db");
    make_client_plaintext(&plain_source);

    encrypt_fixture(&plain_source, PASS, &encrypted).expect("加密成 SQLCipher 库");
    // 加密库确实不是明文（否则这条测试就没在测解密）。
    let head = std::fs::read(&encrypted).expect("读密文");
    assert_ne!(&head[..16], b"SQLite format 3\0", "加密库不该有明文头");

    // 解密前的加密库：`client_import` 必须**认栽**（这正是它会返回 NotPlaintext 的场景）。
    let before = client_import::probe(&encrypted.to_string_lossy());
    assert_eq!(before["ok"], false, "加密库不该被当成可读库：{before}");
    assert_eq!(
        before["error"]["code"], "not_plaintext",
        "加密库必须报 not_plaintext：{before}"
    );

    // 内置解密。
    let info = export_plaintext(&encrypted, PASS, &exported).expect("导出明文副本");
    assert!(!info.tables.is_empty(), "导出后应能列出表：{info:?}");

    // 解密后：能读，且数字对。
    let stats = client_import::summarize(&exported.to_string_lossy(), None, TraeTokenScope::All);
    assert!(stats.get("error").is_none(), "导出副本应当可读：{stats}");
    assert_eq!(stats["source"], "trae-client");
    assert_eq!(stats["summary"]["total"], 212, "100+20+50+5+30+7");
    assert_eq!(stats["summary"]["input"], 180);
    assert_eq!(stats["summary"]["output"], 32);
    assert_eq!(stats["summary"]["records"], 3);

    let models = stats["models"].as_array().expect("models 是数组");
    assert_eq!(models.len(), 2, "两个模型：{models:?}");
    assert_eq!(models[0]["key"], "claude-4", "按 total 降序");
    assert_eq!(models[0]["total"], 175);
    assert_eq!(models[1]["key"], "gpt-5");
}

#[test]
fn unlocked_copy_overwrites_previous_one() {
    let dir = temp_dir();
    let plain_source = dir.join("plain_source.db");
    let encrypted = dir.join("database.db");
    let exported = dir.join("exported.db");
    make_client_plaintext(&plain_source);
    encrypt_fixture(&plain_source, PASS, &encrypted).expect("加密");

    export_plaintext(&encrypted, PASS, &exported).expect("第一次导出");
    let first = std::fs::metadata(&exported).expect("stat").len();

    // 再导一次：必须**覆盖**而不是往里追加（旧副本被当新数据是最隐蔽的一类错）。
    export_plaintext(&encrypted, PASS, &exported).expect("第二次导出");
    let stats = client_import::summarize(&exported.to_string_lossy(), None, TraeTokenScope::All);
    assert_eq!(
        stats["summary"]["total"], 212,
        "重复导出不该让总量翻倍：{stats}"
    );

    let second = std::fs::metadata(&exported).expect("stat").len();
    assert_eq!(first, second, "同一份源库导出两次，体积应当一致");
}
