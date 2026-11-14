//! 用户**显式排除**的账号表（跨平台共享一份文件）。
//!
//! ## 为什么是「一份文件 + 按 scope 分组」，而不是三份配置各存一份
//!
//! 排除是「用户对**这个账号**的意图」，与产品线无关。三份配置意味着三套读写、
//! 三处 DTO、三种默认值 —— 任何一个漂移，就会出现「WB 里排除了、Trae 里还在用」。
//! 本仓对这类「同一语义多份实现」的结论一贯是：抽成唯一实现。
//!
//! ## 语义（硬约束）
//!
//! 被排除的账号：**自动选号、全冷却兜底、会话粘性、显式指定**一律跳过。
//! 它与「禁用」的区别必须讲清：
//!
//! - **禁用** = 这个号坏了（三振 / 凭据死），需要人工复活或重新登录；
//! - **排除** = 用户不想让网关用它（例如「留额度给客户端自己用」），账号本身完全正常。
//!
//! ## 失败模式：fail-open
//!
//! 文件读不出来 / JSON 损坏 ⇒ 按「没有排除」运行，同时由管理面提示。
//! **绝不能**因为一个配置文件坏了就让网关停止服务 —— 那是拿小概率故障换全量不可用。
//!
//! ## 为什么路径是**参数**而不是全局函数
//!
//! `read_at` / `set_at` 收 `&Path`，生产侧传 [`file_path()`]。这样测试可以直接用
//! 临时文件，**完全不碰 `BUDDY_SWITCH_HOME`** —— 那个 env 是进程级全局状态，
//! 本仓已被它坑过两次（并行跑时「写 A 目录、读 B 目录」随机失败）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use buddy_switch_core::modules::config as core_config;
use buddy_switch_core::modules::region;

/// 文件名（与 `gateway_config.json` 同目录）。
const FILE_NAME: &str = "gateway_pool_exclusions.json";

/// 单条排除记录。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExclusionItem {
    /// 备注（为什么排除）。空串 = 未填。
    pub note: String,
    /// 记录时刻（毫秒）；`0` = 旧数据/未知。
    pub at: i64,
}

/// 整份文件：`scope → uid → item`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExclusionsFile {
    /// 结构版本（当前恒 `1`）。留字段是为了将来能无损升级。
    pub version: u32,
    /// 按作用域分组（`wb:cn` / `wb:global` / `trae:work` / `trae:cn` / `qoder:cn` / `qoder:global`）。
    pub scopes: BTreeMap<String, BTreeMap<String, ExclusionItem>>,
}

/// 生产侧的默认文件路径（`~/.buddy-switch/gateway_pool_exclusions.json`）。
pub fn file_path() -> PathBuf {
    region::gateway_config_file().with_file_name(FILE_NAME)
}

/// 进程内缓存：`(路径, mtime_ms, len, 解析结果)`。
///
/// 为什么要有缓存：WB 每个请求都要读一次（`relay` 里同步进池），UI 轮询状态也要读。
/// 文件很小，但「每请求打开 + 解析一个 JSON」在并发下是纯浪费；按 `(路径, mtime, len)`
/// 失效完全不影响正确性 —— 文件一改，mtime 必然变。
///
/// ★ 缓存**必须带路径**：测试与「多实例」场景里同时存在多份文件，
/// 只按 `(mtime, len)` 判会串味（两份空文件都是 `(0, 0)`）。
fn cache() -> &'static Mutex<Option<(PathBuf, i64, u64, ExclusionsFile)>> {
    /// 缓存槽：(文件路径, mtime, 大小, 解析结果)。
    ///
    /// mtime + size 一起做失效判据（只比 mtime 会在「同一秒内改写」时漏判）；
    /// 抽别名是为了让 `OnceLock<Mutex<...>>` 这层包装不把判据埋掉。
    type CacheSlot = Option<(PathBuf, i64, u64, ExclusionsFile)>;

    static CACHE: OnceLock<Mutex<CacheSlot>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// 读整份表（带 mtime 缓存；读失败 / 文件不存在 ⇒ 空表）。
pub fn read_at(path: &Path) -> ExclusionsFile {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);

    if let Ok(guard) = cache().lock() {
        if let Some((cached_path, cached_mtime, cached_len, parsed)) = guard.as_ref() {
            if cached_path == path && *cached_mtime == mtime && *cached_len == len {
                return parsed.clone();
            }
        }
    }

    // 文件不存在 ⇒ 空表（不是错误：绝大多数用户从没排除过任何账号）。
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<ExclusionsFile>(&text).ok())
        .unwrap_or_else(|| ExclusionsFile {
            version: 1,
            scopes: BTreeMap::new(),
        });

    if let Ok(mut guard) = cache().lock() {
        *guard = Some((path.to_path_buf(), mtime, len, parsed.clone()));
    }
    parsed
}

/// 写整份表（原子写 + 立刻失效缓存）。
pub fn write_at(path: &Path, file: &ExclusionsFile) -> Result<(), String> {
    let content = serde_json::to_string_pretty(file).map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    core_config::atomic_write(path, &content).map_err(|error| error.to_string())?;
    // 写后立刻失效缓存：下一次读必须看到刚写的值（否则 UI 会「点了没反应」）。
    if let Ok(mut guard) = cache().lock() {
        *guard = None;
    }
    Ok(())
}

