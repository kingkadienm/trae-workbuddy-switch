//! 把旧「产品线」账号库并入新的「区域」账号库（**一次性迁移**）。
//!
//! ## 背景
//!
//! 改造前账号库按**产品线**分家，于是国内被拆成两本库：
//!
//! - `checkin_accounts.json`（旧默认变体 `TraeWork`＝国内 TraeWork）
//! - `checkin_accounts.trae_cn.json`（旧变体 `Trae`＝国内 TraeCode）
//!
//! 而区域模型下国内只有**一本**库（两条程序共用同一套账号体系，见
//! [`super::region`]）。因此必须把 `.trae_cn` 那本并进 `checkin_accounts.json`。
//!
//! ## 三条硬约束
//!
//! 1. **只增不改**：并入的记录**绝不覆盖** CN 库里已有的同 uid 记录
//!    （冲突以现有值为准）。迁移是"把漏掉的捡起来"，不是"用旧的换掉新的"。
//! 2. **旧文件原样保留**：本模块**不删、不改**任何 `.trae_cn` 文件。它们是这次
//!    改写的天然备份，也是「切换前的旧版本程序仍能正常运行」的前提 ——
//!    迁移与读侧切轴是两件事，前者先落地不会让当前版本出现任何行为变化。
//! 3. **可重复执行**：并集语义天然幂等；第二次跑不会新增任何记录，
//!    也不会产生第二份备份（见 [`MergeReport::backup`]）。
//!
//! ## 备份纪律（与仓库既有约定一致）
//!
//! 只在**同时**满足「目标原本存在」且「本次实际发生改写」时才落备份 ——
//! 缺任一都会产生垃圾（空目录 / 无意义的副本）。备份按**用途 + UTC 时间戳**分层：
//! `trae/backups/region-merge/<utc>/<原文件名>`。备份失败**不阻断**写入，
//! 但报告里 `backup` 置 `null`（区分「无需备份」与「备份失败」）。
//!
//! ## 不并哪些东西
//!
//! **导出/导入的交换文件不在此列**（它们的键名契约独立，见 `export_import`）。
//! **登录态快照也不需要并**：`profiles_trae_cn/` 是**程序级**目录，在新模型里
//! 恰好就是「国内 × TraeCode」那一格（见 `paths::profiles_dir_for_program`），
//! 原地有效 —— 这正是把快照按程序分家换来的好处。

use std::collections::HashMap;

use serde_json::json;

use super::account::{self, AccountsFile, Group, GroupsFile};
use super::paths;
use super::region::TraeRegion;
use super::variant::TraeVariant;

/// 迁移结果（报告形态，前端/日志直接可读）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// 旧库里的账号总数。
    pub legacy_accounts: usize,
    /// 本次并入 CN 库的账号数。
    pub accounts_added: usize,
    /// 因 CN 库已有同 uid 而**保留现有值**的账号数。
    pub accounts_kept: usize,
    /// 并入的设备绑定数。
    pub bindings_added: usize,
    /// 并入的分组数（按名字去重）。
    pub groups_added: usize,
    /// 并入的成员关系数。
    pub membership_added: usize,
    /// 备份路径；`None` = 没有发生改写（无需备份）或备份失败（详见 `backup_failed`）。
    pub backup: Option<String>,
    /// 备份是否尝试过但失败（把「无需备份」与「备份失败」在报告里分开）。
    pub backup_failed: bool,
}

impl MergeReport {
    /// 是否什么都没做（用于日志只打一行）。
    pub fn is_noop(&self) -> bool {
        self.accounts_added == 0
            && self.bindings_added == 0
            && self.groups_added == 0
            && self.membership_added == 0
    }

    /// 线上形态（camelCase，与其余 Trae 命令一致）。
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "legacyAccounts": self.legacy_accounts,
            "accountsAdded": self.accounts_added,
            "accountsKept": self.accounts_kept,
            "bindingsAdded": self.bindings_added,
            "groupsAdded": self.groups_added,
            "membershipAdded": self.membership_added,
            "backup": self.backup,
            "backupFailed": self.backup_failed,
            "changed": !self.is_noop(),
        })
    }
}

