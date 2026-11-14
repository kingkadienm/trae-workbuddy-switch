//! 客户端 ↔ 网关**对账**：把「同一个请求被两处各记一次」的那些找出来。
//!
//! 设计依据：`docs/specs/USAGE-GATEWAY-UI-REDESIGN.md` §4。
//!
//! ## 为什么需要它（issue #63）
//!
//! 客户端走本机网关时，一个请求会在两处各留一条记录（客户端 jsonl + 网关日志）。
//! 于是「合计 = 客户端 + 网关」会**双计**。而聚合值里推不出重叠量：
//! `C = 2.9M`、`G = 18.5M` 既可能是「完全重叠」（真值 18.5M），也可能是
//! 「完全独立」（真值 21.4M）—— 任何单一数字都是猜。
//!
//! 所以本模块做两件事：
//!
//! 1. **给出可证明的区间**：`max(C, G) ≤ 真值 ≤ C + G`（因为 `0 ≤ overlap ≤ min(C,G)`）；
//! 2. **逐条匹配**，把 `overlap` 尽量算准，再用匹配率判定拓扑（独立 / 子集 / 混合）。
//!
//! ## 匹配键（四条同时满足才算同一条请求）
//!
//! | 条件 | 值 | 为什么 |
//! |---|---|---|
//! | 模型 | 完全相同 | 不同模型不可能是同一请求 |
//! | 时间差 | ≤ [`MAX_GAP_MS`] | 两端都记「流结束」时刻，天然可比 |
//! | token | 相差 ≤ [`TOKEN_TOLERANCE`] | 容得下计法差异，容不下误配 |
//! | 一对一 | 一条网关记录最多抵一条客户端记录 | 否则一条会被扣多次 |
//!
//! ## ★ 账号**不参与**匹配
//!
//! 网关按请求选号（P12 无缝切号）：同一条请求在客户端账本里记的是**登录账号**、
//! 在网关账本里记的是**被选中的账号** —— 两端天然不同。拿账号当匹配键会把该对上的
//! 全漏掉，从而把「客户端走网关」误判成「两本账独立」，然后给出一个双计 100% 的合计。
//!
//! ## 宁可少扣，不多扣
//!
//! 少扣的后果是合计偏大，而「未匹配 N 条」会明写出来；多扣的后果是凭空少算，
//! 用户永远发现不了。故所有判定都取**保守侧**：容差取小、时间窗取窄、一对一。
//!
//! ## 边界（能力边界必须写在脸上）
//!
//! 网关日志只留最近 `log_keep` 条（缺省 200）⇒ **只有今天能逐条对账**，
//! 更早的只能给区间。调用方（`token_daily::reconcile_today`）只扫今日，
//! 并在返回值里带上「覆盖了多少条」。

use serde_json::{json, Value};

/// 两端时间戳的最大容差（毫秒）。
///
/// 120 秒：流式请求的 `totalMs` 实测中位 4.3s、最大 14.5s（issue #63 样本），
/// 而两端都记「流结束」时刻 ⇒ 正常应在秒级。放到 120s 是为了容忍
/// 「客户端写完 jsonl 落后于网关写日志」这类延迟，同时又不至于把两条无关请求配到一起。
pub const MAX_GAP_MS: i64 = 120_000;

/// token 相对容差（1%）。
///
/// 两端对**缓存 token 的归属**可能有细微差别（`cache_read` 算不算 input），
/// 所以不能要求精确相等；但也不能放到 10% —— 那会把相邻两次同类请求配错。
pub const TOKEN_TOLERANCE: f64 = 0.01;

/// 一条可对账的记录（客户端 / 网关两侧同形）。
#[derive(Clone, Debug, PartialEq)]
pub struct ReconRow {
    pub ts: i64,
    pub model: String,
    /// 该请求的 token 总量（input + output）。
    pub tokens: u64,
    /// 该请求的积分（`usage.credit`）；0.0 = 不知道。
    pub credit: f64,
}

