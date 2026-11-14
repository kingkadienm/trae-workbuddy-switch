//! 自动更新：检查公开 GitHub Releases 版本 + 更新源配置。
//!
//! 对照 server.py `load_github_config` / `save_github_config` /
//! `compare_versions` / `update_check`。下载安装走 tauri-plugin-updater（整包更新）。
//!
//! 版本检查不走 GitHub API（避免 60 次/小时/IP 限流）：
//! 1. 主端点：release 资产的 updater manifest（下载不计 API 配额）；
//! 2. 兜底端点：`/releases/latest` 的 302 `Location` 头解析 tag；
//! 3. 成功结果进程级缓存 5 分钟（**必须短于前端 30 分钟轮询间隔**，见 [`CACHE_TTL_SECS`]）。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::modules::config::{
    atomic_write, http_request_raw, http_request_with_proxy, now_secs, store_dir,
};

/// 应用当前版本（来自 Cargo.toml package.version）。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GITHUB_OWNER: &str = "kingkadienm";
pub const GITHUB_REPO: &str = "trae-workbuddy-switch";

/// 自动轮询间隔，与前端 `src/lib/update-preference.ts` 的
/// `AUTO_CHECK_UPDATE_INTERVAL_MS`（30 分钟）保持一致。
///
/// 后端**不参与调度**，只用来钉住一条不变量：缓存窗口必须严格短于轮询间隔
/// （见 [`CACHE_TTL_SECS`]）。
const AUTO_CHECK_POLL_SECS: i64 = 30 * 60;

/// 成功结果缓存有效期（5 分钟）。
///
/// **必须严格短于 [`AUTO_CHECK_POLL_SECS`]，且要留出足够余量。**
///
/// 这条不是洁癖，是 2026-10-17 的真实事故：缓存窗口曾是 **6 小时**，而轮询是
/// 30 分钟 ⇒ 每一次自动轮询都命中缓存，**实际检查周期退化成 6 小时**。当晚
/// 18:45 发布 `2026.10.171833`，本地 08:54 启动的进程在 14:54 刚刷新过缓存，
/// 于是到 20:08 仍显示「已是最新」——用户看到的就是「自动检测更新没用，几个小时
/// 都不提示」。与本仓 `COVERAGE-REPORT.md` §A⁷.6 记下的同一条纪律：
/// **窗口必须严格小于轮询间隔，否则看起来在刷、实际永远错过**。
///
/// 5 分钟足够吸收重复调用（多窗口挂载、webui 多标签页、用户连点），
/// 而一次真实检查只是两个 release 资产下载（manifest + 版本说明），
/// **不计 GitHub API 配额**（本模块整体就是为此不走 API 的），
/// 每 30 分钟发一次请求的成本可以忽略。设置页手动检查传 `force=true` 绕过缓存。
const CACHE_TTL_SECS: i64 = 5 * 60;

/// 把上面那条「缓存窗口必须严格短于轮询间隔」钉在**编译期**：
/// 只写在注释里的不变量，改一个常量就悄悄破了（这正是 2026-10-17 事故的成因）。
const _: () = assert!(CACHE_TTL_SECS < AUTO_CHECK_POLL_SECS);

/// 进程级内存缓存，只缓存 ok=true 的结果；失败不写缓存。
struct CachedCheck {
    checked_at: i64,
    value: Value,
}

static CACHE: Mutex<Option<CachedCheck>> = Mutex::new(None);

pub fn github_config_file() -> PathBuf {
    store_dir().join("github_config.json")
}

/// 读取更新源配置（兼容旧配置文件，但永不返回 token）。
///
/// 配置里的 `owner` / `repo` 原样采信，不做任何改写：本项目的升级源只由
/// [`GITHUB_OWNER`] / [`GITHUB_REPO`] 与用户显式配置决定，不会把用户配置的
/// 其他仓库坐标替换成本项目自己的坐标。
pub fn load_github_config() -> Value {
    let mut owner = GITHUB_OWNER.to_string();
    let mut repo = GITHUB_REPO.to_string();
    let mut proxy = String::new();
    let f = github_config_file();
    let mut should_normalize = false;
    if f.exists() {
        if let Ok(text) = std::fs::read_to_string(&f) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                if let Some(value) = v.get("owner").and_then(|v| v.as_str()) {
                    if !value.trim().is_empty() {
                        owner = value.to_string();
                    }
                }
                if let Some(value) = v.get("repo").and_then(|v| v.as_str()) {
                    if !value.trim().is_empty() {
                        repo = value.to_string();
                    }
                }
                if let Some(value) = v.get("proxy").and_then(|v| v.as_str()) {
                    proxy = value.trim().to_string();
                }
                should_normalize = v.get("token").is_some();
            }
        }
    }
    let normalized = json!({"owner": owner, "repo": repo, "proxy": proxy});
    if should_normalize {
        let _ = atomic_write(
            &f,
            &serde_json::to_string_pretty(&normalized).unwrap_or_default(),
        );
    }
    normalized
}

