//! Qoder 账号回收站 —— 软删除的**可恢复**落点。
//!
//! ## 为什么删除必须是「软」的
//!
//! 账号库里内嵌着凭据（`access_token` / `refresh_token` / `personal_token`）。硬删除
//! 一旦点错就是**不可逆**的数据丢失 —— 令牌
//! 不在别处，用户只能重新走一遍设备授权。把删除改成「移入回收站」后，误删的代价从
//! 「重新登录 + 可能丢 PAT」降到「点一次恢复」，而「真正销毁」被收窄成**唯一一个**显式
//! 动作（[`purge_account_for`]，前端必须过 danger 二次确认）。
//!
//! ## 保留期依据
//!
//! 留存窗口与账号侧（[`crate::modules::account::TRASH_RETENTION_MS`]）**同值：30 天**。
//! 两平台回收站对用户是同一个心智模型（「删了还能捡回来一阵子」），保留期不一致只会
//! 让用户困惑：同样的操作，一边还在、另一边已经没了。条数上限同样对齐
//! （[`TRASH_MAX_ENTRIES`] = 200，先到先出）—— 回收站**含凭据**，不能无限增长，
//! 否则「批量导入 → 批量删除」会把一份永不清理的令牌库留在盘上。
//!
//! ⚠️ 本模块的时间是 **ISO-8601 字符串**（`chrono::Local` + 秒级精度，格式与
//! [`super::store::now_iso`] 完全一致），**不是**账号侧用的毫秒整数。原因：Qoder 的
//! 账号记录本就用 ISO 表示时刻（`addedAt` / `updatedAt`），回收站与账号记录保持同一种
//! 时间表示，落盘文件人读 / diff 都直观。**代价**是「过期判定」与「排序」必须先解析 ——
//! 见 [`sweep_trash_entries`] 与 [`deleted_sort_key`] 里对「解析不出来」的显式处理。
//!
//! ## `key` 的语义（三者必须同源）
//!
//! `key` 就是账号库的唯一键 `uid`（[`RawAccount::uid`]），也是 [`restore_account_for`] /
//! [`purge_account_for`] 的入参。**下发、恢复、永久删除三处必须用同一个谓词**（`entry.key ==
//! key`）—— 若下发用 `uid`、恢复却按 `name` 找，界面会出现「列得出来、点恢复却报找不到」
//! 的静默不一致（用户只会认为「回收站坏了」）。
//!
//! ## 落盘顺序：**新增侧先落，删除侧后落**
//!
//! 软删除（live → trash）与恢复（trash → live）都是「跨两个文件的搬移」，两次写不可能
//! 原子。这里统一采用**先写「接收方」、后写「来源方」**：
//!
//! - 软删除：先写 trash，再写 live。中途失败 ⇒ 账号**同时在两边**（可恢复的冗余），
//!   而不是「live 没了、trash 也没有」⇒ **数据丢失**。
//! - 恢复：先写 live（经 [`super::account::upsert_account_for`]），再写 trash。同理，
//!   最坏是「两边都有」，绝不是「两边都没有」。
//!
//! 反例（**不要**写成先删来源）：先 `save_accounts_for` 去掉账号、再写 trash，一旦第二步
//! 的 IO 失败，账号就从两个文件里**同时消失**，且没有任何报错回滚点。
//!
//! ## fail-closed
//!
//! - 认不出的 `key` ⇒ 恢复 / 永久删除**拒绝**（`Err`），不静默当成功；
//! - 归属与本区不符的账号（`domain` 为空 / 认不出 / 属于另一区）⇒ 软删除与恢复**都拒绝**
//!   （软删除在搬移前用 [`super::account::ensure_region_matches`] 挡一道，恢复由
//!   [`super::account::upsert_account_for`] 挡）。理由：把国际版记录放进国内版库，用户看到
//!   的是「账号在、但登录不了」，错误是无声的。

use chrono::{DateTime, Duration, FixedOffset, Local, SecondsFormat};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::modules::product::scope_of_qoder_region;
use crate::modules::qoder::account::{self, RawAccount};
use crate::modules::qoder::paths;
use crate::modules::qoder::region::QoderRegion;
use crate::modules::qoder::store;

/// 回收站留存窗口（天）。与账号侧保留期同值 —— 改这里必须同步改账号侧，
/// 否则同一个「删除」操作在两个平台上的可恢复时长会不一致。
pub const TRASH_RETENTION_DAYS: i64 = 30;