/// 旧库文件路径：把区域库路径的文件名插入 `.trae_cn` 中缀。
///
/// ⚠️ **不能**再用 `paths::*_for(TraeVariant::Trae)` 取旧库：持久化轴翻到区域后，
/// 那个函数返回的**就是国内区域的路径**（两个旧变体共用一本库），拿它当"旧库"
/// 会读到目标库自身 —— 迁移会变成空转，且"旧文件原样保留"这条约束也没了对象。
/// 旧库是**历史文件**，不是某个"变体"的文件，因此这里按文件名显式构造。
fn legacy_path_for(region_path: &std::path::Path) -> std::path::PathBuf {
    let name = region_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let legacy = match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}.trae_cn.{ext}"),
        None => format!("{name}.trae_cn"),
    };
    region_path.with_file_name(legacy)
}

/// 把旧产品线库并入 CN 区域库。**幂等**，可安全重复调用。
///
/// 旧库不存在时直接返回空报告（不建库、不备份）：新装用户与从未用过
/// TraeCode 线的用户都会走到这条分支。
pub fn merge_legacy_cn_into_region() -> Result<MergeReport, String> {
    let legacy_accounts_path = legacy_path_for(&paths::accounts_file_for_region(TraeRegion::Cn));
    let legacy_groups_path = legacy_path_for(&paths::groups_file_for_region(TraeRegion::Cn));
    let target_accounts_path = paths::accounts_file_for_region(TraeRegion::Cn);
    let target_groups_path = paths::groups_file_for_region(TraeRegion::Cn);

    // 用「能否读到内容」判断存在性，而不是 `Path::exists()` ——
    // 本工作区的元数据探测偶发假报（见 `.workbuddy/memory/MEMORY-2-tooling.md`）。
    let legacy_raw = std::fs::read_to_string(&legacy_accounts_path).ok();
    if legacy_raw.is_none() {
        return Ok(MergeReport::default());
    }

    // 旧库解析失败时**不猜**：宁可报错让调用方决定，也不要把半份数据并进去
    // （一旦并入就无法反向区分"本来就这些"和"解析漏了"）。
    let legacy: AccountsFile = serde_json::from_str(legacy_raw.as_deref().unwrap_or(""))
        .map_err(|e| format!("旧账号库解析失败（{}）：{e}", legacy_accounts_path.display()))?;

    // 目标库原本是否存在 —— 决定"要不要备份"的一半条件。
    let target_existed = std::fs::read_to_string(&target_accounts_path).is_ok();
    let mut target: AccountsFile = account::load_accounts_for(TraeVariant::TraeWork);

    let mut report = MergeReport {
        legacy_accounts: legacy.accounts.len(),
        ..Default::default()
    };

    // ---- 账号：只增不改 ----
    for record in &legacy.accounts {
        let uid = account::resolve_user_id(record);
        let already = !uid.is_empty()
            && target
                .accounts
                .iter()
                .any(|existing| account::resolve_user_id(existing) == uid);
        if already {
            report.accounts_kept += 1;
            continue;
        }
        target.accounts.push(record.clone());
        report.accounts_added += 1;
    }

    // ---- 设备绑定：并集，目标优先 ----
    // 绑定是**本机绑定**：同一个 uid 在两条产品线上各有一份，语义上"这个账号在这条
    // 线上用的设备"。并进区域库时目标已有值就保留目标值（目标＝国内 TraeWork，
    // 是用户实际在用的那条线）。
    for (uid, device_id) in &legacy.device_bindings {
        if target.device_bindings.contains_key(uid) {
            continue;
        }
        target.device_bindings.insert(uid.clone(), device_id.clone());
        report.bindings_added += 1;
    }

    // ---- 分组与成员关系：按名字去重，保留目标的 id ----
    let legacy_groups: GroupsFile = std::fs::read_to_string(&legacy_groups_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let mut target_groups: GroupsFile = account::load_groups_for(TraeVariant::TraeWork);
    // 旧分组 id 可能撞上目标库里**同名以外**的分组 → 撞了就换一个新 id，
    // 绝不覆盖既有分组（否则会静默改变用户已有的分组定义）。
    let mut id_remap: HashMap<String, String> = HashMap::new();
    for group in &legacy_groups.groups {
        let existing_same_name = target_groups
            .groups
            .iter()
            .find(|candidate| candidate.name == group.name)
            .map(|candidate| candidate.id.clone());
        match existing_same_name {
            Some(id) => {
                id_remap.insert(group.id.clone(), id);
            }
            None => {
                let id_taken = target_groups
                    .groups
                    .iter()
                    .any(|candidate| candidate.id == group.id);
                let id = if id_taken {
                    format!("g-{}", uuid::Uuid::new_v4().simple())
                } else {
                    group.id.clone()
                };
                id_remap.insert(group.id.clone(), id.clone());
                target_groups.groups.push(Group {
                    id,
                    name: group.name.clone(),
                    color: group.color.clone(),
                    order: group.order,
                });
                report.groups_added += 1;
            }
        }
    }
    for (uid, group_id) in &legacy_groups.membership {
        if target_groups.membership.contains_key(uid) {
            continue;
        }
        let Some(mapped) = id_remap.get(group_id) else {
            // 成员指向一个旧库里不存在的分组 → 不并（悬空成员比没有更坏）。
            continue;
        };
        target_groups
            .membership
            .insert(uid.clone(), mapped.clone());
        report.membership_added += 1;
    }

    if report.is_noop() {
        return Ok(report);
    }

    // ---- 备份（仅当目标原本存在且确实要改写）----
    if target_existed {
        match write_backup(&target_accounts_path, &target_groups_path) {
            Ok(path) => report.backup = Some(path),
            Err(_) => report.backup_failed = true,
        }
    }

    // 写回走**区域版**接口：它内部用 `paths::accounts_file_for_region(Cn)`，
    // 与迁移的读取目标同源，避免这里再拼一次路径（拼错会写进另一本库）。
    account::save_accounts_for_region(TraeRegion::Cn, &target)
        .map_err(|e| format!("写入 CN 账号库失败：{e}"))?;
    if report.groups_added > 0 || report.membership_added > 0 {
        account::save_groups_for(TraeVariant::TraeWork, &target_groups)
            .map_err(|e| format!("写入 CN 分组文件失败：{e}"))?;
    }
    Ok(report)
}

/// 备份目标库文件到 `trae/backups/region-merge/<utc>/`，返回目录路径。
fn write_backup(accounts_path: &std::path::Path, groups_path: &std::path::Path) -> Result<String, String> {
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let dir = paths::trae_dir().join("backups").join("region-merge").join(&stamp);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for source in [accounts_path, groups_path] {
        if !source.is_file() {
            continue;
        }
        let name = source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("无法取得文件名：{}", source.display()))?;
        std::fs::copy(source, dir.join(name)).map_err(|e| e.to_string())?;
    }
    Ok(dir.to_string_lossy().to_string())
}

