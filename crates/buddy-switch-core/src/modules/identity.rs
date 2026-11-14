//! 客户端身份：版本解析链 + 国际版 UA 构造。
//!
//! 对照参考实现 `app-version.ts` + `client-identity.ts`。
//!
//! - 版本解析链：已安装 App → 保存值 → 内置兜底（永不阻断请求）。
//! - 目录请求 UA：`WorkBuddyAI/<version>`（**绝对不能带空格**，带空格会被上游
//!   拒为 HTTP 400 / code 12403）。
//! - chat UA：`WorkBuddy/<v> WorkBuddy AI/<v>`（global）或
//!   `WorkBuddy/<v> WorkBuddy/<v>`（cn），可选追加 `CLI/<cli>`。
//!
//! 版本号必须通过 [`valid_app_version`] 校验后才可拼进 header（防 header 注入）。

use std::path::Path;

use crate::modules::config::store_dir;
use crate::modules::region::{region_spec, Region};

/// 已安装 App 版本来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppVersionSource {
    /// 从已安装 App bundle 读出。
    Installed,
    /// 上次成功保存的值。
    Saved,
    /// 编译进程序的内置兜底。
    Fallback,
}

/// 解析出的版本及其来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppVersionInfo {
    /// 版本号。
    pub version: String,
    /// 来源。
    pub source: AppVersionSource,
    /// 已安装 App bundle 路径（仅 `Installed` 时存在）。
    pub bundle: Option<String>,
}

/// chat 请求呈现的客户端身份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatIdentity {
    /// 桌面 App 版本，驱动两个 `WorkBuddy/<v>` 产品 token。
    pub client_version: String,
    /// 内置 CLI 版本；缺失时省略 `CLI/…` token。
    pub cli_version: Option<String>,
}

/// 校验 App 版本是否可安全拼进 header。
///
/// 严格匹配 `^\d{1,6}(?:\.\d{1,6}){1,3}$`：任何空白、CR/LF 或多余 token 都不通过。
pub fn valid_app_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() < 2 || parts.len() > 4 {
        return false;
    }
    parts
        .iter()
        .all(|part| !part.is_empty() && part.len() <= 6 && part.bytes().all(|b| b.is_ascii_digit()))
}

/// 校验 CLI 版本是否可安全拼进 header（允许 `-rc.1` 之类预发布后缀）。
///
/// 匹配 `^\d{1,6}(?:\.\d{1,6}){1,3}(?:-[0-9A-Za-z.]+)?$`。
pub fn valid_cli_version(value: &str) -> bool {
    let (base, suffix) = match value.split_once('-') {
        Some((base, suffix)) => (base, Some(suffix)),
        None => (value, None),
    };
    if !valid_app_version(base) {
        return false;
    }
    if let Some(suffix) = suffix {
        if suffix.is_empty()
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.')
        {
            return false;
        }
    }
    true
}

/// 构造 App 形态 UA（目录请求用）。版本非法即报错，不发送畸形 header。
pub fn app_user_agent(version: &str) -> Result<String, String> {
    if !valid_app_version(version) {
        return Err(format!(
            "invalid WorkBuddy AI version for User-Agent: {version:?}"
        ));
    }
    Ok(format!("WorkBuddyAI/{version}"))
}

/// 构造 chat 形态 UA。
///
/// `region` 决定产品 token：Global 用 `WorkBuddy AI`，CN 用 `WorkBuddy`。
/// `cli_version` 存在时追加 `CLI/<cli>`。版本非法即报错。
pub fn chat_user_agent(
    region: Region,
    client_version: &str,
    cli_version: Option<&str>,
) -> Result<String, String> {
    if !valid_app_version(client_version) {
        return Err(format!(
            "invalid client version for chat User-Agent: {client_version:?}"
        ));
    }
    let product = match region {
        Region::Global => "WorkBuddy AI",
        Region::Cn => "WorkBuddy",
    };
    let mut parts = vec![
        format!("WorkBuddy/{client_version}"),
        format!("{product}/{client_version}"),
    ];
    if let Some(cli) = cli_version {
        if !valid_cli_version(cli) {
            return Err(format!("invalid CLI version for chat User-Agent: {cli:?}"));
        }
        parts.push(format!("CLI/{cli}"));
    }
    Ok(parts.join(" "))
}

