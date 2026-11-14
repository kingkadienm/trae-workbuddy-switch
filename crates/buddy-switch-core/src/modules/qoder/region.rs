//! Qoder 的两个**区域（realm）**及其全部差异。
//!
//! ## 为什么区域是「一个平台内的两个 realm」，而不是两个平台
//!
//! 国内版与国际版共用**同一套客户端形态**（同一份协议、同一套令牌家族、同一组能力），
//! 差别只在**端点 / 客户端 id / 本地目录 / 账号体系**这几张常量表上。
//! 因此在本仓的 L1/L3 模型里，它们是 [`Product::Qoder`](crate::modules::product::Product::Qoder)
//! 下的两个 [`Scope`](crate::modules::product::Scope)（`qoder-cn` / `qoder-global`），
//! 而不是两条产品线 —— 与 WorkBuddy / Trae 的区域处理**同构**。
//!
//! ## 表驱动
//!
//! 全部差异集中在 [`RealmSpec`] 一张表里（对照 `region::RegionSpec` / `trae::region` 的风格），
//! **调用方一律经 [`realm_spec`] 查表**，不得在别处手写端点或目录名。
//!
//! ⚠️ **路径派生唯一规则**：作用域字符串 → 解析成枚举 → 查表派生路径。
//! 中间任何一步都不许用原始字符串（见 `product.rs` 模块头）。

/// Qoder 的区域（realm）—— **持久化轴**。
///
/// 决定账号库文件后缀、OAuth 端点与客户端 id、本机凭据目录、以及全部联网端点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QoderRegion {
    /// 国内版（`qoder.com.cn` / `gateway.qoder.com.cn`）。
    Cn,
    /// 国际版（`qoder.com` / `api1.qoder.sh`，另有 api2 / api3 故障切换）。
    Global,
}

impl QoderRegion {
    /// 稳定标识（文件名后缀、序列化、前端传参）。
    pub fn as_str(self) -> &'static str {
        match self {
            QoderRegion::Cn => "cn",
            QoderRegion::Global => "global",
        }
    }

    /// 展示名 —— 取**客户端自己的名字**（国内带 ` CN` 后缀）。
    ///
    /// ⚠️ 只用于 Qoder 自己的文案。跨平台的作用域芯片（「平台 + 区域」）用的是
    /// `shared.region.version.*` 那组通用词条 —— 把本值塞进去会得到「Qoder Qoder CN」。
    pub fn display_name(self) -> &'static str {
        match self {
            QoderRegion::Cn => "Qoder CN",
            QoderRegion::Global => "Qoder",
        }
    }

    /// 从字符串解析区域，**宽容**（供诊断与旧值兼容；`scope` 路径请用
    /// [`Scope::parse`](crate::modules::product::Scope::parse)，那条是 fail-closed）。
    pub fn parse(s: &str) -> Option<QoderRegion> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cn" | "china" | "domestic" | "qoder_cn" => Some(QoderRegion::Cn),
            "global" | "intl" | "international" | "qoder" => Some(QoderRegion::Global),
            _ => None,
        }
    }

    /// 全部区域（遍历用，顺序稳定）。
    pub fn all() -> [QoderRegion; 2] {
        [QoderRegion::Cn, QoderRegion::Global]
    }
}

impl Default for QoderRegion {
    /// 默认国内版 —— 与前端 `DEFAULT_SCOPE`（`wb-cn`）之外唯一「冷启动兜底」无关，
    /// 这里只服务**旧 `region` 参数**的兼容语义（缺省即国内版）。
    fn default() -> Self {
        QoderRegion::Cn
    }
}