/// 回收站条数上限（先到先出）。防「批量导入 → 批量删除」把含凭据的文件撑到无限大。
pub const TRASH_MAX_ENTRIES: usize = 200;

/// 回收站条目（**落盘形态**）。
///
/// `camelCase` 序列化：落盘键名（`deletedAt` / `expiresAt`）与下发给前端的键名**刻意同名**
/// —— 少一层「落盘叫 A、下发叫 B」的翻译，就少一处两者漂移的机会。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    /// 唯一键，等于 `uid`（恢复 / 永久删除的入参，见模块头「`key` 的语义」）。
    pub key: String,
    /// 冗余存一份 `uid`，便于审计与界面展示（恢复不依赖它，只认 `key`）。
    pub uid: String,
    /// 删除时刻的**展示名快照**（落盘时取自 [`super::account::display_name`]）。
    ///
    /// 为什么是快照而不是恢复时现算：账号一旦被删，`nickname` 等来源都可能变；回收站里
    /// 展示的应该是「你删的那个名字」，而不是「它现在的名字」。
    pub name: String,
    /// 冗余存 `domain`，便于跨区审计（恢复不依赖它，归属由 upsert 重新校验）。
    pub domain: String,
    /// 删除时间（ISO-8601）。
    #[serde(default)]
    pub deleted_at: String,
    /// 到期时间（ISO-8601）= `deleted_at + TRASH_RETENTION_DAYS`。
    #[serde(default)]
    pub expires_at: String,
    /// 原账号记录**原样保留** ⇒ 恢复无损（含全部令牌）。
    pub account: RawAccount,
}

/// 读某区域的回收站（**容错**：文件缺失 / 损坏 ⇒ 空表）。
///
/// 与 [`load_trash_checked_for`] 的分工：读路径要「界面能起来」，故损坏退化成空表；
/// 写路径必须知道底数，故用 checked 版。回收站损坏退化成空表，代价是「已删账号的凭据
/// 不再出现在列表里」——但它本来就已过期待清，比「整个界面打不开」轻得多。
pub fn load_trash_for(region: QoderRegion) -> Vec<TrashEntry> {
    store::read_json(&paths::accounts_trash_file_for(region))
}

/// 读某区域的回收站，并把「读不到 / 解析失败」如实报出来（**写路径专用**）。
///
/// `Ok(None)`（文件不存在）与 `Ok(Some(vec![]))`（空文件）都归一成空表；只有**解析失败**
/// 才 `Err` —— 那正是「在损坏库上覆盖」必须被调用方看见的情形。
fn load_trash_checked_for(region: QoderRegion) -> Result<Vec<TrashEntry>, String> {
    Ok(
        store::read_json_checked::<Vec<TrashEntry>>(&paths::accounts_trash_file_for(region))?
            .unwrap_or_default(),
    )
}

/// 原子写回收站（先建父目录）。
fn save_trash_to(region: QoderRegion, entries: &[TrashEntry]) -> Result<(), String> {
    store::write_json(&paths::accounts_trash_file_for(region), &entries.to_vec())
}

/// 把 `DateTime<Local>` 格式化成与 [`super::store::now_iso`] **逐字同款**的 ISO-8601。
///
/// 刻意不调 `store::now_iso()`：那个函数内部固定取「此刻」，而这里要格式化的是
/// 「此刻 + 30 天」这个**派生**时刻。自己拼格式则必须与它保持一致，否则同一份文件里会
/// 出现两种时间写法，解析（[`parse_iso`]）就得兼容两套。
fn format_iso(moment: DateTime<Local>) -> String {
    moment.to_rfc3339_opts(SecondsFormat::Secs, false)
}

/// 由「删除时刻」派生 `(deleted_at, expires_at)`。
fn iso_window_from(now: DateTime<Local>) -> (String, String) {
    (
        format_iso(now),
        format_iso(now + Duration::days(TRASH_RETENTION_DAYS)),
    )
}

/// 解析 ISO-8601 时刻（容忍前后空白）。**认不出 ⇒ `None`**，不回落成「现在」——
/// 回落会把一条坏记录伪装成刚写入的，让「过期判定」永远为假。
fn parse_iso(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value.trim()).ok()
}

