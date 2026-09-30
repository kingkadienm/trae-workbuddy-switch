//! 网关逐请求用量桶（移植 workbuddy2api-panel `internal/usage`）。
//!
//! 与池内成本账本的区别：池账本只保留「最近一次 + 累计」，没有时间维度；
//! 本模块按（时间片 × region × uid × model）分桶，可出「今天各模型各用了多少」、
//! 「这一小时 prompt 涨得多快」，且长期保留。
//!
//! 保留策略（分片粒度自动降级，总量有界）：
//! - 近 [`HOURLY_KEEP_HOURS`] 小时：小时桶（细粒度看尖峰）；
//! - 更早：折叠为日桶，永久保留（看长期趋势）。
//!
//! 落盘：`~/.buddy-switch/usage.json`，每请求即时原子写（调用点本身已低频——
//! 每条请求一次，无需防抖）。缺失/损坏文件视为空，不报错。

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};

use crate::modules::region::{Region, RegionFilter};

/// 小时桶保留时长：超出后折叠为日桶。panel 口径 90 天。
pub const HOURLY_KEEP_HOURS: i64 = 90 * 24;

/// 桶数硬上限：超出立即折叠（异常流量下内存/文件不至于膨胀）。
const MAX_BUCKETS: usize = 400_000;

/// 快照输出形状（camelCase，与前端 TokenStats 其他源的字段命名对齐）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageAgg {
    pub requests: u64,
    pub errors: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub avg_latency_ms: Option<f64>,
    pub avg_tps: Option<f64>,
}

impl Default for UsageAgg {
    fn default() -> Self {
        Self {
            requests: 0,
            errors: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            avg_latency_ms: None,
            avg_tps: None,
        }
    }
}

/// 落盘桶。JSON 字段取短名（桶数随时间增长，文件要小）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Bucket {
    /// "h:2006-01-02T15"（小时桶，本地时区）或 "d:2006-01-02"（日桶）。
    scope: String,
    realm: String,
    uid: String,
    model: String,
    req: u64,
    err: u64,
    pt: u64,
    ct: u64,
    tt: u64,
    lat_ms: u64,
    lat_n: u64,
    tps: f64,
    tps_n: u64,
}

impl Bucket {
    fn merge_from(&mut self, other: &Bucket) {
        self.req = self.req.saturating_add(other.req);
        self.err = self.err.saturating_add(other.err);
        self.pt = self.pt.saturating_add(other.pt);
        self.ct = self.ct.saturating_add(other.ct);
        self.tt = self.tt.saturating_add(other.tt);
        self.lat_ms = self.lat_ms.saturating_add(other.lat_ms);
        self.lat_n = self.lat_n.saturating_add(other.lat_n);
        self.tps = self.tps + other.tps;
        self.tps_n = self.tps_n.saturating_add(other.tps_n);
    }

    fn to_agg(&self) -> UsageAgg {
        UsageAgg {
            requests: self.req,
            errors: self.err,
            prompt_tokens: self.pt,
            completion_tokens: self.ct,
            total_tokens: self.tt,
            avg_latency_ms: (self.lat_n > 0)
                .then(|| (self.lat_ms as f64) / (self.lat_n as f64)),
            avg_tps: (self.tps_n > 0).then(|| self.tps / (self.tps_n as f64)),
        }
    }
}

/// 落盘结构。
#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    saved: String,
    buckets: Vec<Bucket>,
}

/// 记录器。`path == None` 时为纯内存（测试用，不落盘）。
pub struct UsageRecorder {
    path: Option<PathBuf>,
    buckets: BTreeMap<String, Bucket>,
}

impl UsageRecorder {
    /// 从落盘文件载入；文件缺失/损坏从零开始（与 panel 同口径，不报错）。
    pub fn load(path: Option<PathBuf>) -> Self {
        let buckets = match &path {
            Some(p) => match std::fs::read_to_string(p) {
                Ok(text) => serde_json::from_str::<File>(&text)
                    .map(|file| {
                        file.buckets
                            .into_iter()
                            .map(|b| (bucket_key(&b), b))
                            .collect()
                    })
                    .unwrap_or_default(),
                Err(_) => BTreeMap::new(),
            },
            None => BTreeMap::new(),
        };
        Self { path, buckets }
    }

