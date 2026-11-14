//! Trae 账号导出/导入：解析导入文件、按 uid 去重合并、计数。
//!
//! 与 WorkBuddy 的 `modules::export_import` 同构（同一套「勾选 → 导出 JSON 数组 →
//! 导入时按 uid 去重合并」的交互契约），差异只在**账号记录形状**：
//! Trae 的记录是 `RawAccount`（含 `UserID` / `jwt` / `refresh_token`），
//! 而 WorkBuddy 的记录是自由形态的 `Value`。
//!
//! ## 与参考实现的兼容（不可动摇）
//!
//! 导出文件里的键名**必须是持久化形态**（大写 `UserID`），不能是线上 camelCase：
//! 参考实现的 `device_proxy.py` 与用户的既有备份都按持久化形状读写，
//! 导出成 `user_id` 会让文件在其他工具里读不出归属。
//! 因此这里全程用 `RawAccount` 的 serde 表示，**不手工拼 `json!`**。
//!
//! 纯逻辑（`parse_accounts_json` / `merge_import_records` / `select_export_records`）
//! 不碰文件系统，便于单测；文件读写只在 `export_accounts` / `import_accounts`。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::modules::trae::account::{self, RawAccount};
use crate::modules::trae::variant::TraeVariant;

/// 导入结果计数（与 WorkBuddy 的 `ImportResult` 同名同义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportResult {
    /// 成功合入账号库的数量（含覆盖与新增）。
    pub imported: usize,
    /// 未导入的数量（缺 jwt / 索引越界）。
    pub skipped: usize,
    /// 其中覆盖了同 uid 本地账号的数量。
    pub overwritten: usize,
}

/// 解析导出/导入文件文本：必须是 JSON 数组，且每项为 JSON 对象。
///
/// 失败时返回带位置的明确错误文案（非法 JSON / 非数组 / 元素不是对象）。
pub fn parse_accounts_json(text: &str) -> Result<Vec<Value>, String> {
    if text.trim().is_empty() {
        return Err("文件内容为空".to_string());
    }
    let parsed: Value =
        serde_json::from_str(text).map_err(|e| format!("文件不是合法的 JSON：{e}"))?;
    let array = parsed
        .as_array()
        .ok_or_else(|| "文件内容应为 JSON 数组（账号列表）".to_string())?;
    for (index, item) in array.iter().enumerate() {
        if !item.is_object() {
            return Err(format!("文件第 {} 项不是合法的账号对象", index + 1));
        }
    }
    Ok(array.clone())
}

