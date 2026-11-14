//! Qoder 账号的**导出 / 脱敏预览 / 按索引导入**。
//!
//! 本模块只管「在用户与账号库之间搬一份 JSON」这一件事：账号库本身的读写全部经
//! [`super::account`]（带归属校验），这里不另起一套落盘逻辑。
//!
//! ## 为什么导出要带安全警告
//!
//! 导出文件是**凭据库的快照** —— `accessToken` / `refreshToken` / `personalToken`
//! 全是明文，拿到文件的人可以冒充这些账号。所以导出是一次**需要用户知情的**显式操作：
//! 界面必须在导出前给出「文件含明文凭据」的警告（见共享导出弹框的安全提示区）。
//! 本模块**不做静默脱敏**：脱敏后的文件搬不回新机器，而用户以为自己备份成功了 ——
//! 那是比泄露更隐蔽的失败（「备份了，却恢复不出来」）。
//!
//! ## 为什么脱敏预览必须存在
//!
//! 导入文件来自**外部**（另一台机器、另一份备份、甚至手改过的文本），在真正落库前，
//! 用户需要一个「这份文件里到底是谁」的确认界面，而那个界面**绝不能把令牌明文渲染出来**
//! —— 本项目用户经常贴截图报障，明文一旦上屏就等于泄露。
//! 所以预览只回展示字段（`uid` / 昵称）与**有无凭据**的布尔量；令牌至多以
//! [`store::mask`] 的掩码形式出现（短值整体打码，不留可辨识中段）。
//!
//! ## 为什么是「按索引导入」而不是「按 uid 导入」
//!
//! `uid` 不是**文件内**的唯一句柄：外部文件可能缺 `uid`，也可能两条记录同 `uid`
//! （按 uid 选会去重成一条，用户勾了三条却只进一条）。而**文件内下标**是稳定的：
//! 预览列表按下标渲染、用户按下标勾选、导入按下标取记录 —— 三处指的是同一条。
//! 因此选择集用下标表达（`indexes`），与预览里的 `index` 一一对应。
//!
//! ⚠️ `indexes = None` 表示「全部」，`Some(&[])` 表示「一条都不导」——
//! 两者语义**必须不同**。这里踩过的坑：空选择曾被当成「全选」，于是用户一个都没勾，
//! 却把整份文件导了进来。同理，`uids = None`（该区域全部）与 `Some(&[])`（空表）也不同。
//!
//! ## 为什么跨区条目必须拒（而不是静默跳过）
//!
//! 两个区域的账号库是**两本互不相通的库**。把国际版账号写进国内版库，用户看到的是
//! 「账号在，但登录不了」—— 错误无声、排查昂贵。因此导入前逐条做归属校验
//! （[`ensure_region_matches`]）：**不符即拒并计数上报**（`rejected`），而不是混进
//! `skipped` 里让用户以为「只是被去重了」。空域名 / 认不出的域名同样拒（fail-closed）。
//!
//! ## 导出文档的形状
//!
//! 落盘文件是**一个 JSON 数组**，每条是一个账号对象、`domain` 在**条目内**
//! （`RawAccount` 的 `domain` 字段恒定序列化）—— 与本仓既有的账号导入/导出契约
//! 同一格式家族，通用导入器能直接读它，而本模块的导入器额外按 `domain` 做区域锁定。
//! 接口返回值则是给界面用的信封（`{ ok, region, domain, accounts }`）；前端下载的是
//! `accounts` 那一段（即数组本身），所以桌面端（本模块写文件）与浏览器端（下载同一段）
//! 产出的文件**内容一致**。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::modules::qoder::account::{
    ensure_region_matches, find_account_for, load_accounts_checked_for, upsert_account_for,
    RawAccount, UpsertOutcome,
};
use crate::modules::qoder::region::{realm_spec, region_of_domain, QoderRegion};
use crate::modules::qoder::store;
use crate::modules::qoder::token::{credential_shape, CredentialShape};

