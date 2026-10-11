//! 选号算法：四因子加权 + 成本分层硬过滤 + Top-N 短名单 + 防惊群 + LRU 兜底。
//!
//! 移植自参考实现 `internal/pool/pick.go`。
//!
//! **为什么不只是「按权重排序取第一名」**：单点最优会让同一账号被连续打死（惊群），
//! 也会让「快过期积分优先消耗」与「闲置补偿」这些目标互相压制。因此实际策略是
//! 「分层 → 排序 → 取短名单 → 在短名单内按权重抽签」，把确定性排序转化为有界随机。
//!
//! 实现约束：候选快照在**同一把锁窗口内**一次性拷贝出所需字段，之后不再持有对池的
//! 借用——否则「选出结果后回写 `last_used`」会与候选列表的借用冲突。

use std::collections::{BTreeMap, HashSet};

use crate::pool::entry::{CostTier, PoolEntry};
use crate::pool::RealmTag;
use crate::rng::Pcg32;

/// 成本分层与短名单共用的策略参数。
#[derive(Debug, Clone, Copy)]
pub struct PickPolicy {
    /// 快过期积分占比的权重（参考实现 `expiringWeight`）。
    pub expiring_weight: f64,
    /// 闲置补偿：每小时增加的权重。
    pub idle_weight_per_hour: f64,
    /// 闲置补偿权重上限。
    pub idle_weight_max: f64,
    /// 防惊群间隔：此毫秒数内刚被选中的账号不进抽签池。
    pub min_pick_gap_ms: i64,
    /// 成本账本有效期。
    pub model_cost_ttl_ms: i64,
    /// 单账号在途上限（0 = 不限）。
    pub max_in_flight: i64,
    /// 国际版账号在途上限（0 = 沿用 `max_in_flight`）。
    pub max_in_flight_global: i64,
}

/// 短名单长度。
pub const SHORTLIST_SIZE: usize = 5;
/// 权重比较的浮点容差（判定「等权重」用）。
const WEIGHT_EPSILON: f64 = 1e-9;
/// 加权抽签的定点缩放。
const DRAW_SCALE: f64 = 1_000_000.0;

/// 候选快照（不持有对池的借用）。
#[derive(Debug, Clone)]
struct Candidate {
    uid: String,
    weight: f64,
    cost_per_1k: f64,
    used_seq: u64,
    last_used_ms: i64,
    tier: u8,
}

/// 计算四因子权重。
///
/// ```text
/// w = 1
///   + credits/max_credits * 10                     （积分比例，归一化到候选集最大值）
///   + credits_expiring/credits * expiring_weight   （快过期占比，越紧迫越高）
///   + idle_weight                                  （闲置补偿，0.5/小时，封顶 5.0）
///   + success_ratio * 3                            （成功率 EMA 比率；无记录取 1.5）
/// ```
///
/// 归一化口径：**积分除以分层过滤前的候选集最大值**（截断与抽签共用同一基准），
/// 因此「积分最多的账号」恒得满配 10 分。候选积分全为 0 时该项跳过，
/// 而不是退化为均匀随机——否则零积分账号会获得与有余额账号相同的权重。
pub fn weight_of(entry: &PoolEntry, max_credits: i64, now_ms: i64, policy: &PickPolicy) -> f64 {
    let mut weight = 1.0;

    if max_credits > 0 {
        weight += (entry.credits.max(0) as f64 / max_credits as f64) * 10.0;
    }

    if entry.credits > 0 && entry.credits_expiring > 0 {
        let ratio = (entry.credits_expiring as f64 / entry.credits as f64).clamp(0.0, 1.0);
        weight += ratio * policy.expiring_weight;
    }

    weight += idle_weight(entry.last_used_ms, now_ms, policy);

    let success = entry.success_ema.max(0.0);
    let error = entry.error_ema.max(0.0);
    let total = success + error;
    weight += if total > 0.0 {
        (success / total) * 3.0
    } else {
        // 无历史记录：给中性值，既不被惩罚也不被优待。
        1.5
    };

    weight
}

