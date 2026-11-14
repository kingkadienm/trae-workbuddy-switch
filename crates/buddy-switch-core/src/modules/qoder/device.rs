//! 设备身份的**稳定派生**。
//!
//! ## 为什么必须「按 uid 稳定派生」而不是随机生成
//!
//! 上游按**机器身份**做风控与设备定向。若每次运行随机生成机器码：
//!
//! - 同一账号会在上游看来不断换设备 ⇒ 触发风控；
//! - 同机的多个账号会**共享**一个随机码 ⇒ 多号被关联。
//!
//! 按 uid 稳定派生同时解决这两件事：**同一账号长期固定在同一台「虚拟设备」上**，
//! 而不同账号天然落在不同的机器码上。
//!
//! ## ⚠️ 派生值是**兜底**，不是首选
//!
//! 官方客户端会从自带的风控桥取**真机器身份**；用派生假值时，上游的
//! **设备定向活动列表会静默少条目**（不是报错，是「看起来本来就没有」）。
//! ⇒ 实现上游调用时应当「真值优先、派生兜底」，并**在状态里标注用了哪种**
//! （`identity = native | derived`），**不要静默降级**。
//!
//! ## 算法与官方客户端**逐字节对齐**（2026-10-15 修订）
//!
//! `derive_id` 用 **MD5（加盐前缀 `salt:uid`）取 32 位十六进制** —— 与官方客户端的
//! 派生行为一致（`cosy.rs` 的交叉向量测试钉死了三个值）。
//!
//! ⚠️ 此前本文件用 **SHA-256 截断**并注明「不追求与外部实现逐字节相同」，理由是
//! 「MD5 会引入新依赖」。**该理由已失效**：网关（`cosy` 签名）会把这些值作为
//! `cosy-machineid / machinetoken / machinetype` 头**发给上游**，而上游只看过
//! 官方派生形态的值 —— 未观测过的形态无法假设被接受（本机无真机账号，改错无法发现）。
//! `md-5` 依赖随 `cosy.rs` 引入后，对齐的代价为零。
//! ⇒ **不要**把这里改回自创摘要；要换算法，先拿到上游接受新形态的一手证据。

use base64::Engine;
use md5::{Digest, Md5};
use sha2::Sha512;

/// 以 `uid` 与 `salt` **稳定派生**一个 32 位十六进制标识（MD5，与官方客户端同构）。
///
/// 同一个 `(uid, salt)` 恒得同一结果；不同 `salt` 得到互不相关的值
/// （这样同一个 uid 的机器码 / 会话码不会互相暴露关系）。
///
/// ⚠️ 加盐格式是 `"{salt}:{uid}"`（盐在前）——官方客户端的 `aid`/`uid` 空值会落成
/// 字面量 `anonymous`；本函数保持调用方传入原样（调用方保证 uid 非空）。
pub fn derive_id(uid: &str, salt: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(format!("{salt}:{uid}").as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 一个账号的**全部派生身份**。
///
/// ⚠️ 四个值刻意用**不同的 salt**：它们会出现在不同的请求头里，
/// 用同一个值会让上游能把「机器码」与「会话码」直接关联起来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineIdentity {
    /// 机器标识（32 位十六进制）。
    pub machine_id: String,
    /// 机器令牌（base64url 无填充，43 字符 —— 32 字节的编码长度）。
    pub machine_token: String,
    /// 机器类型（18 字符）。
    pub machine_type: String,
    /// 会话标识（32 位十六进制）。
    pub session_id: String,
}

impl MachineIdentity {
    /// 这份身份是不是**派生**出来的（而非取自官方客户端）。
    ///
    /// 调用方用它决定要不要在状态里标注 `identity = derived`
    /// —— 见模块头「静默降级」那条。
    pub fn is_derived(&self) -> bool {
        true
    }

    /// 稳定标识字符串（日志 / 诊断）。
    pub fn describe(&self) -> String {
        format!(
            "machine_id={} · machine_type={} · identity=derived",
            mask(&self.machine_id),
            self.machine_type
        )
    }
}

/// 派生一个账号的全部身份。
pub fn machine_identity(uid: &str) -> MachineIdentity {
    let mut hasher = Sha512::new();
    hasher.update(format!("machinetoken:{uid}").as_bytes());
    let digest = hasher.finalize();
    let machine_token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    let machine_token = machine_token.chars().take(43).collect::<String>();

    MachineIdentity {
        machine_id: derive_id(uid, "machine"),
        machine_token,
        machine_type: derive_id(uid, "machinetype").chars().take(18).collect(),
        session_id: derive_id(uid, "session"),
    }
}

/// 脱敏：只留前 6 位（日志里不该出现完整机器码）。
fn mask(value: &str) -> String {
    value.chars().take(6).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ 稳定性：同一 uid 恒得同一结果（这是「同一账号固定同一台虚拟设备」的全部依据）。
    ///
    /// 可证伪：在派生里掺入 `uuid::Uuid::new_v4()`（随机），本用例立即变红。
    #[test]
    fn derivation_is_stable_for_the_same_uid() {
        for uid in ["uid-1", "另一个账号", "", "🦀"] {
            assert_eq!(derive_id(uid, "machine"), derive_id(uid, "machine"));
            assert_eq!(machine_identity(uid), machine_identity(uid));
        }
    }

    /// ★ 隔离性：不同 uid / 不同 salt 必须得到不同值（否则多号会被关联）。
    #[test]
    fn derivation_separates_accounts_and_salts() {
        assert_ne!(derive_id("uid-1", "machine"), derive_id("uid-2", "machine"));
        assert_ne!(derive_id("uid-1", "machine"), derive_id("uid-1", "session"));

        let a = machine_identity("uid-1");
        let b = machine_identity("uid-2");
        assert_ne!(a.machine_id, b.machine_id);
        assert_ne!(a.machine_token, b.machine_token);
        assert_ne!(a.machine_type, b.machine_type);
        assert_ne!(a.session_id, b.session_id);
        // 同一个账号的四个值也两两不同（salt 分家）。
        let values = [
            &a.machine_id,
            &a.machine_token,
            &a.machine_type,
            &a.session_id,
        ];
        for i in 0..values.len() {
            for j in (i + 1)..values.len() {
                assert_ne!(values[i], values[j], "第 {i} 与第 {j} 个派生值相同");
            }
        }
    }

    /// 形态契约：机器码 32 位十六进制、令牌 43 字符 base64url、类型 18 字符。
    #[test]
    fn shapes_match_the_expected_wire_format() {
        let id = machine_identity("uid-abc");
        assert_eq!(id.machine_id.len(), 32);
        assert!(id.machine_id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id.session_id.len(), 32);
        assert_eq!(id.machine_token.len(), 43);
        assert!(
            id.machine_token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "机器令牌必须是 base64url 无填充字符集：{}",
            id.machine_token
        );
        assert_eq!(id.machine_type.len(), 18);
        assert!(id.is_derived());
    }

    /// 诊断文案**不得**输出完整机器码（日志与截图会外泄）。
    #[test]
    fn describe_masks_the_machine_id() {
        let id = machine_identity("uid-abc");
        let text = id.describe();
        assert!(
            !text.contains(&id.machine_id),
            "诊断文案泄露了完整机器码：{text}"
        );
        assert!(text.contains("identity=derived"), "{text}");
    }
}