/// 排序键：删除时刻的时间戳；**解析不出来记 `i64::MIN`**（当作最旧，最先被裁）。
///
/// 为什么不用字符串直接比大小：RFC3339 带时区偏移，跨偏移的字符串序**不等于**时间序
/// （`+08:00` 与 `+00:00` 的同一时刻，字符串大小关系是反的）。用户跨时区使用时就会
/// 看到「新删的排在旧删的后面」。
fn deleted_sort_key(entry: &TrashEntry) -> i64 {
    parse_iso(&entry.deleted_at)
        .map(|moment| moment.timestamp())
        .unwrap_or(i64::MIN)
}

/// 留存策略（**纯函数**，可单测）：清过期 + 裁上限（先到先出）。返回被清理条数。
///
/// - **过期**：`expires_at` 解析不出（字段缺失 / 被手改坏）⇒ 一律视为**已过期**。
///   反例：把解析失败当作「永不过期」，一条读不懂的记录就会把凭据**永久**留在盘上 ——
///   回收站是凭据的临时存放点，不是长期仓库。
/// - **上限**：按 `deleted_at` 升序删最旧的（先到先出）。
fn sweep_trash_entries(entries: &mut Vec<TrashEntry>, now: DateTime<Local>) -> usize {
    let now_ts = now.timestamp();
    let before = entries.len();
    entries.retain(|entry| match parse_iso(&entry.expires_at) {
        Some(expires) => expires.timestamp() > now_ts,
        None => false,
    });
    if entries.len() > TRASH_MAX_ENTRIES {
        entries.sort_by_key(deleted_sort_key);
        let excess = entries.len() - TRASH_MAX_ENTRIES;
        entries.drain(0..excess);
    }
    before - entries.len()
}

/// 软删除：把账号从 live **移入**回收站。
///
/// `uid` 不存在 ⇒ `Ok(false)`（**幂等**：重复删除 / 并发删除不报错，调用方据此区分
/// 「删掉了」与「本来就没有」）。账号归属与本区不符 ⇒ `Err`（fail-closed，见模块头）。
///
/// 落盘顺序见模块头「新增侧先落」。
pub fn trash_account_for(region: QoderRegion, uid: &str) -> Result<bool, String> {
    let mut live = account::load_accounts_checked_for(region)?;
    let Some(index) = live.accounts.iter().position(|account| account.uid == uid) else {
        return Ok(false);
    };
    // ★ 归属校验**刻意不在这里**（与「写库前必须校验」看似矛盾，其实是分工）：
    //   删除是「把它从这份列表里拿掉」，而账号页**故意把归属异常的账号列出来**
    //   （见 `QoderAccountsPage` 的 `domainMismatch` 提示条）。若这里 fail-closed，
    //   那条提示条就变成死路 —— 用户看得到它、却删不掉它，只能手改 JSON。
    //   ★ 真正需要护栏的是**反向**（把记录写进库里），所以校验留在
    //   [`restore_account_for`]（经 `upsert_account_for`）—— 那里才可能污染账号库。
    let account = live.accounts.remove(index);

    let now = Local::now();
    let (deleted_at, expires_at) = iso_window_from(now);
    let mut trash = load_trash_checked_for(region)?;
    // 幂等：先清掉同 key 的旧条目，避免「上次 trash 写成功、live 写失败」的重试留下双份。
    trash.retain(|entry| entry.key != account.uid);
    trash.push(TrashEntry {
        key: account.uid.clone(),
        uid: account.uid.clone(),
        name: account::display_name(&account),
        domain: account.domain.clone(),
        deleted_at,
        expires_at,
        account,
    });
    sweep_trash_entries(&mut trash, now);

    // 新增侧（trash）先落，删除侧（live）后落 —— 中途失败是「两边都有」，不是数据丢失。
    save_trash_to(region, &trash)?;
    account::save_accounts_for(region, &live)?;
    Ok(true)
}

