//! Trae 模块设置。
//!
//! 字段**统一使用 camelCase**（含持久化文件 `trae/settings.json`）。
//!
//! 与账号库（`checkin_accounts.json`，刻意保持参考实现的 `UserID` 等键名以兼容既有
//! 数据）不同，设置文件里装的是端口、客户端路径、日志保留天数这类**本机专有**配置，
//! 换一台机器就没有迁移价值，因此不值得为「与参考实现同名」付出
//! 「磁盘一套命名、响应体另一套命名」的双形态维护成本。
//!
//! 结果：本模块的设置对象在磁盘、Tauri 响应、HTTP 响应三处同名，
//! 前端 `TraeSettings` 类型可直接映射，无需任何字段转换。
//!
//! 新增字段一律给 `#[serde(default = "...")]`：字段缺失会回落默认值而非报错，
//! 使旧版本配置文件在升级后仍可读。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::region::TraeRegion;
use crate::modules::trae::{paths, store, TRAE_DEFAULT_API_PORT, TRAE_DEFAULT_PROXY_PORT};

/// Trae 模块设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraeSettings {
    /// 本地代理监听端口。
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,
    /// 主题：`system` / `light` / `dark`。
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 启动时最小化到托盘。
    #[serde(default)]
    pub launch_minimized: bool,
    /// 应用启动时自动开启代理。
    #[serde(default = "default_true")]
    pub auto_start_proxy: bool,
    /// 启用托盘图标。
    #[serde(default = "default_true")]
    pub tray: bool,
    /// 界面语言。
    #[serde(default = "default_language")]
    pub language: String,
    /// 批量签到时跳过今日已签到账号。
    #[serde(default = "default_true")]
    pub checkin_skip_checked: bool,
    /// 批量签到时跳过 JWT 已过期账号。
    #[serde(default = "default_true")]
    pub checkin_skip_expired: bool,
    /// 签到网络异常时的重试次数。
    #[serde(default = "default_retry")]
    pub retry: i32,
    /// 通知方式：`toast` / `system` / `none`。
    #[serde(default = "default_notify")]
    pub notify: String,
    /// 用户指定的 Trae 客户端可执行文件路径（覆盖自动探测），**按区域分家**（issue #20）。
    ///
    /// 键 = 区域稳定标识（[`TraeRegion::as_str`]：`"cn"` / `"global"`），值 = exe 绝对路径。
    /// 缺失的键 = 该区域未指定 ⇒ 回落自动探测。
    ///
    /// ## 为什么必须按区域分家（用户报障原话）
    ///
    /// 「设置Trae 路径国内版和国际版 两个都设置是一样的，以最后的设置为准了，没有隔离开来」。
    ///
    /// 国内版与国际版是**两个物理上不同的客户端**（`Trae CN.exe` / `Trae.exe`，
    /// 各有独立安装目录），单个全局字段必然让后设置的那条覆盖前一条。
    /// 用 [`BTreeMap`] 而非 `HashMap`：落盘键序稳定 ⇒ 同一份设置两次序列化字节相同
    /// （便于 diff 与「设置未变」判定），与仓库其它落盘结构一致。
    #[serde(default)]
    pub trae_paths: BTreeMap<String, String>,
    /// 用户指定的 Trae 客户端可执行文件路径（**旧版单值字段：兼容读，不得删除**）。
    ///
    /// ## 为什么保留（与 [`Self::api_key`] 同一类处理）
    ///
    /// 旧版 `settings.json` 只存了这一个值，且**无法区分区域**。删掉本字段会让老用户在
    /// 升级后**静默丢掉已配置的路径**（表现为「明明设置过，升级后却说没设置」）。
    ///
    /// [`load`] 会把它**单向迁移**进 [`Self::trae_paths`]（区域按 exe 名反推，推不出落
    /// 国内版）并就地清空；下一次 `patch()` 写回时把它落盘成 `null`。
    /// 因此本字段**只被读、不再被写路径更新**。
    #[serde(default)]
    pub trae_path: Option<String>,
    /// 用户指定的浏览器可执行文件路径（浏览器提取 JWT 用）。
    #[serde(default)]
    pub browser_path: Option<String>,
    /// 日志保留天数。
    #[serde(default = "default_log_retention_days")]
    pub log_retention_days: i64,
    /// 代理需要解密（MITM）的域名白名单，逗号分隔。
    #[serde(default = "default_proxy_domains")]
    pub proxy_domains: String,
    /// API 网关监听端口。
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    /// API 网关 Bearer Token；留空则不鉴权。
    ///
    /// ## ★ 多 Key 化后本字段降级为「**兼容读**」，**不得删除**
    ///
    /// 网关的 API Key 已迁移到独立的 `trae/api_gateway_keys.json`（多 Key + 归属产品线，
    /// 见 `buddy_switch_gateway::trae::apikey`）。但**旧用户**的 Key 只存在这里：
    /// `TraeApiKeyStore::load()` 在 Key 库缺失/损坏时会回落本字段，合成一条
    /// `variant = TraeWork`（= `TraeVariant::default()`）的 legacy 记录，从而让
    /// 「升级前能用的 Key，升级后仍然能用」（R1：多 Key 迁移失败 → 全量 401）。
    ///
    /// 本字段**原地保留、不清理**：删掉它会让老用户在升级后立刻 401。
    /// 它不再被任何写路径更新（旧 `patch({apiKey})` 已随 `regenerate_api_key` 一并删除）。
    #[serde(default)]
    pub api_key: String,
    /// API 网关默认模型。
    #[serde(default = "default_api_model")]
    pub api_default_model: String,
}