/// 版本保存文件路径：`~/.buddy-switch/app_version.<region>.json`。
///
/// 两版各存一份：CN App 把版本写进国际版缓存会污染后者的 UA。
pub fn app_version_cache_file(region: Region) -> std::path::PathBuf {
    store_dir().join(format!("app_version.{}.json", region.as_str()))
}

/// 从 `Info.plist` 读 `<key>…</key>` 后面紧跟的 `<string>…</string>` 值（macOS）。
///
/// [`read_bundle_version`] 与 [`read_bundle_executable`] 共用这一份，避免两处 plist 读法漂移。
fn read_plist_string(plist_path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(plist_path).ok()?;
    let needle = format!("<key>{key}</key>");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let open = rest.find("<string>")? + "<string>".len();
    let close = rest[open..].find("</string>")? + open;
    Some(rest[open..close].trim().to_string())
}

/// 从 `Info.plist` 读 `CFBundleShortVersionString`（macOS）。
pub fn read_bundle_version(plist_path: &Path) -> Option<String> {
    let version = read_plist_string(plist_path, "CFBundleShortVersionString")?;
    valid_app_version(&version).then_some(version)
}

/// 从 `Info.plist` 读 `CFBundleExecutable` —— `.app` 内可执行体的名称（macOS）。
///
/// 该键与包名**不同**：Electron 的可执行体名由 plist 声明。实测两个 WorkBuddy
/// bundle 均为 `Electron`，按包名拼接得到的路径不存在，且该错误表现为
/// 「客户端未安装」而非「启动失败」。见 [`crate::modules::at_rest::client_executable_for`]。
pub fn read_bundle_executable(plist_path: &Path) -> Option<String> {
    let name = read_plist_string(plist_path, "CFBundleExecutable")?;
    (!name.is_empty()).then_some(name)
}

/// macOS App bundle 根目录（系统级优先，其次用户级）。
#[cfg(target_os = "macos")]
fn mac_app_roots() -> Vec<std::path::PathBuf> {
    let home = crate::modules::config::home_dir();
    vec![
        std::path::PathBuf::from("/Applications"),
        home.join("Applications"),
    ]
}