/// 恢复：把账号从回收站**移回** live（谓词 = `entry.key == key`）。
///
/// - 认不出的 `key` ⇒ `Err`（**不静默成功**：界面上的条目是具体的，找不到就是状态陈旧，
///   必须让调用方看见，而不是假装恢复了一条不存在的账号）；
/// - live 已存在同 `uid` ⇒ `Err`（**不覆盖**：恢复不能成为「把现有账号冲掉」的隐蔽入口）；
/// - 归属校验**必须**走 [`super::account::upsert_account_for`]（它带 `ensure_region_matches`），
///   不得绕过 —— 直接 `push` 会把一条错区记录写进本区库。
pub fn restore_account_for(region: QoderRegion, key: &str) -> Result<bool, String> {
    let mut trash = load_trash_checked_for(region)?;
    let Some(index) = trash.iter().position(|entry| entry.key == key) else {
        return Err("回收站中没有该账号".to_string());
    };
    // 先查重、后搬移：命中即拒绝，且此时**尚未**改动任何内存态，更没落盘。
    let live = account::load_accounts_checked_for(region)?;
    if live
        .accounts
        .iter()
        .any(|account| account.uid == trash[index].uid)
    {
        return Err("账号库中已存在该账号，无法恢复".to_string());
    }

    let entry = trash.remove(index);
    // 新增侧（live）先落：upsert 会做归属 + uid 非空校验，失败即 `?` 返回，
    // 此时 trash 的改动只停在内存里、**没有落盘** ⇒ 条目原样保留（fail-closed 不丢数据）。
    account::upsert_account_for(region, entry.account.clone())?;
    // 删除侧（trash）后落。
    save_trash_to(region, &trash)?;
    Ok(true)
}

/// 永久删除（**唯一不可逆步**）：从回收站真删。
///
/// 认不出的 `key` ⇒ `Err`。Qoder 的凭据内嵌在记录里，随记录一并销毁 ⇒ 无需像「登录态
/// 存在独立目录」的平台那样额外清理快照。前端必须走 danger 二次确认。
pub fn purge_account_for(region: QoderRegion, key: &str) -> Result<bool, String> {
    let mut trash = load_trash_checked_for(region)?;
    let before = trash.len();
    // `retain` 会清掉**所有**同 key 条目：即便盘上因历史缺陷存在重复，也能一次清干净。
    trash.retain(|entry| entry.key != key);
    if trash.len() == before {
        return Err("回收站中没有该账号".to_string());
    }
    save_trash_to(region, &trash)?;
    Ok(true)
}

/// 清空某区域回收站，返回删除条数（**不可逆**）。
///
/// 文件损坏（解析失败）时**不覆盖**：返回 0 如实表示「没数出条目」。反例：把损坏当空库
/// 直接写空，等于在一次「界面以为回收站是空的」的误判上，把用户仅存的、也许还能手工抢救的
/// 数据彻底抹掉 —— fail-closed 在这里是「宁可不写」。
pub fn clear_trash_for(region: QoderRegion) -> usize {
    let before = match load_trash_checked_for(region) {
        Ok(entries) => entries.len(),
        Err(_) => 0,
    };
    if before > 0 {
        let _ = save_trash_to(region, &[]);
    }
    before
}

/// 留存清理（单区域）：清过期 + 裁上限。返回被清理条数（`StartupMaintenance` 调用）。
///
/// 只在**确实清掉了东西**时才落盘：空跑不产生 IO，也不会把「读损坏了」误写成「清空了」。
pub fn sweep_trash_for(region: QoderRegion) -> usize {
    let mut trash = match load_trash_checked_for(region) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    let removed = sweep_trash_entries(&mut trash, Local::now());
    if removed > 0 {
        let _ = save_trash_to(region, &trash);
    }
    removed
}

/// 展示字段归一：**字符串或 `null`**。
///
/// 与 [`crate::modules::account::display_str`] 同一条纪律：前端把展示字段直接当 React 子节点
/// 渲染，任何非字符串形态（`undefined` / 对象）都会让 React 卸载整棵树 ⇒ 白屏。
/// 这里入参是**强类型 `&str`**（不是 `Value`）：Qoder 的账号模型是 typed struct，脏值不可能
/// 来自 JSON 解析，唯一可能的「空」就是空串 / 全空白 —— 归成 `null` 交给前端兜底链。
fn display_str(value: &str) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