/// 保存更新源配置；公开仓库不需要也不保存 GitHub token。
pub fn save_github_config(cfg: &Value) -> std::io::Result<()> {
    let owner = cfg
        .get("owner")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(GITHUB_OWNER);
    let repo = cfg
        .get("repo")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(GITHUB_REPO);
    let proxy = cfg
        .get("proxy")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let clean = json!({"owner": owner, "repo": repo, "proxy": proxy});
    std::fs::create_dir_all(store_dir())?;
    atomic_write(
        &github_config_file(),
        &serde_json::to_string_pretty(&clean).unwrap_or_default(),
    )
}

fn version_tuple(v: &str) -> Vec<i64> {
    v.trim_start_matches('v')
        .split('.')
        .filter_map(|x| x.parse::<i64>().ok())
        .collect()
}

/// 版本比较：a > b 返回 1，a < b 返回 -1，相等返回 0。
pub fn compare_versions(a: &str, b: &str) -> i64 {
    let ta = version_tuple(a);
    let tb = version_tuple(b);
    for i in 0..ta.len().max(tb.len()) {
        let x = ta.get(i).copied().unwrap_or(0);
        let y = tb.get(i).copied().unwrap_or(0);
        if x != y {
            return if x > y { 1 } else { -1 };
        }
    }
    0
}

/// updater manifest 候选 URL（按优先级）。
///
/// 1. 合并后的 `latest.json`（含各平台）；
/// 2. 当前系统的 `latest-<os>-<arch>.json`；
/// 3. 兼容旧 Windows 安装包：它们仍请求 `latest-macos-<arch>.json`。
pub fn updater_manifest_urls(owner: &str, repo: &str, os: &str, arch: &str) -> Vec<String> {
    let os_slug = match os {
        "macos" | "darwin" => "macos",
        other => other,
    };
    let mut urls = vec![format!(
        "https://github.com/{owner}/{repo}/releases/latest/download/latest.json"
    )];
    urls.push(format!(
        "https://github.com/{owner}/{repo}/releases/latest/download/latest-{os_slug}-{arch}.json"
    ));
    if os_slug != "macos" {
        urls.push(format!(
            "https://github.com/{owner}/{repo}/releases/latest/download/latest-macos-{arch}.json"
        ));
    }
    urls.dedup();
    urls
}

/// 主端点：拉取 updater manifest。成功返回解析后的 JSON（含 version / pub_date），
/// 失败返回可读错误信息。
async fn fetch_manifest_version(
    owner: &str,
    repo: &str,
    proxy: Option<&str>,
) -> Result<Value, String> {
    let mut headers = HashMap::new();
    headers.insert("Accept".to_string(), "application/json".to_string());
    headers.insert("User-Agent".to_string(), "buddy-switch".to_string());
    let mut last_err = "更新清单解析失败".to_string();
    for url in updater_manifest_urls(owner, repo, std::env::consts::OS, std::env::consts::ARCH) {
        let resp = http_request_with_proxy(&url, "GET", None, Some(&headers), proxy).await;
        let version = resp.get("version").and_then(|v| v.as_str()).unwrap_or("");
        if !version.trim().is_empty() {
            return Ok(resp);
        }
        last_err = resp
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("更新清单解析失败")
            .to_string();
    }
    Err(last_err)
}

/// 版本说明资产（`release-notes.json`）的候选 URL。
///
/// 与 updater manifest 同源：走 **release 资产下载**，不计 GitHub API 配额
/// （本模块整体就是为绕开 60 次/小时限流才不用 API 的，别在这里破例）。
pub fn release_notes_urls(owner: &str, repo: &str, tag: &str) -> Vec<String> {
    let mut urls = vec![format!(
        "https://github.com/{owner}/{repo}/releases/latest/download/release-notes.json"
    )];
    if !tag.trim().is_empty() {
        urls.push(format!(
            "https://github.com/{owner}/{repo}/releases/download/{tag}/release-notes.json"
        ));
    }
    urls
}