/// 一个区域的**全部差异**。
///
/// ⚠️ 新增字段时必须同步 [`CN_REALM`] / [`GLOBAL_REALM`] 两处（`const` 结构体字面量
/// 缺字段会编译失败，这正是想要的效果）。
pub struct RealmSpec {
    /// 该表属于哪个区域。
    pub region: QoderRegion,
    /// 官网。
    pub website: &'static str,
    /// 开放接口基址（账号 / 令牌 / 额度）。
    pub openapi_base: &'static str,
    /// 推理网关主基址。
    pub gateway_base: &'static str,
    /// 推理网关备用基址（故障切换，顺序即优先级；国内版为空）。
    pub gateway_fallbacks: &'static [&'static str],
    /// OAuth 客户端 id。
    pub client_id: &'static str,
    /// OAuth 回调地址。
    pub redirect_uri: &'static str,
    /// 账号归属域名（写进账号记录、用于跨区校验）。
    pub domain: &'static str,
    /// 上游 User-Agent。
    pub user_agent: &'static str,
    /// CLI 本地目录名（`~/.qoder-cn` / `~/.qoder`）。
    pub cli_home_dir: &'static str,
    /// 桌面 App 的 `%APPDATA%` 目录名（Electron userData）。
    ///
    /// ⚠️ macOS 上叶子名与 Windows **逐字相同**（2026-09-23 真机取证：
    /// `~/Library/Application Support/com.qodercn.app.stable` 与 `%APPDATA%` 同名），
    /// 所以 `paths::roaming_dir()`（`dirs::config_dir()`）在 macOS 上直接可用，
    /// 不需要按平台分叉。
    pub desktop_app_dir: &'static str,
    /// ★ macOS 登录钥匙串里 Electron `safeStorage` 的**服务名**（按区域不同）。
    ///
    /// macOS 与 Windows 的密钥来源是**两套**：Windows 从 `Local State` 的
    /// `os_crypt.encrypted_key` 经 DPAPI 解出；macOS 的 `Local State` 里**没有**
    /// `os_crypt`（实测只有 `uninstall_metrics`），主密钥口令在**登录钥匙串**，
    /// 服务名 = `<Electron 应用名> Safe Storage`。
    ///
    /// 真机取证（2026-09-23，macOS 15.6.1 / arm64，`security dump-keychain`）：
    /// - 国内版：`svce="Qoder CN App Safe Storage"` / `acct="Qoder CN App Key"`
    /// - 国际版：`svce="Qoder Safe Storage"` / `acct="Qoder Key"`
    ///
    /// ⚠️ 两区**不同名**：国内版 Electron 应用名是 `Qoder CN App`（带 ` CN App`），
    /// 国际版是 `Qoder`。曾经读写两侧都硬编码 `"Qoder Safe Storage"`，国内版在
    /// macOS 上永远查不到条目 ⇒ 桌面登录态解不开 ⇒ 导入与切换全失败（issue #81）。
    /// 服务名在本表按区域维护，**不许**在调用方硬编码。
    pub macos_keychain_service: &'static str,
    /// ★ OAuth 设备授权 URL **是否携带 `redirect_uri`**。
    ///
    /// 两区**确实不同**（国内带、国际不带）——这是实测结论，不是猜测。
    /// 把它做成数据而不是 `if region == Cn` 分支，是为了让「新增区域」只加一行表项。
    pub send_redirect_uri: bool,
    /// OAuth 授权 URL 里 `machine_id` / nonce 的形态：`true` = 带横线的 uuid，
    /// `false` = 32 位无横线 hex。两区不同。
    pub nonce_dashed: bool,
}

/// 国内版。
pub const CN_REALM: RealmSpec = RealmSpec {
    region: QoderRegion::Cn,
    website: "https://qoder.com.cn",
    openapi_base: "https://openapi.qoder.com.cn",
    gateway_base: "https://gateway.qoder.com.cn",
    gateway_fallbacks: &[],
    client_id: "1c5e33e1-364d-4ce6-b02c-acaa81274a5c",
    redirect_uri: "qoder-work-cn://",
    domain: "qoder.com.cn",
    user_agent: "QoderWork/1.1.64",
    cli_home_dir: ".qoder-cn",
    desktop_app_dir: "com.qodercn.app.stable",
    macos_keychain_service: "Qoder CN App Safe Storage",
    send_redirect_uri: true,
    nonce_dashed: true,
};

/// 国际版。
pub const GLOBAL_REALM: RealmSpec = RealmSpec {
    region: QoderRegion::Global,
    website: "https://qoder.com",
    openapi_base: "https://openapi.qoder.sh",
    gateway_base: "https://api1.qoder.sh",
    gateway_fallbacks: &["https://api2.qoder.sh", "https://api3.qoder.sh"],
    client_id: "e883ade2-e6e3-4d6d-adf7-f92ceff5fdcb",
    redirect_uri: "qoder://aicoding.aicoding-agent/login-success",
    domain: "qoder.com",
    user_agent: "Qoder/1.1.64",
    cli_home_dir: ".qoder",
    desktop_app_dir: "com.qoder.app.stable",
    macos_keychain_service: "Qoder Safe Storage",
    send_redirect_uri: false,
    nonce_dashed: false,
};