/// 闲置补偿权重：从未使用按上限给（鼓励启用新号）；否则随时长线性增长并封顶。
fn idle_weight(last_used_ms: i64, now_ms: i64, policy: &PickPolicy) -> f64 {
    if last_used_ms <= 0 {
        return policy.idle_weight_max.max(0.0);
    }
    let idle_ms = now_ms.saturating_sub(last_used_ms).max(0);
    let hours = idle_ms as f64 / 3_600_000.0;
    (hours * policy.idle_weight_per_hour).clamp(0.0, policy.idle_weight_max.max(0.0))
}

/// 该账号是否已占满在途额度。
/// 单条目的可用性判据 —— **唯一来源**。
///
/// [`pick`] 的候选过滤走这里；日后若要接「会话粘性」，粘性的可用性检查**也必须**走这里。
///
/// ★ 为什么要有这个名字：粘性要回答「上次那个账号现在还能不能用」。若它另写一套判据
/// 且比 `pick` 宽松，就会把请求钉到一个 `pick` 认为不可用的账号上
/// （症状：**明明有别的号可用却一直失败**）。两处判据一旦分家，这种缺陷不会报错、
/// 只会表现成「偶发失败」。抽成一个函数是唯一可靠的做法。
fn entry_usable(entry: &PoolEntry, policy: &PickPolicy, now_ms: i64, model: &str) -> bool {
    !entry.deactivated && entry.healthy_for_request(now_ms, model) && !in_flight_full(entry, policy)
}

/// 指定 uid 此刻是否可用于该请求（**与 [`pick`] 同一套判据**，见 [`entry_usable`]）。
///
/// 供**会话粘性**使用：粘性会先问「上次那个账号还能不能用」，能用就复用、不能用就解绑换号。
pub fn is_usable(
    entries: &BTreeMap<String, PoolEntry>,
    policy: &PickPolicy,
    uid: &str,
    now_ms: i64,
    model: &str,
) -> bool {
    entries
        .get(uid)
        .map(|entry| entry_usable(entry, policy, now_ms, model))
        .unwrap_or(false)
}

fn in_flight_full(entry: &PoolEntry, policy: &PickPolicy) -> bool {
    let limit = match entry.realm {
        Some(RealmTag::Global) if policy.max_in_flight_global > 0 => policy.max_in_flight_global,
        _ => policy.max_in_flight,
    };
    // 0 = 不限（计数仍累加，但永不拒绝）。
    limit > 0 && entry.in_flight >= limit
}

