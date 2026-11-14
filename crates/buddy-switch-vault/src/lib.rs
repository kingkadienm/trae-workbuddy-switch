//! SQLCipher 解密通道。
//!
//! ## 为什么单独一个 crate
//!
//! SQLCipher 是**编译期**开关（`rusqlite` 的 feature）。如果直接开在 `buddy-switch-core`
//! 上，`session.rs` / `client_limits.rs` / `token_daily.rs` 这三个已有 sqlite 使用点会
//! 一起被拖去编 OpenSSL（Windows 还要 perl + 汇编器）—— 那是「换地基」，不是「加功能」。
//!
//! 所以 SQLCipher 只活在**这一个 crate** 里，对外只暴露一个动作：**把加密库导出成明文副本**。
//! 下游（`trae::client_import`）继续用标准 SQLite 读明文，完全不知道这里发生过解密。
//!
//! ## 口令从哪来
//!
//! **不内置提取**。Trae 的库口令由 native 进程持有、磁盘上没有副本，要自己拿只能逆派生
//! 或读进程内存 —— 那会把本工具变成「内置逆向第三方客户端」的软件。因此口令由**用户输入**，
//! 本模块只负责「给定口令，导出明文」这一件事。
//!
//! ## 幂等与副作用
//!
//! 导出是**纯读源库 + 写目标文件**：源库以只读方式打开，绝不写入。目标文件由调用方指定，
//! 已存在时会被覆盖（调用方负责先清理，避免把旧副本当成新数据）。

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

/// 解密通道的错误分类。
///
/// 分得细是因为**每一种的下一步动作不同**：口令错要重输，兼容代错要换版本，
/// 文件不存在要重新选路径 —— 混成一个 `Err` 用户无从下手。
#[derive(Debug, Clone, PartialEq)]
pub enum VaultError {
    /// 源文件不存在。
    Missing,
    /// 源文件能当**明文**打开（说明它已经是明文副本，或根本不是 SQLCipher 库）。
    ///
    /// 这一条刻意单列：用户最常见的误操作就是「把手工导出的明文库又拿来解锁」。
    AlreadyPlaintext,
    /// 口令错（4 与 3 两种兼容代都打不开）。
    WrongKey,
    /// 文件不是 SQLite / 已损坏。
    NotSqlite,
    /// 目标路径不可写等 IO 问题。
    Io(String),
}

impl VaultError {
    /// 稳定错误码（前端按它选文案）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::AlreadyPlaintext => "already_plaintext",
            Self::WrongKey => "wrong_key",
            Self::NotSqlite => "not_sqlite",
            Self::Io(_) => "io",
        }
    }

    /// 面向用户的一句话（含下一步）。
    pub fn message(&self) -> String {
        match self {
            Self::Missing => "找不到这个数据库文件，请重新选择路径。".into(),
            Self::AlreadyPlaintext => {
                "这个文件不需要口令（它已经是明文库，或不是 SQLCipher 库）。直接把它填进「明文库路径」即可。".into()
            }
            Self::WrongKey => "口令不对，打不开这个库。请确认是当前客户端版本的库口令。".into(),
            Self::NotSqlite => "这个文件不是 SQLite 数据库（可能已损坏，或根本不是数据库文件）。".into(),
            Self::Io(detail) => format!("读写失败：{detail}"),
        }
    }
}

/// 探测结果：确认口令可用后，顺带告诉调用方库长什么样。
#[derive(Debug, Clone)]
pub struct VaultInfo {
    /// 生效的 `cipher_compatibility`（4 是新库默认，3 是老库）。
    pub compatibility: u8,
    /// 库里的表名（最多 40 个），供适配判断。
    pub tables: Vec<String>,
}

/// SQLCipher 的 pragma 值转义：单引号在 SQL 字符串里要写成两个。
///
/// ⚠️ 口令里**允许**出现单引号（用户自己定的东西），拼串必须转义，否则轻则报错、
/// 重则被注入成多条语句（这里拼的是 pragma，纯本地文件，但仍按最坏情况处理）。
fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 试开一次：给定兼容代，能否读到 schema。
fn try_open(conn: &Connection, passphrase: &str, compatibility: u8) -> bool {
    // 每次换兼容代都要重新 key（PRAGMA key 只对「尚未读取过」的连接生效一次）。
    let script = format!(
        "PRAGMA key = {};\nPRAGMA cipher_compatibility = {};",
        sql_string(passphrase),
        compatibility
    );
    if conn.execute_batch(&script).is_err() {
        return false;
    }
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })
    .is_ok()
}

/// 只读打开。`busy_timeout` 与 `session.rs` 同口径（5s）。
fn open_readonly(path: &Path) -> Result<Connection, VaultError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| match error {
        rusqlite::Error::SqliteFailure(_, _) => VaultError::NotSqlite,
        other => VaultError::Io(other.to_string()),
    })?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    Ok(conn)
}