/// 取区域的差异表（**唯一查表入口**）。
pub fn realm_spec(region: QoderRegion) -> &'static RealmSpec {
    match region {
        QoderRegion::Cn => &CN_REALM,
        QoderRegion::Global => &GLOBAL_REALM,
    }
}

/// 推理网关的**候选基址**（主 + 备用，顺序即优先级）。
///
/// 调用方按顺序尝试；全失败才算上游不可达。国内版只有一个候选。
pub fn gateway_candidates(region: QoderRegion) -> Vec<&'static str> {
    let spec = realm_spec(region);
    let mut out = Vec::with_capacity(1 + spec.gateway_fallbacks.len());
    out.push(spec.gateway_base);
    out.extend_from_slice(spec.gateway_fallbacks);
    out
}

/// 由域名反查区域（账号记录里带 `domain`，跨区校验用）。
///
/// **fail-closed**：不认识就 `None`，不回落 —— 回落会让「国际版的账号」被当成
/// 国内版写进错误的库。
///
/// 入参**允许是 URL**（用户常从浏览器地址栏复制）：先剥掉 `scheme://` 与路径 / 查询串，
/// 再按域名比对。剥壳**不削弱** fail-closed —— 剥完仍要整段或后缀命中某区域名，
/// 否则照样 `None`（`evil.com/?x=qoder.com` 不会因为「包含 qoder.com」而被认成国际版）。
pub fn region_of_domain(domain: &str) -> Option<QoderRegion> {
    let normalized = normalize_domain(domain);
    if normalized.is_empty() {
        return None;
    }
    for region in QoderRegion::all() {
        let spec = realm_spec(region);
        if normalized == spec.domain || normalized.ends_with(&format!(".{}", spec.domain)) {
            return Some(region);
        }
    }
    None
}

