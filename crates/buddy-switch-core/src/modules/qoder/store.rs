//! Qoder 模块的落盘助手。
//!
//! 原子写复用 [`crate::modules::config::atomic_write_bytes`]（临时文件 + rename，
//! 临时名带 uuid ⇒ 并发写同一文件不会互相踩），本模块只补两件事：
//!
//! 1. **先建父目录**（`atomic_write_bytes` 不建；账号库所在目录首次使用时并不存在）；
//! 2. **把 `io::Error` 拍成可下发的 `String`**（与两宿主的 `Result<_, String>` 对齐）。
//!
//! ⚠️ 不另起一套「临时文件 + rename」——那正是本仓反复强调的「同款实现散落多处必漂移」。
//! Trae 侧曾因两份实现不一致（一份用固定 `<path>.tmp`）踩过并发写丢文件的坑。

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// 读 JSON。**文件不存在 / 内容损坏一律返回 `Default`**，不抛错。
///
/// 为什么不抛：账号库损坏时若直接报错，用户会**连界面都打不开**（读不到列表 ⇒ 整页失败），
/// 反而没法用界面里的「导入备份」自救。返回默认值让界面能起来，再由调用方给出
/// 「库为空」的结构化空态。
///
/// ⚠️ 代价是「损坏」与「空」在返回值上不可区分。需要区分时用 [`read_json_checked`]。
pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    read_json_checked::<T>(path)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// 读 JSON 并把「读不到 / 解析失败」如实报出来（`Ok(None)` = 文件不存在）。
///
/// 用于**写路径**（改账号库前先读）：写路径必须知道自己是「在空库上新建」还是
/// 「在损坏库上覆盖」，否则会把用户剩下的数据一起抹掉。
pub fn read_json_checked<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| format!("解析 {} 失败：{error}", path.display()))
}

/// 原子写 JSON（先建父目录）。
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text =
        serde_json::to_string_pretty(value).map_err(|error| format!("序列化失败：{error}"))?;
    write_text(path, &text)
}

/// 原子写文本（先建父目录）。
pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录 {} 失败：{error}", parent.display()))?;
    }
    crate::modules::config::atomic_write_bytes(path, text.as_bytes())
        .map_err(|error| format!("写入 {} 失败：{error}", path.display()))
}

/// 本地时间戳：`YYYY-MM-DD HH:MM:SS`（与项目其余文案一致）。
pub fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 本地日期：`YYYY-MM-DD`（台账按日归档用）。
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// ISO-8601（带本地时区偏移）—— 写进账号记录的 `addedAt` / `updatedAt`。
pub fn now_iso() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// 脱敏：只留前 6 位与后 4 位（日志 / 界面里不该出现完整令牌）。
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 10 {
        return "*".repeat(chars.len());
    }
    let head: String = chars.iter().take(6).collect();
    let tail: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_hides_the_middle_and_never_leaks_short_values() {
        let full = "dt-abcdefghijklmnop";
        let masked = mask(full);
        assert!(!masked.contains("abcdefghij"), "泄露了中段：{masked}");
        assert!(masked.starts_with("dt-abc"));
        assert!(masked.ends_with("mnop"));
        // 短值整体打码（不能「前 6 后 4」地把它几乎原样吐出来）。
        assert_eq!(mask("short"), "*****");
        assert_eq!(mask(""), "");
    }

    #[test]
    fn timestamps_have_stable_shapes() {
        assert_eq!(timestamp().len(), 19);
        assert_eq!(today().len(), 10);
        assert!(now_iso().contains('T'), "{}", now_iso());
    }
}
