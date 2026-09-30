//! Trae 侧的文件读写工具：容错读 + 原子写 + 日志追加。
//!
//! 与 WorkBuddy 侧 [`crate::modules::config`] 的 `atomic_write` 同源（tmp + rename），
//! 但额外提供「读 JSON 失败回落默认值」与「按行追加日志」两个 Trae 模块高频需求，
//! 避免在每个模块里重复 `unwrap_or_default()` 与 `OpenOptions` 样板。

use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// 读取 JSON；文件缺失、为空、或解析失败一律回落 `T::default()`。
///
/// 刻意「不报错」：签到/积分这类后台循环必须能在数据文件损坏时继续运行，
/// 而不是整体失败。真正的写入错误仍会由 [`write_json`] 暴露。
pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    match std::fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).unwrap_or_default(),
        _ => T::default(),
    }
}

/// 原子写 JSON：先写 `<path>.tmp` 再 rename，避免断电/崩溃留下半截文件。
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let text =
        serde_json::to_string_pretty(value).map_err(|e| format!("序列化失败: {e}"))?;
    atomic_write_text(path, &text)
}

/// 原子写文本。
///
/// 实现与 [`crate::modules::config::atomic_write`] 逐项对齐（模块头声称「同源」，
/// 此前其实不一致，2026-09-29 补齐）：
///
/// 1. **临时文件名必须唯一**（追加 uuid）。固定 `<path>.tmp` 在**并发写同一文件**时
///    会让两次写共用同一个临时文件：先完成者 rename 成功后该临时文件已不存在，
///    后完成者的 rename 失败，而它的收尾清理会把对方刚写好的内容一并删掉
///    （管理端实现 `ithtelab/workbuddy-manager` 的 `_atomic_write_json` 记录了
///    这个现场：4 个并发线程全部报错、且原文件消失）。
/// 2. **不预先删除目标文件**。原实现在 rename 前 `remove_file(path)`，
///    于是「删除成功、rename 尚未执行」之间存在一个**目标文件不存在的窗口** ——
///    进程若在此刻崩溃/断电，文件就真的没了，而这正是原子写要避免的情形。
///    注释原先写「Windows 上 rename 不覆盖已存在的目标」，**这句是错的**：
///    Rust 的 `std::fs::rename` 在 Windows 走 `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`，
///    会覆盖已存在的目标。本仓的主写入路径 `config::atomic_write` 一直是直接
///    rename 到已存在文件上（账号库、签到配置每天都在走），这就是现成的反证。
pub fn atomic_write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let tmp = tmp_path(path);
    let write_result = (|| -> Result<(), String> {
        let mut file =
            std::fs::File::create(&tmp).map_err(|e| format!("创建临时文件失败: {e}"))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("写入失败: {e}"))?;
        file.flush().map_err(|e| format!("刷新失败: {e}"))?;
        Ok(())
    })();
    if let Err(error) = write_result {
        // 失败时清掉临时文件，别在目录里留垃圾（同 `config::atomic_write` 的意图）。
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换文件失败: {e}"));
    }
    Ok(())
}

/// 临时文件路径：保留原扩展名并追加 `.tmp-<uuid>`。
///
/// 不用 `with_extension`（会把 `checkin_accounts.json` 变成 `checkin_accounts.tmp`，
/// 丢扩展名后不易排查）；uuid 后缀保证并发写各自独立（见 `atomic_write_text` 第 1 条）。
fn tmp_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().map(|s| s.to_os_string()).unwrap_or_default();
    name.push(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    path.with_file_name(name)
}

/// 向日志文件追加一行（带 `[YYYY-MM-DD HH:MM:SS]` 前缀）。
///
/// 失败静默忽略：日志写入不应影响主流程。前缀格式与参考实现一致，
/// 便于沿用既有的「按日期前缀裁剪日志」策略。
pub fn append_log(path: &Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "[{}] {}", timestamp(), message);
    }
}

/// `YYYY-MM-DD HH:MM:SS`（本地时区）。
pub fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// `YYYY-MM-DD`（本地时区）。
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// `YYYY-MM-DDTHH:MM:SS`（本地时区，参考实现的 `now_iso` 形态）。
pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 掩码：保留前 4 后 4，中间用 `…` 连接；长度 ≤ 8 时原样返回。
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return value.to_string();
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{head}…{tail}")
}