/// 把 `https://api.qoder.com/path?x=1` 归一成 `api.qoder.com`。
///
/// ⚠️ 只做「剥壳」：不补全、不猜测、不把裸主机名当域名。剥完为空 ⇒ 调用方返回 `None`。
fn normalize_domain(raw: &str) -> String {
    let mut value = raw.trim().to_ascii_lowercase();
    if let Some(rest) = value.split_once("://").map(|(_, rest)| rest) {
        value = rest.to_string();
    }
    // 去掉路径 / 查询串 / 片段（第一个 `/`、`?`、`#` 之前是 host[:port]）。
    let cut = value.find(['/', '?', '#']).unwrap_or(value.len());
    let mut host = value[..cut].to_string();
    // 去掉端口与可能残留的用户信息（`user@host`）。
    if let Some(at) = host.rfind('@') {
        host = host[at + 1..].to_string();
    }
    if let Some(colon) = host.find(':') {
        host.truncate(colon);
    }
    host
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_and_parse_round_trip() {
        for region in QoderRegion::all() {
            assert_eq!(QoderRegion::parse(region.as_str()), Some(region));
        }
        assert_eq!(QoderRegion::all().len(), 2);
        assert_eq!(QoderRegion::default(), QoderRegion::Cn);
    }

    /// 两区的差异**必须真的不同** —— 否则「区域」这个轴就没有存在意义。
    ///
    /// 可证伪：把国际版的 `openapi_base` 抄成国内版，本用例立即变红。
    #[test]
    fn the_two_realms_differ_on_every_axis_that_matters() {
        let cn = realm_spec(QoderRegion::Cn);
        let global = realm_spec(QoderRegion::Global);
        for (name, a, b) in [
            ("website", cn.website, global.website),
            ("openapi_base", cn.openapi_base, global.openapi_base),
            ("gateway_base", cn.gateway_base, global.gateway_base),
            ("client_id", cn.client_id, global.client_id),
            ("redirect_uri", cn.redirect_uri, global.redirect_uri),
            ("domain", cn.domain, global.domain),
            ("user_agent", cn.user_agent, global.user_agent),
            ("cli_home_dir", cn.cli_home_dir, global.cli_home_dir),
            (
                "desktop_app_dir",
                cn.desktop_app_dir,
                global.desktop_app_dir,
            ),
            (
                "macos_keychain_service",
                cn.macos_keychain_service,
                global.macos_keychain_service,
            ),
        ] {
            assert_ne!(a, b, "{name} 在两区相同 —— 该差异必须真实存在");
        }
        // ★ OAuth 授权 URL 的构造差异（两区确实不同，不是猜测）。
        assert!(cn.send_redirect_uri, "国内版授权 URL 带 redirect_uri");
        assert!(
            !global.send_redirect_uri,
            "国际版授权 URL 不带 redirect_uri"
        );
        assert!(cn.nonce_dashed, "国内版 nonce 带横线");
        assert!(!global.nonce_dashed, "国际版 nonce 为 32 位无横线 hex");
    }

    #[test]
    fn gateway_candidates_start_with_the_primary_and_only_global_has_fallbacks() {
        assert_eq!(
            gateway_candidates(QoderRegion::Cn),
            vec!["https://gateway.qoder.com.cn"]
        );
        let global = gateway_candidates(QoderRegion::Global);
        assert_eq!(global[0], "https://api1.qoder.sh", "主基址必须在第一位");
        assert_eq!(global.len(), 3, "国际版有 3 个候选");
        assert_eq!(realm_spec(QoderRegion::Cn).gateway_fallbacks.len(), 0);
    }

    /// ★ macOS 钥匙串服务名按区域取**实测值**（issue #81 的根因防线）。
    ///
    /// 取证：2026-09-23，macOS 15.6.1 / arm64，`security dump-keychain` ——
    /// 国内版条目 `svce="Qoder CN App Safe Storage"` / `acct="Qoder CN App Key"`，
    /// 国际版条目 `svce="Qoder Safe Storage"` / `acct="Qoder Key"`。
    ///
    /// 可证伪：把国内版改成国际版的值（或反过来），本用例立即变红 ——
    /// 那正是 issue #81 的缺陷形态：macOS 上国内版桌面登录态永远解不开，
    /// 界面呈「桌面端检测不到 / 读不出来」，导入与切换全失败。
    #[test]
    fn macos_keychain_service_names_are_the_measured_ones() {
        assert_eq!(
            realm_spec(QoderRegion::Cn).macos_keychain_service,
            "Qoder CN App Safe Storage"
        );
        assert_eq!(
            realm_spec(QoderRegion::Global).macos_keychain_service,
            "Qoder Safe Storage"
        );
        // 命名规律：`<Electron 应用名> Safe Storage`，两区基名不同（CN 带 ` CN App`）。
        assert!(realm_spec(QoderRegion::Cn)
            .macos_keychain_service
            .starts_with("Qoder CN App "));
        assert_ne!(
            realm_spec(QoderRegion::Cn).macos_keychain_service,
            realm_spec(QoderRegion::Global).macos_keychain_service,
        );
    }

    /// 域名反查 **fail-closed**：不认识就 `None`，**不回落**。
    ///
    /// 可证伪：在末尾加 `_ => Some(QoderRegion::Cn)` 式兜底，第三条断言立即变红
    /// —— 那正是「国际版账号被写进国内版库」的缺陷形态。
    #[test]
    fn region_of_domain_is_fail_closed() {
        assert_eq!(region_of_domain("qoder.com.cn"), Some(QoderRegion::Cn));
        assert_eq!(region_of_domain("qoder.com"), Some(QoderRegion::Global));
        // 允许整条 URL（用户从地址栏复制），剥壳后仍按域名判定。
        assert_eq!(
            region_of_domain("https://qoder.com"),
            Some(QoderRegion::Global)
        );
        assert_eq!(
            region_of_domain("https://api.qoder.com.cn/oauth?x=1#frag"),
            Some(QoderRegion::Cn)
        );
        // 子域按后缀匹配；但**不能**让 qoder.com.cn 命中 qoder.com。
        assert_eq!(region_of_domain("api.qoder.com.cn"), Some(QoderRegion::Cn));
        assert_eq!(region_of_domain("api.qoder.com"), Some(QoderRegion::Global));
        // 大小写与端口不影响判定。
        assert_eq!(
            region_of_domain("API.QODER.COM:443"),
            Some(QoderRegion::Global)
        );
        for bad in [
            "",
            "   ",
            "example.com",
            "qoder",
            "notqoder.com.cn",
            // ★ 剥壳**不得**变成「包含即命中」：域名里出现 `qoder.com` 不算数。
            "evil.com/?x=qoder.com",
            "qoder.com.evil.com",
        ] {
            assert_eq!(region_of_domain(bad), None, "{bad:?} 不该反查出区域");
        }
    }
}