/// 解析导出/导入文件文本：必须是 JSON 数组，且每项为 JSON 对象。
///
/// 失败时返回带位置的明确错误文案（非法 JSON / 非数组 / 元素不是对象）。
fn parse_accounts_json(text: &str) -> Result<Vec<Value>, String> {
    if text.trim().is_empty() {
        return Err("文件内容为空".to_string());
    }
    let parsed: Value =
        serde_json::from_str(text).map_err(|error| format!("文件不是合法的 JSON：{error}"))?;
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

/// 取非空字符串字段（内部逻辑用；缺失 / 非字符串 / 全空白 ⇒ `None`）。
fn opt_str(item: &Value, key: &str) -> Option<String> {
    item.get(key)
        .and_then(Value::as_str)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// 展示字段归一：任意 JSON 值 → **「字符串或 null」**。
///
/// 与前端预览契约（`ImportPreviewAccount` 的 `string | null`）一致：数字转文本、
/// 其余（对象 / 数组 / 布尔 / 缺失）一律 `null`。脏值会被前端直接当 React 子节点渲染，
/// 对象会让 React 卸载整棵树（白屏）。
fn text_of(item: &Value, key: &str) -> Value {
    match item.get(key) {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Number(number)) => Value::String(number.to_string()),
        _ => Value::Null,
    }
}

/// 从一条导入记录读取凭据形态（字段名与 `RawAccount` 的持久化形态一致，camelCase）。
fn credential_shape_of(item: &Value) -> CredentialShape {
    credential_shape(
        opt_str(item, "accessToken").as_deref(),
        opt_str(item, "refreshToken").as_deref(),
        opt_str(item, "personalToken").as_deref(),
    )
}

/// 令牌掩码：优先访问令牌，其次 PAT（长期兜底），最后刷新令牌。**绝不回显明文**。
fn masked_token(item: &Value) -> Value {
    ["accessToken", "personalToken", "refreshToken"]
        .iter()
        .find_map(|key| opt_str(item, key))
        .map(|token| Value::String(store::mask(&token)))
        .unwrap_or(Value::Null)
}

/// 该条目是否与目标区域**不符**（缺 / 空 / 认不出的域名都算不符，fail-closed）。
fn domain_mismatch(region: QoderRegion, item: &Value) -> bool {
    match opt_str(item, "domain") {
        Some(domain) => !matches!(region_of_domain(&domain), Some(actual) if actual == region),
        None => true,
    }
}

/// 按 uid 选择要导出的记录。
///
/// ★ `None` ⇒ 该区域**全部**；`Some(&[])` ⇒ **空表**（不是全部）。
/// 空列表绝不能被当成「未指定」而回落成全部 —— 那会让「清空选择后点导出」导出整库。
fn select_records(accounts: &[RawAccount], uids: Option<&[String]>) -> Vec<RawAccount> {
    match uids {
        None => accounts.to_vec(),
        Some(list) => {
            let wanted: Vec<&str> = list
                .iter()
                .map(|uid| uid.trim())
                .filter(|uid| !uid.is_empty())
                .collect();
            accounts
                .iter()
                .filter(|account| wanted.contains(&account.uid.as_str()))
                .cloned()
                .collect()
        }
    }
}

/// 账号记录 → JSON 值（保留持久化形态，含明文凭据与 `domain`）。
fn records_to_values(records: &[RawAccount]) -> Result<Vec<Value>, String> {
    records
        .iter()
        .map(|record| {
            serde_json::to_value(record).map_err(|error| format!("序列化账号失败：{error}"))
        })
        .collect()
}

/// 校验导出目标路径：必须是绝对路径且以 `.json` 结尾（保存对话框产物）。
fn validate_export_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("导出路径不能为空".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("导出路径必须是绝对路径".to_string());
    }
    let is_json = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    if !is_json {
        return Err("导出文件名必须以 .json 结尾".to_string());
    }
    Ok(path)
}

/// 导出：`uids = None` ⇒ 导出该区域**全部**账号；`Some(&[])` ⇒ **空表**（不是全部）。
///
/// 返回值是界面信封：`{ ok, region, domain, count, accounts }`。
/// `accounts` 是完整记录数组（**含明文凭据**，这是导出安全警告的来源），
/// 前端把它当文件内容下载，故它与 [`export_accounts_to_path_for`] 落盘的内容一致。
pub fn export_accounts_for(region: QoderRegion, uids: Option<&[String]>) -> Result<Value, String> {
    let file = load_accounts_checked_for(region)?;
    let records = select_records(&file.accounts, uids);
    let accounts = records_to_values(&records)?;
    let count = accounts.len();
    Ok(json!({
        "ok": true,
        "region": region.as_str(),
        "domain": realm_spec(region).domain,
        "count": count,
        "accounts": accounts,
    }))
}

