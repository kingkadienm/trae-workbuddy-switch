//! 模型目录：内置兜底目录（CN / Global 两份）+ 目录缓存 store。
//!
//! 对照参考实现 `catalog.ts` + `catalog-store.ts`。
//!
//! 降级链：本次实时（Live）→ 上次成功缓存（Cached）→ 内置兜底（Builtin），
//! 由 [`CatalogSource`] 标注来源。两版的目录缓存**按 region 隔离**（同一模型 id
//! 在两版可能倍率/窗口都不同），落盘到 `~/.buddy-switch/gateway_models.<region>.json`。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use serde_json::Value;

use crate::modules::config::{now_ms, store_dir};
use crate::modules::region::{catalog_cache_file, Region};
use crate::modules::upstream::{parse_model_catalog, UpstreamClient, UpstreamErrorKind, UpstreamFailure};

/// 目录来源标注（P0-4 / L1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogSource {
    /// 本次实时拉取成功。
    Live,
    /// 上次成功缓存的目录。
    Cached,
    /// 内置兜底目录。
    Builtin,
}

/// 一个可用的模型条目。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CatalogModel {
    /// 模型 id。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 上下文窗口。
    pub context_window: u64,
    /// 最大输出 token。
    pub max_tokens: u64,
    /// 是否支持图片输入。
    pub supports_images: bool,
    /// 积分倍率字符串（如 `"x0.79"`），缺省为 None。
    pub credits: Option<String>,
    /// 促销徽标。
    pub badges: Vec<String>,
    /// 当前是否免费。
    pub free: bool,
}

/// 一次目录快照。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CatalogSnapshot {
    /// 所属 region。
    pub region: Region,
    /// 来源。
    pub source: CatalogSource,
    /// 拉取成功时间（毫秒）；Builtin 为 None。
    pub fetched_at: Option<i64>,
    /// 模型列表。
    pub models: Vec<CatalogModel>,
    /// 降级说明（如「上游接口可能已变更」）。
    pub note: Option<String>,
}

/// 落盘格式。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedCatalog {
    region: Region,
    fetched_at: i64,
    models: Vec<CatalogModel>,
}

const DEGRADE_NOTE: &str = "上游接口可能已变更，当前展示的是上次成功或内置兜底目录";