/// 回收站条目的**线上形态**（camelCase，供前端渲染）。
///
/// ⚠️ `key` 必须**原样**回传 `entry.key`，不能在这里做任何「兜底换值」—— 前端拿它回传
/// 恢复 / 永久删除，换过值就再也命中不了（见模块头「`key` 的语义」）。
/// ISO-8601 → **毫秒**时间戳；解析不出 ⇒ `0`。
///
/// ⚠️ 为什么线上形态用毫秒而**不是**原样把 ISO 字符串发出去：回收站抽屉是
/// **三平台共用壳**（`src/components/recycle-bin-sheet.tsx`），它的 `TrashEntry.deletedAt`
/// / `expiresAt` 是 `number` 且直接喂给 `formatDate(ts: number)`。发字符串会
/// ① 与 TS 类型不符、② 让「同一份契约」在第三个平台上出现第二种形态 —— 前端为了兼容它
/// 必须长出平台分支，那正是本仓反复禁止的「同款实现散落多处」。
/// 落盘仍用 ISO（人读 / diff 直观，与 Qoder 账号记录的 `addedAt` / `updatedAt` 同款），
/// 只在**出口**换算一次。
///
/// 解析不出（字段缺失 / 被手改坏）⇒ `0`：前端 `formatDate(0)` 渲染成 `—`（未知），
/// **不是** 1970 —— 用「编一个时间」掩盖读不懂的字段才是谎报。
fn iso_to_ms(value: &str) -> i64 {
    parse_iso(value)
        .map(|moment| moment.timestamp_millis())
        .unwrap_or(0)
}

fn trash_entry_view(entry: &TrashEntry, region: QoderRegion) -> Value {
    // 展示名优先用落盘快照；快照为空（被手改坏）时现算一次 —— `display_name` 恒非空，
    // 因此这里**永远**给出一个非空字符串，不会把空串交给前端。
    let name = if entry.name.trim().is_empty() {
        account::display_name(&entry.account)
    } else {
        entry.name.clone()
    };
    json!({
        "key": Value::String(entry.key.clone()),
        "name": Value::String(name),
        "uid": display_str(&entry.uid),
        "domain": display_str(&entry.domain),
        "deletedAt": iso_to_ms(&entry.deleted_at),
        "expiresAt": iso_to_ms(&entry.expires_at),
        // ★ `originScope` 与 `key` / `name` 同款：共用壳拿它渲染「来源作用域」徽章。
        //   取**作用域 id**（`qoder-cn`）而不是裸 region（`cn`）—— 后者在共用壳里会与
        //   WB / Trae 的同名值撞车，用户分不出这条是哪个平台的。
        "originScope": Value::String(scope_of_qoder_region(region).as_str().to_string()),
    })
}

