//! OAuth 客户端凭证外置配置（`trae/conf/oauth_client.json`）。
//!
//! ## 为什么需要它
//!
//! `client_id` 是**上游会变**的公开标识，把它硬编码在二进制里意味着「上游一改就得发版」。
//! 外置后改文件 + 重启即可。
//!
//! ## ★ `client_id` 是**按产品线**的两把钥匙（2026-09-21 更正）
//!
//! 客户端 `product.json` 的 `iCubeApp.authConfig` 里 **SOLO 与 TRAE 各有一把**
//! （本机三台客户端实测逐字相同）：
//!
//! | 产品线 | `auth_from` | `client_id`（stable） | 额外参数 |
//! |:---|:---|:---|:---|
//! | SOLO（`packageType` ∈ `SOLO_CN` / `SOLO_I18N`） | `solo` | `en1oxy7wnw8j9n` | `hide_saas_login=true` |
//! | TRAE（其余） | `trae` | `ono9krqynydwx5` | 无 |
//!
//! 本模块**曾经只有一把**（IDE 那把），于是 SOLO 线的登录拿着 IDE 的钥匙 ——
//! 症状是授权页停在 billing status 后不回跳。现在按线取值：
//! [`OAuthClientConfig::client_id_for`]`(variant.oauth_line())`。
//!
//! 覆盖优先级：`client_id_solo` / `client_id_trae`（该线专属）> `client_id`（全局）>
//! 内置默认。**正常只用内置默认**，配置项是给「上游换钥匙」用的逃生口。
//!
//! ## 与参考实现的差异（有意）
//!
//! 参考 `oauth.rs:36-84` 外置的是**完整 `exchange_url`**；本模块外置的是 **path**。
//! 理由：新协议下 host 由**回调回传**（`${host}/trae/api/v3/oauth/ExchangeToken`），
//! 外置整条 URL 会与回调 host 打架 —— 一个是「配置说打 A 域」、一个是「回调说打 B 域」，
//! 二者冲突时的行为无从定义。外置 path 则与 host 正交，不会产生这种二义。
//!
//! ## 生效时机
//!
//! 进程内 `OnceLock` 缓存 ⇒ **改文件后需重启应用**。这是刻意的取舍：
//! 每次登录都读一次盘虽然可行，但「配置在会话中途被改」会让同一次登录的
//! 授权 URL 与交换请求用上不同的 client_id，属于极难定位的偶发缺陷。

use std::sync::OnceLock;

use crate::modules::trae::paths;
use crate::modules::trae::variant::OAuthLine;
use crate::modules::trae::TRAE_EXCHANGE_TOKEN_PATH;

/// OAuth 客户端凭证（`client_id` / `client_secret` / 交换路径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClientConfig {
    /// **全局** ClientID 覆盖（两条产品线共用；`None` ⇒ 各线用自己的内置默认）。
    ///
    /// ⚠️ 正常**不要设**：两条产品线的 `client_id` 本来就不同
    /// （见 [`OAuthClientConfig::client_id_for`]），设了它等于把两条线强行统一 ——
    /// 那只在「上游把两把钥匙都换了、要临时整体替换」时才用。
    /// 只想改一条线请用 [`OAuthClientConfig::client_id_solo`] /
    /// [`OAuthClientConfig::client_id_trae`]。
    pub client_id: Option<String>,
    /// SOLO 产品线（TraeWork）的 ClientID 覆盖；`None` ⇒ 用内置默认。
    pub client_id_solo: Option<String>,
    /// TRAE 产品线（TraeCode / IDE）的 ClientID 覆盖；`None` ⇒ 用内置默认。
    pub client_id_trae: Option<String>,
    /// 旧协议的 `ClientSecret`（默认 `"-"`，仅兜底链末位用到）。
    pub client_secret: String,
    /// 交换端点 **path**（默认 [`TRAE_EXCHANGE_TOKEN_PATH`]；host 由回调回传）。
    pub exchange_path: String,
}

impl OAuthClientConfig {
    /// 取某条产品线实际要用的 `client_id`。
    ///
    /// 优先级：**该线的专属覆盖** > **全局覆盖** > **内置默认**
    /// （内置默认 = 客户端 `product.json` 的
    /// `iCubeApp.authConfig.<线>.stable`，见 [`OAuthLine::default_client_id`]）。
    ///
    /// ★ 这里**必须按产品线分**，不能只有一把钥匙：`client_id` 与 `auth_from`
    /// 是客户端同一处分支派生出来的一对（见 [`OAuthLine::from_package_type`]），
    /// 拿 IDE 的钥匙去走 SOLO 的流程，授权页会停在 billing status 后不回跳。
    pub fn client_id_for(&self, line: OAuthLine) -> &str {
        let per_line = match line {
            OAuthLine::Solo => self.client_id_solo.as_deref(),
            OAuthLine::Trae => self.client_id_trae.as_deref(),
        };
        per_line
            .or(self.client_id.as_deref())
            .unwrap_or_else(|| line.default_client_id())
    }
}