/// 按保留天数裁剪日志文件中的历史行。
///
/// 仅丢弃带 `[YYYY-MM-DD` 前缀且早于 cutoff 的行；无日期前缀的行
/// （外部脚本输出）一律保留。任何错误静默忽略。
pub fn trim_log(path: &Path, retention_days: i64) {
    if retention_days <= 0 {
        return;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return,
    };
    let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(retention_days);
    let mut kept: Vec<&str> = Vec::with_capacity(text.lines().count());
    for line in text.lines() {
        let date_part = line.strip_prefix('[').and_then(|rest| rest.get(..10));
        let expired = match date_part {
            Some(day) => chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map(|parsed| parsed < cutoff)
                .unwrap_or(false),
            None => false,
        };
        if !expired {
            kept.push(line);
        }
    }
    let trimmed = kept.join("\n");
    if trimmed != text {
        let _ = std::fs::write(path, trimmed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
    struct Sample {
        a: i32,
        #[serde(default)]
        b: String,
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-trae-store-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn read_json_falls_back_to_default_on_missing_or_broken() {
        let dir = temp_dir("read");
        let missing = dir.join("nope.json");
        assert_eq!(read_json::<Sample>(&missing), Sample::default());

        let broken = dir.join("broken.json");
        std::fs::write(&broken, "{ not json").unwrap();
        assert_eq!(read_json::<Sample>(&broken), Sample::default());

        let empty = dir.join("empty.json");
        std::fs::write(&empty, "   \n").unwrap();
        assert_eq!(read_json::<Sample>(&empty), Sample::default());
    }

    /// 目录里残留的临时文件（`atomic_write_text` 的 uuid 后缀形态）。
    fn leftover_temp_files(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn write_json_is_atomic_and_roundtrips() {
        let dir = temp_dir("write");
        let path = dir.join("data.json");
        // 先写一份旧内容，再覆盖，确认 rename 路径不会留下半截文件。
        std::fs::write(&path, "{\"a\":1}").unwrap();
        write_json(&path, &Sample { a: 7, b: "x".into() }).unwrap();
        assert_eq!(read_json::<Sample>(&path), Sample { a: 7, b: "x".into() });
        // 临时文件必须已被 rename 掉，不能残留。
        //
        // 断言**整目录**而不是 `!tmp_path(&path).exists()`：临时名现在带 uuid，
        // 再调一次 `tmp_path` 会得到另一个随机名、永远不存在，那条断言就恒真了。
        assert_eq!(
            leftover_temp_files(&dir),
            Vec::<String>::new(),
            "不得残留临时文件"
        );
    }

    #[test]
    fn tmp_path_keeps_original_extension_and_is_unique() {
        let path = std::path::Path::new("/tmp/checkin_accounts.json");
        let tmp = tmp_path(path);
        let name = tmp.file_name().and_then(|s| s.to_str()).unwrap_or_default();
        assert!(
            name.starts_with("checkin_accounts.json.tmp-"),
            "必须保留原扩展名（不得被 with_extension 吃掉），实际 {name}"
        );
        // 唯一性：并发写同一文件时两次写必须各自独立，
        // 否则后完成者的清理会删掉先完成者刚写好的内容。
        assert_ne!(tmp_path(path), tmp, "两次派生的临时路径必须不同");
    }

    #[test]
    fn mask_keeps_head_and_tail() {
        assert_eq!(mask("1234567890123456"), "1234…3456");
        assert_eq!(mask("short"), "short");
        assert_eq!(mask("12345678"), "12345678");
        assert_eq!(mask("123456789"), "1234…6789");
    }

    #[test]
    fn trim_log_drops_expired_dated_lines_and_keeps_others() {
        let dir = temp_dir("trim");
        let path = dir.join("app.log");
        let content = "[2000-01-01 00:00:00] 陈旧\n[2999-01-01 00:00:00] 未来\n无前缀行\n";
        std::fs::write(&path, content).unwrap();
        trim_log(&path, 30);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("陈旧"), "{after}");
        assert!(after.contains("未来"), "{after}");
        assert!(after.contains("无前缀行"), "{after}");
    }

    #[test]
    fn trim_log_with_zero_retention_is_noop() {
        let dir = temp_dir("trim0");
        let path = dir.join("app.log");
        let content = "[2000-01-01 00:00:00] 陈旧\n";
        std::fs::write(&path, content).unwrap();
        trim_log(&path, 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }
}
