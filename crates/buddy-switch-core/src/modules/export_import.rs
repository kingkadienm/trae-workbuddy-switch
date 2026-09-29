//! 账号导出/导入：解析导入文件、按 uid 去重合并、计数。
//!
//! 纯逻辑（`parse_accounts_json` / `merge_import_records` / `select_export_records`）
//! 不依赖文件系统，便于无 UI 环境单测；`export_accounts` / `import_accounts`
//! 负责读写账号库（CN `~/.buddy-switch/accounts.json` / Global `accounts.global.json`）。
//!
//! **region 化**：导出/导入按 region 作用域（`*_for(region, …)`）；旧 CN 签名保留为
//! 薄包装，内部转调 `Region::Cn`，保证 P0-1「CN 行为零变化」。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::modules::account;
use crate::modules::auth_file;
use crate::modules::config::norm_ts;
use crate::modules::region::{region_of, Region};

/// 导入结果计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportResult {
    /// 成功合入账号库的数量（含覆盖与新增）。
    pub imported: usize,
    /// 未导入的数量（缺 access_token / 索引越界）。
    pub skipped: usize,
    /// 其中覆盖了同 uid 本地账号的数量。
    pub overwritten: usize,
}

/// 目录导入（panel `auths/`）结果计数：在 [`ImportResult`] 基础上多一个
/// 「域不符被跳过」的计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuthsDirImportResult {
    /// 成功合入账号库的数量（含覆盖与新增）。
    pub imported: usize,
    /// 缺 access_token 的数量。
    pub skipped: usize,
    /// 其中覆盖了同 uid 本地账号的数量。
    pub overwritten: usize,
    /// 域（realm）与目标 region 不符而被跳过的数量。
    pub mismatch: usize,
}

/// 解析导出/导入文件文本：必须是 JSON 数组，且每项为 JSON 对象。
///
/// 失败时返回带位置的明确错误文案（非法 JSON / 非数组 / 元素不是对象）。
///
/// 每项解析后做**嵌套形态归一**：workbuddy2api-panel 的 `auths/*.json`
/// （`{"auth":{...},"account":{...}}`）在数组里被内联成单项时，归一成
/// 账号库记录形态（对照 `auth_file::normalize_imported_account`）；
/// 扁平记录（本仓自己的导出格式）原样保留。
pub fn parse_accounts_json(text: &str) -> Result<Vec<Value>, String> {
    if text.trim().is_empty() {
        return Err("文件内容为空".to_string());
    }
    let parsed: Value =
        serde_json::from_str(text).map_err(|e| format!("文件不是合法的 JSON：{e}"))?;
    let array = parsed
        .as_array()
        .ok_or_else(|| "文件内容应为 JSON 数组（账号列表）".to_string())?;
    let mut items = Vec::with_capacity(array.len());
    for (index, item) in array.iter().enumerate() {
        if !item.is_object() {
            return Err(format!("文件第 {} 项不是合法的账号对象", index + 1));
        }
        items.push(normalize_import_item(item));
    }
    Ok(items)
}

/// 导入单项的形态归一：嵌套（顶层含 `auth`/`account` 对象且无顶层 `access_token`）
/// → 账号库记录形态；归一失败（缺 token 等）原样返回，由 [`merge_import_record`]
/// 按缺 token 跳过，错误文案口径不变。
pub fn normalize_import_item(item: &Value) -> Value {
    let nested = item.get("auth").map(|v| v.is_object()) == Some(true)
        || item.get("account").map(|v| v.is_object()) == Some(true);
    if !nested {
        return item.clone();
    }
    auth_file::normalize_imported_account(item.clone()).unwrap_or_else(|| item.clone())
}