/// 一条可显示文本：非字符串 / 空白 / 超长 ⇒ `None`。
///
/// 上限 400 字符：一条版本说明不可能更长；超过就是脏数据（或别人换了个资产）。
const MAX_NOTE_TEXT: usize = 400;

fn note_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.is_empty() || text.chars().count() > MAX_NOTE_TEXT {
        return None;
    }
    Some(text.to_string())
}

fn note_string_list(value: Option<&Value>, max_items: usize) -> Vec<Value> {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| note_text(Some(item)))
        .take(max_items)
        .map(Value::String)
        .collect()
}

/// `[{ title, items[] }]` → 只保留白名单字段；空分组丢弃。
fn note_sections(value: Option<&Value>, max_sections: usize) -> Vec<Value> {
    let Some(sections) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    sections
        .iter()
        .filter_map(|section| {
            let title = note_text(section.get("title"))?;
            let items = note_string_list(section.get("items"), 30);
            if items.is_empty() {
                return None;
            }
            Some(json!({ "title": title, "items": items }))
        })
        .take(max_sections)
        .collect()
}

/// `[{ title, desc? }]` → 升级前必读。
fn note_breaking(value: Option<&Value>) -> Vec<Value> {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let title = note_text(item.get("title"))?;
            let mut entry = json!({ "title": title });
            if let Some(desc) = note_text(item.get("desc")) {
                entry["desc"] = Value::String(desc);
            }
            Some(entry)
        })
        .take(8)
        .collect()
}

/// 版本说明的**白名单重建**。
///
/// 为什么不直接透传资产内容：这份 JSON 来自发布资产（仓库 owner 可被用户配置、资产
/// 也可被替换），属于**外部输入**。前端只认这套结构，所以在这里把不认识的东西全部丢掉，
/// 并且：
/// - 版本号必须等于本次检查到的 `latest`（否则就是别的版本的说明，宁可不给）；
/// - 只产出 `version` / `publishedAt` / `size` / `sections` / `breaking` / `history` 六个键。
fn sanitize_release_notes(raw: &Value, expect_version: &str) -> Option<Value> {
    let version = note_text(raw.get("version"))?;
    if !expect_version.is_empty() && version.trim_start_matches('v') != expect_version {
        return None;
    }

    let breaking = note_breaking(raw.get("breaking"));
    let sections = note_sections(raw.get("sections"), 12);
    let history: Vec<Value> = raw
        .get("history")
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let version = note_text(entry.get("version"))?;
                    let sections = note_sections(entry.get("sections"), 12);
                    if sections.is_empty() {
                        return None;
                    }
                    let mut value = json!({ "version": version, "sections": sections });
                    if let Some(published) = note_text(entry.get("publishedAt")) {
                        value["publishedAt"] = Value::String(published);
                    }
                    Some(value)
                })
                .take(20)
                .collect()
        })
        .unwrap_or_default();

    /* 一个分组都没有 ⇒ 这份说明没有信息量，当作「没拿到」（前端会如实显示空态）。 */
    if breaking.is_empty() && sections.is_empty() && history.is_empty() {
        return None;
    }

    let mut out = json!({ "version": version });
    if let Some(published) = note_text(raw.get("publishedAt")) {
        out["publishedAt"] = Value::String(published);
    }
    if let Some(size) = raw.get("size").and_then(|v| v.as_u64()) {
        if size > 0 {
            out["size"] = json!(size);
        }
    }
    if !breaking.is_empty() {
        out["breaking"] = Value::Array(breaking);
    }
    if !sections.is_empty() {
        out["sections"] = Value::Array(sections);
    }
    if !history.is_empty() {
        out["history"] = Value::Array(history);
    }
    Some(out)
}