fn default_proxy_port() -> u16 {
    TRAE_DEFAULT_PROXY_PORT
}

fn default_api_port() -> u16 {
    TRAE_DEFAULT_API_PORT
}

fn default_api_model() -> String {
    "deepseek-v4-flash".into()
}

fn default_theme() -> String {
    "system".into()
}

fn default_true() -> bool {
    true
}

fn default_language() -> String {
    "zh-CN".into()
}

fn default_retry() -> i32 {
    1
}

fn default_notify() -> String {
    "toast".into()
}

fn default_log_retention_days() -> i64 {
    30
}

/// 代理默认解密域名白名单。
///
/// 只解密这些域名才能捕获 Trae 的登录态；其余流量走隧道直连（见第 C 组代理实现），
/// 避免把用户所有 HTTPS 流量都置于 MITM 之下。
pub fn default_proxy_domains() -> String {
    "trae.cn,trae.com.cn,mchost.guru,zijieapi.com,bytedance.com,volcengine.com,volces.com,treecode.com"
        .into()
}

impl Default for TraeSettings {
    fn default() -> Self {
        Self {
            proxy_port: default_proxy_port(),
            theme: default_theme(),
            launch_minimized: false,
            auto_start_proxy: default_true(),
            tray: default_true(),
            language: default_language(),
            checkin_skip_checked: default_true(),
            checkin_skip_expired: default_true(),
            retry: default_retry(),
            notify: default_notify(),
            trae_paths: BTreeMap::new(),
            trae_path: None,
            browser_path: None,
            log_retention_days: default_log_retention_days(),
            proxy_domains: default_proxy_domains(),
            api_port: default_api_port(),
            api_key: String::new(),
            api_default_model: default_api_model(),
        }
    }
}

impl TraeSettings {
    /// 代理白名单解析为小写域名列表（忽略空白项）。
    ///
    /// 匹配时按「等于该域名或是其子域」判断，因此列表里写 `trae.cn` 即可覆盖
    /// `api.trae.cn`。统一小写避免大小写差异导致漏匹配。
    pub fn proxy_domain_list(&self) -> Vec<String> {
        self.proxy_domains
            .split(',')
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| !part.is_empty())
            .collect()
    }

    /// 判断某主机是否属于需要 MITM 解密的域名。
    pub fn should_intercept(&self, host: &str) -> bool {
        let host = host
            .split(':')
            .next()
            .unwrap_or(host)
            .trim()
            .to_ascii_lowercase();
        if host.is_empty() {
            return false;
        }
        self.proxy_domain_list().iter().any(|domain| {
            host == *domain
                || (host.len() > domain.len()
                    && host.ends_with(domain.as_str())
                    && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
        })
    }
}