/// 设置/取消一个排除项，返回该作用域的最新条目。
///
/// 原子写：半截文件会让下次启动读到损坏表，而 fail-open 会把它当成「没排除」
/// —— 用户的设置**静默丢失**。
pub fn set_at(
    path: &Path,
    scope: &str,
    uid: &str,
    excluded: bool,
    note: &str,
    now_ms: i64,
) -> Result<BTreeMap<String, ExclusionItem>, String> {
    if scope.trim().is_empty() {
        return Err("scope 不能为空".to_string());
    }
    if uid.trim().is_empty() {
        return Err("uid 不能为空".to_string());
    }

    let mut file = read_at(path);
    if file.version == 0 {
        file.version = 1;
    }
    {
        let entry = file.scopes.entry(scope.to_string()).or_default();
        if excluded {
            entry.insert(
                uid.to_string(),
                ExclusionItem {
                    note: note.to_string(),
                    at: now_ms,
                },
            );
        } else {
            entry.remove(uid);
        }
        // 空表就不落盘，避免文件里堆一堆空对象。
        if entry.is_empty() {
            file.scopes.remove(scope);
        }
    }
    write_at(path, &file)?;
    Ok(items_at(path, scope))
}

/// 某作用域的原始条目（带 `note` / `at`，供端点回显）。
pub fn items_at(path: &Path, scope: &str) -> BTreeMap<String, ExclusionItem> {
    read_at(path).scopes.get(scope).cloned().unwrap_or_default()
}

/// 某作用域的排除表（`uid → note`）。**选号与状态展示的唯一读口**。
pub fn load_for_at(path: &Path, scope: &str) -> BTreeMap<String, String> {
    items_at(path, scope)
        .into_iter()
        .map(|(uid, item)| (uid, item.note))
        .collect()
}

// ---------------------------------------------------------------------------
// 生产侧便捷包装（默认路径）——**唯一被 `relay` / 端点使用的那一层**
// ---------------------------------------------------------------------------

/// 读整份表（默认路径）。
pub fn snapshot() -> ExclusionsFile {
    read_at(&file_path())
}

/// 读某作用域的排除表（默认路径）。
pub fn load_for(scope: &str) -> BTreeMap<String, String> {
    load_for_at(&file_path(), scope)
}

/// 读某作用域的原始条目（默认路径）。
pub fn items_for(scope: &str) -> BTreeMap<String, ExclusionItem> {
    items_at(&file_path(), scope)
}

/// 某作用域被排除的 uid 集合（`pick` 的输入）。
pub fn uid_set(scope: &str) -> std::collections::HashSet<String> {
    load_for(scope).into_keys().collect()
}

/// 设置/取消一个排除项（默认路径）。
pub fn set(
    scope: &str,
    uid: &str,
    excluded: bool,
    note: &str,
    now_ms: i64,
) -> Result<BTreeMap<String, ExclusionItem>, String> {
    set_at(&file_path(), scope, uid, excluded, note, now_ms)
}

/// 端点响应用的 JSON 视图（键名 camelCase）。
pub fn to_value(scope: &str, items: &BTreeMap<String, ExclusionItem>) -> Value {
    json!({
        "scope": scope,
        "items": items
            .iter()
            .map(|(uid, item)| json!({
                "uid": uid,
                "note": item.note,
                "at": item.at,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例一个独立临时文件。**不碰 `BUDDY_SWITCH_HOME`** —— 那是进程级全局状态，
    /// 与其它模块的测试并行时会互相踩（本仓已踩过两次）。
    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bs-excl-{tag}-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn set_round_trips_and_is_scoped() {
        let path = temp_path("roundtrip");
        let _ = std::fs::remove_file(&path);

        set_at(&path, "wb:cn", "u1", true, "留给客户端", 1000).expect("写");
        set_at(&path, "wb:global", "u2", true, "", 2000).expect("写");

        let cn = load_for_at(&path, "wb:cn");
        assert_eq!(cn.get("u1").map(String::as_str), Some("留给客户端"));
        assert!(!cn.contains_key("u2"), "跨作用域不得串味");

        let global = load_for_at(&path, "wb:global");
        assert!(global.contains_key("u2"));
        assert!(!global.contains_key("u1"));

        // 取消排除 ⇒ 表里消失
        let left = set_at(&path, "wb:cn", "u1", false, "", 3000).expect("删");
        assert!(left.is_empty());
        assert!(load_for_at(&path, "wb:cn").is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_or_corrupt_file_is_empty_not_fatal() {
        let path = temp_path("corrupt");
        let _ = std::fs::remove_file(&path);
        // 从未写过 ⇒ 空表
        assert!(load_for_at(&path, "wb:cn").is_empty());

        // 写一份损坏内容 ⇒ 仍然空表（fail-open），不 panic
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        assert!(
            load_for_at(&path, "wb:cn").is_empty(),
            "损坏文件必须按「没排除」降级"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_scope_is_removed_from_the_file() {
        let path = temp_path("prune");
        let _ = std::fs::remove_file(&path);
        set_at(&path, "trae:work", "u9", true, "", 1).expect("写");
        set_at(&path, "trae:work", "u9", false, "", 2).expect("删");
        let file = read_at(&path);
        assert!(
            !file.scopes.contains_key("trae:work"),
            "空作用域不得留在文件里"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// ★ 两个不同的文件**不得**共享缓存条目（都可能是 `(mtime 0, len 0)` 的空表）。
    #[test]
    fn cache_is_keyed_by_path() {
        let a = temp_path("cache-a");
        let b = temp_path("cache-b");
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);

        set_at(&a, "wb:cn", "only-in-a", true, "", 1).expect("写 A");
        // B 此时还不存在 ⇒ 若缓存只按 (mtime,len) 判，会把 A 的解析结果错发给 B。
        assert!(load_for_at(&b, "wb:cn").is_empty(), "不同路径不得共用缓存");
        assert!(load_for_at(&a, "wb:cn").contains_key("only-in-a"));
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }
}