    /// 一次请求尝试的用量增量（失败尝试 `ok=false` 仍计请求数与失败数）。
    pub fn add(
        &mut self,
        now_ms: i64,
        region: Region,
        uid: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        latency_ms: i64,
        tps: Option<f64>,
        ok: bool,
    ) {        let model = if model.is_empty() {
            "(unknown)"
        } else {
            model
        };
        let realm = region.as_str();
        let scope = hour_scope(now_ms);
        let key = bucket_key_from_parts(&scope, realm, uid, model);

        let bucket = self.buckets.entry(key).or_insert_with(|| Bucket {
            scope,
            realm: realm.to_string(),
            uid: uid.to_string(),
            model: model.to_string(),
            ..Default::default()
        });
        bucket.req = bucket.req.saturating_add(1);
        if !ok {
            bucket.err = bucket.err.saturating_add(1);
        }
        bucket.pt = bucket.pt.saturating_add(prompt_tokens);
        bucket.ct = bucket.ct.saturating_add(completion_tokens);
        // 上游没给 total：prompt+completion 兜底，保证总量口径连续（panel 同形）。
        bucket.tt = bucket
            .tt
            .saturating_add(prompt_tokens.saturating_add(completion_tokens));
        if latency_ms >= 0 {
            bucket.lat_ms = bucket.lat_ms.saturating_add(latency_ms as u64);
            bucket.lat_n = bucket.lat_n.saturating_add(1);
        }
        if let Some(tps) = tps.filter(|v| v.is_finite() && *v >= 0.0) {
            bucket.tps += tps;
            bucket.tps_n = bucket.tps_n.saturating_add(1);
        }

        if self.buckets.len() > MAX_BUCKETS {
            self.rollup(now_ms);
        }
        self.save();
    }

    /// 把超出保留窗口的小时桶折叠为日桶（幂等：先累加再删源桶）。
    pub fn rollup(&mut self, now_ms: i64) {
        let cutoff_ms = now_ms.saturating_sub(HOURLY_KEEP_HOURS * 3_600_000);
        let mut moves: Vec<(String, String)> = Vec::new();
        for (key, bucket) in self.buckets.iter() {
            if !bucket.scope.starts_with('h') {
                continue;
            }
            if parse_hour_scope_ms(&bucket.scope).is_some_and(|ts| ts < cutoff_ms) {
                let day = day_scope_of_hour(&bucket.scope);
                let target = bucket_key_from_parts(&day, &bucket.realm, &bucket.uid, &bucket.model);
                moves.push((key.clone(), target));
            }
        }
        let mut moved = 0usize;
        for (from, to) in &moves {
            let source = self.buckets.remove(from).unwrap_or_else(|| {
                unreachable!("move 列表在锁外不会变")
            });
            let scope = to.split_once('|').unwrap_or_else(|| (&to, "")).0.to_string();
            match self.buckets.get_mut(to) {
                Some(existing) => {
                    existing.merge_from(&source);
                }
                None => {
                    let mut merged = source;
                    merged.scope = scope;
                    self.buckets.insert(to.clone(), merged);
                }
            }
            moved += 1;
        }
        if moved > 0 {
            self.save();
        }
    }

    fn save(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        let file = File {
            version: 1,
            saved: now_iso(crate::modules::config::now_ms()),
            buckets: self.buckets.values().cloned().collect(),
        };
        let Ok(content) = serde_json::to_string(&file) else {
            return;
        };
        let _ = crate::modules::config::atomic_write(path, &content);
    }
}

fn bucket_key(b: &Bucket) -> String {
    bucket_key_from_parts(&b.scope, &b.realm, &b.uid, &b.model)
}

fn bucket_key_from_parts(scope: &str, realm: &str, uid: &str, model: &str) -> String {
    format!("{scope}|{realm}|{uid}|{model}")
}

