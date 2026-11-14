//! 结构化工具历史的**运行时降级记忆**（issue #72）。
//!
//! ## 为什么需要它
//!
//! 「上游是否接受结构化工具历史」是**运行时事实**，不是编译期常量：模型清单会变、
//! 上游契约会变，而本仓没有 Qoder 真机账号 ⇒ 任何静态白名单都只是**猜**。
//! 猜错的代价原本是「用户整轮 400 报障」，现在变成「一次内部重试 + 一条记忆」：
//!
//! 1. 结构化请求被上游按**参数错**拒（400 / 422）；
//! 2. 网关用**同一账号**、文本模式重发一次（参数错跟账号无关，换号只是浪费）；
//! 3. 成功 ⇒ 在 [`note_rejected`] 里记下 `(区域, 模型)`，[`is_unsupported`] 为真的
//!    模型在 TTL 内**直接走文本模式**，不再每次先撞一次 400。
//!
//! ## 为什么放进程内、不落盘
//!
//! 落盘要处理并发写 / 版本 / 清理 / 迁移，收益只是「重启后少撞一次 400」。
//! 重启重学的成本是每条请求多一次 400 + 一次重发 —— 不值得为它写一个文件。
//! TTL 到期也会重学（上游可能哪天就支持了）。
//!
//! ## 与 `QD_STRUCTURED_TOOL_HISTORY` 的分工
//!
//! 环境变量是**用户的手动开关**（全局），本模块是**自动学到的结论**（按区域 + 模型）。
//! 两者都只能把请求往「文本模式」推，不会把请求推向结构化 —— 猜错的方向永远是安全的那侧。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use buddy_switch_core::modules::qoder::region::QoderRegion;

/// 记忆存活时间：到期后重新试一次结构化（上游可能已修）。
const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// 键 = `区域|模型 key`（模型 key 是**判据本身**：同一区域里不同模型的上游通道不同）。
fn storage() -> &'static Mutex<HashMap<String, Instant>> {
    static STORAGE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    STORAGE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn entry_key(region: QoderRegion, model: &str) -> String {
    format!("{}|{}", region.as_str(), model.trim().to_ascii_lowercase())
}

/// 这条记忆是否仍然有效（纯函数，便于测 TTL 而不必真等 6 小时）。
fn is_fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) < TTL
}

/// 记下「该 (区域, 模型) 的结构化历史被上游拒了」。
pub fn note_rejected(region: QoderRegion, model: &str) {
    let key = entry_key(region, model);
    if key.ends_with('|') {
        // 模型 key 为空 ⇒ 不记（空 key 会命中一大片无关模型）。
        return;
    }
    storage()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key, Instant::now());
}

/// 该 (区域, 模型) 是否在 TTL 内已知「结构化不被接受」。
pub fn is_unsupported(region: QoderRegion, model: &str) -> bool {
    let key = entry_key(region, model);
    let mut map = storage()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    match map.get(&key) {
        Some(at) if is_fresh(*at, now) => true,
        Some(_) => {
            // 过期即忘：下一次请求重新试结构化，失败再记。
            map.remove(&key);
            false
        }
        None => false,
    }
}

/// 当前记忆条数（管理面 / 诊断用；不触发任何上游往返）。
pub fn tracked_models() -> usize {
    storage()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len()
}

/// 清空（测试用：进程内是全局状态，用例之间必须隔离）。
#[cfg(test)]
pub fn reset() {
    storage()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

/// 这次失败该不该降级重试（**纯函数**，网关热路径与测试共用同一个判据）。
///
/// 只认**参数类**拒绝：401/403 是凭据、429 是限流、5xx 是上游故障 —— 那些跟
/// 「请求体形状」无关，换文本模式重发只会白烧一次额度。
///
/// `attempt` 从 1 开始：只降级一次，绝不循环（否则上游持续 400 时会把一条请求
/// 放大成两条、三条）。
pub fn should_degrade(status: u16, structured_used: bool, attempt: u32) -> bool {
    structured_used && attempt == 1 && matches!(status, 400 | 422)
}

/// 降级重试时给用户看的说明（进 `failures`，出问题能查到「为什么多发了一次」）。
pub const DEGRADED_NOTE: &str = "结构化工具历史被拒，已用文本模式重试";

#[cfg(test)]
mod tests {
    use super::*;

    /// 记忆的写入 / 命中 / 区域隔离 / 空 key 不记。
    ///
    /// 变异：把 `entry_key` 里的 `region.as_str()` 去掉 ⇒ 区域隔离那条红。
    #[test]
    fn rejected_models_are_remembered_per_region() {
        reset();
        assert!(!is_unsupported(QoderRegion::Cn, "dfmodel"));
        note_rejected(QoderRegion::Cn, "dfmodel");
        assert!(is_unsupported(QoderRegion::Cn, "dfmodel"));
        // 大小写 / 空白归一后再记、再查。
        assert!(is_unsupported(QoderRegion::Cn, "  DFModel "));
        // 区域隔离：同一个模型 key 在国际版仍是「未知」，不该被 CN 的结论连累。
        assert!(!is_unsupported(QoderRegion::Global, "dfmodel"));
        // 空模型 key 不记（否则会命中一大片无关模型）。
        note_rejected(QoderRegion::Cn, "   ");
        assert_eq!(tracked_models(), 1);
        reset();
    }

    /// TTL：到期即忘（用时间戳直测，不真等 6 小时）。
    #[test]
    fn memory_expires_after_ttl() {
        let now = Instant::now();
        assert!(is_fresh(now, now));
        let stale = now
            .checked_sub(TTL + Duration::from_secs(1))
            .expect("Instant 可回退");
        assert!(!is_fresh(stale, now));
    }

    /// 降级判据：只对参数类拒绝、只对「本次真的用了结构化」、只重试一次。
    ///
    /// 变异：把 `matches!(status, 400 | 422)` 放宽成 `true` ⇒ 401/429/500 三条红。
    #[test]
    fn degrade_only_on_parameter_rejections_and_only_once() {
        assert!(should_degrade(400, true, 1));
        assert!(should_degrade(422, true, 1));
        // 没用结构化 ⇒ 降级无意义（已经是文本模式了）。
        assert!(!should_degrade(400, false, 1));
        // 只重试一次。
        assert!(!should_degrade(400, true, 2));
        // 凭据 / 限流 / 上游故障与请求体形状无关。
        for status in [401, 403, 404, 429, 500, 502, 503] {
            assert!(!should_degrade(status, true, 1), "{status} 不该降级");
        }
    }
}
