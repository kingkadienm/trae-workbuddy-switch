//! Memory / Connector 迁移的**改前备份**可证伪集成测试。
//!
//! ## 为什么需要它
//!
//! 本仓库的去重合并会**原地改写目标账号的文件**（memory 追加、connector 深合并），
//! 而参考实现 `workbuddy-account-migrate` 把「必须先备份」列为安全规则 #1。
//! 去重逻辑比参考实现更复杂（规范化指纹 / 递归下钻 / 数组元素级去重），
//! 一旦判定有误就会污染目标文件，因此备份是**可回滚性**的唯一保障。
//!
//! ## 可证伪性
//!
//! 若把 `merge_memory_files` / `merge_connectors_for` 里的备份调用删掉：
//! - `memory_merge_backs_up_original_target_before_overwrite` 会因 `backup` 为 `None` panic；
//! - `connector_merge_backs_up_original_target_file` 同理，且备份文件不存在。
//!
//! 若把「只在 changed 时备份」改成「无条件备份」：
//! - `memory_merge_second_run_creates_no_second_backup` 会因第二次仍产生备份而 panic。
//!
//! ## 隔离方式
//!
//! 全部 fixture 写入临时目录，并把进程级 `BUDDY_SWITCH_HOME` 指向它
//! （`config::home_dir()` 的覆盖缝）；测试结束恢复原值并清理。
//! 环境变量是进程级全局状态，故用 [`ENV_LOCK`] 串行化本文件内的测试。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use buddy_switch_core::modules::config::BUDDY_SWITCH_HOME_ENV;
use buddy_switch_core::modules::region::Region;
use buddy_switch_core::modules::{config, connector, memory, migrate};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("buddy-switch-migrate-{label}-{nanos}"))
}

/// 把 `BUDDY_SWITCH_HOME` 指向临时目录，Drop 时恢复并清理。
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

const SOURCE_MEMORY: &str = "# 来源账号偏好\n- 偏好 A\n- 偏好 B\n";
const TARGET_MEMORY: &str = "# 目标账号偏好\n- 偏好 A\n- 偏好 C\n";

/// 把 `content` 写到 `path`，必要时建父目录。
fn write_at(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write fixture");
}

#[test]
fn memory_merge_backs_up_original_target_before_overwrite() {
    let home = IsolatedHome::new("mem-backup");
    assert_eq!(config::home_dir(), home.path().to_path_buf());

    let source_path = memory::memory_file_for(Region::Cn, "uid-mem-source");
    let target_path = memory::memory_file_for(Region::Cn, "uid-mem-target");
    write_at(&source_path, SOURCE_MEMORY);
    write_at(&target_path, TARGET_MEMORY);

    let result = memory::merge_memory_files(Region::Cn, "uid-mem-source", "uid-mem-target")
        .expect("merge should succeed");

    // 源中有、目标中无的行：「来源账号偏好」标题与「偏好 B」→ 追加 2 行；
    // 「偏好 A」已存在（指纹「偏好 A」）→ 去重跳过 1 行。
    assert_eq!(result.appended, 2, "只应追加目标缺失的那两行");
    assert_eq!(result.skipped_duplicate, 1, "重复行应被规范化指纹判重");

    // 核心断言：备份必须存在，且内容 == **改前**的目标原文。
    let backup = result.backup.expect("目标原本存在且被改写，必须产生备份");
    assert!(backup.is_file(), "备份文件应真实存在：{}", backup.display());
    assert_eq!(
        fs::read_to_string(&backup).expect("read backup"),
        TARGET_MEMORY,
        "备份内容必须等于改前的目标原文"
    );

    // 目标已被改写为合并结果：新旧内容同时存在。
    let merged = fs::read_to_string(&target_path).expect("read merged target");
    assert!(merged.contains("偏好 C"), "目标原有内容不得丢失");
    assert!(merged.contains("偏好 B"), "源的新内容应已追加");
    assert!(
        merged.contains("来源账号偏好"),
        "源独有的标题行也应作为新内容追加"
    );
}

#[test]
fn memory_merge_second_run_creates_no_second_backup() {
    let home = IsolatedHome::new("mem-idem");
    assert_eq!(config::home_dir(), home.path().to_path_buf());

    write_at(
        &memory::memory_file_for(Region::Cn, "uid-idem-source"),
        SOURCE_MEMORY,
    );
    write_at(
        &memory::memory_file_for(Region::Cn, "uid-idem-target"),
        TARGET_MEMORY,
    );

    let first = memory::merge_memory_files(Region::Cn, "uid-idem-source", "uid-idem-target")
        .expect("first merge");
    assert_eq!(first.appended, 2);
    assert!(first.backup.is_some(), "首次改写应产生备份");

    let after_first = fs::read_to_string(memory::memory_file_for(Region::Cn, "uid-idem-target"))
        .expect("read target after first merge");

    // 第二次：源内容已全部存在 → 无新增 → 不应改写，也不应产生新备份。
    let second = memory::merge_memory_files(Region::Cn, "uid-idem-source", "uid-idem-target")
        .expect("second merge");
    assert_eq!(second.appended, 0, "幂等重跑不得再追加");
    assert!(
        second.backup.is_none(),
        "无改写就不该产生备份（否则会持续污染 backups 目录）"
    );

    let after_second = fs::read_to_string(memory::memory_file_for(Region::Cn, "uid-idem-target"))
        .expect("read target after second merge");
    assert_eq!(after_first, after_second, "幂等重跑不得改变文件内容");
}