/// 旧库里还有多少条账号**尚未**进入 CN 库（0 表示迁移已完成）。
///
/// 供调用方判断「要不要提示用户迁移」——它读的是旧库，
/// 因此即使一次都没执行过迁移也能给出准确答案。
pub fn pending_legacy_accounts() -> Result<usize, String> {
    let legacy_path = legacy_path_for(&paths::accounts_file_for_region(TraeRegion::Cn));
    let Some(text) = std::fs::read_to_string(&legacy_path).ok() else {
        return Ok(0);
    };
    let legacy: AccountsFile = serde_json::from_str(&text)
        .map_err(|e| format!("旧账号库解析失败（{}）：{e}", legacy_path.display()))?;
    let target = account::load_accounts_for(TraeVariant::TraeWork);
    Ok(legacy
        .accounts
        .iter()
        .filter(|record| {
            let uid = account::resolve_user_id(record);
            uid.is_empty()
                || !target
                    .accounts
                    .iter()
                    .any(|existing| account::resolve_user_id(existing) == uid)
        })
        .count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::config::HomeOverrideGuard;

    /// 建一个隔离 home，并返回 guard（测试纪律：不得写用户真实数据目录）。
    fn isolated_home(tag: &str) -> (std::path::PathBuf, HomeOverrideGuard) {
        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-region-merge-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时 home 应能创建");
        let guard = HomeOverrideGuard::set(&dir);
        (dir, guard)
    }

    fn legacy_account(uid: &str, name: &str) -> account::RawAccount {
        account::RawAccount {
            name: name.into(),
            user_id: Some(uid.into()),
            jwt: format!("Cloud-IDE-JWT {uid}"),
            refresh_token: Some(format!("rt-{uid}")),
            added_at: Some("2026-09-01T00:00:00Z".into()),
            updated_at: None,
        }
    }

    fn write_legacy(accounts: Vec<account::RawAccount>, groups: GroupsFile) {
        let file = AccountsFile {
            accounts,
            device_bindings: HashMap::from([("u-legacy".to_string(), "dev-legacy".to_string())]),
            web_devices: std::collections::HashSet::new(),
        };
        store_write(&legacy_path_for(&paths::accounts_file_for_region(TraeRegion::Cn)), &file);
        store_write(&legacy_path_for(&paths::groups_file_for_region(TraeRegion::Cn)), &groups);
    }

    fn store_write<T: serde::Serialize>(path: &std::path::Path, value: &T) {
        crate::modules::trae::store::write_json(path, value).expect("写测试文件应成功");
    }

    fn load_target() -> AccountsFile {
        account::load_accounts_for(TraeVariant::TraeWork)
    }

    /// 核心承诺：旧库账号进了 CN 库，**且旧文件原样保留**。
    #[test]
    fn 迁移把旧库账号并入cn库且不动旧文件() {
        let (dir, guard) = isolated_home("basic");
        // ★ 先 seed 一份**已存在**的 CN 账号库（真实场景：用户一直在用国内 TraeWork 线）。
        // 这是「要不要备份」的两个条件之一 —— 不 seed 就根本测不到备份分支。
        let cn_seed = AccountsFile {
            accounts: vec![legacy_account("u-cn", "主号")],
            device_bindings: HashMap::new(),
            web_devices: std::collections::HashSet::new(),
        };
        store_write(&paths::accounts_file_for_region(TraeRegion::Cn), &cn_seed);
        write_legacy(
            vec![legacy_account("u-legacy", "小号 A"), legacy_account("u-2", "小号 B")],
            GroupsFile::default(),
        );
        let legacy_before =
            std::fs::read_to_string(legacy_path_for(&paths::accounts_file_for_region(TraeRegion::Cn))).unwrap();

        let report = merge_legacy_cn_into_region().expect("迁移应成功");

        assert_eq!(report.legacy_accounts, 2);
        assert_eq!(report.accounts_added, 2);
        assert_eq!(report.accounts_kept, 0);
        assert_eq!(report.bindings_added, 1);
        assert!(report.backup.is_some(), "目标原本存在且发生了改写 → 必须有备份");

        let target = load_target();
        let mut uids: Vec<String> = target
            .accounts
            .iter()
            .map(|record| account::resolve_user_id(record))
            .collect();
        uids.sort();
        assert_eq!(
            uids,
            vec!["u-2".to_string(), "u-cn".to_string(), "u-legacy".to_string()],
            "原有的 CN 账号必须留着，旧库两条并进来"
        );
        assert_eq!(
            target.device_bindings.get("u-legacy").map(String::as_str),
            Some("dev-legacy")
        );
        // ★ 旧文件必须逐字未动。
        assert_eq!(
            std::fs::read_to_string(legacy_path_for(&paths::accounts_file_for_region(TraeRegion::Cn))).unwrap(),
            legacy_before,
            "迁移不得改写旧库文件"
        );

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 冲突时**以 CN 库现有值为准**：迁移不是"用旧的换掉新的"。
    #[test]
    fn 同名uid冲突时保留cn库现有值() {
        let (dir, guard) = isolated_home("conflict");
        // 目标库里已有 u-1（JWT 是新的），旧库里 u-1 是旧的。
        let target_seed = AccountsFile {
            accounts: vec![account::RawAccount {
                name: "主号".into(),
                user_id: Some("u-1".into()),
                jwt: "Cloud-IDE-JWT NEW".into(),
                refresh_token: Some("rt-new".into()),
                added_at: Some("2026-09-20T00:00:00Z".into()),
                updated_at: None,
            }],
            device_bindings: HashMap::new(),
            web_devices: std::collections::HashSet::new(),
        };
        store_write(&paths::accounts_file_for_region(TraeRegion::Cn), &target_seed);
        write_legacy(vec![legacy_account("u-1", "旧主号")], GroupsFile::default());

        let report = merge_legacy_cn_into_region().expect("迁移应成功");

        assert_eq!(report.accounts_added, 0);
        assert_eq!(report.accounts_kept, 1);
        let target = load_target();
        assert_eq!(target.accounts.len(), 1);
        assert_eq!(account::resolve_user_id(&target.accounts[0]), "u-1");
        assert_eq!(target.accounts[0].jwt, "Cloud-IDE-JWT NEW", "不得被旧值覆盖");
        assert_eq!(target.accounts[0].name, "主号");
        // 账号一条没加，但旧库那条**目标库里没有**的设备绑定确实并了进来 ——
        // 绑定/分组的变化同样是改写，所以这里应当有备份。
        // （`is_noop` 只看四个计数，正是为此：不能只看"账号加了几条"。）
        assert_eq!(report.bindings_added, 1);
        assert!(report.backup.is_some(), "绑定并入也算改写 → 必须备份");

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 幂等：跑第二次不再新增、不再备份。
    #[test]
    fn 迁移可重复执行且第二次无副作用() {
        let (dir, guard) = isolated_home("idempotent");
        // 同「基础」用例：目标库已存在，否则第一次跑就没有备份分支可测。
        store_write(
            &paths::accounts_file_for_region(TraeRegion::Cn),
            &AccountsFile {
                accounts: vec![legacy_account("u-cn", "主号")],
                device_bindings: HashMap::new(),
                web_devices: std::collections::HashSet::new(),
            },
        );
        write_legacy(vec![legacy_account("u-x", "X")], GroupsFile::default());

        let first = merge_legacy_cn_into_region().expect("首次迁移应成功");
        assert_eq!(first.accounts_added, 1);
        assert!(first.backup.is_some());

        let second = merge_legacy_cn_into_region().expect("二次迁移应成功");
        assert_eq!(second.accounts_added, 0);
        assert_eq!(second.accounts_kept, 1);
        assert!(second.is_noop());
        assert!(second.backup.is_none(), "第二次无改写 → 不应再备份");
        assert_eq!(load_target().accounts.len(), 2, "seed 的 u-cn 与并入的 u-x");

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 没装过旧产品线（旧库不存在）时：什么都不做、不建库、不报错。
    #[test]
    fn 旧库不存在时什么都不做() {
        let (dir, guard) = isolated_home("missing");

        let report = merge_legacy_cn_into_region().expect("缺失旧库不应报错");
        assert_eq!(report, MergeReport::default());
        assert!(report.backup.is_none());
        // 不得凭空创建 CN 账号库文件。
        assert!(!paths::accounts_file_for_region(TraeRegion::Cn).is_file());
        assert_eq!(pending_legacy_accounts().unwrap(), 0);

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 旧库存在内容损坏时必须**报错**而不是猜（宁可失败也不并半份数据）。
    #[test]
    fn 旧库损坏时报错而不是静默跳过() {
        let (dir, guard) = isolated_home("broken");
        // 用裸 `std::fs::write` 写"坏文件"：`store::write_json` 只收可序列化的值，
        // 写不出语法非法的 JSON。裸写前必须自己把父目录建出来。
        let _ = std::fs::create_dir_all(paths::trae_dir());
        std::fs::write(legacy_path_for(&paths::accounts_file_for_region(TraeRegion::Cn)), "{不是数组")
            .expect("写坏文件应成功");

        let err = merge_legacy_cn_into_region().expect_err("损坏的旧库必须报错");
        assert!(err.contains("解析失败"), "错误信息应指明原因：{err}");

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 分组：同名去重、成员关系跟着重映射；指向不存在分组的成员不并。
    #[test]
    fn 分组按名字去重且成员关系重映射() {
        let (dir, guard) = isolated_home("groups");
        let target_groups = GroupsFile {
            groups: vec![Group {
                id: "g-main".into(),
                name: "主力".into(),
                color: "#111".into(),
                order: 0,
            }],
            membership: HashMap::from([("u-cn".to_string(), "g-main".to_string())]),
        };
        store_write(&paths::groups_file_for(TraeVariant::TraeWork), &target_groups);
        write_legacy(
            vec![legacy_account("u-a", "A"), legacy_account("u-b", "B")],
            GroupsFile {
                groups: vec![
                    // 同名不同 id → 必须复用目标的 id，不新建。
                    Group {
                        id: "g-legacy-same".into(),
                        name: "主力".into(),
                        color: "#222".into(),
                        order: 5,
                    },
                    // 新名字 → 并进来。
                    Group {
                        id: "g-new".into(),
                        name: "备用".into(),
                        color: "#333".into(),
                        order: 7,
                    },
                ],
                membership: HashMap::from([
                    ("u-a".to_string(), "g-legacy-same".to_string()),
                    ("u-b".to_string(), "g-new".to_string()),
                    // 指向不存在的分组 → 不并。
                    ("u-c".to_string(), "g-missing".to_string()),
                ]),
            },
        );

        let report = merge_legacy_cn_into_region().expect("迁移应成功");

        assert_eq!(report.groups_added, 1, "只应新建「备用」一个分组");
        assert_eq!(report.membership_added, 2);
        let merged: GroupsFile = account::load_groups_for(TraeVariant::TraeWork);
        assert_eq!(merged.groups.len(), 2);
        assert_eq!(
            merged.membership.get("u-a").map(String::as_str),
            Some("g-main"),
            "同名分组必须重映射到目标的 id"
        );
        assert_eq!(merged.membership.get("u-b").map(String::as_str), Some("g-new"));
        assert!(!merged.membership.contains_key("u-c"), "悬空成员不得并入");

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
