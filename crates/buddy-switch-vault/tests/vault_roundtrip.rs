//! `buddy-switch-vault` 的闭环验证：**同一份引擎**加密 → 解密 → 比对内容。
//!
//! 为什么这样测：真实的 Trae 库需要口令，CI 上不可能有；而「凭自己写的实现解一份
//! 别人造的库」根本证明不了兼容性。用 SQLCipher 自己加密、自己导出、再把导出的副本
//! 当明文读回来比对，才能证明这条链路真的通。
//!
//! ⚠️ 这组测试只有在**编了 SQLCipher**的 crate 里才有意义（`sqlcipher_export` 存在）；
//! 在标准 SQLite 上会直接失败。

use std::path::PathBuf;

use buddy_switch_vault::{export_plaintext, verify, VaultError};
use rusqlite::Connection;

const PASS: &str = "test-passphrase-123";

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "buddy-vault-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 造一份明文库：表结构与 `client_import` 认的形状一致，便于验证下游能读。
fn make_plaintext(path: &PathBuf) {
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
    conn.execute(
        "INSERT INTO message (session_id, created_at, content) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            "s1",
            1_772_712_000i64,
            r#"{"role":"assistant","usage":{"input_tokens":10,"output_tokens":2}}"#
        ],
    )
    .expect("插行");
}

#[test]
fn export_roundtrip_preserves_rows() {
    let dir = temp_dir();
    let plain = dir.join("plain.db");
    let encrypted = dir.join("database.db");
    let exported = dir.join("exported.db");
    make_plaintext(&plain);

    buddy_switch_vault::encrypt_fixture(&plain, PASS, &encrypted).expect("加密");
    assert!(encrypted.is_file());

    // 加密库不该有明文头。
    let head = std::fs::read(&encrypted).expect("读密文");
    assert_ne!(&head[..16], b"SQLite format 3\0", "加密库不该留着明文头");

    // 口令对：能校验、能导出。
    let info = verify(&encrypted, PASS).expect("口令对应该能打开");
    assert!(!info.tables.is_empty(), "应该读到表名：{info:?}");

    export_plaintext(&encrypted, PASS, &exported).expect("导出明文");

    let conn = Connection::open(&exported).expect("打开导出的明文库");
    let total: i64 = conn
        .query_row("SELECT count(*) FROM message", [], |row| row.get(0))
        .expect("查询");
    assert_eq!(total, 1, "导出的副本必须含原始行");
    let content: String = conn
        .query_row("SELECT content FROM message LIMIT 1", [], |row| row.get(0))
        .expect("读正文");
    assert!(
        content.contains("input_tokens"),
        "正文必须原样保留：{content}"
    );
}

#[test]
fn wrong_passphrase_is_reported_as_wrong_key() {
    let dir = temp_dir();
    let plain = dir.join("plain.db");
    let encrypted = dir.join("database.db");
    make_plaintext(&plain);
    buddy_switch_vault::encrypt_fixture(&plain, PASS, &encrypted).expect("加密");

    assert_eq!(
        verify(&encrypted, "not-the-passphrase").err(),
        Some(VaultError::WrongKey)
    );
}

#[test]
fn plaintext_file_is_reported_as_already_plaintext() {
    let dir = temp_dir();
    let plain = dir.join("plain.db");
    make_plaintext(&plain);

    // 用户最常见的误操作：把已导出的明文库又拿来「解锁」。
    assert_eq!(
        verify(&plain, PASS).err(),
        Some(VaultError::AlreadyPlaintext)
    );
}

#[test]
fn missing_file_is_reported_as_missing() {
    let dir = temp_dir();
    assert_eq!(
        verify(&dir.join("nope.db"), PASS).err(),
        Some(VaultError::Missing)
    );
}