#[test]
fn memory_merge_without_existing_target_creates_no_backup() {
    let home = IsolatedHome::new("mem-no-target");
    assert_eq!(config::home_dir(), home.path().to_path_buf());

    write_at(
        &memory::memory_file_for(Region::Cn, "uid-fresh-source"),
        SOURCE_MEMORY,
    );

    let result = memory::merge_memory_files(Region::Cn, "uid-fresh-source", "uid-fresh-target")
        .expect("merge into a non-existent target should succeed");

    assert!(result.appended > 0, "目标为空时应直接采用源内容");
    assert!(
        result.backup.is_none(),
        "目标原本不存在 → 无原文可备份，不得产生空备份"
    );
    let merged = fs::read_to_string(memory::memory_file_for(Region::Cn, "uid-fresh-target"))
        .expect("target should now exist");
    assert!(merged.contains("偏好 A"));
}

#[test]
fn connector_merge_backs_up_original_target_file() {
    let home = IsolatedHome::new("conn-backup");
    assert_eq!(config::home_dir(), home.path().to_path_buf());

    let source_dir = connector::connector_account_dir_for(Region::Cn, "uid-conn-source");
    let target_dir = connector::connector_account_dir_for(Region::Cn, "uid-conn-target");
    let target_mcp = target_dir.join("mcp.json");

    write_at(
        &source_dir.join("mcp.json"),
        r#"{"mcpServers":{"shared":{"command":"src"},"onlyInSource":{"command":"new"}}}"#,
    );
    let target_original = r#"{"mcpServers":{"shared":{"command":"target"}}}"#;
    write_at(&target_mcp, target_original);

    let result = connector::merge_connectors_for(Region::Cn, "uid-conn-source", "uid-conn-target")
        .expect("merge should succeed");

    assert!(result.changed(), "同名 server 下应下钻出新键，故必须改写");
    // added_keys 口径 = 新增的**键本身** + 其子树内的所有键：
    // onlyInSource（1）+ 其下的 command（1）= 2。shared 走目标优先，不计入。
    assert_eq!(
        result.added_keys(),
        2,
        "只应新增 onlyInSource 及其 command 共 2 个键（shared 走目标优先）"
    );

    // 核心断言：备份目录存在，且其中的目标文件内容 == 改前原文。
    let backup_dir = result.backup.expect("目标原本存在且被改写，必须产生备份");
    let backed_up = backup_dir.join("mcp.json");
    assert!(
        backed_up.is_file(),
        "备份文件应真实存在：{}",
        backed_up.display()
    );
    assert_eq!(
        fs::read_to_string(&backed_up).expect("read backup"),
        target_original,
        "备份内容必须等于改前的目标原文"
    );

    // 目标已被深合并：onlyInSource 进来，且 shared.command 保持目标值。
    let merged: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&target_mcp).expect("read merged"))
            .expect("merged target must stay valid JSON");
    assert_eq!(merged["mcpServers"]["onlyInSource"]["command"], "new");
    assert_eq!(
        merged["mcpServers"]["shared"]["command"], "target",
        "标量冲突必须保留目标值"
    );
}

#[test]
fn migrate_report_exposes_backup_paths_and_is_idempotent() {
    let home = IsolatedHome::new("migrate-report");
    assert_eq!(config::home_dir(), home.path().to_path_buf());

    write_at(
        &memory::memory_file_for(Region::Cn, "uid-rep-source"),
        SOURCE_MEMORY,
    );
    write_at(
        &memory::memory_file_for(Region::Cn, "uid-rep-target"),
        TARGET_MEMORY,
    );
    let conn_source = connector::connector_account_dir_for(Region::Cn, "uid-rep-source");
    let conn_target = connector::connector_account_dir_for(Region::Cn, "uid-rep-target");
    write_at(
        &conn_source.join("mcp.json"),
        r#"{"mcpServers":{"onlyInSource":{"command":"new"}}}"#,
    );
    write_at(&conn_target.join("mcp.json"), r#"{"mcpServers":{}}"#);

    let report = migrate::migrate_account_data_for(
        Region::Cn,
        "uid-rep-source",
        "uid-rep-target",
        migrate::MigrateScope::default(),
    )
    .expect("migration should succeed");

    assert_eq!(report["changed"], true);
    assert_eq!(report["memory"]["appended"], 2);
    assert_eq!(report["memory"]["skippedDuplicate"], 1);
    // onlyInSource（1）+ 其下 command（1）= 2
    assert_eq!(report["connectors"]["addedKeys"], 2);

    // 报告必须把备份路径透出去，供 UI / 用户回滚。
    let mem_backup = report["memory"]["backup"]
        .as_str()
        .expect("memory 备份路径应出现在报告中");
    assert!(Path::new(mem_backup).is_file(), "memory 备份应真实存在");
    let conn_backup = report["connectors"]["backup"]
        .as_str()
        .expect("connector 备份路径应出现在报告中");
    assert!(
        Path::new(conn_backup).is_dir(),
        "connector 备份目录应真实存在"
    );

    // 二次迁移：全部命中指纹 → 无新增、无改写、无新备份。
    let second = migrate::migrate_account_data_for(
        Region::Cn,
        "uid-rep-source",
        "uid-rep-target",
        migrate::MigrateScope::default(),
    )
    .expect("second migration should succeed");

    assert_eq!(second["changed"], false, "幂等重跑不得报告改写");
    assert_eq!(second["memory"]["appended"], 0);
    assert_eq!(second["connectors"]["addedKeys"], 0);
    assert_eq!(
        second["memory"]["backup"],
        serde_json::Value::Null,
        "无改写时不得产生新备份"
    );
    assert_eq!(second["connectors"]["backup"], serde_json::Value::Null);
}