/// 从池中选出一个账号；返回被选中账号的 uid。
///
/// `tried` 是本轮已尝试过的 uid（换号重试时由调用方累加）。
/// `seed` 决定抽签与洗牌结果——生产用时间戳+序号，测试用固定值以保证可复现。
pub fn pick(
    entries: &mut BTreeMap<String, PoolEntry>,
    pick_seq: &mut u64,
    policy: &PickPolicy,
    now_ms: i64,
    realm: Option<RealmTag>,
    model: &str,
    tried: &HashSet<String>,
    seed: u64,
) -> Option<String> {
    // 惰性清理过期的模型级冷却（与选号共用同一次写锁窗口）。
    for entry in entries.values_mut() {
        entry.prune_model_cooldowns(now_ms);
    }

    let candidate_uids: Vec<String> = entries
        .values()
        .filter(|entry| {
            !tried.contains(&entry.uid)
                && entry_usable(entry, policy, now_ms, model)
                && realm.map(|wanted| entry.realm == Some(wanted)).unwrap_or(true)
        })
        .map(|entry| entry.uid.clone())
        .collect();

    if candidate_uids.is_empty() {
        return pick_earliest_expiry(entries, policy, now_ms, realm, tried);
    }

    // 归一化基准：分层过滤**之前**的候选集最大值。
    let max_credits = candidate_uids
        .iter()
        .filter_map(|uid| entries.get(uid))
        .map(|entry| entry.credits)
        .max()
        .unwrap_or(0);

    let mut candidates: Vec<Candidate> = candidate_uids
        .iter()
        .filter_map(|uid| entries.get(uid))
        .map(|entry| Candidate {
            uid: entry.uid.clone(),
            weight: weight_of(entry, max_credits, now_ms, policy),
            cost_per_1k: entry.cost_per_1k(now_ms, model, policy.model_cost_ttl_ms),
            used_seq: entry.used_seq,
            last_used_ms: entry.last_used_ms,
            tier: entry
                .cost_tier(now_ms, model, policy.model_cost_ttl_ms)
                .rank(),
        })
        .collect();

    // 成本分层硬过滤：免费 > 无观测 > 收费。
    // 设计意图：让「未知号」保留被观测的机会（否则一旦有号实测收费，
    // 所有无观测号会被永久排除，账本永远学不到新数据）。
    let best_tier = candidates
        .iter()
        .map(|candidate| candidate.tier)
        .min()
        .unwrap_or(CostTier::Unknown.rank());
    candidates.retain(|candidate| candidate.tier == best_tier);

    // 等权重洗牌：仅当候选超过短名单长度且存在权重并列时执行。
    // 作用：让并列组的 Top-5 不总是同一批 uid（否则短名单会退化成 uid 序前 5 名）。
    let mut rng = Pcg32::new(seed, candidates.len() as u64 + 1);
    if candidates.len() > SHORTLIST_SIZE {
        let reference = candidates[0].weight;
        let has_tie = candidates
            .iter()
            .any(|candidate| (candidate.weight - reference).abs() < WEIGHT_EPSILON);
        if has_tie {
            rng.shuffle(&mut candidates);
        }
    }

    // 稳定排序：单价升序 → 权重降序。并列项保持上一步的（可能已打乱的）相对顺序；
    // 未洗牌时并列项保持 BTreeMap 的 uid 升序，因此测试可复现。
    candidates.sort_by(|left, right| {
        left.cost_per_1k
            .partial_cmp(&right.cost_per_1k)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                right
                    .weight
                    .partial_cmp(&left.weight)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });

    let shortlist_len = candidates.len().min(SHORTLIST_SIZE);
    let shortlist = &candidates[..shortlist_len];

    // 防惊群：短名单内剔除「刚被选过」的账号。被挤出的号**不回填**——
    // 回填会让短名单失去「限流」作用。
    let eligible: Vec<&Candidate> = shortlist
        .iter()
        .filter(|candidate| {
            candidate.last_used_ms <= 0
                || now_ms.saturating_sub(candidate.last_used_ms) >= policy.min_pick_gap_ms
        })
        .collect();

    let chosen_uid = if eligible.is_empty() {
        // LRU 兜底：短名单全被防惊群剔除时，取**全局** used_seq 最小者
        // （与时钟精度无关，因此不受系统时间回拨影响）。
        candidates
            .iter()
            .min_by_key(|candidate| candidate.used_seq)
            .map(|candidate| candidate.uid.clone())
    } else {
        draw_weighted(&eligible, &mut rng).map(|candidate| candidate.uid.clone())
    };

    chosen_uid.map(|uid| {
        *pick_seq += 1;
        if let Some(entry) = entries.get_mut(&uid) {
            entry.last_used_ms = now_ms;
            entry.used_seq = *pick_seq;
        }
        uid
    })
}

/// 加权抽签：`wi = round(w * 1e6)`，下界 1；前缀累积首个命中者胜出。
fn draw_weighted<'a>(items: &[&'a Candidate], rng: &mut Pcg32) -> Option<&'a Candidate> {
    if items.is_empty() {
        return None;
    }
    let weights: Vec<i64> = items
        .iter()
        .map(|item| {
            let scaled = item.weight * DRAW_SCALE;
            if !scaled.is_finite() || scaled < 1.0 {
                1
            } else {
                scaled.round() as i64
            }
        })
        .collect();
    let total: i64 = weights.iter().sum();
    if total <= 0 {
        return items.first().copied();
    }

    let mut ticket = rng.below(total as u64) as i64;
    for (index, weight) in weights.iter().enumerate() {
        if ticket < *weight {
            return Some(items[index]);
        }
        ticket -= weight;
    }
    items.last().copied()
}

