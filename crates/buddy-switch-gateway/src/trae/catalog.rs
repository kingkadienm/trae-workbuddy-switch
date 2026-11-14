//! Trae **官方模型目录**：每个模型真实的上下文口径（`dev` / `max`）。
//!
//! ## 为什么需要它（issue #53 三版）
//!
//! 客户端 `state.vscdb` 里那份清单只有 `context_window_size.default`，`max` **恒为 null**
//! （2026-10-19 实测 19 个模型全空）⇒ 想告诉客户端「这个模型在 Max Mode 下能装多少」
//! 没有数据源，只能靠手写常量表 —— 而各模型的上限并不相同（实测 `mimo-v2.6-pro` 是
//! 981504、`glm-5.3` 是 1000000、`Doubao-Seed-2.1-Turbo` 根本没有更高的上限）。
//!
//! 客户端自己选择模型时走的是 `batch_get_detail_param` 配置接口，返回里每个模型都带
//! `context_window_tokens: { dev, max }`：
//! - `dev`：`__dev` 通道**实际可用**的窗口（网关卡用的就是这条通道）；
//! - `max`：Max Mode 下的上限（可能不存在）。
//!
//! 本模块重放该接口（与参考实现同一套 header / body），把结果落成一份**紧凑缓存**
//! （只留 id / dev / max），供 `/v1/models` 与出站注入使用。
//!
//! ## 缓存与失败
//!
//! - TTL 24h；`ensure_fresh()` 过期才联网，失败**静默沿用旧缓存**（网关不因目录拉不到而拒服务）。
//! - 拉不到 ⇒ 调用方回落到客户端清单的 `default`（宁缺毋滥，不虚报窗口）。
//! - 缓存文件 `~/.buddy-switch/trae/model_catalog.<variant>.json`。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use buddy_switch_core::modules::trae::{account, device, paths, variant::TraeVariant};

use super::{TRAE_APP_ID, TRAE_IDE_VERSION, TRAE_IDE_VERSION_CODE};

/// 上游目录接口（客户端模型选择器用的那一支）。
pub const CATALOG_ENDPOINT: &str =
    "https://api5-normal.mchost.guru/api/ide/v1/batch_get_detail_param";

/// 目录缓存有效期：24 小时（与「模型清单来源」里 `cached` 的语义一致）。
pub const CATALOG_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// 一次请求最多试几个账号（与参考实现一致）。
const MAX_ACCOUNTS: usize = 3;

/// 单个模型的上下文口径（**只留声明所需的两个数**）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// 上游 `config_name`（= 客户端清单里的模型名）。
    pub id: String,
    /// `context_window_tokens.dev`：`__dev` 通道实际窗口。
    pub dev: Option<i64>,
    /// `context_window_tokens.max`：Max Mode 上限（可能不存在）。
    pub max: Option<i64>,
}

/// 一个程序位的目录快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub fetched_at: i64,
    pub entries: Vec<CatalogEntry>,
}

impl ModelCatalog {
    fn into_map(self) -> HashMap<String, (Option<i64>, Option<i64>)> {
        self.entries
            .into_iter()
            .map(|entry| (entry.id.to_lowercase(), (entry.dev, entry.max)))
            .collect()
    }
}

/// 进程内缓存：variant key → （模型名小写 → (dev, max)）。
type CatalogMap = HashMap<String, (Option<i64>, Option<i64>)>;

static MEMORY: OnceLock<Mutex<HashMap<String, Arc<CatalogMap>>>> = OnceLock::new();

fn memory() -> &'static Mutex<HashMap<String, Arc<CatalogMap>>> {
    MEMORY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 正在同步的 variant（单飞：多个入口同时发现过期时只发一次请求）。
static SYNCING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn syncing() -> &'static Mutex<HashSet<String>> {
    SYNCING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 缓存与内存的键：**区域**（同一区域的两个程序位共用同一份目录 ——
/// 目录请求本身与程序位无关，按 variant 分会白拉两次）。
fn key(variant: TraeVariant) -> String {
    variant.region().as_str().to_string()
}