/// 文件是否**明文** SQLite（读它的 header 就够了，不需要口令）。
///
/// 明文库的前 16 字节固定是 `b"SQLite format 3\0"`；SQLCipher 库这里是 PBKDF2 salt
/// （随机）⇒ 这一步能**在问用户要口令之前**就把「你已经导出了明文」这种常见误操作挡掉。
fn looks_plaintext(path: &Path) -> Result<bool, VaultError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|_| VaultError::Missing)?;
    let mut header = [0u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(&header == b"SQLite format 3\0"),
        Err(_) => Err(VaultError::NotSqlite),
    }
}

/// 校验口令（不写任何文件）。
pub fn verify(src: &Path, passphrase: &str) -> Result<VaultInfo, VaultError> {
    if !src.is_file() {
        return Err(VaultError::Missing);
    }
    if looks_plaintext(src)? {
        return Err(VaultError::AlreadyPlaintext);
    }
    for compatibility in [4u8, 3u8] {
        let conn = open_readonly(src)?;
        if !try_open(&conn, passphrase, compatibility) {
            continue;
        }
        let mut statement = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .map_err(|error| VaultError::Io(error.to_string()))?;
        let tables: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| VaultError::Io(error.to_string()))?
            .filter_map(Result::ok)
            .take(40)
            .collect();
        return Ok(VaultInfo {
            compatibility,
            tables,
        });
    }
    Err(VaultError::WrongKey)
}

/// 把加密库导出成**明文副本**（SQLCipher 官方的 `sqlcipher_export`）。
///
/// 实现要点：
/// - 源库以**只读**打开 —— 导出是对用户数据的读操作，不该有任何写入风险；
/// - `ATTACH ... KEY ''`（空口令）就是「建一个不加密的目标库」；
/// - 返回生效的兼容代，便于调用方在界面上说清「按哪一代解开的」。
pub fn export_plaintext(src: &Path, passphrase: &str, dst: &Path) -> Result<VaultInfo, VaultError> {
    let info = verify(src, passphrase)?;

    // 目标已存在就先删：`sqlcipher_export` 是「建表 + 灌数据」，残留的旧表会让
    // 结果混进上一次的内容（这正是「旧副本被当成新数据」的经典坑）。
    if dst.exists() {
        std::fs::remove_file(dst).map_err(|error| VaultError::Io(error.to_string()))?;
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|error| VaultError::Io(error.to_string()))?;
    }

    // ⚠️ 需要**可写**连接：`ATTACH ... KEY ''` 要在目标路径上建库，只读连接会以
    // `unable to open database` 失败。可写只针对**目标**——本函数不对源库执行任何写语句
    // （只用 `PRAGMA key` 与 `SELECT sqlcipher_export(...)`），源库内容保持不变。
    let conn = Connection::open(src)
        .map_err(|error| VaultError::Io(format!("打开源库失败（可能正被 Trae 独占）：{error}")))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    let keyed = format!(
        "PRAGMA key = {};\nPRAGMA cipher_compatibility = {};",
        sql_string(passphrase),
        info.compatibility
    );
    conn.execute_batch(&keyed)
        .map_err(|error| VaultError::Io(error.to_string()))?;
    let attach = format!(
        "ATTACH DATABASE {} AS plain KEY '';",
        sql_string(&dst.to_string_lossy())
    );
    conn.execute_batch(&attach)
        .map_err(|error| VaultError::Io(format!("建立目标库失败：{error}")))?;
    conn.execute_batch("SELECT sqlcipher_export('plain');")
        .map_err(|error| VaultError::Io(format!("导出失败：{error}")))?;
    conn.execute_batch("DETACH DATABASE plain;")
        .map_err(|error| VaultError::Io(format!("收尾失败：{error}")))?;

    Ok(info)
}

/// **仅测试用**：把明文库加密成 SQLCipher 库。
///
/// 存在的意义只有一个 —— 让 `export_plaintext` 有**可闭环验证**的输入。没有它，
/// 这个模块只能靠「读真实 Trae 库」来验证，而那个库需要口令，CI 上不可能有。
/// 用同一份引擎先加密、再解密、再比对内容，才能证明导出路径真的通。
#[doc(hidden)]
pub fn encrypt_fixture(plain: &Path, passphrase: &str, encrypted: &Path) -> Result<(), VaultError> {
    if encrypted.exists() {
        std::fs::remove_file(encrypted).map_err(|error| VaultError::Io(error.to_string()))?;
    }
    // ⚠️ 必须是**读写**连接：`ATTACH ... KEY` 要在目标路径上建库，只读连接会以
    // `unable to open database` 直接失败（第一次写这行测试就是这么红的）。
    let conn = Connection::open(plain)
        .map_err(|error| VaultError::Io(format!("打开明文夹具失败：{error}")))?;
    let attach = format!(
        "ATTACH DATABASE {} AS encrypted KEY {};",
        sql_string(&encrypted.to_string_lossy()),
        sql_string(passphrase)
    );
    conn.execute_batch(&attach)
        .map_err(|error| VaultError::Io(error.to_string()))?;
    conn.execute_batch("SELECT sqlcipher_export('encrypted');")
        .map_err(|error| VaultError::Io(error.to_string()))?;
    conn.execute_batch("DETACH DATABASE encrypted;")
        .map_err(|error| VaultError::Io(error.to_string()))?;
    Ok(())
}
