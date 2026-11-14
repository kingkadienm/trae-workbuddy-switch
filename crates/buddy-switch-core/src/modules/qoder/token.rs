//! Qoder 的**令牌家族**与前缀语义。
//!
//! ## 为什么令牌前缀是一等公民
//!
//! Qoder 的访问令牌与刷新令牌各有两种来源（设备流 / jobToken），PAT 又是第三种。
//! **刷新端点由刷新令牌的前缀决定**，拿错端点会得到 401 而不是「静默失败」；
//! 反过来，把「前缀不认识」当成「没有凭据」会让用户被要求重新登录，而其实凭据好着
//! —— 这正是本仓反复强调的「**信封 ≠ 缺失**」那条纪律在 Qoder 侧的形态。
//!
//! 因此：
//!
//! - 判据一律走本模块（**整段前缀精确匹配**，不 `contains`）；
//! - 认不出前缀 ⇒ `None` ⇒ 调用方给**可读原因**，**不猜端点、不假装没凭据**。

use crate::modules::qoder::{PATH_DEVICE_TOKEN_REFRESH, PATH_JOB_TOKEN_REFRESH};

/// 令牌家族的一个成员。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    /// `dt-`：设备流**访问**令牌（可直接调用上游）。
    DeviceAccess,
    /// `drt-`：设备流**刷新**令牌。
    DeviceRefresh,
    /// `jt-`：jobToken **访问**令牌（由 PAT 交换而来）。
    JobAccess,
    /// `jrt-`：jobToken **刷新**令牌。
    JobRefresh,
    /// `pt-`：PAT（个人访问令牌，长期；用于交换出 jobToken）。
    Personal,
}

impl TokenKind {
    /// 全部取值（顺序即文档顺序，供遍历与诊断）。
    pub const ALL: [TokenKind; 5] = [
        TokenKind::DeviceAccess,
        TokenKind::DeviceRefresh,
        TokenKind::JobAccess,
        TokenKind::JobRefresh,
        TokenKind::Personal,
    ];

    /// 字面前缀（含结尾的 `-`）。
    pub fn prefix(self) -> &'static str {
        match self {
            TokenKind::DeviceAccess => "dt-",
            TokenKind::DeviceRefresh => "drt-",
            TokenKind::JobAccess => "jt-",
            TokenKind::JobRefresh => "jrt-",
            TokenKind::Personal => "pt-",
        }
    }

    /// 稳定标识（日志 / 序列化 / 诊断文案）。
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::DeviceAccess => "device_access",
            TokenKind::DeviceRefresh => "device_refresh",
            TokenKind::JobAccess => "job_access",
            TokenKind::JobRefresh => "job_refresh",
            TokenKind::Personal => "personal",
        }
    }

    /// 是不是**访问**令牌（可直接打上游推理接口）。
    pub fn is_access(self) -> bool {
        matches!(self, TokenKind::DeviceAccess | TokenKind::JobAccess)
    }

    /// 是不是**刷新**令牌（用于换新的访问令牌）。
    pub fn is_refresh(self) -> bool {
        matches!(self, TokenKind::DeviceRefresh | TokenKind::JobRefresh)
    }

    /// 该刷新令牌应该打到哪个端点。
    ///
    /// ★ **穷举、无兜底**：访问令牌与 PAT 返回 `None`（它们不刷新），
    /// 新增令牌种类时编译失败而不是静默打到错误端点。
    pub fn refresh_path(self) -> Option<&'static str> {
        match self {
            TokenKind::DeviceRefresh => Some(PATH_DEVICE_TOKEN_REFRESH),
            TokenKind::JobRefresh => Some(PATH_JOB_TOKEN_REFRESH),
            TokenKind::DeviceAccess | TokenKind::JobAccess | TokenKind::Personal => None,
        }
    }
}