fn hour_scope(now_ms: i64) -> String {
    let local = chrono::DateTime::from_timestamp_millis(now_ms)
        .map(|utc| utc.with_timezone(&Local))
        .unwrap_or_default();
    format!("h:{}T{:02}", local.format("%Y-%m-%d"), local.hour())
}

fn day_scope_of_hour(hour_scope: &str) -> String {
    let date = hour_scope
        .strip_prefix("h:")
        .and_then(|rest| rest.split('T').next())
        .unwrap_or("");
    format!("d:{date}")
}

/// 小时桶 scope（"h:2006-01-02T15"）→ 该小时起点的本地毫秒。
fn parse_hour_scope_ms(scope: &str) -> Option<i64> {
    let rest = scope.strip_prefix("h:")?;
    let (date, hour) = rest.split_once('T')?;
    let date_parts: Vec<&str> = date.split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }
    let year = date_parts[0].parse::<i32>().ok()?;
    let month = date_parts[1].parse::<u32>().ok()?;
    let day = date_parts[2].parse::<u32>().ok()?;
    let hour = hour.parse::<u32>().ok()?;
    let naive = chrono::NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, 0, 0)?;
    naive
        .and_local_timezone(Local)
        .earliest()
        .map(|local| local.timestamp_millis())
}

fn now_iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|utc| utc.to_rfc3339())
        .unwrap_or_default()
}

/// 快照：按窗口 + 区域过滤聚合。
pub struct UsageSnapshot {
    pub totals: UsageAgg,
    pub by_account: Vec<KeyedUsageAgg>,
    pub by_model: Vec<KeyedUsageAgg>,
    pub by_region: Vec<KeyedUsageAgg>,
    pub series: Vec<UsagePoint>,
    pub buckets: usize,
    pub since: Option<String>,
}

pub struct KeyedUsageAgg {
    pub key: String,
    pub realm: Option<String>,
    pub agg: UsageAgg,
}

pub struct UsagePoint {
    pub t: String,
    pub scope: &'static str,
    pub agg: UsageAgg,
}

fn merge_agg(a: &mut UsageAgg, b: &UsageAgg) {
    a.requests = a.requests.saturating_add(b.requests);
    a.errors = a.errors.saturating_add(b.errors);
    a.prompt_tokens = a.prompt_tokens.saturating_add(b.prompt_tokens);
    a.completion_tokens = a.completion_tokens.saturating_add(b.completion_tokens);
    a.total_tokens = a.total_tokens.saturating_add(b.total_tokens);
    if a.avg_latency_ms.is_none() {
        a.avg_latency_ms = b.avg_latency_ms;
    }
    if a.avg_tps.is_none() {
        a.avg_tps = b.avg_tps;
    }
}

