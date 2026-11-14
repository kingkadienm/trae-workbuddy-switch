//! 客户端限流扫描的**性能基线**（临时工具，用于本轮优化的「改前 / 改后」对照）。
//!
//! 用法：
//!
//! ```text
//! cargo run --release -p buddy-switch-core --example client_limits_bench -- [region] [days...]
//! ```
//!
//! 输出每轮耗时 + `counts`，冷/热各跑一遍（热 = 命中扫描缓存）。

use std::time::Instant;

use buddy_switch_core::modules::client_limits;
use buddy_switch_core::modules::region::Region;

fn summarize(value: &serde_json::Value) -> String {
    let counts = value.get("counts").cloned().unwrap_or_default();
    let get = |key: &str| {
        counts
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    let mut states = std::collections::BTreeMap::new();
    if let Some(limits) = value.get("limits").and_then(|v| v.as_array()) {
        for entry in limits {
            let state = entry
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            *states.entry(state).or_insert(0u64) += 1;
        }
    }
    format!(
        "scanned={:<5} entries={:<4} active={:<3} unknown={:<3} skipped={:<5} states={:?}",
        get("scanned"),
        get("entries"),
        get("active"),
        get("unknown"),
        get("skipped"),
        states
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let region = match args.first().map(String::as_str) {
        Some("cn") => Region::Cn,
        _ => Region::Global,
    };
    let days: Vec<Option<i64>> = if args.len() > 1 {
        args[1..]
            .iter()
            .map(|raw| raw.parse::<i64>().ok())
            .collect()
    } else {
        vec![Some(7), Some(30), Some(0)]
    };

    for window in days {
        let label = window
            .map(|d| d.to_string())
            .unwrap_or_else(|| "all".into());
        for pass in ["cold", "warm"] {
            let started = Instant::now();
            let value = client_limits::client_limits_for(region, window);
            let elapsed = started.elapsed();
            println!(
                "region={:<6} days={:<4} pass={:<4} elapsed={:>7.2}s {}",
                region.as_str(),
                label,
                pass,
                elapsed.as_secs_f64(),
                summarize(&value)
            );
            if std::env::var("CL_DUMP").is_ok() {
                for entry in value
                    .get("limits")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    println!(
                        "    {:<9} {:<8} {:<22} code={:<6} reset={:<14} occurred={:<14} evidence={:<14} x{} {}",
                        entry.get("state").and_then(|v| v.as_str()).unwrap_or("?"),
                        entry.get("account").and_then(|v| v.as_str()).unwrap_or("未知"),
                        entry.get("model").and_then(|v| v.as_str()).unwrap_or("未知"),
                        entry.get("code").map(|v| v.to_string()).unwrap_or_default(),
                        fmt(entry.get("resetAt")),
                        fmt(entry.get("occurredAt")),
                        fmt(entry.get("evidenceAt")),
                        entry.get("occurrences").and_then(|v| v.as_u64()).unwrap_or(0),
                        entry.get("sessionId").and_then(|v| v.as_str()).unwrap_or(""),
                    );
                }
            }
        }
    }
}

fn fmt(value: Option<&serde_json::Value>) -> String {
    use chrono::TimeZone;
    match value.and_then(serde_json::Value::as_i64) {
        Some(ms) if ms > 0 => chrono::FixedOffset::east_opt(8 * 3600)
            .expect("UTC+8")
            .timestamp_millis_opt(ms)
            .single()
            .map(|dt| dt.format("%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "-".into()),
        _ => "-".into(),
    }
}