/// 从令牌字面量判类型 —— **整段前缀精确匹配**（不 trim、不折叠大小写、不 `contains`）。
///
/// 认不出 ⇒ `None`。调用方**不得**把 `None` 当成「没有凭据」。
///
/// ⚠️ **含空白字符的一律判为认不出**：令牌里出现空格 / 换行只可能是读取时夹带了
/// 行尾或引号，把它当合法令牌发出去会得到上游一个与「前缀不对」毫不相干的 401，
/// 用户根本联想不到根因。这里提前拒掉，让调用方给「凭据形态异常」的明确文案。
pub fn kind_of(token: &str) -> Option<TokenKind> {
    if token.chars().any(char::is_whitespace) {
        return None;
    }
    // ⚠️ 必须先比 `drt-` / `jrt-` 再比 `dt-` / `jt-`：前者是后者的**超串**
    // （`drt-x` 也以 `dt-` 之外的形状开头，但同族前缀只差一个 `r`），
    // 顺序错了会把 `drt-abc` 判成未知。
    // 这里用「整段前缀 + 逐项精确 starts_with」而不是排序技巧：表短、可读性优先。
    [
        TokenKind::DeviceRefresh,
        TokenKind::JobRefresh,
        TokenKind::DeviceAccess,
        TokenKind::JobAccess,
        TokenKind::Personal,
    ]
    .into_iter()
    .find(|kind| token.starts_with(kind.prefix()))
}

/// 由刷新令牌决定刷新路线（端点）。**fail-closed**：认不出前缀 ⇒ `None`，**不猜**。
pub fn refresh_route(refresh_token: &str) -> Option<&'static str> {
    let kind = kind_of(refresh_token)?;
    kind.refresh_path()
}

/// 一个账号**当前可用凭据**的形态。
///
/// ⚠️ 判据分家（与 WorkBuddy 侧同一条纪律）：
/// - **能不能刷新**看 `refresh`；
/// - **能不能直接调用**看 `access`；
/// - **PAT 是长期兜底**：即便没有 `access`，只要有 `pt-` 就还能交换出 jobToken ⇒
///   不算「缺凭据」。
///
/// ⇒ 三者的组合**不是**简单的「有 / 无」，压成一个布尔值就会给出错误的用户引导。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialShape {
    /// 访问令牌的种类（`None` = 没有 / 认不出）。
    pub access: Option<TokenKind>,
    /// 刷新令牌的种类（`None` = 没有 / 认不出）。
    pub refresh: Option<TokenKind>,
    /// 是否有 PAT。
    pub pat: bool,
    /// 是否有「认不出前缀」的令牌字面量（诊断用；**不等于没有凭据**）。
    pub unrecognized: bool,
}

impl CredentialShape {
    /// 能不能刷新（有刷新令牌且前缀可识别）。
    pub fn can_refresh(&self) -> bool {
        self.refresh.is_some()
    }

    /// 能不能直接打上游推理接口。
    pub fn can_call(&self) -> bool {
        self.access.is_some()
    }

    /// 有没有**任何**可用凭据（访问 / 刷新 / PAT 任一）。
    ///
    /// ★ 用它来判「账号是否真的没有凭据」——**不要**只看 `access_token` 是否为 `None`。
    pub fn has_any_credential(&self) -> bool {
        self.access.is_some() || self.refresh.is_some() || self.pat
    }

    /// 可读形态描述（给诊断与错误文案用）。
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        match self.access {
            Some(kind) => parts.push(format!("访问令牌({})", kind.as_str())),
            None => parts.push("无访问令牌".to_string()),
        }
        match self.refresh {
            Some(kind) => parts.push(format!("刷新令牌({})", kind.as_str())),
            None => parts.push("无刷新令牌".to_string()),
        }
        if self.pat {
            parts.push("PAT".to_string());
        }
        if self.unrecognized {
            parts.push("存在无法识别的令牌前缀".to_string());
        }
        parts.join(" · ")
    }
}