impl TraeSettings {
    /// 把旧版单值 `traePath` **单向迁移**进按区域分家的 [`Self::trae_paths`]（幂等）。
    ///
    /// 区域判据 = exe 文件名经 [`variant_of_name`] 反推出的变体的区域；反推不出时落
    /// **国内版**（与 [`TraeRegion::default`] 一致 —— 升级前默认变体 `trae_work` 也是国内）。
    ///
    /// 迁移后**就地清空** `trae_path`：它已是死字段，留着只会让「设置页显示哪个值」
    /// 出现第二个来源。清空后由 `patch()` 的下一次写回落盘。
    ///
    /// `or_insert` 而非 `insert`：若目标区域**已有**显式配置（用户在新版里设过），
    /// 旧值不得覆盖新值 —— 迁移只补空缺。
    fn migrate_legacy_trae_path(&mut self) {
        let Some(legacy) = self
            .trae_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_string)
        else {
            return;
        };
        let region = legacy_region(&legacy);
        self.trae_paths
            .entry(region.as_str().to_string())
            .or_insert(legacy);
        self.trae_path = None;
    }

    /// 取某区域配置的客户端路径（已去空白；空串视为未配置）。
    pub fn path_for(&self, region: TraeRegion) -> Option<&str> {
        self.trae_paths
            .get(region.as_str())
            .map(String::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
    }

    /// 全部**已配置**路径（按区域稳定顺序：`cn` 在 `global` 前）。
    ///
    /// 供**跨区域**探测使用（[`super::platform::env_status`] / `detected_variant`）——
    /// 那两处问的是「本机装的是哪一条线」，不能只看单一区域。
    pub fn all_paths(&self) -> Vec<String> {
        TraeRegion::all()
            .into_iter()
            .filter_map(|region| self.path_for(region).map(str::to_string))
            .collect()
    }

    /// 按区域**就地**写入/清除客户端路径（`None` 或空白 = 清除该区域）。
    ///
    /// 设置页走这里而非直接改 `trae_paths`：它保证「清除」与「写入」不会留下空串键
    /// （空串键会在 `path_for` 里被过滤成「未配置」，但会让落盘文件出现无意义条目）。
    pub fn set_path_for(&mut self, region: TraeRegion, path: Option<&str>) {
        let key = region.as_str().to_string();
        match path.map(str::trim).filter(|path| !path.is_empty()) {
            Some(path) => {
                self.trae_paths.insert(key, path.to_string());
            }
            None => {
                self.trae_paths.remove(&key);
            }
        }
    }
}

/// 从 exe 路径反推它属于哪个区域（迁移用）。
///
/// 反推不出（文件名不认识）时落国内版：旧版单值字段的用户绝大多数是国内版，
/// 且国际版用户重设一次路径的成本远低于国内版用户升级后「配置凭空消失」。
fn legacy_region(path: &str) -> TraeRegion {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(super::variant::variant_of_name)
        .map(super::variant::TraeVariant::region)
        .unwrap_or(TraeRegion::Cn)
}

/// 读取设置；文件缺失或损坏时回落全默认值。
pub fn load() -> TraeSettings {
    let mut settings: TraeSettings = store::read_json(&paths::settings_file());
    // 白名单为空时回填默认值，避免设置页展示空白、且代理退化为「什么都不解密」。
    if settings.proxy_domains.trim().is_empty() {
        settings.proxy_domains = default_proxy_domains();
    }
    settings.migrate_legacy_trae_path();
    settings
}

/// 全量写入设置。
pub fn save(settings: &TraeSettings) -> Result<(), String> {
    store::write_json(&paths::settings_file(), settings)
}