impl Default for OAuthClientConfig {
    fn default() -> Self {
        Self {
            // 三个覆盖都缺省：各线用自己的内置默认（这才是「什么都不配」的正确语义）。
            // 注意**不能**把 `client_id` 缺省成 `TRAE_OAUTH_CLIENT_ID` ——
            // 那会让 SOLO 线永远拿到 IDE 的钥匙，正是本轮要修的缺陷。
            client_id: None,
            client_id_solo: None,
            client_id_trae: None,
            client_secret: DEFAULT_CLIENT_SECRET.to_string(),
            exchange_path: TRAE_EXCHANGE_TOKEN_PATH.to_string(),
        }
    }
}

/// 旧协议兜底链使用的默认 `ClientSecret`（与参考 `oauth.rs:23` 逐字一致）。
pub const DEFAULT_CLIENT_SECRET: &str = "-";

/// 外置文件的 JSON 形状（字段全部可缺省 ⇒ 回落默认值）。
#[derive(serde::Deserialize)]
struct OAuthClientFile {
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_id_solo: Option<String>,
    #[serde(default)]
    client_id_trae: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    exchange_path: Option<String>,
}

/// 取非空字符串，否则回落默认值。
fn pick(value: Option<String>, fallback: &str) -> String {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// 取非空字符串；缺省 / 空串 / 纯空白一律视为「没配」。
fn pick_opt(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// 解析外置配置文本；字段缺省 / 空串 / 整份损坏一律回落默认值。
///
/// **不报错**：这是「上游凭证的兜底来源」，读不到就应该用内置默认值继续工作，
/// 而不是让整个 OAuth 登录因为一个配置文件写坏了而不可用。
fn from_json_text(text: &str) -> OAuthClientConfig {
    let default = OAuthClientConfig::default();
    let Ok(parsed) = serde_json::from_str::<OAuthClientFile>(text) else {
        return default;
    };
    OAuthClientConfig {
        client_id: pick_opt(parsed.client_id),
        client_id_solo: pick_opt(parsed.client_id_solo),
        client_id_trae: pick_opt(parsed.client_id_trae),
        client_secret: pick(parsed.client_secret, &default.client_secret),
        exchange_path: pick(parsed.exchange_path, &default.exchange_path),
    }
}

/// 从指定路径读配置（文件缺失 / 不可读 ⇒ 默认值）。
fn from_file(path: &std::path::Path) -> OAuthClientConfig {
    match std::fs::read_to_string(path) {
        Ok(text) => from_json_text(&text),
        Err(_) => OAuthClientConfig::default(),
    }
}

/// 读取外置 OAuth 客户端配置（全局一次；缺失/损坏回退内置默认）。
pub fn oauth_client() -> &'static OAuthClientConfig {
    static CONFIG: OnceLock<OAuthClientConfig> = OnceLock::new();
    CONFIG.get_or_init(|| from_file(&paths::oauth_client_config_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ 文件缺失 ⇒ 回落内置默认（登录不该因为少一个可选配置文件而不可用）。
    #[test]
    fn oauth_client_config_falls_back_when_file_missing() {
        let missing = std::env::temp_dir().join("definitely-not-here-oauth-client.json");
        assert_eq!(from_file(&missing), OAuthClientConfig::default());

        // 默认值必须与抓包固化常量逐字一致。
        let default = OAuthClientConfig::default();
        assert_eq!(
            default.client_id, None,
            "全局覆盖必须缺省，否则会盖掉 SOLO 线"
        );
        assert_eq!(default.client_secret, "-");
        assert_eq!(default.exchange_path, "/trae/api/v3/oauth/ExchangeToken");
    }

    /// ★★ `client_id` 必须**按产品线**分（2026-09-21 修的真实缺陷）。
    ///
    /// 依据是客户端自己的分派（国际版 `out/main.js`）：
    /// `clientId = Pr(t) ? authConfig.SOLO[channel] : authConfig.TRAE[channel]`，
    /// 而本机三台客户端的 `iCubeApp.authConfig` **逐字相同**：
    /// SOLO.stable = `en1oxy7wnw8j9n`、TRAE.stable = `ono9krqynydwx5`。
    ///
    /// 缺陷形态：只有一把钥匙（IDE 的那把），两条线都用它 ⇒ SOLO 线的登录
    /// 授权页停在 billing status 后不回跳。
    #[test]
    fn client_id_is_split_by_oauth_line() {
        let default = OAuthClientConfig::default();
        assert_eq!(default.client_id_for(OAuthLine::Solo), "en1oxy7wnw8j9n");
        assert_eq!(default.client_id_for(OAuthLine::Trae), "ono9krqynydwx5");
        assert_ne!(
            default.client_id_for(OAuthLine::Solo),
            default.client_id_for(OAuthLine::Trae),
            "两条产品线的 client_id 必须不同（客户端 authConfig 里就是两把钥匙）"
        );
        // TRAE 线的默认值 = 既有契约常量（回归护栏：不能把别名改漂）。
        assert_eq!(
            default.client_id_for(OAuthLine::Trae),
            crate::modules::trae::TRAE_OAUTH_CLIENT_ID
        );
    }

    /// 覆盖优先级：**该线专属** > **全局** > **内置默认**。
    #[test]
    fn client_id_override_precedence_is_per_line_then_global() {
        // 只设该线专属：另一条线不受影响。
        let solo_only = from_json_text(r#"{"client_id_solo":"solo-x"}"#);
        assert_eq!(solo_only.client_id_for(OAuthLine::Solo), "solo-x");
        assert_eq!(solo_only.client_id_for(OAuthLine::Trae), "ono9krqynydwx5");

        let trae_only = from_json_text(r#"{"client_id_trae":"trae-x"}"#);
        assert_eq!(trae_only.client_id_for(OAuthLine::Trae), "trae-x");
        assert_eq!(trae_only.client_id_for(OAuthLine::Solo), "en1oxy7wnw8j9n");

        // 只设全局：两条线都跟着走（这是「上游整体换钥匙」的用法）。
        let global = from_json_text(r#"{"client_id":"both-x"}"#);
        assert_eq!(global.client_id_for(OAuthLine::Solo), "both-x");
        assert_eq!(global.client_id_for(OAuthLine::Trae), "both-x");

        // 全局 + 该线专属：专属赢。
        let both = from_json_text(r#"{"client_id":"both-x","client_id_solo":"solo-y"}"#);
        assert_eq!(both.client_id_for(OAuthLine::Solo), "solo-y");
        assert_eq!(both.client_id_for(OAuthLine::Trae), "both-x");

        // 空串 / 纯空白不算有效覆盖 ⇒ 回落内置默认（不是回落成空 client_id）。
        let blank = from_json_text(r#"{"client_id":"   ","client_id_solo":""}"#);
        assert_eq!(blank.client_id_for(OAuthLine::Solo), "en1oxy7wnw8j9n");
        assert_eq!(blank.client_id_for(OAuthLine::Trae), "ono9krqynydwx5");
    }

    /// ★ 外置字段可覆盖；部分缺省时只覆盖写了的那个。
    #[test]
    fn oauth_client_config_reads_overrides() {
        let full = from_json_text(
            r#"{"client_id":"custom-id","client_secret":"s3cret","exchange_path":"/custom/Exchange"}"#,
        );
        assert_eq!(full.client_id.as_deref(), Some("custom-id"));
        assert_eq!(full.client_secret, "s3cret");
        assert_eq!(full.exchange_path, "/custom/Exchange");

        // 只写 client_secret：其余回落默认。
        let partial = from_json_text(r#"{"client_secret":"only-secret"}"#);
        assert_eq!(partial.client_id, None);
        assert_eq!(partial.client_secret, "only-secret");
        assert_eq!(partial.exchange_path, "/trae/api/v3/oauth/ExchangeToken");

        // 空串与纯空白不算有效覆盖。
        let blank = from_json_text(r#"{"client_secret":""}"#);
        assert_eq!(blank.client_secret, "-");

        // 整份损坏 ⇒ 全部回落默认，绝不 panic。
        assert_eq!(
            from_json_text("{ this is not json"),
            OAuthClientConfig::default()
        );
        assert_eq!(from_json_text("[]"), OAuthClientConfig::default());
    }

    /// 默认交换路径必须与 `mod.rs` 的契约常量同源（避免两处漂移）。
    #[test]
    fn default_exchange_path_matches_contract_constant() {
        assert_eq!(
            OAuthClientConfig::default().exchange_path,
            crate::modules::trae::TRAE_EXCHANGE_TOKEN_PATH
        );
        // `TRAE_OAUTH_CLIENT_ID` 是 `OAuthLine::Trae` 的别名 —— 单一来源在
        // `OAuthLine::default_client_id`，这里钉住别名没有漂。
        assert_eq!(
            crate::modules::trae::TRAE_OAUTH_CLIENT_ID,
            OAuthLine::Trae.default_client_id()
        );
    }
}