/// 由三个原始字段算出凭据形态。
///
/// ⚠️ 空串按「没有」处理（上游 / 导入文件里确实会写空串）；
/// 但**非空且前缀不认识**的令牌会被标成 [`CredentialShape::unrecognized`]，
/// 而不是被静默丢弃 —— 后者会让用户看到「凭据缺失」而实际只是我们认不出。
pub fn credential_shape(
    access_token: Option<&str>,
    refresh_token: Option<&str>,
    personal_token: Option<&str>,
) -> CredentialShape {
    fn classify(raw: Option<&str>) -> (Option<TokenKind>, bool) {
        match raw {
            None => (None, false),
            Some("") => (None, false),
            Some(value) => match kind_of(value) {
                Some(kind) => (Some(kind), false),
                None => (None, true),
            },
        }
    }
    let (access, access_unknown) = classify(access_token);
    let (refresh, refresh_unknown) = classify(refresh_token);
    let (pat_kind, pat_unknown) = classify(personal_token);
    CredentialShape {
        access,
        refresh,
        pat: pat_kind == Some(TokenKind::Personal),
        unrecognized: access_unknown || refresh_unknown || pat_unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_are_unique_and_round_trip() {
        let mut seen = std::collections::BTreeSet::new();
        for kind in TokenKind::ALL {
            assert!(seen.insert(kind.prefix()), "前缀重复：{}", kind.prefix());
            assert_eq!(kind_of(&format!("{}abc", kind.prefix())), Some(kind));
        }
        assert_eq!(TokenKind::ALL.len(), 5);
    }

    /// ★ `drt-` / `jrt-` 是 `dt-` / `jt-` 的**超串**，判型顺序错了会静默判成未知。
    ///
    /// 可证伪：把 `kind_of` 的遍历顺序改成「先 `dt-` 再 `drt-`」，前两条断言立即变红。
    #[test]
    fn refresh_prefixes_are_not_shadowed_by_access_prefixes() {
        assert_eq!(kind_of("drt-xyz"), Some(TokenKind::DeviceRefresh));
        assert_eq!(kind_of("jrt-xyz"), Some(TokenKind::JobRefresh));
        assert_eq!(kind_of("dt-xyz"), Some(TokenKind::DeviceAccess));
        assert_eq!(kind_of("jt-xyz"), Some(TokenKind::JobAccess));
        assert_eq!(kind_of("pt-xyz"), Some(TokenKind::Personal));
    }

    /// 整段前缀精确匹配：**不 trim、不折叠大小写、不 contains**；含空白一律拒。
    #[test]
    fn kind_of_is_exact_and_fail_closed() {
        for bad in [
            "", "dt", "dt_xyz", "DT-xyz", " dt-xyz", "dt-xyz ", "dt-xy z", "dt-xyz\n", "abc",
            "jwt-xyz",
        ] {
            assert_eq!(kind_of(bad), None, "{bad:?} 不该被判出类型");
        }
    }

    /// ★ 刷新路线**由前缀决定**，认不出就 `None`（不猜端点）。
    #[test]
    fn refresh_route_is_derived_from_prefix_and_fails_closed() {
        assert_eq!(refresh_route("drt-abc"), Some(PATH_DEVICE_TOKEN_REFRESH));
        assert_eq!(refresh_route("jrt-abc"), Some(PATH_JOB_TOKEN_REFRESH));
        // 访问令牌 / PAT 不是刷新令牌 ⇒ 没有刷新路线。
        assert_eq!(refresh_route("dt-abc"), None);
        assert_eq!(refresh_route("pt-abc"), None);
        // 认不出 ⇒ None（**不是**回落到某个默认端点）。
        assert_eq!(refresh_route("mystery-token"), None);
    }

    /// ★★ 判据分家：**只有 PAT 的账号不算「没有凭据」**。
    ///
    /// 可证伪：把 `has_any_credential` 改成 `self.access.is_some()`，
    /// 第二条断言立即变红 —— 那正是「有长期令牌却被要求重新登录」的缺陷形态。
    #[test]
    fn credential_shape_separates_axes_and_pat_is_a_real_credential() {
        let only_pat = credential_shape(None, None, Some("pt-abc"));
        assert!(only_pat.pat);
        assert!(!only_pat.can_call());
        assert!(!only_pat.can_refresh());
        assert!(
            only_pat.has_any_credential(),
            "只有 PAT 也算有凭据（可交换出 jobToken）"
        );

        let only_refresh = credential_shape(None, Some("drt-abc"), None);
        assert!(only_refresh.can_refresh());
        assert!(!only_refresh.can_call());
        assert!(only_refresh.has_any_credential());

        let full = credential_shape(Some("dt-a"), Some("drt-b"), None);
        assert!(full.can_call() && full.can_refresh());

        let empty = credential_shape(None, None, None);
        assert!(!empty.has_any_credential());
        // 空串按「没有」处理。
        assert!(!credential_shape(Some(""), Some(""), Some("")).has_any_credential());
    }

    /// ★ **信封 ≠ 缺失**：非空但前缀不认识的令牌必须被标成 `unrecognized`，
    /// 而不是被当成「没有凭据」。
    #[test]
    fn unrecognized_prefix_is_reported_not_silently_dropped() {
        let shape = credential_shape(Some("brand-new-token-format"), None, None);
        assert!(shape.unrecognized, "无法识别的前缀必须被标出来");
        assert!(!shape.can_call(), "认不出就不能声称可调用");
        assert!(
            shape.describe().contains("无法识别"),
            "诊断文案必须说明是「认不出」而不是「没有」：{}",
            shape.describe()
        );
    }
}