/// 拓扑判定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 两本账基本独立（客户端直连上游）⇒ 合计 = 相加 − 重叠。
    Independent,
    /// 客户端走的正是网关（客户端 ⊆ 网关）⇒ 合计 ≈ 网关 + 直连客户端。
    Subset,
    /// 部分重叠（中途开网关 / 只有部分客户端走网关）⇒ 合计给区间。
    Mixed,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Independent => "independent",
            Verdict::Subset => "subset",
            Verdict::Mixed => "mixed",
        }
    }
}

/// 一次对账的结果。
#[derive(Clone, Debug)]
pub struct Report {
    pub client_rows: usize,
    pub gateway_rows: usize,
    pub matched: usize,
    pub matched_tokens: u64,
    pub matched_credit: f64,
    /// 客户端侧的 token / 积分合计（区间与估计都基于它）。
    pub client_tokens: u64,
    pub client_credit: f64,
    pub gateway_tokens: u64,
    pub gateway_credit: f64,
    pub verdict: Verdict,
    /// `max(C, G)` —— 可证明的下界（最坏情况：全部重叠）。
    pub lower: u64,
    /// `C + G` —— 可证明的上界（最好情况：完全无重叠）。
    pub upper: u64,
    /// `C + G − matched`：取「未匹配的客户端请求都是直连」这一侧的估计值。
    pub estimate: u64,
    /// 未匹配的客户端样本（最多 [`SAMPLE_LIMIT`] 条，供 UI 说明「为什么没对上」）。
    pub unmatched: Vec<ReconRow>,
}

/// 未匹配样本上限：UI 只展示前几条，多了没人看。
pub const SAMPLE_LIMIT: usize = 3;

/// token 是否在容差内（相对差 ≤ 1%，且对极小的值给 1 的绝对兜底）。
fn tokens_close(a: u64, b: u64) -> bool {
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    let diff = (hi - lo) as f64;
    diff <= (hi as f64 * TOKEN_TOLERANCE).max(1.0)
}

/// 逐条匹配 + 拓扑判定（**纯函数**，无 I/O，可单测）。
///
/// `proxy_enabled`：客户端是否配置为走本机网关。
/// `None` = **未知**（当前没有可靠的配置读取点）⇒ 高匹配率仍判 `Subset`，
/// 因为「逐条对上」本身就是比配置更强的证据；`Some(false)` 时高匹配率只判 `Mixed`
/// （那更可能是**别的客户端**在跑同样模型，见设计文档 §4.4.1）。
pub fn reconcile(client: &[ReconRow], gateway: &[ReconRow], proxy_enabled: Option<bool>) -> Report {
    let client_tokens = client.iter().map(|row| row.tokens).sum();
    let client_credit: f64 = client.iter().map(|row| row.credit).sum();
    let gateway_tokens = gateway.iter().map(|row| row.tokens).sum();
    let gateway_credit: f64 = gateway.iter().map(|row| row.credit).sum();

    let mut used = vec![false; gateway.len()];
    let mut matched = 0usize;
    let mut matched_tokens = 0u64;
    let mut matched_credit = 0.0f64;
    let mut unmatched: Vec<ReconRow> = Vec::new();

    for row in client {
        // 没有模型名 / 没有 token 的记录**不参与匹配**（不知道模型就没法断言是同一条请求）。
        if row.model.is_empty() || row.tokens == 0 {
            if unmatched.len() < SAMPLE_LIMIT {
                unmatched.push(row.clone());
            }
            continue;
        }
        let mut best: Option<(usize, i64)> = None;
        for (index, candidate) in gateway.iter().enumerate() {
            if used[index] || candidate.model != row.model {
                continue;
            }
            let gap = (candidate.ts - row.ts).abs();
            if gap > MAX_GAP_MS || !tokens_close(candidate.tokens, row.tokens) {
                continue;
            }
            // 取时间差最小的那条（同一模型下可能有并发请求，就近配对最可靠）。
            if best.is_none_or(|(_, best_gap)| gap < best_gap) {
                best = Some((index, gap));
            }
        }
        match best {
            Some((index, _)) => {
                used[index] = true;
                matched += 1;
                matched_tokens = matched_tokens.saturating_add(row.tokens);
                matched_credit += row.credit;
            }
            None => {
                if unmatched.len() < SAMPLE_LIMIT {
                    unmatched.push(row.clone());
                }
            }
        }
    }

    let ratio = if client.is_empty() {
        0.0
    } else {
        matched as f64 / client.len() as f64
    };
    let verdict = if ratio < 0.1 {
        Verdict::Independent
    } else if ratio > 0.9 && proxy_enabled != Some(false) {
        Verdict::Subset
    } else {
        Verdict::Mixed
    };

    Report {
        client_rows: client.len(),
        gateway_rows: gateway.len(),
        matched,
        matched_tokens,
        matched_credit,
        client_tokens,
        client_credit,
        gateway_tokens,
        gateway_credit,
        verdict,
        lower: client_tokens.max(gateway_tokens),
        upper: client_tokens.saturating_add(gateway_tokens),
        estimate: client_tokens
            .saturating_add(gateway_tokens)
            .saturating_sub(matched_tokens),
        unmatched,
    }
}