/// 导出：按 uid 选择把完整记录写入用户选择的路径（保存对话框产物），返回落地路径。
///
/// ★ 落盘的是**裸数组**（与本仓既有导出文件的格式家族一致），不是信封：
/// 桌面端写文件、浏览器端下载 `accounts` 段，两条路径必须产出**同一份内容**。
pub fn export_accounts_to_path_for(
    region: QoderRegion,
    uids: Option<&[String]>,
    path: &str,
) -> Result<Value, String> {
    let path = validate_export_path(path)?;
    let file = load_accounts_checked_for(region)?;
    let records = select_records(&file.accounts, uids);
    let accounts = records_to_values(&records)?;
    let count = accounts.len();
    store::write_json(&path, &accounts)?;
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy(),
        "count": count,
    }))
}

/// 解析导入文件并给出**脱敏预览**（含文件内索引，绝不回显令牌明文）。
///
/// 返回值：`{ region, domain, accounts, total }`。
/// 每个条目固定包含共享壳消费的四个键：`index` / `uid` / `nickname` / `email` /
/// `hasToken`；另附归属诊断（`domain` / `domainMismatch` / `credentialSummary`）
/// 与掩码令牌（`tokenMasked`，经 [`store::mask`]）。
///
/// `hasToken` 的判据是「有没有**任一**凭据」（PAT 也算），**不是** `accessToken` 是否为空 ——
/// 只有 PAT 的账号是可用的，标成「缺令牌」会误导用户。
pub fn preview_accounts(region: QoderRegion, text: &str) -> Result<Value, String> {
    let array = parse_accounts_json(text)?;
    let items: Vec<Value> = array
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let shape = credential_shape_of(item);
            json!({
                "index": index,
                // 共享预览契约：这三个字段必须是「字符串或 null」。
                "uid": text_of(item, "uid"),
                "nickname": text_of(item, "nickname"),
                // Qoder 的账号模型没有邮箱字段 —— 恒为 null，保持契约形状不变。
                "email": Value::Null,
                // ★ 「有没有凭据」按 has_any_credential 判（PAT 也算），不是看 accessToken 是否为空。
                "hasToken": shape.has_any_credential(),
                // 归属诊断（共享壳忽略；Qoder 面板可用）：跨区条目导入时会被拒。
                "domain": text_of(item, "domain"),
                "domainMismatch": domain_mismatch(region, item),
                "credentialSummary": shape.describe(),
                // 令牌**至多以掩码出现**，绝不回显明文。
                "tokenMasked": masked_token(item),
            })
        })
        .collect();
    Ok(json!({
        "region": region.as_str(),
        "domain": realm_spec(region).domain,
        "accounts": items,
        "total": array.len(),
    }))
}

