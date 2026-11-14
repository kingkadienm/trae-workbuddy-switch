//! Trae 侧「恢复」出口的**端到端**验收：冷却文件 → 池 `disabled` → 清冷却 → 复活。
//!
//! # 为什么需要它（这条链此前**没有任何断言**）
//!
//! Trae 的 `TraePoolEntry::disabled` **不是池自己的状态**，而是每次 `sync_for` 从
//! **冷却文件**派生的：
//!
//! ```text
//! disabled = cooldowns[uid].error_type == "SessionDead"
//! ```
//!
//! 于是「恢复」= 清掉那条冷却记录（`credits::clear_cooldown_for`，无条件 `remove`）。
//! 而这条链的每一环**只在源码里成立**，没有任何测试把它串起来：
//!
//! - `TraePool` 的单测全是**手搓条目**（`disabled: false`），从不碰磁盘；
//! - `trae::credits` 的测试不碰 `save_cooldown_for` / `clear_cooldown_for`。
//!
//! ⇒ 「派生判据写错（比如拿 `until` 当判据）」「清冷却没落盘」「池没重新同步」
//! 这三类接线缺陷**构建全绿、类型全对**，只有端到端断言能挡住。
//!
//! # 环境隔离
//!
//! 池从磁盘派生（账号库 + 冷却 + 剩余积分），因此本测试把 `BUDDY_SWITCH_HOME`
//! 指向一个**已存在**的临时目录（该变量要求目录存在，否则会被忽略并**静默回落真实
//! home** —— 那会让断言读到真机数据）。集成测试是独立进程 ⇒ 进程级 env 不干扰
//! lib 单测；本文件只有一个用例，故也不存在用例间竞态。

use std::collections::HashMap;

use buddy_switch_core::modules::trae::account::{self, AccountsFile, RawAccount};
use buddy_switch_core::modules::trae::credits;
use buddy_switch_core::modules::trae::variant::TraeVariant;
use buddy_switch_gateway::trae::now_secs;
use buddy_switch_gateway::trae::pool::TraePool;

/// 池里第一个条目的「此刻可用」判据。
///
/// ★ 刻意用 `healthy()`（无回落）而不是 `pick()`：共用选号内核在**候选全不可用**时
/// 会回落到 `pick_earliest_expiry`（宁可试一个也不报「没有账号」）⇒
/// 「只是被回落捞出来」与「真的可用」在 `pick` 眼里**长得一样**，判不出可用性。
fn sole_entry(pool: &TraePool) -> &buddy_switch_gateway::trae::pool::TraePoolEntry {
    assert_eq!(pool.entries().len(), 1, "本用例只铺了一个账号");
    &pool.entries()[0]
}

#[test]
fn clearing_the_cooldown_revives_the_account_in_the_trae_pool() {
    let dir = std::env::temp_dir().join(format!(
        "buddy-switch-trae-revive-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).expect("临时 home 应可创建");
    std::env::set_var("BUDDY_SWITCH_HOME", &dir);

    let variant = TraeVariant::default();
    let uid = "trae-user-1";

    // 铺一个账号。池会**过滤掉 `jwt` 为空的条目** ⇒ 必须给非空 jwt，
    // 否则池恒为空、下面的断言会在「没有条目」上假失败。
    let file = AccountsFile {
        web_devices: Default::default(),
        accounts: vec![RawAccount {
            name: "测试账号".to_string(),
            user_id: Some(uid.to_string()),
            jwt: "fake-jwt".to_string(),
            refresh_token: None,
            added_at: None,
            updated_at: None,
            remark: None,
        }],
        device_bindings: HashMap::new(),
    };
    account::save_accounts_for_region(variant.region(), &file).expect("账号库应可写");

    // ① 前置对照：**无冷却**时该账号可用。
    //    没有这一步，「复活成功」可能只是「它本来就没被禁用」的空转。
    let mut pool = TraePool::for_variant(variant);
    assert_eq!(pool.sync_for(variant), 1, "账号库应有 1 条（jwt 非空）");
    assert!(!sole_entry(&pool).disabled, "前置：无冷却时不该是禁用");
    assert!(
        sole_entry(&pool).healthy(now_secs()),
        "前置：无冷却时应可用"
    );

    // ② 会话失效（`-1` = 永久）⇒ 池必须把它**派生为禁用**，且不可用。
    credits::save_cooldown_for(variant, uid, "SessionDead", -1, "12153 session dead");

    let mut pool = TraePool::for_variant(variant);
    pool.sync_for(variant);
    assert!(
        sole_entry(&pool).disabled,
        "`SessionDead` 冷却必须派生为 `disabled` —— 这正是「恢复」要解的那个终态"
    );
    assert!(
        !sole_entry(&pool).healthy(now_secs()),
        "被禁用的账号不得被判为可用"
    );
    // 池卡上用户看到的就是这两个计数格 ⇒ 顺带钉住「界面确实会显示已禁用」。
    let summary = pool.summary(now_secs());
    assert_eq!(summary.disabled, 1, "计数格应显示 1 个已禁用");
    assert_eq!(summary.available, 0, "计数格应显示 0 个可用");

    // ③ 走「恢复」实际调用的那条路（`clear_cooldown_for`）⇒ 必须复活。
    credits::clear_cooldown_for(variant, uid).expect("清冷却应成功");

    let mut pool = TraePool::for_variant(variant);
    pool.sync_for(variant);
    assert!(
        !sole_entry(&pool).disabled,
        "清掉冷却记录后必须不再派生为禁用（这是池卡「恢复」按钮的全部意义）"
    );
    assert!(
        sole_entry(&pool).healthy(now_secs()),
        "复活后必须**真的可用**（`healthy()` 无回落，判的就是可用性）"
    );
    let summary = pool.summary(now_secs());
    assert_eq!(summary.disabled, 0, "计数格应回到 0 个已禁用");
    assert_eq!(summary.available, 1, "计数格应显示 1 个可用");

    // ④ 幂等：再清一次不报错（池卡允许重复点；后端 `remove` 无记录时也是 `Ok`）。
    credits::clear_cooldown_for(variant, uid).expect("重复清冷却不应报错");

    std::fs::remove_dir_all(&dir).ok();
}