/// 已安装 App 的版本（仅 macOS；Windows/Linux 未验证 bundle 元数据位置，返回 None）。
pub fn installed_app_version(region: Region) -> Option<(String, String)> {
    #[cfg(target_os = "macos")]
    {
        let bundle_name = match region {
            Region::Global => "WorkBuddy AI.app",
            Region::Cn => "WorkBuddy.app",
        };
        for root in mac_app_roots() {
            let bundle = root.join(bundle_name);
            let plist = bundle.join("Contents").join("Info.plist");
            if let Some(version) = read_bundle_version(&plist) {
                return Some((version, bundle.to_string_lossy().into_owned()));
            }
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = region;
        None
    }
}

/// 读取 bundle 内 `cli/package.json` 的真实 CLI 版本。
pub fn read_cli_version(bundle: &str) -> Option<String> {
    let path = Path::new(bundle)
        .join("Contents")
        .join("Resources")
        .join("app.asar.unpacked")
        .join("cli")
        .join("package.json");
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let declared = value.get("version").and_then(|v| v.as_str());
    if let Some(declared) = declared {
        if valid_cli_version(declared) && declared != "0.0.0" {
            return Some(declared.to_string());
        }
    }
    let custom = value
        .get("publishConfig")
        .and_then(|p| p.get("customPackage"))
        .and_then(|c| c.get("version"))
        .and_then(|v| v.as_str());
    custom
        .filter(|v| valid_cli_version(v))
        .map(|v| v.to_string())
}

fn load_saved_version(region: Region) -> Option<String> {
    let path = app_version_cache_file(region);
    let text = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let version = value.get("version").and_then(|v| v.as_str())?;
    valid_app_version(version).then(|| version.to_string())
}

fn save_version_cache(region: Region, version: &str) {
    let path = app_version_cache_file(region);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = serde_json::json!({
        "version": version,
        "observedAt": crate::modules::config::now_ms(),
    });
    let _ = crate::modules::config::atomic_write(
        &path,
        &serde_json::to_string_pretty(&content).unwrap_or_default(),
    );
}

/// 解析 App 版本：已安装 App → 保存值 → 内置兜底。永不失败。
pub fn resolve_app_version(region: Region) -> AppVersionInfo {
    if let Some((version, bundle)) = installed_app_version(region) {
        if valid_app_version(&version) {
            save_version_cache(region, &version);
            return AppVersionInfo {
                version,
                source: AppVersionSource::Installed,
                bundle: Some(bundle),
            };
        }
    }
    if let Some(version) = load_saved_version(region) {
        return AppVersionInfo {
            version,
            source: AppVersionSource::Saved,
            bundle: None,
        };
    }
    AppVersionInfo {
        version: region_spec(region).fallback_app_version.to_string(),
        source: AppVersionSource::Fallback,
        bundle: None,
    }
}

/// region 的内置兜底身份：桌面形态 + 内置版本 + 无 `CLI/…`。
pub fn fallback_chat_identity(region: Region) -> ChatIdentity {
    ChatIdentity {
        client_version: region_spec(region).fallback_app_version.to_string(),
        cli_version: None,
    }
}

/// 解析 chat 身份：已安装 App → 该 region 保存值 → 内置兜底。永不失败。
pub fn resolve_chat_identity(region: Region) -> ChatIdentity {
    let info = resolve_app_version(region);
    let client_version = if valid_app_version(&info.version) {
        info.version.clone()
    } else {
        region_spec(region).fallback_app_version.to_string()
    };
    let cli_version = info
        .bundle
        .as_deref()
        .and_then(read_cli_version)
        .filter(|v| valid_cli_version(v));
    ChatIdentity {
        client_version,
        cli_version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_user_agent_has_no_space() {
        let ua = app_user_agent("5.5.2").expect("valid version");
        assert_eq!(ua, "WorkBuddyAI/5.5.2");
        assert!(!ua.contains(' '), "目录 UA 绝不能带空格: {ua}");
    }

    #[test]
    fn app_user_agent_rejects_invalid_versions() {
        assert!(app_user_agent("WorkBuddy AI/5.5.2").is_err());
        assert!(app_user_agent("5").is_err());
        assert!(app_user_agent("5.5.2.3.4").is_err());
        assert!(app_user_agent("5.5.2\r\nX: y").is_err());
        assert!(app_user_agent("").is_err());
        assert!(app_user_agent("5.5.2-x").is_err());
    }

    #[test]
    fn chat_user_agent_has_two_product_forms() {
        assert_eq!(
            chat_user_agent(Region::Global, "5.5.2", None).unwrap(),
            "WorkBuddy/5.5.2 WorkBuddy AI/5.5.2"
        );
        assert_eq!(
            chat_user_agent(Region::Cn, "5.5.6", None).unwrap(),
            "WorkBuddy/5.5.6 WorkBuddy/5.5.6"
        );
    }

    #[test]
    fn chat_user_agent_appends_cli_token_when_present() {
        assert_eq!(
            chat_user_agent(Region::Cn, "5.5.6", Some("2.63.2")).unwrap(),
            "WorkBuddy/5.5.6 WorkBuddy/5.5.6 CLI/2.63.2"
        );
        assert_eq!(
            chat_user_agent(Region::Global, "5.5.2", Some("2.137.1-rc.1")).unwrap(),
            "WorkBuddy/5.5.2 WorkBuddy AI/5.5.2 CLI/2.137.1-rc.1"
        );
    }

    #[test]
    fn version_validators_match_reference_contract() {
        assert!(valid_app_version("5.5.2"));
        assert!(valid_app_version("5.5.2.3"));
        // 参考实现 `validAppVersion('5.5') === true`（`^\d{1,6}(?:\.\d{1,6}){1,3}$`）。
        assert!(valid_app_version("5.5"));
        assert!(!valid_app_version("5"));
        assert!(!valid_app_version("v5.5.2"));
        assert!(!valid_app_version("5.5.2 "));

        assert!(valid_cli_version("2.63.2"));
        assert!(valid_cli_version("2.137.1-rc.1"));
        assert!(!valid_cli_version("2.63.2 rc"));
        assert!(!valid_cli_version("2"));
    }

    #[test]
    fn resolve_app_version_falls_back_when_absent() {
        // 无注入：真实文件系统上大概率无已安装 App，至少不会 panic 且版本合法。
        let info = resolve_app_version(Region::Global);
        assert!(valid_app_version(&info.version), "{info:?}");
    }

    #[test]
    fn resolve_chat_identity_never_panics_and_is_valid() {
        let identity = resolve_chat_identity(Region::Cn);
        assert!(valid_app_version(&identity.client_version));
        assert!(chat_user_agent(
            Region::Cn,
            &identity.client_version,
            identity.cli_version.as_deref()
        )
        .is_ok());
    }

    /// 构造仅含 plist 的临时 `.app`，返回 `(临时根目录, bundle 路径, plist 路径)`。
    fn fake_bundle(
        plist_body: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "buddy-switch-plist-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let bundle = root.join("WorkBuddy AI.app");
        let plist = bundle.join("Contents").join("Info.plist");
        std::fs::create_dir_all(plist.parent().expect("parent")).expect("create bundle");
        std::fs::write(&plist, plist_body).expect("write plist");
        (root, bundle, plist)
    }

    /// 真实 `Info.plist` 的最小形态。Electron 客户端的可执行体名与包名不同。
    const REAL_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>Electron</string>
	<key>CFBundleShortVersionString</key>
	<string>5.8.4</string>
	<key>CFBundleName</key>
	<string>WorkBuddy AI</string>
</dict>
</plist>
"#;

    #[test]
    fn read_plist_string_reads_the_value_right_after_its_key() {
        let (root, _bundle, plist) = fake_bundle(REAL_PLIST);
        assert_eq!(
            read_plist_string(&plist, "CFBundleExecutable").as_deref(),
            Some("Electron")
        );
        assert_eq!(
            read_plist_string(&plist, "CFBundleShortVersionString").as_deref(),
            Some("5.8.4")
        );
        // 键缺失与文件缺失均返回 None，不 panic。
        assert!(read_plist_string(&plist, "CFBundleIdentifier").is_none());
        assert!(
            read_plist_string(&plist.with_extension("missing"), "CFBundleExecutable").is_none()
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    /// 回归：可执行体名取自 plist，不按包名推断。
    ///
    /// 可证伪：`read_bundle_executable` 改为返回包名时，第一条断言得到 `WorkBuddy AI`。
    #[test]
    fn read_bundle_executable_does_not_assume_the_bundle_name() {
        let (root, bundle, plist) = fake_bundle(REAL_PLIST);
        let name = read_bundle_executable(&plist).expect("应能读出可执行体名");
        assert_eq!(name, "Electron");
        assert_ne!(
            name,
            bundle.file_stem().expect("stem").to_string_lossy(),
            "可执行体名与包名不同，按包名拼会得到一个不存在的路径"
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn read_bundle_executable_rejects_blank_and_missing_key() {
        let (root, _bundle, plist) = fake_bundle(
            "<plist><dict><key>CFBundleExecutable</key><string>  </string></dict></plist>",
        );
        assert!(
            read_bundle_executable(&plist).is_none(),
            "空白名不得当成有效值"
        );
        let (root2, _bundle2, plist2) = fake_bundle("<plist><dict></dict></plist>");
        assert!(read_bundle_executable(&plist2).is_none());
        std::fs::remove_dir_all(root).expect("cleanup");
        std::fs::remove_dir_all(root2).expect("cleanup");
    }

    /// `read_bundle_version` 改成走共用解析后，行为必须与改动前逐字一致。
    #[test]
    fn read_bundle_version_keeps_its_validation_after_sharing_the_parser() {
        let (root, _bundle, plist) = fake_bundle(REAL_PLIST);
        assert_eq!(read_bundle_version(&plist).as_deref(), Some("5.8.4"));
        std::fs::remove_dir_all(root).expect("cleanup");

        // 非法版本仍由 `valid_app_version` 拦截，共用解析不得放行。
        let (root2, _bundle2, plist2) = fake_bundle(
            "<plist><dict><key>CFBundleShortVersionString</key><string>not-a-version</string></dict></plist>",
        );
        assert!(read_bundle_version(&plist2).is_none());
        std::fs::remove_dir_all(root2).expect("cleanup");
    }
}