/// 回收站列表的线上形态：`{ scope, entries }`（按删除时间**倒序**，最新在前）。
///
/// 键名是 `scope` 而**不是** `region`：这是三平台共用壳的契约
/// （`TrashList = { scope: string; entries: TrashEntry[] }`）。用 `region` 会让前端
/// 读不到作用域，得为 Qoder 加一条分支 —— 见 [`iso_to_ms`] 的同类理由。
pub fn trash_list_value(region: QoderRegion) -> Value {
    let mut entries = load_trash_for(region);
    entries.sort_by_key(|entry| std::cmp::Reverse(deleted_sort_key(entry)));
    let views: Vec<Value> = entries
        .iter()
        .map(|entry| trash_entry_view(entry, region))
        .collect();
    json!({ "scope": scope_of_qoder_region(region).as_str(), "entries": views })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::qoder::region::realm_spec;

    /// 隔离 home：所有落盘测试都跑在临时目录，绝不触碰真实 `~/.buddy-switch`。
    ///
    /// 复用 [`crate::modules::config::HomeOverrideGuard`]（它内部取进程级 env 锁，测试结束
    /// 恢复原值），与账号侧 / 另一平台侧的测试手法一致 —— 不自己 `set_var`，否则并发测试
    /// 会互相看到对方的 home。
    fn with_temp_home<T>(f: impl FnOnce() -> T) -> T {
        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-qoder-trash-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("临时 home 应能创建");
        let guard = crate::modules::config::HomeOverrideGuard::set(&dir);
        let out = f();
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    /// 往某区 live 库写入一条**归属正确**的账号，返回它。
    fn seed_account(region: QoderRegion, uid: &str) -> RawAccount {
        let account = RawAccount {
            uid: uid.to_string(),
            nickname: Some(format!("昵称-{uid}")),
            domain: realm_spec(region).domain.to_string(),
            access_token: Some("dt-access".to_string()),
            refresh_token: Some("drt-refresh".to_string()),
            ..Default::default()
        };
        account::upsert_account_for(region, account.clone()).expect("seed account");
        account
    }

    /// 造一条回收站条目（绕过 soft-delete，用于直接摆布 trash 文件）。
    fn entry_with(region: QoderRegion, uid: &str, expires_at: String) -> TrashEntry {
        let account = RawAccount {
            uid: uid.to_string(),
            domain: realm_spec(region).domain.to_string(),
            ..Default::default()
        };
        TrashEntry {
            key: uid.to_string(),
            uid: uid.to_string(),
            name: uid.to_string(),
            domain: account.domain.clone(),
            deleted_at: format_iso(Local::now() - Duration::days(1)),
            expires_at,
            account,
        }
    }

    /// ★ 软删除：账号离开 live、进入 trash，且**凭据随记录一起搬走**（恢复无损）。
    ///
    /// 可证伪：把 `trash_account_for` 里的 `return Ok(false)` 改成 `Ok(true)`（或把
    /// `position` 谓词写成恒 false），末尾两条「不存在 ⇒ false」的断言立即变红 ——
    /// 那正是「删了不存在的账号却报成功」的静默缺陷。
    #[test]
    fn trash_moves_the_account_out_of_live_and_is_idempotent() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            seed_account(region, "u1");
            seed_account(region, "u2");

            assert!(trash_account_for(region, "u1").expect("soft delete"));

            let live = account::load_accounts_for(region);
            assert_eq!(live.accounts.len(), 1, "live 应只剩 u2");
            assert_eq!(live.accounts[0].uid, "u2");

            let trash = load_trash_for(region);
            assert_eq!(trash.len(), 1);
            assert_eq!(trash[0].key, "u1");
            assert_eq!(trash[0].uid, "u1");
            assert_eq!(trash[0].account.uid, "u1");
            // 凭据随记录原样搬走（恢复无损的关键）。
            assert_eq!(trash[0].account.access_token.as_deref(), Some("dt-access"));
            assert_eq!(trash[0].name, "昵称-u1", "展示名应是删除时的快照");

            // 不存在的 uid ⇒ Ok(false)，且不动 trash。
            assert!(!trash_account_for(region, "nope").expect("missing is a no-op"));
            assert_eq!(
                load_trash_for(region).len(),
                1,
                "no-op 不得往回收站里塞东西"
            );
        });
    }

    /// ★ 归属异常的账号**允许软删除**，但**恢复会被拒**（护栏在反向那条路上）。
    ///
    /// 分工理由（见 [`trash_account_for`] 的注释）：账号页**故意把归属异常的账号列出来**
    /// 并给提示条，若删除 fail-closed，那条提示条就是死路（看得到、删不掉）。
    /// 而「写回账号库」才可能污染库 ⇒ 恢复必须 fail-closed。
    ///
    /// 可证伪：
    /// - 把 `ensure_region_matches` 加回 `trash_account_for` ⇒ 第一条断言变红；
    /// - 把 `restore_account_for` 里的 `upsert_account_for` 换成直接 `push` ⇒ 第三条变红。
    #[test]
    fn foreign_domain_account_can_be_deleted_but_not_restored() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            // 绕过 upsert 的归属校验，直接把一条国际版记录塞进国内库。
            let foreign = RawAccount {
                uid: "g1".to_string(),
                domain: "qoder.com".to_string(),
                ..Default::default()
            };
            account::save_accounts_for(
                region,
                &account::AccountsFile {
                    accounts: vec![foreign],
                },
            )
            .expect("seed foreign record");

            assert!(
                trash_account_for(region, "g1").expect("删除必须放行"),
                "归属异常也必须能删掉（否则提示条是死路）"
            );
            assert!(account::load_accounts_for(region).accounts.is_empty());
            assert_eq!(load_trash_for(region).len(), 1);

            // 反向必须拒：恢复会把错区记录写进国内库。
            assert!(
                restore_account_for(region, "g1").is_err(),
                "错区记录必须拒绝恢复"
            );
            assert_eq!(load_trash_for(region).len(), 1, "被拒后条目不丢");
        });
    }

    /// 恢复：账号回到 live、离开 trash；未知 key 与「live 已存在同 uid」都被拒绝。
    ///
    /// 可证伪：
    /// - 把未知 key 的 `Err` 改成 `Ok(false)` ⇒ 第一条断言变红（不静默成功）；
    /// - 删掉「live 已存在同 uid」的查重 ⇒ 最后一条断言变红（会把现有账号冲掉）。
    #[test]
    fn restore_moves_back_and_refuses_unknown_key_and_overwrite() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            seed_account(region, "u1");
            trash_account_for(region, "u1").expect("soft delete");

            assert!(restore_account_for(region, "u1").expect("restore"));
            let live = account::load_accounts_for(region);
            assert_eq!(live.accounts.len(), 1);
            assert_eq!(live.accounts[0].uid, "u1");
            assert!(
                load_trash_for(region).is_empty(),
                "恢复后回收站应清空该条目"
            );

            // 认不出的 key ⇒ 拒绝（fail-closed）。
            assert!(restore_account_for(region, "ghost").is_err());

            // live 已存在同 uid ⇒ 拒绝，不覆盖。
            seed_account(region, "u2");
            trash_account_for(region, "u2").expect("soft delete");
            seed_account(region, "u2"); // 恢复前又出现了一条同 uid
            assert!(
                restore_account_for(region, "u2").is_err(),
                "live 已有同 uid 时必须拒绝，不能覆盖"
            );
        });
    }

    /// ★ fail-closed：恢复一条**归属不符**的条目会被 upsert 挡下，且条目不丢。
    ///
    /// 可证伪：把 `restore_account_for` 里的 `upsert_account_for(...)` 换成直接
    /// `live.accounts.push(entry.account)`，本用例立即变红（错区记录被写进国内库）。
    #[test]
    fn restore_is_refused_when_the_entry_does_not_belong_to_the_region() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            // 直接往回收站塞一条归属国际版的条目（模拟手改 / 跨区搬来的脏数据）。
            let foreign = TrashEntry {
                key: "g1".to_string(),
                uid: "g1".to_string(),
                name: "国际版".to_string(),
                domain: "qoder.com".to_string(),
                deleted_at: format_iso(Local::now()),
                expires_at: format_iso(Local::now() + Duration::days(TRASH_RETENTION_DAYS)),
                account: RawAccount {
                    uid: "g1".to_string(),
                    domain: "qoder.com".to_string(),
                    ..Default::default()
                },
            };
            store::write_json(&paths::accounts_trash_file_for(region), &vec![foreign])
                .expect("seed trash");

            assert!(
                restore_account_for(region, "g1").is_err(),
                "错区条目必须被拒"
            );
            // 拒绝后条目仍在回收站（不丢数据），live 仍为空。
            assert_eq!(load_trash_for(region).len(), 1);
            assert!(account::load_accounts_for(region).accounts.is_empty());
        });
    }

    /// 永久删除：条目消失；再删一次（认不出 key）⇒ 拒绝。
    ///
    /// 可证伪：把 `purge_account_for` 的 `Err` 改成 `Ok(false)`，最后一条断言变红。
    #[test]
    fn purge_removes_the_entry_and_rejects_unknown_key() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            seed_account(region, "u1");
            trash_account_for(region, "u1").expect("soft delete");

            assert!(purge_account_for(region, "u1").expect("purge"));
            assert!(load_trash_for(region).is_empty());

            assert!(
                purge_account_for(region, "u1").is_err(),
                "已不存在的 key 必须拒绝，不静默成功"
            );
        });
    }

    /// 清空：返回被删条数；已空时返回 0 且不报错。
    ///
    /// 可证伪：把返回值改成 `0` 常量，第一条断言变红。
    #[test]
    fn clear_trash_returns_the_removed_count() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            seed_account(region, "u1");
            seed_account(region, "u2");
            trash_account_for(region, "u1").expect("soft delete u1");
            trash_account_for(region, "u2").expect("soft delete u2");

            assert_eq!(clear_trash_for(region), 2);
            assert!(load_trash_for(region).is_empty());
            assert_eq!(clear_trash_for(region), 0, "空回收站应返回 0");
        });
    }

    /// ★ 留存清理：过期的清掉、未过期的留下，返回值 = 清理条数。
    ///
    /// 可证伪：把 `sweep_trash_entries` 的 `retain` 条件写成恒 true（或恒 false），
    /// 本用例的「清理 1 条 / 剩 1 条」立即变红。
    #[test]
    fn sweep_drops_expired_entries_and_keeps_fresh_ones() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            let past = format_iso(Local::now() - Duration::days(1));
            let future = format_iso(Local::now() + Duration::days(1));
            store::write_json(
                &paths::accounts_trash_file_for(region),
                &vec![
                    entry_with(region, "expired", past),
                    entry_with(region, "fresh", future),
                ],
            )
            .expect("seed trash");

            assert_eq!(sweep_trash_for(region), 1, "只应清掉过期的那条");
            let left = load_trash_for(region);
            assert_eq!(left.len(), 1);
            assert_eq!(left[0].key, "fresh");
        });
    }

    /// ★ 下发形状**逐字**等于三平台共用壳的契约：`{ scope, entries:[{key,name,uid,deletedAt,expiresAt,originScope}] }`，
    /// 且 `deletedAt` / `expiresAt` 是**毫秒数字**（不是 ISO 字符串）。
    ///
    /// 可证伪：
    /// - 把 `trash_list_value` 的 `"scope"` 改回 `"region"` ⇒ 第一条断言变红；
    /// - 把 `iso_to_ms` 换成 `display_str`（发字符串）⇒ 「毫秒数字」断言变红 ——
    ///   那正是「同一份回收站契约在第三个平台上长出第二种形态」的缺陷形态；
    /// - 抽掉 `originScope` ⇒ 最后一条断言变红（共用壳的「来源作用域」徽章会渲染空白）。
    #[test]
    fn trash_list_value_has_the_shared_frontend_shape() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            seed_account(region, "u1");
            trash_account_for(region, "u1").expect("soft delete");

            let value = trash_list_value(region);
            assert_eq!(
                value["scope"],
                json!("qoder-cn"),
                "键名必须是 scope（共用壳契约）"
            );

            let entries = value["entries"].as_array().expect("entries 必须是数组");
            assert_eq!(entries.len(), 1);
            let entry = &entries[0];
            assert_eq!(entry["key"], json!("u1"));
            assert_eq!(entry["uid"], json!("u1"));
            assert_eq!(entry["domain"], json!("qoder.com.cn"));
            assert_eq!(entry["name"], json!("昵称-u1"));
            assert_eq!(entry["originScope"], json!("qoder-cn"));

            // 时间字段必须是**毫秒数字**（前端 `formatDate(ts: number)`）。
            let deleted = entry["deletedAt"].as_i64().expect("deletedAt 必须是数字");
            let expires = entry["expiresAt"].as_i64().expect("expiresAt 必须是数字");
            assert!(deleted > 1_600_000_000_000, "应是毫秒而非秒：{deleted}");
            assert!(expires > deleted, "expiresAt 应晚于 deletedAt");
            assert_eq!(
                (expires - deleted) / 86_400_000,
                TRASH_RETENTION_DAYS,
                "线上形态的保留期也必须与共用保留期一致"
            );
        });
    }

    /// ★ 保留期：`expires_at - deleted_at` 恰好是 [`TRASH_RETENTION_DAYS`] 天。
    ///
    /// 可证伪：把 `TRASH_RETENTION_DAYS` 改成 7，本用例立即变红 —— 那正是「两个平台的
    /// 回收站保留期悄悄不一致」的缺陷形态。
    #[test]
    fn retention_window_matches_the_shared_retention_period() {
        with_temp_home(|| {
            let region = QoderRegion::Cn;
            seed_account(region, "u1");
            trash_account_for(region, "u1").expect("soft delete");

            let entry = &load_trash_for(region)[0];
            let deleted = parse_iso(&entry.deleted_at).expect("deleted_at 可解析");
            let expires = parse_iso(&entry.expires_at).expect("expires_at 可解析");
            assert_eq!((expires - deleted).num_days(), TRASH_RETENTION_DAYS);
        });
    }

    /// ★ 两个区域的回收站**互相独立**（软删除不会跨区泄漏）。
    ///
    /// 可证伪：把 `load_trash_for` 的路径派生改成不带 region，本用例立即变红 ——
    /// 那正是「国内版删掉的账号出现在国际版回收站」的隔离破裂。
    #[test]
    fn the_two_regions_have_independent_trash() {
        with_temp_home(|| {
            let cn = QoderRegion::Cn;
            let global = QoderRegion::Global;
            seed_account(cn, "cn-1");
            seed_account(global, "g-1");

            trash_account_for(cn, "cn-1").expect("soft delete cn");

            assert_eq!(load_trash_for(cn).len(), 1);
            assert!(
                load_trash_for(global).is_empty(),
                "国内版的删除不得出现在国际版回收站"
            );
            assert!(trash_list_value(global)["entries"]
                .as_array()
                .expect("entries 必须是数组")
                .is_empty());
        });
    }
}