/// 从一条导入记录里取 jwt（容忍 `Cloud-IDE-JWT ` 前缀与空白）。
fn record_jwt(item: &Value) -> Option<String> {
    item.get("jwt")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 从一条导入记录里取 uid：优先大写 `UserID`，回落小写 `user_id`。
///
/// 小写分支是**对外部导出文件的容错**：用户可能从别的工具导出成 camelCase，
/// 不该因此整条丢弃。
fn record_user_id(item: &Value) -> Option<String> {
    ["UserID", "user_id"].iter().find_map(|key| {
        item.get(*key)
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// 生成导入文件的脱敏预览（含文件内索引，不含 JWT 明文）。
pub fn preview_accounts(text: &str) -> Result<Value, String> {
    let array = parse_accounts_json(text)?;
    let items: Vec<Value> = array
        .iter()
        .enumerate()
        .map(|(index, item)| {
            json!({
                "index": index,
                "userId": record_user_id(item),
                "name": item.get("name").and_then(|value| value.as_str()),
                "hasJwt": record_jwt(item).is_some(),
                "hasRefreshToken": item
                    .get("refresh_token")
                    .and_then(|value| value.as_str())
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false),
            })
        })
        .collect();
    Ok(json!({ "accounts": items, "total": array.len() }))
}

/// 单条导入记录的合并动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeOutcome {
    /// 追加为新账号。
    Appended,
    /// 覆盖同 uid 的本地账号。
    Overwritten,
    /// 缺少 jwt，跳过。
    Skipped,
}

/// 纯函数：把一条导入记录合并进账号列表。
///
/// 按 uid 去重：同 uid 覆盖（以导入记录为准）；无法确定 uid 时按追加处理——
/// Trae 的 uid 可从 JWT 现算，追加后 `entries()` 仍能解析出归属，不会变成孤儿记录。
fn merge_import_record(accounts: &mut Vec<RawAccount>, item: &Value) -> MergeOutcome {
    if record_jwt(item).is_none() {
        return MergeOutcome::Skipped;
    }
    // 反序列化成持久化形态：未知字段被忽略、缺失字段走 `#[serde(default)]`，
    // 与账号库自身的容错策略一致（见 `account::RawAccount` 的 docblock）。
    let incoming: RawAccount = match serde_json::from_value(item.clone()) {
        Ok(record) => record,
        Err(_) => return MergeOutcome::Skipped,
    };
    let incoming_uid = account::resolve_user_id(&incoming);
    if !incoming_uid.is_empty() {
        if let Some(existing) = accounts
            .iter_mut()
            .find(|record| account::resolve_user_id(record) == incoming_uid)
        {
            let mut replaced = incoming;
            // 导入记录缺 UserID 时保留本地值：分组、设备映射、积分历史都按 uid 索引，
            // 抹掉这个键会让它们全部断链。
            if replaced
                .user_id
                .as_deref()
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
            {
                replaced.user_id = existing.user_id.clone();
            }
            // 同为「缺乏」语义的字段也做一次保留，避免导入低配版本文件时丢信息。
            if replaced.added_at.is_none() {
                replaced.added_at = existing.added_at.clone();
            }
            // `updated_at` 与 `added_at` 同款：导入文件不带它时**不能**把本地的
            // 已知值抹成 `null` —— 卡片页脚的「最近更新」会因此变成「未记录更新时间」，
            // 而用户刚刚并没有更新过任何东西（2026-10-01 实测复现）。
            if replaced.updated_at.is_none() {
                replaced.updated_at = existing.updated_at.clone();
            }
            // 备注是**纯本地用户输入**：参考实现产出的文件、或旧版本导出的文件里
            // 根本没有这个键。`*existing = replaced` 会把它整条抹掉 —— 那是用户
            // 自己写的东西，丢了不会有任何提示。与 `added_at` 同一条规则。
            if replaced.remark.is_none() {
                replaced.remark = existing.remark.clone();
            }
            *existing = replaced;
            return MergeOutcome::Overwritten;
        }
    }
    accounts.push(incoming);
    MergeOutcome::Appended
}

/// 纯函数：解析文件文本并按选中索引把记录合并进账号列表，返回计数。
///
/// 不做文件读写，便于单测。索引越界视为跳过。
pub fn merge_import_records(
    accounts: &mut Vec<RawAccount>,
    text: &str,
    indexes: &[usize],
) -> Result<ImportResult, String> {
    let array = parse_accounts_json(text)?;
    let mut result = ImportResult::default();
    for &index in indexes {
        match array.get(index) {
            None => result.skipped += 1,
            Some(item) => match merge_import_record(accounts, item) {
                MergeOutcome::Appended => result.imported += 1,
                MergeOutcome::Overwritten => {
                    result.imported += 1;
                    result.overwritten += 1;
                }
                MergeOutcome::Skipped => result.skipped += 1,
            },
        }
    }
    Ok(result)
}

/// 导入：读账号库 → 合并 → 写回，返回计数（默认变体，兼容壳）。
pub fn import_accounts(text: &str, indexes: &[usize]) -> Result<ImportResult, String> {
    import_accounts_for(TraeVariant::default(), text, indexes)
}

/// 导入（按变体分家）：读该变体账号库 → 合并 → 写回，返回计数。
///
/// **必须分家**：导入文件可能来自任一条产品线，合并进哪一本账号库
/// 决定了账号归属。混写会让导入的账号出现在错误的产品线分区里。
pub fn import_accounts_for(
    variant: TraeVariant,
    text: &str,
    indexes: &[usize],
) -> Result<ImportResult, String> {
    let mut file = account::load_accounts_for(variant);
    let result = merge_import_records(&mut file.accounts, text, indexes)?;
    // ★ 必须**整体**回存 `file`，不得字面重建 `AccountsFile { accounts: file.accounts }`。
    //
    // 两层理由：
    // 1. 冗余 —— `merge_import_records` 已**就地**改了 `file.accounts`，重建不改变任何行为；
    // 2. 危险 —— `save_accounts_for` 序列化的是**整个** `AccountsFile`，所以一旦该结构
    //    新增第二个容器键，字面重建就会把那个键**静默清空**：用户只是导入一份 JSON，
    //    另一份数据却没了，且没有任何报错。
    //    这不是假想风险：同族的 `GroupsFile`（`account.rs`）已经是 `groups` + `membership`
    //    双键结构，`AccountsFile` 目前单键只是**当前**状态，不是承诺。
    account::save_accounts_for(variant, &file).map_err(|e| format!("保存账号库失败：{e}"))?;
    Ok(result)
}

/// 纯函数：从账号列表中挑出 userId 命中的完整记录（含 JWT）。
///
/// 入参是**已解析 uid 的 (uid, 记录)** 对：调用方负责解析，
/// 避免这里重复实现 `resolve_user_id` 的回落链。
pub fn select_export_records(
    accounts: &[(String, RawAccount)],
    user_ids: &[String],
) -> Result<Vec<RawAccount>, String> {
    let wanted: Vec<&str> = user_ids
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect();
    if wanted.is_empty() {
        return Err("请先选择要导出的账号".to_string());
    }
    let mut exported: Vec<RawAccount> = Vec::new();
    for uid in wanted {
        if let Some((_, record)) = accounts.iter().find(|(id, _)| id == uid) {
            exported.push(record.clone());
        }
    }
    if exported.is_empty() {
        return Err("未找到要导出的账号".to_string());
    }
    Ok(exported)
}

/// 导出：按 userId 列表返回完整记录（含 JWT；默认变体，兼容壳）。
///
/// 序列化时逐条转成 `Value`，**保留持久化键名**（大写 `UserID`）。
pub fn export_accounts(user_ids: &[String]) -> Result<Vec<Value>, String> {
    export_accounts_for(TraeVariant::default(), user_ids)
}

/// 导出（按变体分家）：按 userId 列表返回完整记录（含 JWT）。
pub fn export_accounts_for(
    variant: TraeVariant,
    user_ids: &[String],
) -> Result<Vec<Value>, String> {
    let records = select_export_records(&account::entries_for(variant), user_ids)?;
    records
        .iter()
        .map(|record| serde_json::to_value(record).map_err(|e| e.to_string()))
        .collect()
}

/// 校验导出目标路径：必须是绝对路径且以 `.json` 结尾（保存对话框产物）。
fn validate_export_path(path: &str) -> Result<(), String> {
    let p = Path::new(path.trim());
    if !p.is_absolute() {
        return Err("导出路径必须是绝对路径".to_string());
    }
    if !p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
    {
        return Err("导出文件名必须以 .json 结尾".to_string());
    }
    Ok(())
}

/// 导出：按 userId 列表把完整记录写入用户选择的路径（保存对话框产物），返回该路径。
///
/// 默认变体，兼容壳。
pub fn export_accounts_to_path(user_ids: &[String], path: &str) -> Result<String, String> {
    export_accounts_to_path_for(TraeVariant::default(), user_ids, path)
}

/// 导出（按变体分家）：按 userId 列表把完整记录写入用户选择的路径，返回该路径。
pub fn export_accounts_to_path_for(
    variant: TraeVariant,
    user_ids: &[String],
    path: &str,
) -> Result<String, String> {
    validate_export_path(path)?;
    let records = export_accounts_for(variant, user_ids)?;
    let path_buf = PathBuf::from(path.trim());
    let content = serde_json::to_string_pretty(&records).map_err(|e| e.to_string())?;
    crate::modules::config::atomic_write(&path_buf, &content)
        .map_err(|e| format!("写入导出文件失败：{e}"))?;
    Ok(path_buf.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(uid: &str, name: &str, jwt: &str) -> RawAccount {
        RawAccount {
            name: name.to_string(),
            user_id: if uid.is_empty() {
                None
            } else {
                Some(uid.to_string())
            },
            jwt: jwt.to_string(),
            refresh_token: None,
            added_at: Some("2026-09-01T00:00:00Z".to_string()),
            updated_at: None,
            remark: None,
        }
    }

    #[test]
    fn parse_rejects_empty_invalid_and_non_array() {
        assert!(parse_accounts_json("").is_err());
        assert!(parse_accounts_json("not json").is_err());
        assert!(parse_accounts_json(r#"{ "a": 1 }"#).is_err());
    }

    #[test]
    fn parse_rejects_non_object_element() {
        let err = parse_accounts_json(r#"[{ "jwt": "j" }, 42]"#).unwrap_err();
        assert!(err.contains("第 2 项"), "错误应带位置：{err}");
    }

    #[test]
    fn parse_accepts_object_array() {
        let parsed = parse_accounts_json(r#"[{ "jwt": "j" }, {}]"#).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn merge_appends_new_and_skips_without_jwt() {
        let mut accounts = vec![raw("u1", "主号", "jwt-1")];
        let text = r#"[
            { "UserID": "u2", "name": "小号", "jwt": "jwt-2" },
            { "UserID": "u3", "name": "无凭据", "jwt": "" }
        ]"#;
        let result = merge_import_records(&mut accounts, text, &[0, 1]).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.overwritten, 0);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[1].user_id.as_deref(), Some("u2"));
    }

    #[test]
    fn merge_overwrites_same_uid_and_preserves_local_uid_when_absent() {
        let mut accounts = vec![raw("u1", "旧名", "old-jwt")];
        // 导入记录不带 UserID，但 JWT 的 uid 与本地一致时按覆盖处理。
        let text = r#"[{ "name": "新名", "jwt": "new-jwt" }]"#;
        let result = merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(result.overwritten, 0, "无 uid 无法判定同源，按追加处理");
        assert_eq!(accounts.len(), 2);

        // 带 UserID 时覆盖，且旧 uid 不丢。
        let text = r#"[{ "UserID": "u1", "name": "覆盖名", "jwt": "newer-jwt" }]"#;
        let result = merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(result.overwritten, 1);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].name, "覆盖名");
        assert_eq!(accounts[0].jwt, "newer-jwt");
        assert_eq!(accounts[0].user_id.as_deref(), Some("u1"));
    }

    /// 备注是**纯本地用户输入**，导入文件通常不带它 —— 不能被 `*existing = replaced` 抹掉。
    ///
    /// 反面现场（WorkBuddy 侧同款实现踩过）：用户写好的备注在「导入账号」之后
    /// 无声消失，且没有任何提示。这条就是那个缺陷的护栏。
    #[test]
    fn merge_keeps_local_remark_when_import_record_lacks_it() {
        let mut local = raw("u1", "本地名", "old-jwt");
        local.remark = Some("本地备注".into());
        let mut accounts = vec![local];

        // ① 导入文件不带 remark（参考实现 / 旧版本导出的文件就是这样）⇒ 保留本地值。
        let text = r#"[{ "UserID": "u1", "name": "导入名", "jwt": "new-jwt" }]"#;
        let result = merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(result.overwritten, 1);
        assert_eq!(accounts[0].name, "导入名", "其余字段照常被覆盖");
        assert_eq!(
            accounts[0].remark.as_deref(),
            Some("本地备注"),
            "导入记录缺 remark 时不得抹掉本地备注"
        );

        // ② 导入文件**带** remark ⇒ 以导入值为准（换机器搬账号时备注要跟着走）。
        let text = r#"[{ "UserID": "u1", "jwt": "newer-jwt", "remark": "导入备注" }]"#;
        merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(accounts[0].remark.as_deref(), Some("导入备注"));
    }

    /// 导入文件不带时间戳时，本地的**已知值**不得被抹成 `null`。
    ///
    /// 现场（2026-10-01 真机实测）：导入一份没有 `updated_at` 的文件后，卡片页脚的
    /// 「最近更新」变成「未记录更新时间」—— 用户刚导入完，自己什么都没更新。
    /// 与 `added_at` / `remark` 同属「缺乏 ≠ 应当清空」。
    #[test]
    fn merge_keeps_local_timestamps_when_import_record_lacks_them() {
        let mut local = raw("u1", "本地名", "old-jwt");
        local.updated_at = Some("2026-09-30T10:00:00Z".into());
        let mut accounts = vec![local];

        // ① 缺 `updated_at` ⇒ 保留本地值（`added_at` 早已如此，见上一条规则）。
        let text = r#"[{ "UserID": "u1", "name": "导入名", "jwt": "new-jwt" }]"#;
        merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(accounts[0].name, "导入名", "其余字段照常被覆盖");
        assert_eq!(
            accounts[0].updated_at.as_deref(),
            Some("2026-09-30T10:00:00Z"),
            "导入记录缺 updated_at 时不得抹掉本地值"
        );
        assert_eq!(
            accounts[0].added_at.as_deref(),
            Some("2026-09-01T00:00:00Z")
        );

        // ② 带 `updated_at` ⇒ 以导入值为准（换机器搬账号时时间线要跟着走）。
        let text =
            r#"[{ "UserID": "u1", "jwt": "newer-jwt", "updated_at": "2026-10-01T00:00:00Z" }]"#;
        merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(
            accounts[0].updated_at.as_deref(),
            Some("2026-10-01T00:00:00Z")
        );
    }

    #[test]
    fn merge_tolerates_unknown_fields_and_lowercase_uid() {
        let mut accounts: Vec<RawAccount> = Vec::new();
        let text =
            r#"[{ "user_id": "u9", "jwt": "jwt-9", "future_field": 1, "schema_version": 99 }]"#;
        let result = merge_import_records(&mut accounts, text, &[0]).unwrap();
        // 未知字段被忽略（不 panic）；小写 `user_id` 不是持久化键，
        // 会被当作未知字段丢弃 —— 这是**有意的**：持久化契约只认大写 `UserID`，
        // 若这里也接受小写，就会出现「导入后键名被改写」的静默漂移。
        // 记录仍会入库，靠 JWT 现算 uid 兜底，不会变成孤儿。
        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 0);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].jwt, "jwt-9");

        // 大写 `UserID` 才被持久化接受。
        let mut strict: Vec<RawAccount> = Vec::new();
        let text = r#"[{ "UserID": "u9", "jwt": "jwt-9" }]"#;
        merge_import_records(&mut strict, text, &[0]).unwrap();
        assert_eq!(strict[0].user_id.as_deref(), Some("u9"));
    }

    #[test]
    fn merge_counts_out_of_range_index_as_skipped() {
        let mut accounts: Vec<RawAccount> = Vec::new();
        let result =
            merge_import_records(&mut accounts, r#"[{ "UserID": "u1", "jwt": "j" }]"#, &[5])
                .unwrap();
        assert_eq!(result.skipped, 1);
        assert_eq!(result.imported, 0);
        assert!(accounts.is_empty());
    }

    #[test]
    fn select_export_records_requires_selection_and_hits() {
        let accounts = vec![("u1".to_string(), raw("u1", "主号", "jwt-1"))];
        assert!(select_export_records(&accounts, &[]).is_err());
        assert!(select_export_records(&accounts, &["  ".to_string()]).is_err());
        assert!(select_export_records(&accounts, &["nope".to_string()]).is_err());
        let picked = select_export_records(&accounts, &["u1".to_string()]).unwrap();
        assert_eq!(picked.len(), 1);
    }

    #[test]
    fn export_keeps_reference_persistence_keys() {
        // 导出文件必须用大写 UserID：参考实现与既有备份都按持久化形状读取。
        let record = raw("7512345678901234567", "主号", "Cloud-IDE-JWT abc.def.ghi");
        let value = serde_json::to_value(&record).unwrap();
        let text = serde_json::to_string(&vec![value]).unwrap();
        assert!(
            text.contains("\"UserID\""),
            "导出必须保留大写 UserID：{text}"
        );
        assert!(
            !text.contains("\"user_id\""),
            "不得写出 snake_case 键：{text}"
        );
    }

    #[test]
    fn export_rejects_relative_and_non_json_paths() {
        assert!(validate_export_path("out.json").is_err());
        assert!(validate_export_path("C:\\tmp\\out.txt").is_err());
        // 绝对路径 + .json 才通过。注意 `/tmp/out.json` 在 Windows 上**不是**绝对路径
        // （无盘符），故用平台绝对路径构造，避免测试假设 Unix 语义。
        let abs = std::env::temp_dir().join("out.json");
        assert!(validate_export_path(&abs.to_string_lossy()).is_ok());
        // 相对路径即使扩展名对也必须拒绝。
        assert!(validate_export_path("sub/dir/out.json").is_err());
    }

    #[test]
    fn preview_hides_jwt_but_reports_presence() {
        let text = r#"[{ "UserID": "u1", "name": "主号", "jwt": "secret-token" }]"#;
        let preview = preview_accounts(text).unwrap();
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(
            !serialized.contains("secret-token"),
            "预览不得泄露 JWT：{serialized}"
        );
        assert_eq!(preview["accounts"][0]["hasJwt"], json!(true));
        assert_eq!(preview["accounts"][0]["userId"], json!("u1"));
    }

    /// ★【T13-4 ⑥ / I-1】导入一份 JSON **不得清空**本机的设备绑定。
    ///
    /// 这是 `AccountsFile.device_bindings`（G-b）能成立的**前提护栏**：
    /// `import_accounts_for` 一旦字面重建 `AccountsFile { accounts: … }`，
    /// 那个容器键就会被**静默清空** —— 用户只是导入一份 JSON，本机所有绑定消失，
    /// 续期集体退回「猜活跃目录」，且没有任何报错。
    ///
    /// 它与 [`merge_import_records`] 的签名（只收 `&mut Vec<RawAccount>`）是两重保险：
    /// 合并逻辑**结构上**碰不到容器键，而这条用例钉住**回存**那一步没把容器丢掉。
    #[test]
    fn import_accounts_keeps_device_bindings() {
        let temp =
            std::env::temp_dir().join(format!("buddy-switch-trae-bindings-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp);
        let _guard = crate::modules::config::HomeOverrideGuard::set(&temp);
        let variant = TraeVariant::TraeWork;

        // 先造两条绑定。
        let mut file = account::load_accounts_for(variant);
        file.accounts.push(raw("u1", "一号", "jwt-1"));
        file.device_bindings
            .insert("u1".to_string(), "dev-1".to_string());
        file.device_bindings
            .insert("u2".to_string(), "dev-2".to_string());
        account::save_accounts_for(variant, &file).expect("造绑定");

        // 导入一份**不含任何绑定字段**的 JSON。
        let text = r#"[{ "UserID": "u9", "name": "新账号", "jwt": "jwt-9" }]"#;
        let result = import_accounts_for(variant, text, &[0]).expect("导入应成功");
        assert_eq!(result.imported, 1);

        let after = account::load_accounts_for(variant);
        assert_eq!(
            after.device_bindings.len(),
            2,
            "导入后绑定**一条都不能少**，实际: {:?}",
            after.device_bindings
        );
        assert_eq!(
            after.device_bindings.get("u1").map(String::as_str),
            Some("dev-1")
        );
        assert_eq!(
            after.device_bindings.get("u2").map(String::as_str),
            Some("dev-2")
        );
        // 导入本身仍然生效（否则「绑定没丢」可能只是因为整个导入都没做事）。
        assert!(
            after
                .accounts
                .iter()
                .any(|account| account::resolve_user_id(account) == "u9"),
            "导入的账号应在库里"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }
}
