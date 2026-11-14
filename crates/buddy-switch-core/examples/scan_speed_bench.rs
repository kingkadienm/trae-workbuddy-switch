//! 扫描速度基线：**目录遍历 / 今日 Token / 限流台账**三段分开计时。
//!
//! 用法（必须 release；debug 下 JSON 解析慢一个数量级，不可代表线上）：
//!
//! ```text
//! cargo run --release -p buddy-switch-core --example scan_speed_bench -- [region] [days...]
//! ```
//!
//! 输出：
//! - `walk` 两轮：分别用「`path.is_dir()` + `entry.metadata()`」（旧）与
//!   「`entry.file_type()` + `entry.metadata()`」（新）遍历同一棵树，量出**系统调用**的代价；
//! - `daily`：`token_daily::record_client_today(region, force=true)` 的耗时；
//! - `limits`：`client_limits::client_limits_for(region, Some(days))` 的耗时。

use std::path::{Path, PathBuf};
use std::time::Instant;

use buddy_switch_core::modules::client_limits;
use buddy_switch_core::modules::region::Region;
use buddy_switch_core::modules::session::session_data_dir;
use buddy_switch_core::modules::token_daily;

/// 旧写法：每个条目一次 `path.is_dir()`（= 一次 `stat` 系统调用）。
fn walk_with_is_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_with_is_dir(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

/// 新写法：`entry.file_type()` 直接取自目录枚举结果，**不额外发系统调用**。
fn walk_with_file_type(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk_with_file_type(&entry.path(), out);
        } else if entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(entry.path());
        }
    }
}

/// `entry.file_type()` + `entry.metadata()`（生产用的形态：一次遍历同时判类型与取 mtime）。
fn walk_with_metadata(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk_with_metadata(&entry.path(), out);
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        if entry.metadata().is_ok() {
            out.push(path);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let region = match args.first().map(String::as_str) {
        Some("cn") => Region::Cn,
        _ => Region::Global,
    };
    let days: Vec<i64> = if args.len() > 1 {
        args[1..]
            .iter()
            .filter_map(|raw| raw.parse::<i64>().ok())
            .collect()
    } else {
        vec![7]
    };
    let root = session_data_dir(region).join("projects");
    println!("region={} root={}", region.as_str(), root.display());

    for (label, walk) in [
        (
            "is_dir   ",
            walk_with_is_dir as fn(&Path, &mut Vec<PathBuf>),
        ),
        (
            "file_type",
            walk_with_file_type as fn(&Path, &mut Vec<PathBuf>),
        ),
        (
            "+metadata",
            walk_with_metadata as fn(&Path, &mut Vec<PathBuf>),
        ),
    ] {
        let mut out = Vec::new();
        let started = Instant::now();
        walk(&root, &mut out);
        println!(
            "walk[{label}] elapsed={:>7.2}s files={}",
            started.elapsed().as_secs_f64(),
            out.len()
        );
    }

    for pass in ["cold", "warm"] {
        let started = Instant::now();
        let value = token_daily::record_client_today(region, true);
        let elapsed = started.elapsed();
        let accounts = value
            .get("accounts")
            .and_then(|v| v.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        // 等价性核对：`SCAN_BENCH_DUMP=<目录>` 时把完整响应落盘（`date` 之外无时间字段）。
        if let Ok(dir) = std::env::var("SCAN_BENCH_DUMP") {
            let path =
                std::path::Path::new(&dir).join(format!("daily-{}-{pass}.json", region.as_str()));
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&value).expect("serialize"),
            )
            .expect("write dump");
            println!("dumped {}", path.display());
        }
        println!(
            "daily  pass={pass:<4} elapsed={:>7.2}s accounts={accounts}",
            elapsed.as_secs_f64()
        );
    }

    for days in days {
        for pass in ["cold", "warm"] {
            let started = Instant::now();
            let value = client_limits::client_limits_for(region, Some(days));
            let elapsed = started.elapsed();
            let counts = value.get("counts").cloned().unwrap_or_default();
            println!(
                "limits pass={pass:<4} days={days:<3} elapsed={:>7.2}s counts={counts}",
                elapsed.as_secs_f64()
            );
        }
    }
}