/// 生成导入文件的脱敏预览（含文件内索引，不含 token）。
pub fn preview_accounts(text: &str) -> Result<Value, String> {
    let array = parse_accounts_json(text)?;
    let items: Vec<Value> = array
        .iter()
        .enumerate()
        .map(|(index, item)| {
            json!({
                "index": index,
                // 展示字段归一：导入文件来自外部，`nickname` 可能是对象（issue #2）。
                // 契约见 `ImportPreviewAccount`（`string | null`）。
                "uid": account::display_str(item, "uid"),
                "nickname": account::display_str(item, "nickname"),
                "email": account::display_str(item, "email"),
                "hasToken": account::get_str(item, "access_token").is_some(),
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
    /// 覆盖同 uid 的本地账号（保留导入记录原样）。
    Overwritten,
    /// 缺少 access_token，跳过。
    Skipped,
}

/// 纯函数：把一条导入记录合并进账号列表。
///
/// 按 uid 去重：同 uid 覆盖（保留导入记录原样）；uid 缺失或无法匹配则追加。
/// 缺少 access_token 的记录跳过，不进入账号库。
fn merge_import_record(accounts: &mut Vec<Value>, item: &Value) -> MergeOutcome {
    if account::get_str(item, "access_token").is_none() {
        return MergeOutcome::Skipped;
    }
    if let Some(uid) = account::get_str(item, "uid").as_deref() {
        if let Some(existing) = accounts
            .iter_mut()
            .find(|a| account::get_str(a, "uid").as_deref() == Some(uid))
        {
            let mut replaced = item.clone();
            // 导入记录缺 id 时保留本地 id：账号库不允许出现无 id 记录
            //（删除按 id、列表 key、导出选择都依赖 id）。
            if account::get_str(&replaced, "id").is_none() {
                if let Some(id) = existing.get("id").cloned() {
                    replaced["id"] = id;
                }
            }
            *existing = replaced;
            return MergeOutcome::Overwritten;
        }
    }
    accounts.push(item.clone());
    MergeOutcome::Appended
}

/// 纯函数：解析文件文本并按选中索引把记录合并进账号列表，返回计数。
///
/// 不做文件读写，便于单测。索引越界视为跳过。
pub fn merge_import_records(
    accounts: &mut Vec<Value>,
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

/// 导入（CN）：读账号库 → 合并 → 写回，返回计数。
pub fn import_accounts(text: &str, indexes: &[usize]) -> Result<ImportResult, String> {
    import_accounts_for(Region::Cn, text, indexes)
}

/// 按 region 导入：读该 region 账号库 → 合并 → 写回，返回计数。
pub fn import_accounts_for(
    region: Region,
    text: &str,
    indexes: &[usize],
) -> Result<ImportResult, String> {
    let mut accounts = account::load_accounts_for(region);
    let result = merge_import_records(&mut accounts, text, indexes)?;
    account::save_accounts_for(region, &accounts).map_err(|e| format!("保存账号库失败：{e}"))?;
    Ok(result)
}

/// 一个 panel `auths/*.json` 文件（顶层嵌套对象）归一后与目标 region 的判定。
#[derive(Debug)]
enum AuthsDirItem {
    /// 归一后的账号记录（与目标 region 同域、含 access_token）。
    Record(Value),
    /// 域（realm/domain）与目标 region 不符，跳过。
    Mismatch,
    /// 缺 access_token，跳过。
    Skipped,
}

/// 判定一条（嵌套形）panel auths 记录归属哪个 region：优先 `auth.realm`，
/// 回落 `auth.domain`（`region_of` 对空域归 CN，与账号库既有口径一致）。
///
/// 抽成纯函数便于单测钉住「realm 优先于 domain」的取数顺序。
fn auths_item_region(item: &Value) -> Region {
    let auth = item.get("auth");
    if let Some(realm) = auth
        .and_then(|a| a.get("realm"))
        .and_then(|v| v.as_str())
        .and_then(|r| Region::parse(r))
    {
        return realm;
    }
    let domain = auth
        .and_then(|a| a.get("domain"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    region_of(domain)
}

/// 把一个 panel `auths/*.json` 单项判定为该 region 的导入动作（纯函数，可测）：
/// 归一（嵌套形 → 账号库记录）→ 有 token？→ 域与目标 region 一致？→ 时间戳归一为毫秒。
///
/// 域不符的账号**宁可跳过**也不塞进错库（CN 号进 Global 库会让切换写回被
/// `RegionMismatch` 拦下，见 `region::RegionMismatch`）；缺 token 与既有导入同口径跳过。
///
/// 时间戳：panel 落盘用 Unix **秒**（`expiresAt` 为 10 位），本仓账号库存**毫秒**
/// （`now_ms()`）。归一时过一遍 `norm_ts`：秒 → 毫秒，毫秒（≥ `10^10`）原样保留，
/// 所以对「panel 文件」与「本仓自己导出的备份（已是毫秒）」都安全。
fn classify_auths_item(item: &Value, region: Region) -> AuthsDirItem {
    let mut record = normalize_import_item(item);
    if account::get_str(&record, "access_token").is_none() {
        return AuthsDirItem::Skipped;
    }
    if auths_item_region(item) != region {
        return AuthsDirItem::Mismatch;
    }
    // expiresAt / refreshExpiresAt 秒 → 毫秒（毫秒值不受影响）。
    for key in ["expiresAt", "refreshExpiresAt"] {
        if let Some(obj) = record.as_object_mut() {
            if let Some(value) = obj.get_mut(key) {
                if let Some(ms) = norm_ts(Some(value)) {
                    *value = json!(ms);
                }
            }
        }
    }
    AuthsDirItem::Record(record)
}

/// 导入 panel 的 `auths/` 目录：枚举目录下每个 `.json`（一个文件一个账号，
/// 嵌套形 `{"auth":{...},"account":{...}}`）→ 归一 → realm 分流 → 合并进该 region 账号库。
///
/// 目录枚举是薄壳；逐项判定（归一 + realm + token + 时间戳）走纯函数 [`classify_auths_item`]，
/// 计数走 [`AuthsDirImportResult`]（多一个 `mismatch` 反映域不符被跳过）。
pub fn import_auths_dir(dir: &Path, region: Region) -> Result<AuthsDirImportResult, String> {
    let mut result = AuthsDirImportResult::default();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("读取目录失败（{}）：{e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        // 只认顶层 `.json`（panel 的 `auths/workbuddy-<uid>.json` 都是单层）。
        if !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("读取失败（{}）：{e}", path.display()))?;
        let item: Value =
            serde_json::from_str(&text).map_err(|e| format!("{} 不是合法 JSON：{e}", path.display()))?;
        if !item.is_object() {
            return Err(format!("{} 顶层应为 JSON 对象（账号）", path.display()));
        }
        match classify_auths_item(&item, region) {
            AuthsDirItem::Record(record) => {
                let mut accounts = account::load_accounts_for(region);
                // 归一后走与文件导入完全相同的合并口径（uid 去重、缺 id 保留本地 id）。
                let merged = merge_import_record(&mut accounts, &record);
                // classify 已保证有 token，`merged` 不会为 `Skipped`；仍按实际结果计数。
                match merged {
                    MergeOutcome::Appended => {
                        result.imported += 1;
                    }
                    MergeOutcome::Overwritten => {
                        result.imported += 1;
                        result.overwritten += 1;
                    }
                    MergeOutcome::Skipped => result.skipped += 1,
                }
                account::save_accounts_for(region, &accounts)
                    .map_err(|e| format!("保存账号库失败：{e}"))?;
            }
            AuthsDirItem::Mismatch => result.mismatch += 1,
            AuthsDirItem::Skipped => result.skipped += 1,
        }
    }
    Ok(result)
}

/// 纯函数：从账号列表中挑出 id 命中的完整记录（含 token）。
pub fn select_export_records(accounts: &[Value], ids: &[String]) -> Result<Vec<Value>, String> {
    let ids: Vec<&str> = ids
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if ids.is_empty() {
        return Err("请先选择要导出的账号".to_string());
    }
    let mut exported: Vec<Value> = Vec::new();
    for id in ids {
        if let Some(acc) = accounts
            .iter()
            .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(id))
        {
            exported.push(acc.clone());
        }
    }
    if exported.is_empty() {
        return Err("未找到要导出的账号".to_string());
    }
    Ok(exported)
}

/// 导出（CN）：按账号 id 列表返回完整记录（含 token）。
pub fn export_accounts(ids: &[String]) -> Result<Vec<Value>, String> {
    export_accounts_for(Region::Cn, ids)
}

/// 按 region 导出：按账号 id 列表返回完整记录（含 token）。
pub fn export_accounts_for(region: Region, ids: &[String]) -> Result<Vec<Value>, String> {
    select_export_records(&account::load_accounts_for(region), ids)
}

/// 导出文件名白名单：只允许 `buddy-switch-accounts-YYYY-MM-DD.json` 这类由前端生成的文件名。
fn validate_export_file_name(file_name: &str) -> Result<(), String> {
    let name = file_name.trim();
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !name.ends_with(".json")
    {
        return Err("导出文件名不合法".to_string());
    }
    Ok(())
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

/// 纯函数：把记录数组写入指定目录下的 JSON 文件，返回完整路径。
pub fn write_records_to_file(
    dir: &Path,
    records: &[Value],
    file_name: &str,
) -> Result<String, String> {
    validate_export_file_name(file_name)?;
    let content = serde_json::to_string_pretty(records).map_err(|e| e.to_string())?;
    let path = dir.join(file_name);
    crate::modules::config::atomic_write(&path, &content)
        .map_err(|e| format!("写入导出文件失败：{e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 导出（CN）：按账号 id 列表把完整记录写入用户选择的路径（保存对话框产物），返回该路径。
pub fn export_accounts_to_path(ids: &[String], path: &str) -> Result<String, String> {
    export_accounts_to_path_for(Region::Cn, ids, path)
}

/// 按 region 导出：按账号 id 列表把完整记录写入用户选择的路径，返回该路径。
pub fn export_accounts_to_path_for(
    region: Region,
    ids: &[String],
    path: &str,
) -> Result<String, String> {
    validate_export_path(path)?;
    let records = select_export_records(&account::load_accounts_for(region), ids)?;
    let path_buf = PathBuf::from(path.trim());
    let content = serde_json::to_string_pretty(&records).map_err(|e| e.to_string())?;
    crate::modules::config::atomic_write(&path_buf, &content)
        .map_err(|e| format!("写入导出文件失败：{e}"))?;
    Ok(path_buf.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        id: &str,
        uid: Option<&str>,
        nickname: &str,
        email: Option<&str>,
        token: bool,
    ) -> Value {
        json!({
            "id": id,
            "uid": uid,
            "nickname": nickname,
            "email": email,
            "access_token": if token { format!("token-{id}") } else { String::new() },
            "createdAt": 1,
        })
    }

    #[test]
    fn parse_rejects_empty_invalid_and_non_array() {
        assert!(parse_accounts_json("").is_err());
        assert!(parse_accounts_json("not json").is_err());
        assert!(parse_accounts_json(r#"{ "a": 1 }"#).is_err());
    }

    #[test]
    fn parse_rejects_non_object_element() {
        let err = parse_accounts_json(r#"[{ "uid": "u1" }, 42]"#).unwrap_err();
        assert!(err.contains("第 2 项"), "错误应带位置：{err}");
    }

    #[test]
    fn parse_accepts_object_array() {
        let parsed = parse_accounts_json(r#"[{ "uid": "u1" }, {}]"#).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    /// 回归护栏（issue #2）：导入预览的展示字段同样只能是「字符串或 null」。
    ///
    /// 预览内容来自**外部文件**（用户可能从别的工具导出），`nickname` 完全可能是对象；
    /// 而 `ImportPreviewAccount` 声明为 `string | null`，弹框把它当 **React 子节点**渲染
    /// ⇒ 脏值会让 React 卸载整棵树 ⇒ 白屏。
    ///
    /// 两段式：第一段钉形状不变量，第二段是**阳性对照** —— 没有它，「无脑全置 null」
    /// 的偷懒实现也能让第一段全绿（数字昵称是合法数据，必须保留成文本）。
    #[test]
    fn preview_never_leaks_non_string_display_fields() {
        let dirty = r#"[{"uid":{"nested":true},"nickname":{"zh":"小明"},
            "email":["a@b.c"],"access_token":"tok"}]"#;
        let preview = preview_accounts(dirty).expect("解析应成功");
        for key in ["uid", "nickname", "email"] {
            let value = &preview["accounts"][0][key];
            assert!(
                value.is_null() || value.is_string(),
                "preview.accounts[0].{key} 必须是字符串或 null，实际透出了 {value}：{preview}"
            );
        }

        let clean = r#"[{"uid":"u-1","nickname":12345,"email":"x@y.z","access_token":"tok"}]"#;
        let preview = preview_accounts(clean).expect("解析应成功");
        assert_eq!(preview["accounts"][0]["uid"], json!("u-1"), "{preview}");
        assert_eq!(
            preview["accounts"][0]["nickname"],
            json!("12345"),
            "数字昵称必须归一成字符串、而不是被丢掉：{preview}"
        );
        assert_eq!(preview["accounts"][0]["email"], json!("x@y.z"), "{preview}");
    }

    #[test]
    fn merge_overwrites_same_uid_preserving_imported_record() {
        let mut accounts = vec![record("local", Some("u1"), "旧名称", None, true)];
        let text = r#"[{ "id": "imported", "uid": "u1", "nickname": "新名称", "access_token": "tok-new" }]"#;
        let result = merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.overwritten, 1);
        assert_eq!(result.skipped, 0);
        assert_eq!(accounts.len(), 1, "同 uid 应覆盖而不是新增");
        assert_eq!(accounts[0]["id"], "imported", "覆盖保留导入记录原样");
        assert_eq!(accounts[0]["nickname"], "新名称");
        assert_eq!(accounts[0]["access_token"], "tok-new");
    }

    #[test]
    fn merge_preserves_local_id_when_imported_record_has_none() {
        let mut accounts = vec![record("local-id", Some("u1"), "旧名称", None, true)];
        let text = r#"[{ "uid": "u1", "nickname": "新名称", "access_token": "tok" }]"#;
        let result = merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.overwritten, 1);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0]["id"], "local-id", "导入记录缺 id 时保留本地 id");
        assert_eq!(accounts[0]["nickname"], "新名称");
        assert_eq!(accounts[0]["access_token"], "tok");
    }

    #[test]
    fn merge_appends_when_uid_missing_or_unmatched() {
        let mut accounts = vec![record("local", Some("u1"), "甲", None, true)];
        let text = r#"[
            { "id": "n1", "nickname": "无uid", "access_token": "t1" },
            { "id": "n2", "uid": "u-other", "nickname": "uid不匹配", "access_token": "t2" }
        ]"#;
        let result = merge_import_records(&mut accounts, text, &[0, 1]).unwrap();
        assert_eq!(result.imported, 2);
        assert_eq!(result.overwritten, 0);
        assert_eq!(accounts.len(), 3);
    }

    #[test]
    fn merge_skips_missing_token() {
        let mut accounts: Vec<Value> = vec![];
        let text = r#"[{ "id": "no-token", "uid": "u9", "nickname": "缺token" }]"#;
        let result = merge_import_records(&mut accounts, text, &[0]).unwrap();
        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 1);
        assert!(accounts.is_empty(), "缺 token 的记录不得进入账号库");
    }

    #[test]
    fn merge_counts_out_of_range_index_as_skipped() {
        let mut accounts: Vec<Value> = vec![];
        let result =
            merge_import_records(&mut accounts, r#"[{ "access_token": "t" }]"#, &[5]).unwrap();
        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 1);
    }

    /// 按 panel `auths/*.json` 的**真实落盘形态**钉住目录导入的判定口径
    /// （`realm: "cn"` 优先于 `domain`；`expiresAt` 为 Unix **秒**要转毫秒）。
    #[test]
    fn classify_auths_item_handles_real_panel_shape_cn_seconds() {
        let item = json!({
            "account": { "enterpriseId": "", "nickname": "17538802558", "uid": "u-1" },
            "auth": {
                "accessToken": "tok",
                "refreshToken": "ref",
                "expiresAt": 1793155025,
                "domain": "www.codebuddy.cn",
                "realm": "cn",
            }
        });
        match classify_auths_item(&item, Region::Cn) {
            AuthsDirItem::Record(record) => {
                assert_eq!(record["uid"], json!("u-1"));
                assert_eq!(record["access_token"], json!("tok"));
                assert_eq!(record["domain"], json!("www.codebuddy.cn"));
                // 秒 → 毫秒（`norm_ts`）
                assert_eq!(record["expiresAt"], json!(1_793_155_025_000_i64));
                assert!(record["refreshExpiresAt"].is_null(), "panel 文件无 refreshExpiresAt 时应为 null");
            }
            other => panic!("CN panel 记录应可导入 CN 库，实际 {other:?}"),
        }
        // 同一文件导 Global 库：域不符计入 mismatch，不进错库。
        match classify_auths_item(&item, Region::Global) {
            AuthsDirItem::Mismatch => {}
            other => panic!("CN realm 记录导入 Global 库应计 mismatch，实际 {other:?}"),
        }
    }

    /// `realm` 缺失时回落 `domain` 判定（panel 早期文件 / 自定义 realm 值）。
    #[test]
    fn auths_item_region_falls_back_to_domain_when_realm_missing() {
        let global_item = json!({
            "account": { "uid": "u-2" },
            "auth": { "accessToken": "t", "domain": "www.workbuddy.ai", "realm": "global" }
        });
        let no_realm = json!({
            "account": { "uid": "u-3" },
            "auth": { "accessToken": "t", "domain": "www.codebuddy.cn" }
        });
        assert_eq!(auths_item_region(&global_item), Region::Global);
        assert_eq!(auths_item_region(&no_realm), Region::Cn);
    }

    #[test]
    fn merge_deduplicates_within_file() {
        let mut accounts: Vec<Value> = vec![];
        let text = r#"[
            { "id": "f1", "uid": "u9", "access_token": "t1" },
            { "id": "f2", "uid": "u9", "access_token": "t2" }
        ]"#;
        let result = merge_import_records(&mut accounts, text, &[0, 1]).unwrap();
        assert_eq!(result.imported, 2);
        assert_eq!(result.overwritten, 1);
        assert_eq!(accounts.len(), 1, "文件内重复 uid 也不得产生重复账号");
        assert_eq!(accounts[0]["id"], "f2");
    }

    #[test]
    fn export_requires_selection() {
        let accounts = vec![record("a1", Some("u1"), "甲", None, true)];
        assert!(select_export_records(&accounts, &[]).is_err());
        assert!(select_export_records(&accounts, &["missing".to_string()]).is_err());
    }

    #[test]
    fn region_scoped_export_requires_selection() {
        // 空选择在加载账号库前即被拒绝；两版行为一致，且不触碰写路径。
        assert!(export_accounts_for(Region::Global, &[]).is_err());
        assert!(export_accounts_for(Region::Cn, &[]).is_err());
        assert!(export_accounts_to_path_for(Region::Global, &[], "/tmp/out.json").is_err());
    }

    #[test]
    fn export_returns_full_records_with_tokens() {
        let accounts = vec![
            record("a1", Some("u1"), "甲", Some("a@b.c"), true),
            record("a2", Some("u2"), "乙", None, true),
        ];
        let exported =
            select_export_records(&accounts, &["a1".to_string(), "missing".to_string()]).unwrap();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0]["id"], "a1");
        assert_eq!(exported[0]["access_token"], "token-a1");
    }

    #[test]
    fn exported_records_roundtrip_import() {
        let accounts = vec![json!({
            "id": "a1",
            "uid": "u1",
            "nickname": "甲",
            "email": "a@b.c",
            "access_token": "tok-1",
            "refresh_token": "ref-1",
            "auth_raw": { "k": "v" },
            "createdAt": 1,
        })];
        let exported = select_export_records(&accounts, &["a1".to_string()]).unwrap();
        let text = serde_json::to_string(&exported).unwrap();
        let mut target: Vec<Value> = vec![];
        let result = merge_import_records(&mut target, &text, &[0]).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(target.len(), 1);
        assert_eq!(
            target[0]["access_token"], "tok-1",
            "round-trip 保留 access_token"
        );
        assert_eq!(target[0]["refresh_token"], "ref-1");
        assert_eq!(target[0]["auth_raw"]["k"], "v");
    }

    #[test]
    fn preview_exposes_only_desensitized_fields() {
        let text = r#"[{
            "id": "a1",
            "uid": "u1",
            "nickname": "小明",
            "email": "x@y.z",
            "access_token": "SECRET"
        }]"#;
        let preview = preview_accounts(text).unwrap();
        assert_eq!(preview["total"], 1);
        assert_eq!(preview["accounts"][0]["index"], 0);
        assert_eq!(preview["accounts"][0]["uid"], "u1");
        assert_eq!(preview["accounts"][0]["nickname"], "小明");
        assert_eq!(preview["accounts"][0]["email"], "x@y.z");
        assert_eq!(preview["accounts"][0]["hasToken"], true);
        assert!(
            preview["accounts"][0].get("access_token").is_none(),
            "预览不得泄露 token"
        );
    }

    #[test]
    fn export_file_name_rejects_traversal_and_non_json() {
        assert!(validate_export_file_name("../../etc/passwd.json").is_err());
        assert!(validate_export_file_name("a/b.json").is_err());
        assert!(validate_export_file_name("a\\b.json").is_err());
        assert!(validate_export_file_name("out.txt").is_err());
        assert!(validate_export_file_name("").is_err());
        assert!(validate_export_file_name("buddy-switch-accounts-2026-08-21.json").is_ok());
    }

    #[test]
    fn export_path_validation() {
        // 用当前平台的绝对路径构造用例（Windows 上 `/tmp/...` 并非绝对路径）。
        let dir = std::env::temp_dir();
        assert!(
            validate_export_path("relative/out.json").is_err(),
            "必须绝对路径"
        );
        assert!(
            validate_export_path(&dir.join("out.txt").to_string_lossy()).is_err(),
            "必须 .json"
        );
        assert!(
            validate_export_path(&dir.join("out.JSON").to_string_lossy()).is_ok(),
            "扩展名不区分大小写"
        );
        assert!(validate_export_path(&dir.join("out.json").to_string_lossy()).is_ok());
    }

    #[test]
    fn export_accounts_to_file_writes_json_with_tokens() {
        let dir = std::env::temp_dir();
        let file_name = format!(
            "buddy-switch-accounts-test-{}.json",
            uuid::Uuid::new_v4().simple()
        );
        let records = vec![record("a1", Some("u1"), "甲", None, true)];
        let path = write_records_to_file(&dir, &records, &file_name).unwrap();
        let written: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0]["access_token"], "token-a1", "导出文件保留 token");
    }

    #[test]
    fn write_records_to_file_rejects_bad_name() {
        let dir = std::env::temp_dir();
        let records = vec![record("a1", Some("u1"), "甲", None, true)];
        assert!(write_records_to_file(&dir, &records, "../escape.json").is_err());
        assert!(write_records_to_file(&dir, &records, "no-ext").is_err());
    }
}