/// 窗口口径（panel `Snapshot` 同形）：`hours > 0` 统计最近 `hours` 个整点桶
/// （日桶天然不入小时窗口）；`hours <= 0` 全部历史。
pub fn snapshot(
    recorder: &UsageRecorder,
    hours: i64,
    filter: &RegionFilter,
    now_ms: i64,
) -> UsageSnapshot {
    let windowed = hours > 0;
    let hour_from = if windowed {
        // 起点 = 当前整点往回推 (hours-1) 小时（panel 口径）。桶最多保留 90 天，
        // 更大的窗口自然取不到更多数据，无需额外夹紧。
        let offset_hours = hours - 1;
        let from_ms = now_ms.saturating_sub(offset_hours * 3_600_000);
        Some(parse_hour_scope_ms(&hour_scope(from_ms)).unwrap_or(from_ms))
    } else {
        None
    };

    let mut totals = UsageAgg::default();
    let mut by_account: BTreeMap<String, UsageAgg> = BTreeMap::new();
    let mut by_model: BTreeMap<String, UsageAgg> = BTreeMap::new();
    let mut by_region: BTreeMap<String, UsageAgg> = BTreeMap::new();
    let mut hour_series: BTreeMap<String, UsageAgg> = BTreeMap::new();
    let mut day_series: BTreeMap<String, UsageAgg> = BTreeMap::new();
    let mut since: Option<String> = None;
    let mut matched = 0usize;

    for bucket in recorder.buckets.values() {
        let Some(region) = Region::parse(&bucket.realm) else {
            continue;
        };
        if !filter.regions().contains(&region) {
            continue;
        }
        if windowed {
            let ts = if bucket.scope.starts_with('h') {
                parse_hour_scope_ms(&bucket.scope)
            } else {
                // 日桶不入小时窗口（与 panel 同口径）。
                continue;
            };
            if let Some(ts) = ts {
                if hour_from.is_some_and(|from| ts < from) {
                    continue;
                }
            }
        }
        matched += 1;
        if since.is_none() || bucket.scope < since.clone().unwrap_or_default() {
            since = Some(bucket.scope.clone());
        }

        let agg = bucket.to_agg();
        merge_agg(&mut totals, &agg);
        merge_agg(by_account.entry(bucket.uid.clone()).or_default(), &agg);
        merge_agg(by_model.entry(bucket.model.clone()).or_default(), &agg);
        merge_agg(by_region.entry(bucket.realm.clone()).or_default(), &agg);
        let series = if bucket.scope.starts_with('h') {
            &mut hour_series
        } else {
            &mut day_series
        };
        merge_agg(series.entry(bucket.scope.clone()).or_default(), &agg);
    }

    // 时序：日点（升序）→ 小时点（升序），拼一条连续序列（panel 同形）。
    let mut series = Vec::new();
    for (scope, agg) in day_series {
        series.push(UsagePoint {
            t: scope.trim_start_matches("d:").to_string(),
            scope: "day",
            agg,
        });
    }
    for (scope, agg) in hour_series {
        series.push(UsagePoint {
            t: scope.trim_start_matches("h:").to_string(),
            scope: "hour",
            agg,
        });
    }

    fn keyed(map: &BTreeMap<String, UsageAgg>) -> Vec<KeyedUsageAgg> {
        map.iter()
            .map(|(key, agg)| KeyedUsageAgg {
                key: key.clone(),
                realm: None,
                agg: agg.clone(),
            })
            .collect()
    }

    UsageSnapshot {
        totals,
        by_account: keyed(&by_account),
        by_model: keyed(&by_model),
        by_region: keyed(&by_region),
        series,
        buckets: matched,
        since,
    }
}