/// 抓取版本说明（失败返回 `None`，**不**影响版本检查本身）。
///
/// 为什么附在 `update_check` 里而不是另开命令：说明只在「有更新」时才被用到，
/// 而检查结果本来就有 6 小时缓存 —— 跟着缓存走，一次多一个资产下载，
/// 比新增一个命令 + 一轮 IPC + 一份 webui 契约要便宜得多。
async fn fetch_release_notes(
    owner: &str,
    repo: &str,
    tag: &str,
    proxy: Option<&str>,
    expect_version: &str,
) -> Option<Value> {
    let mut headers = HashMap::new();
    headers.insert("Accept".to_string(), "application/json".to_string());
    headers.insert("User-Agent".to_string(), "buddy-switch".to_string());
    for url in release_notes_urls(owner, repo, tag) {
        let resp = http_request_with_proxy(&url, "GET", None, Some(&headers), proxy).await;
        if let Some(notes) = sanitize_release_notes(&resp, expect_version) {
            return Some(notes);
        }
    }
    None
}

/// 兜底端点：请求 `/releases/latest`，读 302 `Location` 头（形如
/// `.../releases/tag/v0.1.13`）解析 tag。不跟随重定向，避免拉到 HTML 页面。
/// 成功返回 tag，失败返回可读错误 + code（状态码或 -1）。
async fn fetch_latest_tag(
    owner: &str,
    repo: &str,
    proxy: Option<&str>,
) -> Result<String, (String, i64)> {
    let url = format!("https://github.com/{owner}/{repo}/releases/latest");
    let mut headers = HashMap::new();
    headers.insert("Accept".to_string(), "text/html".to_string());
    headers.insert("User-Agent".to_string(), "buddy-switch".to_string());
    let (status, resp_headers, body) =
        http_request_raw(&url, "GET", None, Some(&headers), proxy, false).await;

    if status == 0 {
        let msg = if body.trim().is_empty() {
            "网络请求失败".to_string()
        } else {
            body
        };
        return Err((msg, -1));
    }
    if status == 404 {
        // 无正式 release 或仓库不存在。
        return Err(("未找到可用的发布版本".to_string(), 404));
    }
    let location = resp_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("location"))
        .map(|(_, v)| v.clone());
    let location = match location {
        Some(location) => location,
        None => {
            return Err((
                format!("无法获取发布页跳转地址（HTTP {status}）"),
                status as i64,
            ))
        }
    };
    let tag = location.rsplit('/').next().unwrap_or("").trim().to_string();
    if tag.is_empty() || !location.contains("/releases/tag/") {
        return Err(("无法解析发布版本标签".to_string(), -1));
    }
    Ok(tag)
}