fn cache_file(variant: TraeVariant) -> std::path::PathBuf {
    paths::trae_dir().join(format!("model_catalog.{}.json", key(variant)))
}

/// 读磁盘缓存（不判断 TTL；解析失败当作没有）。
fn load_file(variant: TraeVariant) -> Option<ModelCatalog> {
    let text = std::fs::read_to_string(cache_file(variant)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 把一份目录装进进程内缓存（同步函数，供 `context_for` 热路径用）。
fn install(variant: TraeVariant, catalog: &ModelCatalog) {
    let map: Arc<CatalogMap> = Arc::new(catalog.clone().into_map());
    if let Ok(mut guard) = memory().lock() {
        guard.insert(key(variant), map);
    }
}

/// 已经装过（或尝试装过）的 variant —— 避免每次请求都去读文件。
static LOADED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn loaded() -> &'static Mutex<HashSet<String>> {
    LOADED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 某个模型的 `(dev, max)`（**同步**，供出站注入与清单构造使用）。
///
/// 首次调用才会读磁盘；之后走进程内缓存。目录尚未同步 ⇒ `None`（调用方回落默认窗口）。
pub fn context_for(variant: TraeVariant, model: &str) -> Option<(Option<i64>, Option<i64>)> {
    let key = key(variant);
    let first = loaded()
        .lock()
        .map(|mut set| set.insert(key.clone()))
        .unwrap_or(false);
    if first {
        if let Some(catalog) = load_file(variant) {
            install(variant, &catalog);
        }
    }
    let guard = memory().lock().ok()?;
    let map = guard.get(&key)?;
    map.get(&model.trim().to_lowercase()).copied()
}

/// 目录是否「够新」（没有文件 / 解析失败 / 过期都算旧）。
pub fn is_fresh(variant: TraeVariant) -> bool {
    load_file(variant)
        .map(|catalog| {
            buddy_switch_core::modules::config::now_ms() - catalog.fetched_at < CATALOG_TTL_MS
        })
        .unwrap_or(false)
}

/// 过期才同步；失败**静默**（保留旧缓存）。多处并发调用时只有一处真的发请求。
pub async fn ensure_fresh(variant: TraeVariant) {
    if is_fresh(variant) {
        return;
    }
    let key = key(variant);
    {
        let Ok(mut set) = syncing().lock() else {
            return;
        };
        if !set.insert(key.clone()) {
            return; // 已有请求在飞
        }
    }
    let _ = sync(variant).await;
    if let Ok(mut set) = syncing().lock() {
        set.remove(&key);
    }
}

/// 强制同步一次（不判断 TTL）。成功 ⇒ 落盘 + 刷新内存。
pub async fn sync(variant: TraeVariant) -> Result<ModelCatalog, String> {
    let text = request_catalog(variant).await?;
    let catalog = parse(&text)?;
    if catalog.entries.is_empty() {
        return Err("官方目录没有返回任何模型".to_string());
    }
    if let Some(parent) = cache_file(variant).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&catalog) {
        let _ = buddy_switch_core::modules::config::atomic_write(&cache_file(variant), &json);
    }
    install(variant, &catalog);
    Ok(catalog)
}

/// 发一次目录请求（带一个可用账号；失败换下一个）。
async fn request_catalog(variant: TraeVariant) -> Result<String, String> {
    let candidates: Vec<(String, String, String)> = account::entries_for_region(variant.region())
        .into_iter()
        .filter(|(_, raw)| !raw.jwt.trim().is_empty())
        .take(MAX_ACCOUNTS)
        .map(|(uid, raw)| {
            let jwt = raw
                .jwt
                .trim()
                .strip_prefix("Cloud-IDE-JWT ")
                .unwrap_or(raw.jwt.trim())
                .to_string();
            let device = device::derive(&uid);
            let machine = device::rand_hex_salted(64, "mach", Some(&uid));
            (jwt, device.device_id, machine)
        })
        .collect();
    if candidates.is_empty() {
        return Err("没有可用账号（缺少 JWT）".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let body = json!({
        "functions": [
            "builder", "builder_v3", "chat_v3", "code_reviewer",
            "code_review_summary", "refactor", "solo_agent"
        ],
        "agent_type": "solo_agent",
        "current_config_info": { "config_name": "", "is_custom_model": false },
        "mode_type": 0,
        "access_type": 0,
        "ab_force_vids": "",
        "ab_autotest_advanced_mode": 0,
        "show_custom_model": true,
    });

    let mut last = String::new();
    for (jwt, device_id, machine_id) in candidates {
        let response = client
            .post(CATALOG_ENDPOINT)
            .header("Content-Type", "application/json")
            .header("Request-Traffic-Type", "prod")
            .header("User-Agent", "TraeClient/TTNet")
            .header("x-app-id", TRAE_APP_ID)
            .header("x-app-version", "default")
            .header("x-app-version-code", TRAE_IDE_VERSION_CODE)
            .header("x-bridge-transport", "aha")
            .header("x-device-brand", "CREFG-XX")
            .header("x-device-cpu", "Intel")
            .header("x-device-id", &device_id)
            .header("x-device-type", "windows")
            .header("x-ide-token", &jwt)
            .header("x-ide-version", TRAE_IDE_VERSION)
            .header("x-ide-version-code", TRAE_IDE_VERSION_CODE)
            .header("x-ide-version-type", "stable")
            .header("x-lgw-req-sdk-type", "3")
            .header("x-machine-id", &machine_id)
            .header("x-os-version", "Windows 11 Home China")
            .header("package-type", "stable_cn")
            .header("x-lscbd-aid", "787976")
            .header("x-lscbd-platform", "windows")
            .header("app-version", TRAE_IDE_VERSION)
            .header("x-ss-dp", "787976")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(response) if response.status().is_success() => match response.text().await {
                Ok(text) => return Ok(text),
                Err(error) => last = format!("读取响应失败: {error}"),
            },
            Ok(response) => {
                let status = response.status().as_u16();
                let text = response.text().await.unwrap_or_default();
                last = format!(
                    "HTTP {status}: {}",
                    text.chars().take(120).collect::<String>()
                );
            }
            Err(error) => last = format!("请求失败: {error}"),
        }
    }
    Err(last)
}

/// 解析 `batch_get_detail_param` 响应。
///
/// 每个模型会在多个 `function` 视图里重复出现，且数值可能不同（`chat_v3` 的 `dev` 是
/// 116k、`solo_agent` 是 200k）。我们只关心「这个模型最多能装多少」⇒ 逐字段取
/// **最大值**，避免被某个子系统视图（`summary` / `fast_apply` 等）拉低。
///
/// 只保留 `id / dev / max`：这份缓存要长期落盘，不复制展示名与倍率（那是清单的事）。
pub fn parse(text: &str) -> Result<ModelCatalog, String> {
    let root: Value = serde_json::from_str(text).map_err(|error| format!("解析失败: {error}"))?;
    let configs = root
        .get("function_configs")
        .and_then(Value::as_array)
        .ok_or_else(|| "响应缺少 function_configs".to_string())?;

    let mut merged: HashMap<String, CatalogEntry> = HashMap::new();
    for config in configs {
        let Some(items) = config.get("config_info_list").and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let id = item
                .get("config_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            // 自定义 / 内部条目不进对外清单；与它们同名的概率也极低。
            if id.is_empty() || id.starts_with("custom_") || id.contains(' ') {
                continue;
            }
            let ctx = item.get("context_window_tokens");
            let dev = ctx.and_then(|c| c.get("dev")).and_then(as_i64);
            let max = ctx.and_then(|c| c.get("max")).and_then(as_i64);
            if dev.is_none() && max.is_none() {
                continue;
            }
            let entry = merged
                .entry(id.to_lowercase())
                .or_insert_with(|| CatalogEntry {
                    id: id.to_string(),
                    dev: None,
                    max: None,
                });
            entry.dev = pick_larger(entry.dev, dev);
            entry.max = pick_larger(entry.max, max);
        }
    }

    let mut entries: Vec<CatalogEntry> = merged.into_values().collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(ModelCatalog {
        fetched_at: buddy_switch_core::modules::config::now_ms(),
        entries,
    })
}

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn pick_larger(current: Option<i64>, next: Option<i64>) -> Option<i64> {
    match (current, next) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

/// 测试用：把一份目录直接装进进程内缓存（不联网、不落盘）。
#[cfg(test)]
pub(crate) fn install_for_test(
    variant: TraeVariant,
    entries: Vec<(&str, Option<i64>, Option<i64>)>,
) {
    let catalog = ModelCatalog {
        fetched_at: buddy_switch_core::modules::config::now_ms(),
        entries: entries
            .into_iter()
            .map(|(id, dev, max)| CatalogEntry {
                id: id.to_string(),
                dev,
                max,
            })
            .collect(),
    };
    install(variant, &catalog);
    // 标记「已加载」，否则 `context_for` 首次调用会去读磁盘把测试数据覆盖掉。
    if let Ok(mut set) = loaded().lock() {
        set.insert(key(variant));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "function_configs": [
        { "function": "chat_v3", "config_info_list": [
          { "config_name": "glm-5.3", "context_window_tokens": { "dev": 116000, "max": 1000000 } },
          { "config_name": "kimi-k2.7-code", "context_window_tokens": { "dev": 200000 } },
          { "config_name": "custom_model_1M", "context_window_tokens": { "dev": 1000000 } },
          { "config_name": "summary", "context_window_tokens": { "dev": 256000 } }
        ]},
        { "function": "solo_agent", "config_info_list": [
          { "config_name": "glm-5.3", "context_window_tokens": { "dev": 200000, "max": 1000000 } },
          { "config_name": "mimo-v2.6-pro", "context_window_tokens": { "dev": 200000, "max": 981504 } },
          { "config_name": "Doubao-Seed-2.1-Turbo", "context_window_tokens": { "dev": 256000, "max": 256000 } }
        ]}
      ]
    }"#;

    /// 逐字段取最大值：`glm-5.3` 的 dev 在 `chat_v3` 里是 116k、`solo_agent` 里是 200k
    /// ⇒ 取 200k；max 保持 1M。
    #[test]
    fn parse_keeps_per_model_max_and_the_larger_dev() {
        let catalog = parse(FIXTURE).expect("fixture 必须可解析");
        let by_id: HashMap<&str, &CatalogEntry> = catalog
            .entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry))
            .collect();

        let glm = by_id.get("glm-5.3").expect("glm-5.3 必须在册");
        assert_eq!(glm.dev, Some(200_000));
        assert_eq!(glm.max, Some(1_000_000));

        // 上限**不是**统一 1M：MiMo 是 981504，Doubao Turbo 与 dev 相同。
        assert_eq!(by_id["mimo-v2.6-pro"].max, Some(981_504));
        assert_eq!(by_id["Doubao-Seed-2.1-Turbo"].max, Some(256_000));

        // 没有 max 的模型只有 dev；自定义条目被剔除。
        assert_eq!(by_id["kimi-k2.7-code"].max, None);
        assert!(!by_id.contains_key("custom_model_1m"));

        // 内部小视图（summary）也会入册（同名字的模型会取大值），但不影响对外清单 ——
        // 清单以客户端清单的 id 为准，这里只是上下文查找表。
        assert_eq!(by_id["summary"].dev, Some(256_000));
    }

    /// 数字字符串也要认（上游偶有 `"1000000"` 这种形态）。
    #[test]
    fn parse_accepts_numeric_strings() {
        let text = r#"{"function_configs":[{"config_info_list":[
            {"config_name":"x","context_window_tokens":{"dev":"200000","max":"1000000"}}
        ]}]}"#;
        let catalog = parse(text).expect("合法 JSON");
        assert_eq!(catalog.entries[0].dev, Some(200_000));
        assert_eq!(catalog.entries[0].max, Some(1_000_000));
    }

    /// 缺 `function_configs` ⇒ 明确报错（不静默产出空目录）。
    #[test]
    fn parse_rejects_unexpected_shape() {
        assert!(parse(r#"{"ok":true}"#).is_err());
        assert!(parse("not json").is_err());
    }
}