/// CN 内置兜底目录（对照参考实现 `FALLBACK_WORKBUDDY_MODELS`，16 条）。
///
/// 说明：条目含 `String` / `Vec` 字段，无法在 `const` 上下文中构造（会触发 E0015），
/// 故改用 [`LazyLock`] 惰性构造；对外仍以 `&'static [CatalogModel]` 只读切片暴露，
/// 数据与原先逐字节一致。
pub static FALLBACK_CN_MODELS: LazyLock<Vec<CatalogModel>> = LazyLock::new(|| {
    vec![
        CatalogModel { id: "auto".into(), name: "Auto".into(), context_window: 168_000, max_tokens: 32_000, supports_images: true, credits: None, badges: Vec::new(), free: false },
        CatalogModel { id: "hy4-preview".into(), name: "Hy4 preview".into(), context_window: 1_000_000, max_tokens: 64_000, supports_images: true, credits: None, badges: Vec::new(), free: false },
        CatalogModel { id: "hy3".into(), name: "Hy3".into(), context_window: 192_000, max_tokens: 64_000, supports_images: true, credits: None, badges: Vec::new(), free: false },
        CatalogModel { id: "hy3-x".into(), name: "Hy3".into(), context_window: 192_000, max_tokens: 64_000, supports_images: true, credits: Some("x0.05".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "deepseek-v4.1-flash".into(), name: "Deepseek-V4.1-Flash".into(), context_window: 1_000_000, max_tokens: 128_000, supports_images: true, credits: Some("x0.03 credits".into()), badges: vec!["独家优惠".into()], free: false },
        CatalogModel { id: "glm-5.3".into(), name: "GLM-5.3".into(), context_window: 1_000_000, max_tokens: 48_000, supports_images: true, credits: Some("x0.79".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "glm-5.3-flash".into(), name: "GLM-5.3-Flash".into(), context_window: 1_000_000, max_tokens: 32_000, supports_images: true, credits: Some("x0.06".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "glm-5.2".into(), name: "GLM-5.2".into(), context_window: 1_000_000, max_tokens: 48_000, supports_images: true, credits: Some("x0.79 credits".into()), badges: vec!["夜间折扣".into()], free: false },
        CatalogModel { id: "glm-5.1".into(), name: "GLM-5.1".into(), context_window: 200_000, max_tokens: 48_000, supports_images: false, credits: Some("x0.79 credits".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "glm-5v-turbo".into(), name: "GLM-5v-Turbo".into(), context_window: 200_000, max_tokens: 64_000, supports_images: true, credits: Some("x0.71 credits".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "kimi-k3-1".into(), name: "Kimi-K3".into(), context_window: 1_000_000, max_tokens: 32_000, supports_images: true, credits: Some("x1.62 credits".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "kimi-k2.8-preview".into(), name: "Kimi-K2.8-Preview".into(), context_window: 1_000_000, max_tokens: 32_000, supports_images: true, credits: Some("x0.77 credits".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "kimi-k2.7".into(), name: "Kimi-K2.7-Code".into(), context_window: 256_000, max_tokens: 32_000, supports_images: true, credits: Some("x0.57 credits".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "kimi-k2.6".into(), name: "Kimi-K2.6".into(), context_window: 256_000, max_tokens: 32_000, supports_images: true, credits: Some("x0.52 credits".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "minimax-m3".into(), name: "MiniMax-M3".into(), context_window: 512_000, max_tokens: 128_000, supports_images: true, credits: Some("x0.25 credits".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "deepseek-v4-pro".into(), name: "Deepseek-V4-Pro".into(), context_window: 1_000_000, max_tokens: 50_000, supports_images: true, credits: Some("x0.51 credits".into()), badges: Vec::new(), free: false },
    ]
});

/// Global 内置兜底目录（对照参考实现 `FALLBACK_WORKBUDDY_AI_MODELS`，20 条）。
///
/// 同 CN：条目含 `String` / `Vec` 字段，改用 [`LazyLock`] 惰性构造。
pub static FALLBACK_GLOBAL_MODELS: LazyLock<Vec<CatalogModel>> = LazyLock::new(|| {
    vec![
        CatalogModel { id: "default-model".into(), name: "Auto".into(), context_window: 176_000, max_tokens: 24_000, supports_images: true, credits: None, badges: Vec::new(), free: false },
        CatalogModel { id: "fast-model".into(), name: "Fast".into(), context_window: 200_000, max_tokens: 32_000, supports_images: true, credits: Some("x0.34".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "balanced-model".into(), name: "Balanced".into(), context_window: 256_000, max_tokens: 32_000, supports_images: true, credits: Some("x0.59".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "primary-model".into(), name: "Primary".into(), context_window: 272_000, max_tokens: 72_000, supports_images: true, credits: Some("x3.31".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "deep-model".into(), name: "Deep".into(), context_window: 176_000, max_tokens: 24_000, supports_images: true, credits: Some("x3.33".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "hy4-preview-f".into(), name: "Hy4 preview".into(), context_window: 300_000, max_tokens: 64_000, supports_images: true, credits: None, badges: Vec::new(), free: false },
        CatalogModel { id: "hy3".into(), name: "Hy3".into(), context_window: 192_000, max_tokens: 64_000, supports_images: true, credits: None, badges: Vec::new(), free: false },
        CatalogModel { id: "deepseek-v4.1-flash".into(), name: "Deepseek-V4.1-Flash".into(), context_window: 300_000, max_tokens: 128_000, supports_images: true, credits: None, badges: Vec::new(), free: false },
        CatalogModel { id: "gpt-6-astra".into(), name: "GPT-6-Astra".into(), context_window: 400_000, max_tokens: 128_000, supports_images: true, credits: Some("x6.67".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "gpt-5.6-sol".into(), name: "GPT-5.6-Sol".into(), context_window: 1_000_000, max_tokens: 128_000, supports_images: true, credits: Some("x3.47".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "gpt-5.6-terra".into(), name: "GPT-5.6-Terra".into(), context_window: 1_000_000, max_tokens: 128_000, supports_images: true, credits: Some("x1.39".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "gpt-5.6-luna".into(), name: "GPT-5.6-Luna".into(), context_window: 1_000_000, max_tokens: 128_000, supports_images: true, credits: Some("x0.14".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "gpt-5.5".into(), name: "GPT-5.5".into(), context_window: 1_000_000, max_tokens: 128_000, supports_images: true, credits: Some("x3.31".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "gpt-5.4".into(), name: "GPT-5.4".into(), context_window: 272_000, max_tokens: 72_000, supports_images: true, credits: Some("x1.65".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "gpt-5.3-codex".into(), name: "GPT-5.3-Codex".into(), context_window: 272_000, max_tokens: 72_000, supports_images: true, credits: Some("x1.25".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "gemini-3.5-flash".into(), name: "Gemini-3.5-Flash".into(), context_window: 1_000_000, max_tokens: 65_536, supports_images: true, credits: Some("x0.99".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "glm-5.3".into(), name: "GLM-5.3".into(), context_window: 1_000_000, max_tokens: 48_000, supports_images: true, credits: Some("x0.79".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "glm-5.2".into(), name: "GLM-5.2".into(), context_window: 1_000_000, max_tokens: 48_000, supports_images: true, credits: Some("x0.79".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "kimi-k3".into(), name: "Kimi-K3".into(), context_window: 1_000_000, max_tokens: 32_000, supports_images: true, credits: Some("x1.62".into()), badges: Vec::new(), free: false },
        CatalogModel { id: "kimi-k2.6".into(), name: "Kimi-K2.6".into(), context_window: 256_000, max_tokens: 32_000, supports_images: true, credits: Some("x0.52".into()), badges: Vec::new(), free: false },
    ]
});

/// 目录缓存 store：每 region 一个快照 + 落盘。
pub struct CatalogStore {
    cache_dir: PathBuf,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// 本次进程内实时成功的快照。
    live: HashMap<Region, CatalogSnapshot>,
    /// 从磁盘加载并缓存的快照。
    cached: HashMap<Region, CatalogSnapshot>,
    /// 已尝试从磁盘加载过的 region（避免重复 IO）。
    loaded: HashMap<Region, bool>,
}

impl Default for CatalogStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CatalogStore {
    /// 使用默认缓存目录 `~/.buddy-switch`。
    pub fn new() -> Self {
        Self::with_cache_dir(store_dir())
    }

    /// 使用指定缓存目录（便于测试）。
    pub fn with_cache_dir(dir: PathBuf) -> Self {
        Self {
            cache_dir: dir,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn cache_file(&self, region: Region) -> PathBuf {
        // 默认目录与 `region::catalog_cache_file` 一致；自定义目录时改写文件名。
        if self.cache_dir == store_dir() {
            catalog_cache_file(region)
        } else {
            self.cache_dir
                .join(format!("gateway_models.{}.json", region.as_str()))
        }
    }

    /// 内置兜底目录（按 region 返回对应的 `&'static` 只读切片）。
    pub fn fallback_models(region: Region) -> &'static [CatalogModel] {
        match region {
            Region::Cn => FALLBACK_CN_MODELS.as_slice(),
            Region::Global => FALLBACK_GLOBAL_MODELS.as_slice(),
        }
    }

    /// 当前该 region 的目录快照：live → cached → builtin。
    pub fn current(&self, region: Region) -> CatalogSnapshot {
        if let Some(snapshot) = self.inner.lock().unwrap().live.get(&region) {
            return snapshot.clone();
        }
        if let Some(snapshot) = self.load_cached(region) {
            return snapshot;
        }
        Self::builtin_snapshot(region, None)
    }

    /// 手动写入实时快照（供网关手动刷新 / 单测种子）。
    pub fn set_live(&self, region: Region, models: Vec<CatalogModel>) {
        let snapshot = CatalogSnapshot {
            region,
            source: CatalogSource::Live,
            fetched_at: Some(now_ms()),
            models,
            note: None,
        };
        self.inner.lock().unwrap().live.insert(region, snapshot);
    }

    /// 实时刷新：成功写缓存并返回 Live；失败按 cached → builtin 降级。
    pub async fn refresh(
        &self,
        region: Region,
        client: &UpstreamClient,
        acc: &Value,
    ) -> CatalogSnapshot {
        self.try_refresh(region, client, acc).await.unwrap_or_else(|_| {
            if let Some(mut cached) = self.load_cached(region) {
                cached.note = Some(DEGRADE_NOTE.to_string());
                return cached;
            }
            Self::builtin_snapshot(region, Some(DEGRADE_NOTE.to_string()))
        })
    }

    /// 实时刷新：仅成功时返回 Ok；失败返回 Err（不降级）。
    ///
    /// 供需要**知道是否真正从上游拉到新数据**的场景使用（如模型目录刷新
    /// 的多策略回退：成功即返回，失败才试下一条策略）。
    pub async fn try_refresh(
        &self,
        region: Region,
        client: &UpstreamClient,
        acc: &Value,
    ) -> Result<CatalogSnapshot, String> {
        match client.fetch_models(region, acc).await {
            Ok(models) if !models.is_empty() => {
                let snapshot = CatalogSnapshot {
                    region,
                    source: CatalogSource::Live,
                    fetched_at: Some(now_ms()),
                    models,
                    note: None,
                };
                self.inner
                    .lock()
                    .unwrap()
                    .live
                    .insert(region, snapshot.clone());
                self.save_cached(&snapshot);
                Ok(snapshot)
            }
            Ok(_) => Err("上游返回空目录".to_string()),
            Err(e) => Err(e.message),
        }
    }

    fn builtin_snapshot(region: Region, note: Option<String>) -> CatalogSnapshot {
        CatalogSnapshot {
            region,
            source: CatalogSource::Builtin,
            fetched_at: None,
            models: Self::fallback_models(region).to_vec(),
            note,
        }
    }

    fn load_cached(&self, region: Region) -> Option<CatalogSnapshot> {
        {
            let inner = self.inner.lock().unwrap();
            if inner.loaded.get(&region).copied().unwrap_or(false) {
                return inner.cached.get(&region).cloned();
            }
        }
        let snapshot = self.read_cached_file(region);
        let mut inner = self.inner.lock().unwrap();
        inner.loaded.insert(region, true);
        if let Some(snapshot) = &snapshot {
            inner.cached.insert(region, snapshot.clone());
        }
        snapshot
    }

    fn read_cached_file(&self, region: Region) -> Option<CatalogSnapshot> {
        let path = self.cache_file(region);
        let text = std::fs::read_to_string(&path).ok()?;
        let cached: CachedCatalog = serde_json::from_str(&text).ok()?;
        if cached.models.is_empty() {
            return None;
        }
        Some(CatalogSnapshot {
            region,
            source: CatalogSource::Cached,
            fetched_at: Some(cached.fetched_at),
            models: cached.models,
            note: None,
        })
    }

    fn save_cached(&self, snapshot: &CatalogSnapshot) {
        if let Some(parent) = self.cache_file(snapshot.region).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let cached = CachedCatalog {
            region: snapshot.region,
            fetched_at: snapshot.fetched_at.unwrap_or_else(now_ms),
            models: snapshot.models.clone(),
        };
        let content = serde_json::to_string_pretty(&cached).unwrap_or_default();
        let _ = crate::modules::config::atomic_write(&self.cache_file(snapshot.region), &content);
    }
}

/// 便捷构造一个失败分类（供测试 / 降级路径复用）。
pub fn catalog_failure(status: u16, kind: UpstreamErrorKind, message: impl Into<String>) -> UpstreamFailure {
    UpstreamFailure {
        status,
        kind,
        message: message.into(),
    }
}

/// 解析目录（转调 [`parse_model_catalog`]），供需要直接解析的场景使用。
pub fn parse_catalog(data: &Value, international: bool) -> Result<Vec<CatalogModel>, UpstreamFailure> {
    parse_model_catalog(data, international)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-catalog-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 把内置兜底目录钉死：条目数与 id 序列硬编码，独立于兜底数据源本身。
    ///
    /// 背景：兜底降级链用例里 `builtin.models.len() == FALLBACK_CN_MODELS.len()`
    /// 两端同源（都走同一个 `LazyLock`），误删/误改兜底数据也照样通过。此处以
    /// 参考实现 `catalog.ts` 的 `FALLBACK_WORKBUDDY_MODELS` /
    /// `FALLBACK_WORKBUDDY_AI_MODELS` 为准做字面量校验。
    #[test]
    fn fallback_catalog_matches_hardcoded_contract() {
        const CN_IDS: [&str; 16] = [
            "auto",
            "hy4-preview",
            "hy3",
            "hy3-x",
            "deepseek-v4.1-flash",
            "glm-5.3",
            "glm-5.3-flash",
            "glm-5.2",
            "glm-5.1",
            "glm-5v-turbo",
            "kimi-k3-1",
            "kimi-k2.8-preview",
            "kimi-k2.7",
            "kimi-k2.6",
            "minimax-m3",
            "deepseek-v4-pro",
        ];
        const GLOBAL_IDS: [&str; 20] = [
            "default-model",
            "fast-model",
            "balanced-model",
            "primary-model",
            "deep-model",
            "hy4-preview-f",
            "hy3",
            "deepseek-v4.1-flash",
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.3-codex",
            "gemini-3.5-flash",
            "glm-5.3",
            "glm-5.2",
            "kimi-k3",
            "kimi-k2.6",
        ];

        let cn = CatalogStore::fallback_models(Region::Cn);
        let global = CatalogStore::fallback_models(Region::Global);

        assert_eq!(FALLBACK_CN_MODELS.len(), 16);
        assert_eq!(FALLBACK_GLOBAL_MODELS.len(), 20);
        assert_eq!(cn[0].id, "auto");
        assert_eq!(global[0].id, "default-model");

        assert_eq!(
            cn.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            CN_IDS,
            "CN 兜底目录变更需同步更新本断言"
        );
        assert_eq!(
            global.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            GLOBAL_IDS,
            "Global 兜底目录变更需同步更新本断言"
        );

        assert_ne!(cn, global, "两个 region 的兜底目录不得互相复用");
    }

    #[test]
    fn degradation_chain_live_then_cached_then_builtin() {
        let dir = temp_dir();

        // 1) 无 live、无缓存 → Builtin。
        let store = CatalogStore::with_cache_dir(dir.clone());
        let builtin = store.current(Region::Cn);
        assert_eq!(builtin.source, CatalogSource::Builtin);
        // 硬编码 16：与兜底数据源比较会两端同源，失去回归价值。
        assert_eq!(builtin.models.len(), 16);

        // 2) 写入缓存文件后 → Cached（新 store 无 live）。
        let cached_models = vec![CatalogModel {
            id: "cached-model".into(),
            name: "Cached".into(),
            context_window: 1,
            max_tokens: 1,
            supports_images: false,
            credits: None,
            badges: Vec::new(),
            free: false,
        }];
        let cached_doc = json!({
            "region": "cn",
            "fetched_at": 42,
            "models": cached_models,
        });
        std::fs::write(
            dir.join("gateway_models.cn.json"),
            serde_json::to_string(&cached_doc).unwrap(),
        )
        .unwrap();
        let store2 = CatalogStore::with_cache_dir(dir.clone());
        let cached = store2.current(Region::Cn);
        assert_eq!(cached.source, CatalogSource::Cached);
        assert_eq!(cached.models[0].id, "cached-model");
        assert_eq!(cached.fetched_at, Some(42));

        // 3) 写入 live 后 → Live 优先。
        store2.set_live(
            Region::Cn,
            vec![CatalogModel {
                id: "live-model".into(),
                name: "Live".into(),
                context_window: 2,
                max_tokens: 2,
                supports_images: true,
                credits: Some("x0.10".into()),
                badges: Vec::new(),
                free: false,
            }],
        );
        let live = store2.current(Region::Cn);
        assert_eq!(live.source, CatalogSource::Live);
        assert_eq!(live.models[0].id, "live-model");

        // region 隔离：Global 仍为 Builtin。
        assert_eq!(store2.current(Region::Global).source, CatalogSource::Builtin);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_cache_is_ignored_and_falls_back_to_builtin() {
        let dir = temp_dir();
        std::fs::write(dir.join("gateway_models.global.json"), "not-json").unwrap();
        let store = CatalogStore::with_cache_dir(dir.clone());
        assert_eq!(store.current(Region::Global).source, CatalogSource::Builtin);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