/// 按索引导入（`indexes = None` ⇒ 全部；`Some(&[])` ⇒ 一条都不导）。
///
/// 逐条：反序列化成 [`RawAccount`] → 归属校验 → 凭据存在性 → 按 `overwrite` 决定
/// 是否覆盖同 uid 的本地记录 → 经 [`upsert_account_for`] 落库（**带归属校验的写路径**）。
///
/// 返回值：`{ ok, imported, skipped, overwritten, rejected, rejectedDomains }`。
/// - `imported`：新增 + 覆盖的总数；
/// - `overwritten`：其中覆盖了同 uid 本地记录的数量；
/// - `skipped`：无法入库的数量（下标越界 / 无法解析 / 无任何凭据 / `overwrite=false` 时命中本地）；
/// - `rejected`：**归属不符被拒**的数量（跨区 / 空域 / 认不出的域），
///   `rejectedDomains` 是它们的归属域名 —— **如实报**，不静默跳过。
pub fn import_accounts_for(
    region: QoderRegion,
    text: &str,
    indexes: Option<&[usize]>,
    overwrite: bool,
) -> Result<Value, String> {
    let array = parse_accounts_json(text)?;
    // ★ `None` = 全部；`Some(&[])` = 一条都不导。两者语义不同（见模块头）。
    let selected: Vec<usize> = match indexes {
        None => (0..array.len()).collect(),
        Some(list) => list.to_vec(),
    };

    let mut imported = 0usize;
    let mut overwritten = 0usize;
    let mut skipped = 0usize;
    let mut rejected = 0usize;
    let mut rejected_domains: Vec<String> = Vec::new();

    for index in selected {
        let Some(item) = array.get(index) else {
            skipped += 1;
            continue;
        };
        let Ok(account) = serde_json::from_value::<RawAccount>(item.clone()) else {
            skipped += 1;
            continue;
        };
        if account.uid.trim().is_empty() {
            skipped += 1;
            continue;
        }
        // ★ 归属校验：跨区 / 空域 / 认不出的域一律**拒**并计数上报（不混进 skipped）。
        if ensure_region_matches(region, &account.domain).is_err() {
            rejected += 1;
            rejected_domains.push(account.domain.clone());
            continue;
        }
        // 没有任何凭据的记录不入库 —— 否则用户会看到一个「永远登录不了」的空壳账号。
        if !account.credential_shape().has_any_credential() {
            skipped += 1;
            continue;
        }
        // 命中同 uid 且不允许覆盖 ⇒ 保留本地（幂等的核心）。
        if !overwrite && find_account_for(region, &account.uid).is_some() {
            skipped += 1;
            continue;
        }
        match upsert_account_for(region, account) {
            Ok(UpsertOutcome::Added) => imported += 1,
            Ok(UpsertOutcome::Replaced) => {
                imported += 1;
                overwritten += 1;
            }
            Err(_) => skipped += 1,
        }
    }

    Ok(json!({
        "ok": true,
        "imported": imported,
        "skipped": skipped,
        "overwritten": overwritten,
        "rejected": rejected,
        "rejectedDomains": rejected_domains,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::qoder::account::{save_accounts_for, AccountsFile};

    /// 把 home 指向一个隔离目录（`BUDDY_SWITCH_HOME`），测试结束自动还原。
    ///
    /// ⚠️ `HomeOverrideGuard::set` 内部持有一把**非可重入**的全局锁 ⇒
    /// 同一个测试里**只能有一个** guard，且需用 `--test-threads=1` 串行跑。
    fn isolated_home(
        tag: &str,
    ) -> (
        std::path::PathBuf,
        crate::modules::config::HomeOverrideGuard,
    ) {
        let home = std::env::temp_dir().join(format!(
            "buddy-switch-qoder-export-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&home).expect("创建隔离 home");
        let guard = crate::modules::config::HomeOverrideGuard::set(&home);
        (home, guard)
    }

    fn account(uid: &str, domain: &str, nickname: &str) -> RawAccount {
        RawAccount {
            uid: uid.to_string(),
            nickname: Some(nickname.to_string()),
            domain: domain.to_string(),
            access_token: Some("dt-abcdefghijklmnop".to_string()),
            ..Default::default()
        }
    }

    fn seed(region: QoderRegion, accounts: Vec<RawAccount>) {
        save_accounts_for(region, &AccountsFile { accounts }).expect("预置账号库");
    }

    /// ★ `uids = None` ⇒ 全部；`Some(&[])` ⇒ 空表。
    ///
    /// 可证伪：把 `select_records` 的 `Some(list)` 分支改成「列表为空 ⇒ 当作全部」，
    /// 末条断言立即变红 —— 那正是「用户清空选择却导出了整库」的缺陷形态。
    #[test]
    fn export_none_means_all_but_empty_selection_means_empty_table() {
        let (_home, _guard) = isolated_home("none-vs-empty");
        seed(
            QoderRegion::Cn,
            vec![
                account("u1", "qoder.com.cn", "甲"),
                account("u2", "qoder.com.cn", "乙"),
            ],
        );

        let all = export_accounts_for(QoderRegion::Cn, None).unwrap();
        assert_eq!(
            all["accounts"].as_array().unwrap().len(),
            2,
            "None 必须导出全部"
        );
        assert_eq!(all["count"], json!(2));
        assert_eq!(all["domain"], json!("qoder.com.cn"));

        let none_selected: &[String] = &[];
        let empty = export_accounts_for(QoderRegion::Cn, Some(none_selected)).unwrap();
        assert_eq!(
            empty["accounts"].as_array().unwrap().len(),
            0,
            "Some(&[]) 必须是空表，不是全部"
        );
        assert_eq!(empty["count"], json!(0));
    }

    /// 导出的条目保留 `domain` 与**明文凭据**（导出安全警告的来源），且 `accounts` 段是数组。
    ///
    /// 可证伪：若导出改成只给脱敏字段（抹掉 `accessToken` / `domain`），前两条断言变红。
    #[test]
    fn exported_records_keep_domain_and_credentials() {
        let (_home, _guard) = isolated_home("keeps-domain");
        seed(QoderRegion::Cn, vec![account("u1", "qoder.com.cn", "甲")]);

        let out = export_accounts_for(QoderRegion::Cn, Some(&["u1".to_string()])).unwrap();
        let record = &out["accounts"][0];
        assert_eq!(
            record["domain"],
            json!("qoder.com.cn"),
            "条目必须带 domain（导入时区域锁定的依据）"
        );
        assert_eq!(
            record["accessToken"],
            json!("dt-abcdefghijklmnop"),
            "导出文件含明文凭据 —— 这是导出警告的来源"
        );

        let text = serde_json::to_string(&out["accounts"]).unwrap();
        assert!(text.starts_with('['), "accounts 段是数组：{text}");
        assert!(text.contains("\"domain\""), "{text}");
    }

    /// 落盘文件是**裸数组**（与浏览器端下载内容一致），返回值带 `path`。
    ///
    /// 可证伪：把写出内容改成信封对象（`{ accounts: [...] }`），`is_array()` 断言变红 ——
    /// 那会让桌面端产出的文件与浏览器端下载的文件不一致。
    #[test]
    fn export_to_path_writes_a_bare_array_and_returns_the_path() {
        let (_home, _guard) = isolated_home("to-path");
        seed(QoderRegion::Cn, vec![account("u1", "qoder.com.cn", "甲")]);

        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-qoder-out-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("accounts.json");
        let path_str = file.to_string_lossy().into_owned();

        let out = export_accounts_to_path_for(QoderRegion::Cn, None, &path_str).unwrap();
        assert_eq!(out["ok"], json!(true));
        assert_eq!(out["path"], json!(path_str.as_str()));
        assert_eq!(out["count"], json!(1));

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(written.is_array(), "落盘必须是裸数组：{written}");
        assert_eq!(written[0]["domain"], json!("qoder.com.cn"));

        // 相对路径 / 非 .json 一律拒（保存对话框产物校验）。
        assert!(export_accounts_to_path_for(QoderRegion::Cn, None, "relative/out.json").is_err());
        assert!(export_accounts_to_path_for(
            QoderRegion::Cn,
            None,
            &dir.join("out.txt").to_string_lossy()
        )
        .is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★ 预览**绝不回显令牌明文**，且展示字段满足共享壳的契约（`string | null`）。
    ///
    /// 可证伪：把 `tokenMasked` 改成回显原文，第一条 `!contains("SECRET")` 断言变红。
    #[test]
    fn preview_is_desensitized_and_matches_the_shared_contract() {
        let text = r#"[{
            "uid": "u1",
            "nickname": "小明",
            "domain": "qoder.com.cn",
            "accessToken": "dt-SECRET-abcdefghijkl",
            "refreshToken": "drt-SECRET"
        }]"#;
        let preview = preview_accounts(QoderRegion::Cn, text).unwrap();
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(
            !serialized.contains("SECRET"),
            "预览不得回显令牌明文：{serialized}"
        );

        assert_eq!(preview["total"], json!(1));
        let item = &preview["accounts"][0];
        assert_eq!(item["index"], json!(0));
        assert_eq!(item["uid"], json!("u1"));
        assert_eq!(item["nickname"], json!("小明"));
        assert_eq!(item["hasToken"], json!(true));
        assert_eq!(item["email"], Value::Null, "Qoder 无邮箱字段 ⇒ 恒 null");

        for key in ["uid", "nickname", "email"] {
            assert!(
                item[key].is_string() || item[key].is_null(),
                "preview.accounts[0].{key} 必须破坏契约地透出了非字符串：{}",
                item[key]
            );
        }

        // 掩码可辨识但不含中段明文。
        let masked = item["tokenMasked"].as_str().unwrap();
        assert!(
            masked.contains('…') || masked.chars().all(|c| c == '*'),
            "掩码形态异常：{masked}"
        );
    }

    /// 预览逐条标出**归属不符**（跨区 / 空域），供导入前预警。
    ///
    /// 可证伪：把 `domain_mismatch` 恒置 `false`，第一条断言变红。
    #[test]
    fn preview_marks_cross_region_entries() {
        let cross =
            r#"[{ "uid": "g1", "domain": "qoder.com", "accessToken": "dt-abcdefghijklmnop" }]"#;
        assert_eq!(
            preview_accounts(QoderRegion::Cn, cross).unwrap()["accounts"][0]["domainMismatch"],
            json!(true),
            "国际版条目对国内版必须标为不符"
        );

        let same =
            r#"[{ "uid": "c1", "domain": "qoder.com.cn", "accessToken": "dt-abcdefghijklmnop" }]"#;
        assert_eq!(
            preview_accounts(QoderRegion::Cn, same).unwrap()["accounts"][0]["domainMismatch"],
            json!(false)
        );

        // 空域同样视为不符（fail-closed）。
        let no_domain = r#"[{ "uid": "x", "accessToken": "dt-abcdefghijklmnop" }]"#;
        assert_eq!(
            preview_accounts(QoderRegion::Cn, no_domain).unwrap()["accounts"][0]["domainMismatch"],
            json!(true)
        );
    }

    /// `indexes = None` ⇒ 导入全部。
    #[test]
    fn import_with_none_indexes_imports_everything() {
        let (_home, _guard) = isolated_home("import-none");
        let text = r#"[
            { "uid": "u1", "nickname": "甲", "domain": "qoder.com.cn", "accessToken": "dt-abcdefghijklmnop" },
            { "uid": "u2", "nickname": "乙", "domain": "qoder.com.cn", "accessToken": "dt-abcdefghijklmnop" }
        ]"#;
        let out = import_accounts_for(QoderRegion::Cn, text, None, true).unwrap();
        assert_eq!(out["imported"], json!(2));
        assert_eq!(out["rejected"], json!(0));
        assert_eq!(
            load_accounts_checked_for(QoderRegion::Cn)
                .unwrap()
                .accounts
                .len(),
            2
        );
    }

    /// ★ `indexes = Some(&[])` ⇒ 一条都不导。
    ///
    /// 可证伪：把 `Some(&[])` 当成「全部」，`imported` 会变成 2，断言变红 ——
    /// 那正是「用户没勾任何账号却导入了整份文件」的缺陷形态。
    #[test]
    fn import_with_empty_indexes_imports_nothing() {
        let (_home, _guard) = isolated_home("import-empty");
        let text = r#"[
            { "uid": "u1", "nickname": "甲", "domain": "qoder.com.cn", "accessToken": "dt-abcdefghijklmnop" },
            { "uid": "u2", "nickname": "乙", "domain": "qoder.com.cn", "accessToken": "dt-abcdefghijklmnop" }
        ]"#;
        let none_selected: &[usize] = &[];
        let out = import_accounts_for(QoderRegion::Cn, text, Some(none_selected), true).unwrap();
        assert_eq!(out["imported"], json!(0));
        assert!(load_accounts_checked_for(QoderRegion::Cn)
            .unwrap()
            .accounts
            .is_empty());
    }

    /// ★ 跨区 / 空域条目**被拒并如实上报**，不混进 `skipped`。
    ///
    /// 可证伪：把归属校验分支改成静默 `continue`（不计数），`rejected == 2` 变红。
    #[test]
    fn import_rejects_cross_region_entries_and_reports_them() {
        let (_home, _guard) = isolated_home("reject-cross");
        let text = r#"[
            { "uid": "cn-1", "nickname": "国内", "domain": "qoder.com.cn", "accessToken": "dt-abcdefghijklmnop" },
            { "uid": "gl-1", "nickname": "国际", "domain": "qoder.com", "accessToken": "dt-abcdefghijklmnop" },
            { "uid": "no-1", "nickname": "无域", "accessToken": "dt-abcdefghijklmnop" }
        ]"#;
        let out = import_accounts_for(QoderRegion::Cn, text, None, true).unwrap();
        assert_eq!(out["imported"], json!(1));
        assert_eq!(out["rejected"], json!(2), "跨区 + 空域都必须计入 rejected");
        assert_eq!(out["skipped"], json!(0), "被拒的条目不得混进 skipped");

        let stored = load_accounts_checked_for(QoderRegion::Cn).unwrap().accounts;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].uid, "cn-1");
        assert!(out["rejectedDomains"]
            .as_array()
            .unwrap()
            .contains(&json!("qoder.com")));
    }

    /// `overwrite` 参数真的在生效：`false` 保留本地，`true` 覆盖。
    ///
    /// 可证伪：忽略 `overwrite`（永远覆盖），第一条「保留本地」断言变红。
    #[test]
    fn import_respects_the_overwrite_flag() {
        let (_home, _guard) = isolated_home("overwrite");
        seed(QoderRegion::Cn, vec![account("u1", "qoder.com.cn", "本地")]);
        let text = r#"[{ "uid": "u1", "nickname": "导入", "domain": "qoder.com.cn", "accessToken": "dt-abcdefghijklmnop" }]"#;

        let out = import_accounts_for(QoderRegion::Cn, text, None, false).unwrap();
        assert_eq!(out["overwritten"], json!(0));
        assert_eq!(out["skipped"], json!(1));
        assert_eq!(
            load_accounts_checked_for(QoderRegion::Cn).unwrap().accounts[0]
                .nickname
                .as_deref(),
            Some("本地"),
            "overwrite=false 时本地记录不得被覆盖"
        );

        let out = import_accounts_for(QoderRegion::Cn, text, None, true).unwrap();
        assert_eq!(out["imported"], json!(1));
        assert_eq!(out["overwritten"], json!(1));
        assert_eq!(
            load_accounts_checked_for(QoderRegion::Cn).unwrap().accounts[0]
                .nickname
                .as_deref(),
            Some("导入"),
            "overwrite=true 时以导入记录为准"
        );
    }

    /// 没有任何凭据的记录被跳过（否则会入库一个永远登录不了的空壳）。
    ///
    /// 可证伪：去掉凭据存在性判据，`imported` 会变成 1，断言变红。
    #[test]
    fn import_skips_records_without_any_credential() {
        let (_home, _guard) = isolated_home("no-credential");
        let text = r#"[{ "uid": "u1", "nickname": "空壳", "domain": "qoder.com.cn" }]"#;
        let out = import_accounts_for(QoderRegion::Cn, text, None, true).unwrap();
        assert_eq!(out["imported"], json!(0));
        assert_eq!(out["skipped"], json!(1));
        assert!(load_accounts_checked_for(QoderRegion::Cn)
            .unwrap()
            .accounts
            .is_empty());
    }

    /// ★「信封 ≠ 缺失」：只有 PAT 的账号**必须**能导入。
    ///
    /// 可证伪：把凭据判据改成「只看 `accessToken`」，`imported` 变 0，断言变红。
    #[test]
    fn import_accepts_pat_only_records() {
        let (_home, _guard) = isolated_home("pat-only");
        let text = r#"[{ "uid": "u-pat", "nickname": "PAT", "domain": "qoder.com.cn", "personalToken": "pt-abcdefghijklmnop" }]"#;
        let out = import_accounts_for(QoderRegion::Cn, text, None, true).unwrap();
        assert_eq!(out["imported"], json!(1));
        assert_eq!(
            load_accounts_checked_for(QoderRegion::Cn)
                .unwrap()
                .accounts
                .len(),
            1
        );
    }
}