/// 全冷却兜底：池内无健康候选时，挑「最早解冻」的账号强试一次。
///
/// 约束：非 `tried`、realm 匹配、非禁用、**排除仍在有效硬冷却中的账号**
/// （硬冷却意味着余额不足，强试必然再失败且浪费一次上游调用）。
/// 不推进 `used_seq`（它只是「试一下」，不应影响 LRU 语义）。
fn pick_earliest_expiry(
    entries: &mut BTreeMap<String, PoolEntry>,
    policy: &PickPolicy,
    now_ms: i64,
    realm: Option<RealmTag>,
    tried: &HashSet<String>,
) -> Option<String> {
    let candidate = entries
        .values()
        .filter(|entry| {
            if tried.contains(&entry.uid) || entry.disabled {
                return false;
            }
            if let Some(wanted) = realm {
                if entry.realm != Some(wanted) {
                    return false;
                }
            }
            if in_flight_full(entry, policy) {
                return false;
            }
            // 硬冷却期间余额不足，强试无意义。
            if entry.cool_kind == Some(crate::pool::entry::CoolKind::Hard) && now_ms < entry.until_ms {
                return false;
            }
            entry.expiry_ms() > 0
        })
        .min_by_key(|entry| (entry.expiry_ms(), entry.uid.clone()))
        .map(|entry| entry.uid.clone())?;

    if let Some(entry) = entries.get_mut(&candidate) {
        entry.last_used_ms = now_ms;
    }
    Some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::entry::PoolEntry;
    use crate::pool::ModelCooldown;

    fn policy() -> PickPolicy {
        PickPolicy {
            expiring_weight: 8.0,
            idle_weight_per_hour: 0.5,
            idle_weight_max: 5.0,
            min_pick_gap_ms: 100,
            model_cost_ttl_ms: 6 * 60 * 60 * 1000,
            max_in_flight: 3,
            max_in_flight_global: 2,
        }
    }

    fn pool_of(entries: Vec<PoolEntry>) -> BTreeMap<String, PoolEntry> {
        entries
            .into_iter()
            .map(|entry| (entry.uid.clone(), entry))
            .collect()
    }

    fn base_entry(uid: &str) -> PoolEntry {
        PoolEntry::new(uid)
    }

    const NOW: i64 = 1_000_000_000_000;

    #[test]
    fn weight_increases_with_credit_ratio_and_is_bounded() {
        let mut small = base_entry("small");
        small.credits = 100;
        let mut big = base_entry("big");
        big.credits = 1000;

        let small_weight = weight_of(&small, 1000, NOW, &policy());
        let big_weight = weight_of(&big, 1000, NOW, &policy());

        // 基准 1 + 积分项 10 + 闲置项 5（从未使用）+ 成功率项 1.5 = 17.5
        assert!((big_weight - 17.5).abs() < 1e-9, "满配应约 17.5，实际 {big_weight}");
        assert!(
            (small_weight - (1.0 + 1.0 + 5.0 + 1.5)).abs() < 1e-9,
            "实际 {small_weight}"
        );
        assert!(big_weight > small_weight);
    }

    #[test]
    fn zero_max_credits_skips_credit_term_instead_of_uniformizing() {
        let mut entry = base_entry("a");
        entry.credits = 0;
        // max=0 时积分项跳过：1 + 闲置 5 + 成功率 1.5 = 7.5
        let weight = weight_of(&entry, 0, NOW, &policy());
        assert!((weight - 7.5).abs() < 1e-9, "实际 {weight}");
    }

    #[test]
    fn expiring_ratio_uses_own_credits_as_denominator() {
        let mut entry = base_entry("a");
        entry.credits = 1000;
        entry.credits_expiring = 500;
        entry.last_used_ms = NOW; // 闲置项归零
        // 1 + 10 + 0.5*8 + 1.5 = 16.5
        let weight = weight_of(&entry, 1000, NOW, &policy());
        assert!((weight - 16.5).abs() < 1e-9, "实际 {weight}");

        // 占比超过 100% 时必须钳制（脏数据防御）
        entry.credits_expiring = 5000;
        let clamped = weight_of(&entry, 1000, NOW, &policy());
        assert!((clamped - 20.5).abs() < 1e-9, "占比应钳到 1.0，实际 {clamped}");
    }

    #[test]
    fn idle_weight_is_capped_and_never_used_gets_max() {
        let mut entry = base_entry("a");
        entry.last_used_ms = 0;
        let never_used = weight_of(&entry, 0, NOW, &policy());
        entry.last_used_ms = NOW - 100 * 3_600_000; // 闲置 100 小时
        let long_idle = weight_of(&entry, 0, NOW, &policy());
        assert!((never_used - long_idle).abs() < 1e-9, "两者都应取上限 5.0");

        entry.last_used_ms = NOW - 3_600_000; // 闲置 1 小时
        let one_hour = weight_of(&entry, 0, NOW, &policy());
        assert!(
            (one_hour - (1.0 + 0.5 + 1.5)).abs() < 1e-9,
            "1 小时闲置 → 0.5 权重，实际 {one_hour}"
        );
    }

    #[test]
    fn success_ratio_uses_ema_and_defaults_to_neutral() {
        let mut good = base_entry("good");
        good.success_ema = 0.9;
        good.error_ema = 0.1;
        let good_weight = weight_of(&good, 0, NOW, &policy());
        // 1 + 闲置 5 + 0.9*3 = 8.7
        assert!((good_weight - 8.7).abs() < 1e-9, "实际 {good_weight}");

        let neutral = base_entry("neutral");
        let neutral_weight = weight_of(&neutral, 0, NOW, &policy());
        assert!((neutral_weight - 7.5).abs() < 1e-9, "无记录应取 1.5");
        assert!(good_weight > neutral_weight);
    }

    #[test]
    fn in_flight_full_blocks_candidate() {
        let mut entry = base_entry("a");
        entry.in_flight = 3;
        let mut pool = pool_of(vec![entry]);
        let mut seq = 0;
        assert_eq!(
            pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 1),
            None,
            "占满在途的账号不得被选中"
        );
    }

    #[test]
    fn zero_max_in_flight_means_unlimited() {
        let mut entry = base_entry("a");
        entry.in_flight = 99;
        let mut pool = pool_of(vec![entry]);
        let mut seq = 0;
        let mut relaxed = policy();
        relaxed.max_in_flight = 0;
        assert!(pick(&mut pool, &mut seq, &relaxed, NOW, None, "", &HashSet::new(), 1).is_some());
    }

    #[test]
    fn realm_filter_isolates_cn_and_global() {
        let mut cn = base_entry("cn-1");
        cn.realm = Some(RealmTag::Cn);
        let mut global = base_entry("global-1");
        global.realm = Some(RealmTag::Global);
        let mut pool = pool_of(vec![cn, global]);
        let mut seq = 0;

        assert_eq!(
            pick(&mut pool, &mut seq, &policy(), NOW, Some(RealmTag::Global), "", &HashSet::new(), 1),
            Some("global-1".to_string())
        );
        assert_eq!(
            pick(&mut pool, &mut seq, &policy(), NOW, Some(RealmTag::Cn), "", &HashSet::new(), 2),
            Some("cn-1".to_string())
        );
    }

    #[test]
    fn tried_accounts_are_excluded_across_retries() {
        let mut pool = pool_of(vec![base_entry("a"), base_entry("b")]);
        let mut seq = 0;

        let first = pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 1).unwrap();
        let mut tried = HashSet::new();
        tried.insert(first.clone());
        let second = pick(&mut pool, &mut seq, &policy(), NOW, None, "", &tried, 2).unwrap();
        assert_ne!(first, second, "换号重试必须换到另一个账号");

        tried.insert(second);
        assert_eq!(
            pick(&mut pool, &mut seq, &policy(), NOW, None, "", &tried, 3),
            None,
            "全部试过后不得重复尝试"
        );
    }

    #[test]
    fn anti_thundering_herd_prefers_untouched_account() {
        // 两个账号权重相同；刚被选中的那个在 100ms 内不应再被选中。
        let mut pool = pool_of(vec![base_entry("a"), base_entry("b")]);
        let mut seq = 0;

        for _ in 0..30 {
            let chosen = pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 7)
                .expect("有候选");
            let next = pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 7)
                .expect("有候选");
            assert_ne!(chosen, next, "100ms 内不得重复选中同一账号");
        }
    }

    #[test]
    fn lru_fallback_kicks_in_when_shortlist_all_recently_used() {
        // 单账号池：它刚被选中 → 防惊群会剔除它 → 走 LRU 兜底仍返回它
        let mut pool = pool_of(vec![base_entry("only")]);
        let mut seq = 0;
        let first = pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 1);
        assert_eq!(first, Some("only".to_string()));
        let second = pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 2);
        assert_eq!(second, Some("only".to_string()), "LRU 兜底必须保证不空手而回");
    }

    #[test]
    fn cost_tier_hard_filters_paid_when_free_available() {
        let mut free = base_entry("free");
        free.record_cost(NOW, "m", 0.0, 1000);
        let mut paid = base_entry("paid");
        paid.record_cost(NOW, "m", 9.0, 1000);
        // 让收费号积分极高——若不做分层过滤，它必然胜出
        paid.credits = 1_000_000;
        free.credits = 0;

        let mut pool = pool_of(vec![free, paid]);
        let mut seq = 0;
        for seed in 0..20 {
            let chosen = pick(&mut pool, &mut seq, &policy(), NOW, None, "m", &HashSet::new(), seed)
                .expect("有候选");
            assert_eq!(chosen, "free", "实测免费号必须硬胜过积分高的收费号");
        }
    }

    #[test]
    fn unknown_tier_keeps_learning_opportunity() {
        // 一个实测收费、一个无观测 → 分层为 Paid(2) 与 Unknown(1) → 无观测者优先
        let mut paid = base_entry("paid");
        paid.record_cost(NOW, "m", 9.0, 1000);
        paid.credits = 1_000_000;
        let unknown = base_entry("unknown");

        let mut pool = pool_of(vec![paid, unknown]);
        let mut seq = 0;
        let chosen = pick(&mut pool, &mut seq, &policy(), NOW, None, "m", &HashSet::new(), 3)
            .expect("有候选");
        assert_eq!(chosen, "unknown", "无观测号应保留被观测的机会");
    }

    #[test]
    fn expired_cost_observation_falls_back_to_unknown() {
        let mut entry = base_entry("a");
        entry.record_cost(0, "m", 9.0, 1000); // 很久以前
        assert_eq!(
            entry.cost_tier(NOW, "m", policy().model_cost_ttl_ms),
            CostTier::Unknown,
            "超 TTL 的观测视为无观测"
        );
    }

    #[test]
    fn pick_marks_last_used_and_advances_sequence() {
        let mut pool = pool_of(vec![base_entry("a")]);
        let mut seq = 0;
        pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 1);
        let entry = pool.get("a").expect("存在");
        assert_eq!(entry.last_used_ms, NOW);
        assert_eq!(entry.used_seq, 1);
        assert_eq!(seq, 1);
    }

    #[test]
    fn pick_is_deterministic_for_same_seed() {
        let build = || pool_of(vec![base_entry("a"), base_entry("b"), base_entry("c")]);
        let mut pool_a = build();
        let mut pool_b = build();
        let mut seq_a = 0;
        let mut seq_b = 0;
        for _ in 0..10 {
            let left = pick(&mut pool_a, &mut seq_a, &policy(), NOW, None, "", &HashSet::new(), 42);
            let right = pick(&mut pool_b, &mut seq_b, &policy(), NOW, None, "", &HashSet::new(), 42);
            assert_eq!(left, right, "同种子必须选出同一账号");
        }
    }

    #[test]
    fn empty_pool_returns_none() {
        let mut pool: BTreeMap<String, PoolEntry> = BTreeMap::new();
        let mut seq = 0;
        assert_eq!(pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 1), None);
    }

    #[test]
    fn all_cooling_falls_back_to_earliest_expiry() {
        let mut soon = base_entry("soon");
        soon.cooldown_fixed(NOW, 1000, "404");
        let mut late = base_entry("late");
        late.cooldown_fixed(NOW, 60_000, "429");
        let mut pool = pool_of(vec![soon, late]);
        let mut seq = 0;
        assert_eq!(
            pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 1),
            Some("soon".to_string()),
            "全冷却时选最早解冻者"
        );
    }

    #[test]
    fn hard_cooled_accounts_are_excluded_from_expiry_fallback() {
        let mut hard = base_entry("hard");
        hard.cooldown_until_tomorrow_4am(NOW, "余额不足");
        let mut pool = pool_of(vec![hard]);
        let mut seq = 0;
        assert_eq!(
            pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 1),
            None,
            "硬冷却（余额不足）不得被兜底强试"
        );
    }

    #[test]
    fn disabled_accounts_never_selected() {
        let mut disabled = base_entry("disabled");
        disabled.disable("12153 session dead");
        let mut pool = pool_of(vec![disabled]);
        let mut seq = 0;
        assert_eq!(pick(&mut pool, &mut seq, &policy(), NOW, None, "", &HashSet::new(), 1), None);
    }

    #[test]
    fn model_cooldown_excludes_only_that_model() {
        let mut entry = base_entry("a");
        entry.cooldown_model(NOW, "glm-5.2", 0, 60_000, "6004");
        let mut pool = pool_of(vec![entry]);
        let mut seq = 0;
        assert_eq!(
            pick(&mut pool, &mut seq, &policy(), NOW, None, "glm-5.2", &HashSet::new(), 1),
            None,
            "触发限流的模型应被跳过"
        );
        assert_eq!(
            pick(&mut pool, &mut seq, &policy(), NOW, None, "deepseek-v4-flash", &HashSet::new(), 1),
            Some("a".to_string()),
            "其它模型应立即可用"
        );
    }

    #[test]
    fn draw_weighted_respects_weights() {
        let heavy = Candidate {
            uid: "heavy".to_string(),
            weight: 1000.0,
            cost_per_1k: 0.0,
            used_seq: 1,
            last_used_ms: 0,
            tier: 0,
        };
        let light = Candidate {
            uid: "light".to_string(),
            weight: 1.0,
            cost_per_1k: 0.0,
            used_seq: 2,
            last_used_ms: 0,
            tier: 0,
        };
        let items = vec![&heavy, &light];
        let mut rng = Pcg32::new(1, 1);
        let mut heavy_wins = 0;
        for _ in 0..1000 {
            if draw_weighted(&items, &mut rng).map(|item| item.uid.as_str()) == Some("heavy") {
                heavy_wins += 1;
            }
        }
        assert!(heavy_wins > 900, "权重 1000:1 应几乎必胜，实际 {heavy_wins}/1000");
    }

    #[test]
    fn draw_weighted_handles_empty_and_zero_weights() {
        let mut rng = Pcg32::new(1, 1);
        assert!(draw_weighted(&[], &mut rng).is_none());

        let zero = Candidate {
            uid: "z".to_string(),
            weight: 0.0,
            cost_per_1k: 0.0,
            used_seq: 1,
            last_used_ms: 0,
            tier: 0,
        };
        let items = vec![&zero];
        assert_eq!(
            draw_weighted(&items, &mut rng).map(|item| item.uid.as_str()),
            Some("z"),
            "零权重也应能被选中（下界 1），避免死锁"
        );
    }

    /// ★★ 护栏：`is_usable` 与 `pick` 的可用性判断**必须一致**。
    ///
    /// 会话粘性用 `is_usable` 回答「上次那个账号现在还能不能用」。若两者分家、
    /// 且 `is_usable` 更宽松，粘性就会把请求钉到一个 `pick` 认为不可用的账号上 ——
    /// 症状是「明明有别的号可用却一直失败」，而且**不报错**、只表现为偶发失败。
    ///
    /// 两处现在共用 `entry_usable`；本用例是它的**行为级**印证（改坏任一侧都会红）。
    #[test]
    fn is_usable_agrees_with_pick_on_each_state() {
        let policy = policy();

        let healthy = base_entry("healthy");
        let mut saturated = base_entry("saturated");
        saturated.in_flight = 3; // 与 `in_flight_full_blocks_candidate` 同款：占满在途
        let mut cooling = base_entry("cooling");
        cooling.until_ms = NOW + 60_000;

        let map = pool_of(vec![healthy, saturated, cooling]);

        assert!(is_usable(&map, &policy, "healthy", NOW, ""), "健康账号应可用");
        assert!(!is_usable(&map, &policy, "saturated", NOW, ""), "在途占满应不可用");
        assert!(!is_usable(&map, &policy, "cooling", NOW, ""), "冷却中应不可用");
        assert!(!is_usable(&map, &policy, "absent", NOW, ""), "不在池里应不可用");

        // 同一组条目下 `pick` 必须只挑到 `healthy` —— 与上面三条否定断言互为印证。
        let mut for_pick = map.clone();
        let mut seq = 0;
        assert_eq!(
            pick(&mut for_pick, &mut seq, &policy, NOW, None, "", &HashSet::new(), 1).as_deref(),
            Some("healthy"),
            "pick 只能选到 `is_usable` 为 true 的那个"
        );
    }

    /// 模型级冷却也必须被 `is_usable` 看到 —— 粘性传的是**模型名**，
    /// 若这里漏看模型维度，粘性会把请求钉到一个「该模型正被限流」的账号上。
    #[test]
    fn is_usable_honours_model_scoped_cooldown() {
        let policy = policy();
        let mut blocked = base_entry("blocked");
        blocked.model_cooldowns.insert(
            "m1".to_string(),
            ModelCooldown {
                until_ms: NOW + 60_000,
                reset_at_ms: 0,
                reason: String::new(),
                hits: 0,
            },
        );
        let map = pool_of(vec![blocked]);

        assert!(
            !is_usable(&map, &policy, "blocked", NOW, "m1"),
            "该模型正被冷却 ⇒ 粘性不得钉住它"
        );
        assert!(
            is_usable(&map, &policy, "blocked", NOW, "m2"),
            "换模型立即可用（模型级限流只封锁该模型）"
        );
    }

    /// ★ 已**过期**的模型级冷却不得被当成不可用。
    ///
    /// 这一条专治「两套判据分家」的一个隐蔽入口：`pick` 在过滤前会
    /// `prune_model_cooldowns`（把过期条目**删掉**），而 `is_usable` **不 prune**
    /// （它只读、不持有写锁）。若判据本身不是「自带过期判断」的，两者就会分家 ——
    /// 表现为「粘性认为该账号不可用，而 pick 认为可用」，或反之。
    ///
    /// 现在两者共用 `entry_usable` ⇒ `healthy_for_model` 自带 `now_ms >= until_ms`，
    /// prune 只是清理。本用例把这个前提钉住。
    #[test]
    fn is_usable_treats_expired_model_cooldown_as_available() {
        let policy = policy();
        let mut recovered = base_entry("recovered");
        recovered.model_cooldowns.insert(
            "m1".to_string(),
            ModelCooldown {
                until_ms: NOW - 1, // 刚过期
                reset_at_ms: 0,
                reason: String::new(),
                hits: 1,
            },
        );
        let map = pool_of(vec![recovered.clone()]);

        assert!(
            is_usable(&map, &policy, "recovered", NOW, "m1"),
            "已过期的模型级冷却不得判为不可用（is_usable 不 prune，判据必须自带过期判断）"
        );

        // 与 pick 的行为互为印证：只有这一个候选，必须能选到它。
        let mut for_pick = pool_of(vec![recovered]);
        let mut seq = 0;
        assert_eq!(
            pick(&mut for_pick, &mut seq, &policy, NOW, None, "m1", &HashSet::new(), 1).as_deref(),
            Some("recovered")
        );
    }
}
