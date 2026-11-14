//! Token 统计扫描的**性能基线**（临时工具，用于 P0-4 优化的「改前 / 改后」对照）。
//!
//! 用法（release，debug 的 JSON 解析慢一个数量级、不可代表线上）：
//!
//! ```text
//! cargo run --release -p buddy-switch-core --example token_scan_bench -- [region] [days...]
//! ```
//!
//! 例：`-- cn 30` 只测 CN 源、30 天窗口；不带参数则测 cn/global × (全部 / 30 天)。
//!
//! 输出每轮耗时 + `filesScanned` + 记录数，便于核对「优化前后输出等价」。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use buddy_switch_core::modules::region::RegionFilter;
use buddy_switch_core::modules::token_stats;

/// 只读盘、不解析：把「I/O 上限」与「解析成本」拆开。
///
/// ## 为什么需要这个模式
///
/// 只知道总耗时无法决定往哪优化：若纯读就要 70s，那再快的 JSON 解析也没用
/// （只剩「少读」或「持久化缓存」两条路）；若纯读只要 10s，那 60s 就在解析里，
/// 值得动解析路径。**先量化再投入。**
fn read_only(root: &Path) -> (usize, u64) {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // 与生产一致：跳过 subagents。
                if path.file_name().and_then(|name| name.to_str()) != Some("subagents") {
                    walk(&path, out);
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                out.push(path);
            }
        }
    }

    let mut paths = Vec::new();
    walk(root, &mut paths);
    paths.sort();

    let bytes = AtomicU64::new(0);
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4)
        .clamp(1, 32);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let cursor = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(cursor) else {
                    break;
                };
                let Ok(mut file) = std::fs::File::open(path) else {
                    continue;
                };
                let mut buffer = Vec::new();
                use std::io::Read;
                let read = file.read_to_end(&mut buffer).unwrap_or(0);
                bytes.fetch_add(read as u64, Ordering::Relaxed);
            });
        }
    });

    (paths.len(), bytes.load(Ordering::Relaxed))
}

fn summarize(value: &serde_json::Value) -> (u64, u64, u64) {
    let mut files = 0u64;
    let mut records = 0u64;
    let mut errors = 0u64;
    if let Some(sources) = value.get("sources").and_then(|v| v.as_array()) {
        for source in sources {
            files += source
                .get("filesScanned")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            errors += source
                .get("parseErrors")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            records += source
                .get("summary")
                .and_then(|s| s.get("records"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }
    }
    (files, records, errors)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `read [目录]` —— 只读盘不解析，量 I/O 上限。
    if args.first().map(String::as_str) == Some("read") {
        let root = args.get(1).map(PathBuf::from).unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".workbuddy-ai/projects")
        });
        for pass in ["cold", "warm"] {
            let started = Instant::now();
            let (files, bytes) = read_only(&root);
            let elapsed = started.elapsed();
            println!(
                "read-only pass={pass:<4} elapsed={:>8.2}s files={files:<6} bytes={:.2}GB  throughput={:.0}MB/s",
                elapsed.as_secs_f64(),
                bytes as f64 / 1_073_741_824.0,
                bytes as f64 / 1_048_576.0 / elapsed.as_secs_f64().max(f64::EPSILON),
            );
        }
        return;
    }

    let mut cases: Vec<(RegionFilter, Option<i64>)> = Vec::new();
    if let Some(first) = args.first() {
        let filter = RegionFilter::parse(first).unwrap_or(RegionFilter::Cn);
        if args.len() > 1 {
            for days in &args[1..] {
                cases.push((filter, days.parse::<i64>().ok()));
            }
        } else {
            cases.push((filter, None));
        }
    } else {
        for filter in [RegionFilter::Cn, RegionFilter::Global] {
            cases.push((filter, None));
            cases.push((filter, Some(30)));
        }
    }

    for (filter, days) in cases {
        let label = days.map(|d| d.to_string()).unwrap_or_else(|| "all".into());
        // 等价性核对：`TOKEN_BENCH_DUMP=<目录>` 时把**完整 JSON**落盘。
        // 冷/热两轮各存一份 ⇒ 既能「改前 vs 改后」diff，也能「冷 vs 热」diff。
        let dump = std::env::var("TOKEN_BENCH_DUMP").ok();
        let passes: &[&str] = &["cold", "warm"];

        for pass in passes {
            let started = Instant::now();
            let value = token_stats::get_statistics_for_filter(filter, days);
            let elapsed = started.elapsed();
            let (files, records, errors) = summarize(&value);
            if let Some(dir) = &dump {
                // `generatedAt` 是 `now_ms()`，两次运行必然不同 —— 它是唯一
                // 与「读到的文件」无关的字段，落盘前剔除，否则 diff 恒红。
                let mut value = value.clone();
                if let Some(object) = value.as_object_mut() {
                    object.remove("generatedAt");
                }
                let path = std::path::Path::new(dir)
                    .join(format!("{}-{label}-{pass}.json", filter.as_str()));
                std::fs::write(
                    &path,
                    serde_json::to_string_pretty(&value).expect("serialize"),
                )
                .expect("write dump");
                println!("dumped {}", path.display());
            }
            println!(
                "region={:<6} days={:<5} pass={:<4} elapsed={:>8.2}s files={:<6} records={:<8} parseErrors={}",
                filter.as_str(),
                label,
                pass,
                elapsed.as_secs_f64(),
                files,
                records,
                errors,
            );
        }
    }
}