/// 查询最新 Release，与本地版本对比。对照 server.py `update_check`。
///
/// `force=true` 绕过缓存强制刷新（设置页手动检查）；否则 [`CACHE_TTL_SECS`]（5 分钟）
/// 内的成功结果直接返回，不发网络请求。主端点 manifest 失败时自动走 302 兜底；
/// 两个端点都失败返回可读错误。
pub async fn update_check(proxy: Option<&str>, force: bool) -> Value {
    if !force {
        if let Some(cached) = CACHE.lock().unwrap().as_ref() {
            if now_secs() - cached.checked_at < CACHE_TTL_SECS {
                return cached.value.clone();
            }
        }
    }

    let cfg = load_github_config();
    let owner = cfg
        .get("owner")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let repo = cfg
        .get("repo")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let configured_proxy = cfg
        .get("proxy")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let proxy = proxy.or(configured_proxy);
    let release_url = format!("https://github.com/{owner}/{repo}/releases/latest");
    let current = APP_VERSION.to_string();

    // 主端点：updater manifest（release 资产下载，不计 GitHub API 配额）。
    if let Ok(manifest) = fetch_manifest_version(&owner, &repo, proxy).await {
        let version = manifest
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let latest = version.strip_prefix('v').unwrap_or(version).to_string();
        let tag = format!("v{latest}");
        let release_name = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&tag)
            .to_string();
        let published_at = manifest
            .get("pub_date")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        /* 版本说明（升级前弹窗的内容）。拿不到就是 `null` —— 前端会如实说「没有附带说明」，
        绝不在这里编一段。 */
        let notes = fetch_release_notes(&owner, &repo, &tag, proxy, &latest).await;
        let value = json!({
            "ok": true,
            "current": current,
            "latest": latest,
            "latestTag": tag,
            "hasUpdate": compare_versions(&latest, &current) > 0,
            "releaseName": release_name,
            "releaseUrl": release_url,
            "publishedAt": published_at,
            "notes": notes,
            "checkedAt": now_secs(),
        });
        *CACHE.lock().unwrap() = Some(CachedCheck {
            checked_at: now_secs(),
            value: value.clone(),
        });
        return value;
    }

    // 兜底端点：`/releases/latest` 的 302 Location 头解析 tag（仅 manifest 失败时）。
    match fetch_latest_tag(&owner, &repo, proxy).await {
        Ok(tag) => {
            let latest = tag.strip_prefix('v').unwrap_or(&tag).to_string();
            /* 兜底分支也顺手取说明：能拿到说明是加分项，拿不到不影响「有新版本」这个结论。 */
            let notes = fetch_release_notes(&owner, &repo, &tag, proxy, &latest).await;
            let value = json!({
                "ok": true,
                "current": current,
                "latest": latest,
                "latestTag": tag,
                "hasUpdate": compare_versions(&latest, &current) > 0,
                "releaseName": tag.clone(),
                "releaseUrl": release_url,
                "notes": notes,
                "checkedAt": now_secs(),
            });
            *CACHE.lock().unwrap() = Some(CachedCheck {
                checked_at: now_secs(),
                value: value.clone(),
            });
            value
        }
        Err((msg, code)) => json!({
            "ok": false,
            "error": msg,
            "message": format!("{msg}（code={code}）"),
            "releaseUrl": release_url,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /* ── 版本说明（升级前弹窗的数据源）：白名单重建 + 版本对齐 ──────────────────
     *
     * 这份 JSON 来自**发布资产**（可被替换的外部输入），所以后端在交给前端之前
     * 必须把它洗成一个**只有已知键**的对象。下面四条对应四个真实风险：
     *   ① 资产被换成别的版本的说明 ⇒ 版本号对不上就不能用；
     *   ② 夹带未知字段（前端按契约渲染，未知字段会变成「永远不显示的死数据」）⇒ 丢弃；
     *   ③ 空壳说明（只有 version，没有分组）⇒ 当作没拿到，让前端说实话；
     *   ④ 超长 / 非字符串条目 ⇒ 丢弃，不让一条脏数据把弹窗撑爆。
     */
    #[test]
    fn release_notes_are_rebuilt_from_whitelist() {
        let raw = json!({
            "version": "2026.10.171833",
            "publishedAt": "2026-10-17T10:33:00Z",
            "size": 28731392u64,
            "sections": [
                { "title": "网关", "items": ["一条", "  ", "两条"] },
                { "title": "", "items": ["空标题分组应被丢弃"] },
                { "title": "空分组", "items": [] }
            ],
            "breaking": [{ "title": "端口统一", "desc": "只改默认值" }],
            "history": [{ "version": "2026.10.161110", "sections": [{ "title": "旧版", "items": ["x"] }] }],
            "evil": { "nested": "<script>alert(1)</script>" }
        });
        let clean = sanitize_release_notes(&raw, "2026.10.171833").expect("应通过白名单重建");
        assert_eq!(clean["version"], "2026.10.171833");
        assert!(clean.get("evil").is_none(), "未知字段必须被丢掉");
        let sections = clean["sections"].as_array().unwrap();
        assert_eq!(sections.len(), 1, "空标题 / 空条目的分组要被丢弃");
        assert_eq!(
            sections[0]["items"].as_array().unwrap().len(),
            2,
            "空白条目要被丢弃"
        );
        assert_eq!(clean["breaking"][0]["title"], "端口统一");
        assert_eq!(clean["history"][0]["version"], "2026.10.161110");
        assert_eq!(clean["size"], 28731392u64);
    }

    #[test]
    fn release_notes_reject_version_mismatch_and_empty_shell() {
        let other_version = json!({
            "version": "2026.9.221126",
            "sections": [{ "title": "网关", "items": ["x"] }]
        });
        assert!(
            sanitize_release_notes(&other_version, "2026.10.171833").is_none(),
            "版本号对不上的说明不能冒充这一版"
        );
        /* 带 `v` 前缀的对齐比较：资产里写 vX 也算对得上。 */
        let prefixed = json!({
            "version": "v2026.10.171833",
            "sections": [{ "title": "网关", "items": ["x"] }]
        });
        assert!(sanitize_release_notes(&prefixed, "2026.10.171833").is_some());

        let shell = json!({ "version": "2026.10.171833" });
        assert!(
            sanitize_release_notes(&shell, "2026.10.171833").is_none(),
            "没有分组的空壳说明要当作「没拿到」"
        );
    }

    /// 配置文件里的仓库坐标必须**原样采信**：本项目与其它项目各自独立，
    /// 绝不允许把用户配置的仓库坐标改写成本项目自己的坐标。
    ///
    /// 这条改写曾以 `migrate_legacy_coordinates` 的形式存在（按 owner 一刀切，
    /// 且因配置优先级高于常量而**落盘覆盖**用户原配置、不可逆），已于 2026-09-23
    /// 整体删除。此测试钉住「不再改写」，防止它以任何形式回归。
    ///
    /// 用隔离的临时 home，绝不触碰真实 `~/.buddy-switch`。**不要**在此再手动
    /// `env_lock()`：`HomeOverrideGuard::set()` 内部已取锁并持到 drop，重复取用会自死锁。
    #[test]
    fn load_github_config_keeps_configured_coordinates_verbatim() {
        let home = std::env::temp_dir().join(format!(
            "buddy-switch-update-cfg-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&home).expect("create isolated home");
        let _guard = crate::modules::config::HomeOverrideGuard::set(&home);

        let file = github_config_file();
        std::fs::create_dir_all(file.parent().expect("config file has parent"))
            .expect("create store dir");

        // 1) 任意第三方坐标必须原样返回，且文件**不被改写**。
        let foreign =
            r#"{"owner":"someone-else","repo":"their-repo","proxy":"http://127.0.0.1:7890"}"#;
        std::fs::write(&file, foreign).expect("seed config");
        let cfg = load_github_config();
        assert_eq!(
            cfg.get("owner").and_then(|v| v.as_str()),
            Some("someone-else")
        );
        assert_eq!(cfg.get("repo").and_then(|v| v.as_str()), Some("their-repo"));
        assert_eq!(
            cfg.get("proxy").and_then(|v| v.as_str()),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            std::fs::read_to_string(&file).expect("read back"),
            foreign,
            "坐标原样采信时不应产生任何落盘改写"
        );

        // 2) 归一化路径（旧配置含 token → 剥离）同样不得顺手改写坐标。
        std::fs::write(
            &file,
            r#"{"owner":"someone-else","repo":"their-repo","proxy":"","token":"ghp_secret"}"#,
        )
        .expect("seed legacy config");
        let cfg = load_github_config();
        assert_eq!(
            cfg.get("owner").and_then(|v| v.as_str()),
            Some("someone-else")
        );
        assert_eq!(cfg.get("repo").and_then(|v| v.as_str()), Some("their-repo"));
        assert!(cfg.get("token").is_none(), "token 不得被返回");
        let rewritten = std::fs::read_to_string(&file).expect("read back normalized");
        assert!(!rewritten.contains("ghp_secret"), "token 必须被剥离");
        assert!(rewritten.contains("someone-else"), "归一化不得改写坐标");

        // 3) 无配置文件时回落到本项目自己的坐标。
        std::fs::remove_file(&file).expect("remove config");
        let cfg = load_github_config();
        assert_eq!(
            cfg.get("owner").and_then(|v| v.as_str()),
            Some(GITHUB_OWNER)
        );
        assert_eq!(cfg.get("repo").and_then(|v| v.as_str()), Some(GITHUB_REPO));

        std::fs::remove_dir_all(&home).expect("cleanup isolated home");
    }

    #[test]
    fn updater_manifest_urls_macos_skips_duplicate_fallback() {
        let urls = updater_manifest_urls(GITHUB_OWNER, GITHUB_REPO, "macos", "aarch64");
        assert_eq!(
            urls,
            vec![
                "https://github.com/kingkadienm/trae-workbuddy-switch/releases/latest/download/latest.json",
                "https://github.com/kingkadienm/trae-workbuddy-switch/releases/latest/download/latest-macos-aarch64.json",
            ]
        );
    }

    #[test]
    fn updater_manifest_urls_windows_keeps_macos_compat() {
        let urls = updater_manifest_urls(GITHUB_OWNER, GITHUB_REPO, "windows", "x86_64");
        assert_eq!(
            urls,
            vec![
                "https://github.com/kingkadienm/trae-workbuddy-switch/releases/latest/download/latest.json",
                "https://github.com/kingkadienm/trae-workbuddy-switch/releases/latest/download/latest-windows-x86_64.json",
                "https://github.com/kingkadienm/trae-workbuddy-switch/releases/latest/download/latest-macos-x86_64.json",
            ]
        );
    }

    #[test]
    fn compare_versions_orders_semver() {
        assert_eq!(compare_versions("0.1.18", "0.1.18"), 0);
        assert_eq!(compare_versions("0.1.19", "0.1.18"), 1);
        assert_eq!(compare_versions("v0.1.17", "0.1.18"), -1);
    }

    /// 缓存窗口必须**严格短于**自动轮询间隔，且留出足够余量。
    ///
    /// 钉的是 2026-10-17 的事故：缓存 6 小时 + 轮询 30 分钟 ⇒ 每次轮询都命中缓存，
    /// 实际检查周期退化成 6 小时，发布 1.5 小时后本地仍显示「已是最新」。
    /// 只要这条断言在，把 TTL 调回「几小时」就会直接测试失败。
    #[test]
    fn cache_ttl_is_strictly_shorter_than_poll_interval() {
        /* 两条都钉在**编译期**：对常量做运行时断言是恒真的，测不出任何东西
        （clippy::assertions_on_constants 说的就是这件事）。放在测试里是为了让
        「为什么必须有这条」与上面的事故记录待在一起，改常量时第一眼能看到。 */
        const _: () = assert!(
            CACHE_TTL_SECS < AUTO_CHECK_POLL_SECS,
            "缓存窗口必须短于轮询间隔，否则自动轮询永远命中缓存"
        );
        const _: () = assert!(
            CACHE_TTL_SECS * 2 <= AUTO_CHECK_POLL_SECS,
            "缓存窗口要留余量：轮询要能稳定打到网络，而不是在缓存边界上撞运气"
        );
    }

    /// 打包版本号形态 `<YYYY>.<M>.<DHHMM>` 必须**严格单调递增**。
    ///
    /// 这是自动更新的正确性前提：若同日两次打包产生相等的版本号，
    /// 客户端会判定「已是最新」，当天推的修复永远收不到。
    /// 版本由 `scripts/stamp-version.sh` 生成，格式说明见该脚本头部。
    #[test]
    fn compare_versions_orders_build_stamped_versions() {
        // 同日分钟级递增
        assert_eq!(compare_versions("2026.9.181216", "2026.9.181215"), 1);
        assert_eq!(compare_versions("2026.9.181215", "2026.9.181216"), -1);
        // 跨日（含月末 → 月初，日从 30 跳到 1）
        assert_eq!(compare_versions("2026.9.190000", "2026.9.182359"), 1);
        assert_eq!(compare_versions("2026.10.010000", "2026.9.302359"), 1);
        // 跨月（9 → 10）与跨年（2026 → 2027）
        assert_eq!(compare_versions("2026.10.010909", "2026.9.300909"), 1);
        assert_eq!(compare_versions("2027.1.010000", "2026.12.312359"), 1);
        // 日的定长补零不破坏跨日顺序（9 日 → 10 日）
        assert_eq!(compare_versions("2026.9.100909", "2026.9.090909"), 1);
        // 与历史 3 段产物（`<YYYY>.<M>.<D>`）的过渡关系。
        //
        // 新格式第三段是 `DHHMM`（6 位数，形如 `180000`），旧格式第三段是 `D`
        // （1~2 位数，形如 `19`），所以**同一月内**新格式恒大于旧格式：
        assert_eq!(compare_versions("2026.9.180000", "2026.9.18"), 1);
        assert_eq!(compare_versions("2026.9.18", "2026.9.180000"), -1);
        assert_eq!(compare_versions("2026.9.180909", "2026.9.18"), 1);
        // 新版本大于上一轮历史产物
        assert_eq!(compare_versions("2026.9.181216", "2026.9.17"), 1);
        //
        // **已知边界（刻意保留，不要"修"）**：第二段是「月」，比较优先级高于第三段。
        // 因此跨月比较时，旧格式里月更大的版本会胜出：
        assert_eq!(compare_versions("2026.9.181216", "2026.12.31"), -1);
        // 这只发生在「旧格式的未来月份 vs 新格式的较早月份」这种真实不会出现的组合上：
        // 一旦开始用新格式，人就不会再产出旧格式；而历史上旧格式的月份必然 ≤ 当前月。
        // 反过来「旧格式的过去月份 vs 新格式的当前月」方向是对的：
        assert_eq!(compare_versions("2026.9.181216", "2026.8.20"), 1);
    }
}