/// 局部更新：仅覆盖 `patch` 中出现的字段，其余保持原值。
///
/// 走 JSON 中间层而非直接反序列化成 `TraeSettings`，是为了区分
/// 「字段未提供」与「字段显式设为 null」——后者用于清空 `trae_path` / `browser_path`
/// 这类可选设置。前端只需提交要改的键。
pub fn patch(patch: serde_json::Value) -> Result<TraeSettings, String> {
    let mut current = serde_json::to_value(load()).map_err(|e| e.to_string())?;
    let (Some(base), Some(delta)) = (current.as_object_mut(), patch.as_object()) else {
        return Err("设置必须是 JSON 对象".into());
    };
    for (key, value) in delta {
        base.insert(key.clone(), value.clone());
    }
    let merged: TraeSettings =
        serde_json::from_value(current).map_err(|e| format!("设置字段无效: {e}"))?;
    save(&merged)?;
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_trae_ports_not_workbuddy_ports() {
        let settings = TraeSettings::default();
        assert_eq!(settings.proxy_port, TRAE_DEFAULT_PROXY_PORT);
        assert_eq!(settings.api_port, TRAE_DEFAULT_API_PORT);
        // WorkBuddy 网关默认 57891，两者不能相同，否则同时启用会抢端口。
        assert_ne!(settings.api_port, 57891);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // 模拟旧版本配置文件：只含极少数键，其余必须自动补默认值。
        let partial = serde_json::json!({ "proxyPort": 18899 });
        let settings: TraeSettings = serde_json::from_value(partial).unwrap();
        assert_eq!(settings.proxy_port, 18899);
        assert_eq!(settings.api_port, TRAE_DEFAULT_API_PORT);
        assert!(settings.auto_start_proxy);
        assert_eq!(settings.retry, 1);
        assert!(!settings.proxy_domains.is_empty());
    }

    #[test]
    fn unknown_fields_are_ignored_not_fatal() {
        let value = serde_json::json!({ "proxyPort": 1, "legacy_field": "x" });
        assert!(serde_json::from_value::<TraeSettings>(value).is_ok());
    }

    #[test]
    fn settings_use_camel_case_on_disk_and_on_wire() {
        // 关键护栏：磁盘、Tauri 响应、HTTP 响应必须是同一套字段名。
        let settings = TraeSettings::default();
        let wire = serde_json::to_value(&settings).unwrap();
        for key in [
            "proxyPort",
            "autoStartProxy",
            "traePath",
            "traePaths",
            "proxyDomains",
            "apiPort",
            "apiKey",
        ] {
            assert!(wire.get(key).is_some(), "缺少 camelCase 字段 {key}");
        }
        // snake_case 不得出现（会让前端 TraeSettings 类型映射成 undefined）
        assert!(wire.get("proxy_port").is_none());
        assert!(wire.get("auto_start_proxy").is_none());
        // snake_case 输入必须被当成未知字段而非可识别的配置
        let from_snake: TraeSettings =
            serde_json::from_value(serde_json::json!({ "proxy_port": 1234 })).unwrap();
        assert_eq!(from_snake.proxy_port, TRAE_DEFAULT_PROXY_PORT);
    }

    #[test]
    fn patch_accepts_camel_case_and_preserves_untouched_fields() {
        // 走真实文件系统会污染 ~/.buddy-switch，这里只验证「JSON 合并 + 反序列化」规则本身。
        let current = serde_json::to_value(TraeSettings::default()).unwrap();
        let mut base = current.as_object().unwrap().clone();
        for (key, value) in serde_json::json!({ "proxyPort": 19999, "notify": "none" })
            .as_object()
            .unwrap()
        {
            base.insert(key.clone(), value.clone());
        }
        let merged: TraeSettings = serde_json::from_value(serde_json::Value::Object(base)).unwrap();
        assert_eq!(merged.proxy_port, 19999);
        assert_eq!(merged.notify, "none");
        // 未提交的字段保持原值
        assert_eq!(merged.api_port, TRAE_DEFAULT_API_PORT);
        assert!(merged.auto_start_proxy);
    }

    #[test]
    fn proxy_domain_list_trims_and_lowercases() {
        let settings = TraeSettings {
            proxy_domains: " Trae.CN , , api.trae.com.cn ,".into(),
            ..Default::default()
        };
        assert_eq!(
            settings.proxy_domain_list(),
            vec!["trae.cn", "api.trae.com.cn"]
        );
    }

    #[test]
    fn should_intercept_matches_domain_and_subdomains_only() {
        let settings = TraeSettings {
            proxy_domains: "trae.cn,trae.com.cn".into(),
            ..Default::default()
        };
        // 精确域名与子域命中
        assert!(settings.should_intercept("trae.cn"));
        assert!(settings.should_intercept("api.trae.cn"));
        assert!(settings.should_intercept("a.b.trae.cn"));
        // 带端口
        assert!(settings.should_intercept("api.trae.cn:443"));
        // 大小写不敏感
        assert!(settings.should_intercept("API.TRAE.CN"));
        // 关键反例：不能因为后缀相同就误命中（nottrae.cn / eviltrae.cn）
        assert!(!settings.should_intercept("nottrae.cn"));
        assert!(!settings.should_intercept("eviltrae.cn"));
        assert!(!settings.should_intercept("example.com"));
        assert!(!settings.should_intercept(""));
    }

    #[test]
    fn empty_proxy_domains_intercepts_nothing() {
        let settings = TraeSettings {
            proxy_domains: String::new(),
            ..Default::default()
        };
        assert!(settings.proxy_domain_list().is_empty());
        assert!(!settings.should_intercept("api.trae.cn"));
    }

    // -----------------------------------------------------------------------
    // 按区域分家的客户端路径（issue #20）
    // -----------------------------------------------------------------------

    /// ★ 区域必须**互相隔离**：给一个区域设路径不得影响另一个。
    ///
    /// 用户报障原话：「设置Trae 路径国内版和国际版 两个都设置是一样的，
    /// 以最后的设置为准了，没有隔离开来」。这条就是那句话的回归护栏 ——
    /// 若退回单个全局字段，本用例必红。
    #[test]
    fn trae_paths_are_isolated_per_region() {
        let mut settings = TraeSettings::default();
        settings.set_path_for(TraeRegion::Cn, Some(r"D:\A\Trae CN.exe"));

        assert_eq!(settings.path_for(TraeRegion::Cn), Some(r"D:\A\Trae CN.exe"));
        assert_eq!(
            settings.path_for(TraeRegion::Global),
            None,
            "★ 给国内版设路径不得让国际版也变成已配置"
        );

        // 再设国际版：国内版的值必须**原样保留**（这正是旧实现的缺陷所在）。
        settings.set_path_for(TraeRegion::Global, Some(r"D:\B\Trae.exe"));
        assert_eq!(settings.path_for(TraeRegion::Cn), Some(r"D:\A\Trae CN.exe"));
        assert_eq!(
            settings.path_for(TraeRegion::Global),
            Some(r"D:\B\Trae.exe")
        );

        // 清除一个区域不影响另一个。
        settings.set_path_for(TraeRegion::Cn, None);
        assert_eq!(settings.path_for(TraeRegion::Cn), None);
        assert_eq!(
            settings.path_for(TraeRegion::Global),
            Some(r"D:\B\Trae.exe")
        );
    }

    /// 空白路径视为「未配置」；`set_path_for` 不得留下空串键。
    #[test]
    fn blank_path_is_treated_as_unset() {
        let mut settings = TraeSettings::default();
        settings.set_path_for(TraeRegion::Cn, Some("   "));
        assert_eq!(settings.path_for(TraeRegion::Cn), None);
        assert!(
            settings.trae_paths.is_empty(),
            "空白路径不得留下空串键（会让落盘文件出现无意义条目）"
        );

        // 手工塞一个空串（模拟外部写坏的文件）也必须被 `path_for` 过滤。
        settings
            .trae_paths
            .insert(TraeRegion::Cn.as_str().to_string(), "  ".into());
        assert_eq!(settings.path_for(TraeRegion::Cn), None);
    }

    /// `all_paths` 按**区域稳定顺序**（`cn` 在 `global` 前）返回已配置项。
    #[test]
    fn all_paths_is_region_ordered_and_skips_unset() {
        let mut settings = TraeSettings::default();
        settings.set_path_for(TraeRegion::Global, Some(r"D:\B\Trae.exe"));
        settings.set_path_for(TraeRegion::Cn, Some(r"D:\A\Trae CN.exe"));

        assert_eq!(
            settings.all_paths(),
            vec![
                r"D:\A\Trae CN.exe".to_string(),
                r"D:\B\Trae.exe".to_string()
            ],
            "顺序必须稳定为 cn → global（BTreeMap 落盘键序也据此）"
        );

        settings.set_path_for(TraeRegion::Cn, None);
        assert_eq!(settings.all_paths(), vec![r"D:\B\Trae.exe".to_string()]);
    }

    /// 旧版单值 `traePath` 必须**单向迁移**进按区域字段，且区域按 exe 名反推。
    #[test]
    fn legacy_trae_path_migrates_into_its_region() {
        // 路径**统一用正斜杠**：盘符路径只是字符串，`Path::file_name()` 在 Unix 上
        // 不按 `\` 切分（整串会被当成文件名，`variant_of_name` 认不出 ⇒ 落默认区域），
        // 断言就会在 macOS / Linux 上红（issue #37 的第 6 条）。正斜杠三平台都能切。
        // 国际版 exe（`Trae.exe` 精确命中国际 TraeCode）⇒ 落 global。
        let mut settings = TraeSettings {
            trae_path: Some("D:/Apps/Trae/Trae.exe".into()),
            ..TraeSettings::default()
        };
        settings.migrate_legacy_trae_path();
        assert_eq!(
            settings.path_for(TraeRegion::Global),
            Some("D:/Apps/Trae/Trae.exe"),
            "`Trae.exe` 属于国际版，迁移必须落 global"
        );
        assert_eq!(settings.path_for(TraeRegion::Cn), None);
        assert_eq!(settings.trae_path, None, "迁移后旧字段必须就地清空");

        // 国内版 exe ⇒ 落 cn。
        let mut settings = TraeSettings {
            trae_path: Some("D:/Apps/Trae CN/Trae CN.exe".into()),
            ..TraeSettings::default()
        };
        settings.migrate_legacy_trae_path();
        assert_eq!(
            settings.path_for(TraeRegion::Cn),
            Some("D:/Apps/Trae CN/Trae CN.exe")
        );
        assert_eq!(settings.path_for(TraeRegion::Global), None);

        // 名字认不出 ⇒ 落国内版（默认区域），且不得丢值。
        let mut settings = TraeSettings {
            trae_path: Some("D:/Weird/whatever.exe".into()),
            ..TraeSettings::default()
        };
        settings.migrate_legacy_trae_path();
        assert_eq!(
            settings.path_for(TraeRegion::Cn),
            Some("D:/Weird/whatever.exe"),
            "认不出区域时落默认（国内版），绝不能把用户的路径丢掉"
        );
    }

    /// 迁移**只补空缺**：目标区域已有显式配置时，旧值不得覆盖它。
    #[test]
    fn legacy_migration_does_not_clobber_existing_value() {
        let mut settings = TraeSettings {
            trae_path: Some(r"D:\Old\Trae CN.exe".into()),
            ..TraeSettings::default()
        };
        settings.set_path_for(TraeRegion::Cn, Some(r"D:\New\Trae CN.exe"));

        settings.migrate_legacy_trae_path();

        assert_eq!(
            settings.path_for(TraeRegion::Cn),
            Some(r"D:\New\Trae CN.exe"),
            "新版里已显式设置的值优先于旧字段"
        );
    }

    /// 迁移必须**幂等**：跑两次结果相同（`load()` 每次都会跑它）。
    #[test]
    fn legacy_migration_is_idempotent() {
        let mut settings = TraeSettings {
            trae_path: Some(r"D:\Apps\Trae\Trae.exe".into()),
            ..TraeSettings::default()
        };
        settings.migrate_legacy_trae_path();
        let once = settings.clone();
        settings.migrate_legacy_trae_path();
        assert_eq!(settings, once, "迁移必须幂等（load 每次都会调用它）");
    }

    /// 没有旧值时迁移不得凭空造出条目。
    #[test]
    fn legacy_migration_noop_without_legacy_value() {
        let mut settings = TraeSettings::default();
        settings.migrate_legacy_trae_path();
        assert!(settings.trae_paths.is_empty());

        // 空白旧值同样不得造出条目。
        let mut settings = TraeSettings {
            trae_path: Some("   ".into()),
            ..TraeSettings::default()
        };
        settings.migrate_legacy_trae_path();
        assert!(settings.trae_paths.is_empty());
    }
}