/// 落盘路径：`~/.buddy-switch/usage.json`（刻意不分 region 文件——桶自带 realm）。
pub fn usage_file() -> PathBuf {
    crate::modules::config::store_dir().join("usage.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms_in_local(date: &str, hour: u32) -> i64 {
        // "2026-09-30" 这类字面量 → 本地毫秒
        let (y, m, d) = {
            let parts: Vec<&str> = date.split('-').collect();
            (
                parts[0].parse().unwrap(),
                parts[1].parse().unwrap(),
                parts[2].parse().unwrap(),
            )
        };
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|d| d.and_hms_opt(hour, 0, 0))
            .unwrap()
            .and_local_timezone(chrono::Local)
            .earliest()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn hourly_bucket_aggregates_and_keeps_failed_requests() {
        let now = ms_in_local("2026-09-30", 10);
        let mut recorder = UsageRecorder::load(None);
        recorder.add(now, Region::Cn, "u1", "glm-5.2", 100, 50, 1000, Some(50.0), true);
        recorder.add(now, Region::Cn, "u1", "glm-5.2", 0, 0, 800, None, false);
        recorder.add(now, Region::Cn, "u2", "deepseek", 10, 5, 200, None, true);

        let snap = snapshot(&recorder, 24, &RegionFilter::Cn, now + 60_000);
        assert_eq!(snap.totals.requests, 3);
        assert_eq!(snap.totals.errors, 1);
        assert_eq!(snap.totals.prompt_tokens, 110);
        assert_eq!(snap.totals.completion_tokens, 55);
        // 桶按（region|uid|model）去重：u1/glm-5.2 两次请求 + u2/deepseek 一次 = 2 个桶。
        assert_eq!(snap.buckets, 2);
        assert_eq!(snap.series.len(), 1, "同一小时内只有一个小时点");
        assert_eq!(snap.series[0].scope, "hour");
    }

    #[test]
    fn hour_window_filters_older_buckets() {
        let now = ms_in_local("2026-09-30", 12);
        let mut recorder = UsageRecorder::load(None);
        // 10 点：在 3 小时窗口（10:00..12:00）内；24 小时前必不在。
        recorder.add(now - 2 * 3_600_000, Region::Cn, "u1", "m", 5, 5, 100, None, true);
        recorder.add(now - 24 * 3_600_000, Region::Cn, "u1", "m", 99, 99, 100, None, true);

        let snap = snapshot(&recorder, 3, &RegionFilter::Cn, now);
        assert_eq!(snap.totals.requests, 1);

        let all = snapshot(&recorder, 0, &RegionFilter::Cn, now);
        assert_eq!(all.totals.requests, 2);
    }

    #[test]
    fn region_filter_excludes_other_realm() {
        let now = ms_in_local("2026-09-30", 12);
        let mut recorder = UsageRecorder::load(None);
        recorder.add(now, Region::Cn, "u1", "m", 10, 10, 0, None, true);
        recorder.add(now, Region::Global, "u2", "m", 90, 90, 0, None, true);

        let cn = snapshot(&recorder, 0, &RegionFilter::Cn, now);
        assert_eq!(cn.totals.prompt_tokens, 10);
        let all = snapshot(&recorder, 0, &RegionFilter::All, now);
        assert_eq!(all.totals.prompt_tokens, 100);
    }

    #[test]
    fn rollup_collapses_old_hourly_buckets_into_daily() {
        let now = ms_in_local("2026-09-30", 12);
        let old = now - (HOURLY_KEEP_HOURS + 1) * 3_600_000;
        let mut recorder = UsageRecorder::load(None);
        recorder.add(old, Region::Cn, "u1", "m", 10, 10, 100, None, true);
        recorder.add(now, Region::Cn, "u1", "m", 1, 1, 50, None, true);

        recorder.rollup(now);

        let scoped: Vec<_> = recorder.buckets.values().collect();
        assert_eq!(scoped.len(), 2);
        assert!(scoped.iter().any(|b| b.scope.starts_with("d:")));
        assert!(scoped.iter().any(|b| b.scope.starts_with("h:")));
        let day = scoped.iter().find(|b| b.scope.starts_with("d:")).unwrap();
        assert_eq!(day.pt, 10, "日桶只含被折叠的那个小时");
    }

    #[test]
    fn rollup_is_idempotent() {
        let now = ms_in_local("2026-09-30", 12);
        let old = now - (HOURLY_KEEP_HOURS + 1) * 3_600_000;
        let mut recorder = UsageRecorder::load(None);
        recorder.add(old, Region::Cn, "u1", "m", 10, 10, 100, None, true);
        recorder.rollup(now);
        let before: Vec<_> = recorder.buckets.values().map(|b| (b.clone().scope, b.tt)).collect();
        recorder.rollup(now);
        let after: Vec<_> = recorder.buckets.values().map(|b| (b.clone().scope, b.tt)).collect();
        assert_eq!(before, after, "重复折叠不得重复计数");
    }

    #[test]
    fn roundtrip_through_disk() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("usage_test_{}.json", std::process::id()));
        {
            let now = ms_in_local("2026-09-30", 12);
            let mut recorder = UsageRecorder::load(Some(path.clone()));
            recorder.add(now, Region::Cn, "u1", "m", 7, 3, 100, Some(42.0), true);
        }
        let reloaded = UsageRecorder::load(Some(path.clone()));
        let snap = snapshot(&reloaded, 0, &RegionFilter::All, ms_in_local("2026-09-30", 13));
        assert_eq!(snap.totals.prompt_tokens, 7);
        let _ = std::fs::remove_file(&path);
    }
}