impl Report {
    /// 序列化为命令返回（字段名与前端 `GatewayUsageRecon` 一一对应）。
    pub fn to_value(&self, proxy_signal: Option<bool>) -> Value {
        json!({
            "clientRows": self.client_rows,
            "gatewayRows": self.gateway_rows,
            "matched": self.matched,
            "matchedTokens": self.matched_tokens,
            "matchedCredit": round3(self.matched_credit),
            "clientTokens": self.client_tokens,
            "clientCredit": round3(self.client_credit),
            "gatewayTokens": self.gateway_tokens,
            "gatewayCredit": round3(self.gateway_credit),
            "verdict": self.verdict.as_str(),
            /* 配置信号：`true` / `false` / `null`（未知）。UI 据此决定要不要说
               「配置显示已开启代理」—— 未知时只陈述匹配率，不替用户下结论。 */
            "proxySignal": proxy_signal,
            "bounds": { "lower": self.lower, "upper": self.upper },
            "estimate": self.estimate,
            "unmatchedSamples": self.unmatched.iter().map(|row| json!({
                "ts": row.ts,
                "model": row.model,
                "tokens": row.tokens,
                "credit": round3(row.credit),
            })).collect::<Vec<_>>(),
        })
    }
}

/// 保留 3 位小数（积分是小数；`0.1+0.2` 那类 f64 噪声不该进 UI）。
fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ts: i64, model: &str, tokens: u64, credit: f64) -> ReconRow {
        ReconRow {
            ts,
            model: model.to_string(),
            tokens,
            credit,
        }
    }

    /// ★ 一对一：一条网关记录**最多**抵一条客户端记录。
    ///
    /// 缺陷现场：若允许一条网关记录被多条客户端记录重复抵扣，重叠量会被放大到
    /// 超过 `min(C, G)`，合计算出比下界还小的数 —— 那是**不可能**的。
    #[test]
    fn reconcile_matches_one_to_one() {
        let client = vec![row(1_000, "m", 1000, 0.1), row(1_100, "m", 1000, 0.1)];
        let gateway = vec![row(1_000, "m", 1000, 0.1)];
        let report = reconcile(&client, &gateway, None);
        assert_eq!(report.matched, 1, "一条网关记录只能抵一条客户端记录");
        assert_eq!(report.matched_tokens, 1000);
    }

    /// ★ 账号不参与匹配：两端账号不同（网关切号）仍必须对上。
    ///
    /// `ReconRow` 里根本没有账号字段 —— 这条用例守的是「以后别有人把它加回来」。
    #[test]
    fn reconcile_ignores_account_identity() {
        let client = vec![row(1_000, "m", 1000, 0.1)];
        let gateway = vec![row(1_050, "m", 1000, 0.1)];
        assert_eq!(reconcile(&client, &gateway, None).matched, 1);
    }

    #[test]
    fn reconcile_rejects_time_gap_over_the_limit() {
        let client = vec![row(0, "m", 1000, 0.1)];
        let gateway = vec![row(MAX_GAP_MS + 1, "m", 1000, 0.1)];
        assert_eq!(
            reconcile(&client, &gateway, None).matched,
            0,
            "超时窗不得配对"
        );
        let close = vec![row(MAX_GAP_MS, "m", 1000, 0.1)];
        assert_eq!(
            reconcile(&client, &close, None).matched,
            1,
            "恰好等于上限应配对"
        );
    }

    #[test]
    fn reconcile_rejects_token_drift_over_the_tolerance() {
        let client = vec![row(0, "m", 1000, 0.1)];
        let ok = vec![row(0, "m", 1005, 0.1)];
        assert_eq!(reconcile(&client, &ok, None).matched, 1, "0.5% 偏差应容下");
        let bad = vec![row(0, "m", 1030, 0.1)];
        assert_eq!(reconcile(&client, &bad, None).matched, 0, "3% 偏差必须拒绝");
    }

    #[test]
    fn reconcile_never_matches_across_models() {
        let client = vec![row(0, "m-1", 1000, 0.1)];
        let gateway = vec![row(0, "m-2", 1000, 0.1)];
        assert_eq!(reconcile(&client, &gateway, None).matched, 0);
    }

    /// 区间恒成立：`max(C,G) ≤ estimate ≤ C+G`。
    ///
    /// 这条是给 UI 的承诺 —— 页面上那个 `≈` 数字**永远**落在区间里。
    #[test]
    fn bounds_hold_for_every_overlap_shape() {
        // 完全独立：一条都没对上
        let independent = reconcile(
            &[row(0, "m", 100, 1.0)],
            &[row(999_999, "m", 400, 4.0)],
            None,
        );
        assert_eq!(independent.estimate, 500);
        assert_eq!(independent.lower, 400);
        assert_eq!(independent.upper, 500);

        // 完全重叠：客户端就是网关的一部分
        let subset = reconcile(&[row(0, "m", 400, 4.0)], &[row(0, "m", 400, 4.0)], None);
        assert_eq!(subset.estimate, 400);
        assert_eq!(subset.lower, 400, "全重叠时 estimate 必须等于下界");
        assert_eq!(subset.verdict, Verdict::Subset);
    }

    #[test]
    fn verdict_thresholds_follow_the_match_ratio() {
        let client: Vec<ReconRow> = (0..100).map(|i| row(i * 1000, "m", 1000, 0.1)).collect();
        let gateway_all: Vec<ReconRow> = (0..100).map(|i| row(i * 1000, "m", 1000, 0.1)).collect();
        assert_eq!(
            reconcile(&client, &gateway_all, Some(true)).verdict,
            Verdict::Subset
        );
        // 配置明确说「没开代理」⇒ 高匹配率只判 mixed（那更可能是别的客户端在跑同样模型）
        assert_eq!(
            reconcile(&client, &gateway_all, Some(false)).verdict,
            Verdict::Mixed
        );

        let gateway_none: Vec<ReconRow> = (0..100)
            .map(|i| row(9_999_999 + i, "m", 1000, 0.1))
            .collect();
        assert_eq!(
            reconcile(&client, &gateway_none, Some(true)).verdict,
            Verdict::Independent
        );
    }

    /// 空输入不能 panic，也不能给出 `estimate < 0`。
    #[test]
    fn reconcile_handles_empty_sides() {
        let empty: Vec<ReconRow> = Vec::new();
        let only_client = reconcile(&[row(0, "m", 10, 0.1)], &empty, None);
        assert_eq!(
            (only_client.lower, only_client.upper, only_client.estimate),
            (10, 10, 10)
        );
        assert_eq!(only_client.verdict, Verdict::Independent);
        let none = reconcile(&empty, &empty, None);
        assert_eq!((none.estimate, none.matched), (0, 0));
    }
}
