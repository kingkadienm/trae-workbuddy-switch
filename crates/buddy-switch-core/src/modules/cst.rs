//! CST（Asia/Shanghai，固定 UTC+8）时间助手 —— **全仓唯一实现**。
//!
//! 参考实现把「小时窗口」「自然日锚点」统一按固定 +8 计算（`time.FixedZone("CST", 8*3600)`），
//! 不依赖部署机本地时区；本项目面向国内用户且需要可复现单测，故同样**固定 UTC+8**。
//!
//! 依赖方向：`buddy-switch-gateway` 通过 `pub use` 再导出本模块（见其 `timeutil.rs`），
//! 因此**改这里等于同时改网关**。历史背景：网关曾先行自实现一份等价助手，core 后来
//! 因「签到跨天 / 夜猫窗口 / 冷却跨天」也需要同一口径而复制了一份，两份实现存在漂移
//! 风险；现收敛为「core 为唯一来源，网关只再导出」，避免同一个时区口径有两种写法。

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};

/// 中国标准时间偏移（秒）。
pub const CST_OFFSET_SECONDS: i32 = 8 * 3600;

/// CST 固定偏移。字面量恒定合法，故 `expect` 不会触发。
pub fn cst_offset() -> FixedOffset {
    FixedOffset::east_opt(CST_OFFSET_SECONDS).expect("UTC+8 是合法偏移")
}

/// 毫秒时间戳 → CST 本地时间。
///
/// 越界（`|ms|` 超出 chrono 可表示范围）时回落到 epoch，而不是 panic——面向不可信的
/// 客户端时钟输入，不应因一个畸形时间戳让进程崩溃。
pub fn cst_datetime(now_ms: i64) -> DateTime<FixedOffset> {
    let utc = DateTime::<Utc>::from_timestamp_millis(now_ms)
        .or_else(|| DateTime::<Utc>::from_timestamp_millis(0))
        .expect("epoch 在 chrono 可表示范围内");
    utc.with_timezone(&cst_offset())
}

/// 毫秒时间戳 → CST 的 `(年, 月, 日)` 自然日。
pub fn cst_date(now_ms: i64) -> NaiveDate {
    cst_datetime(now_ms).date_naive()
}

/// 毫秒时间戳 → CST 的 `YYYY-MM-DD` 字符串（用于按自然日幂等）。
pub fn cst_date_str(now_ms: i64) -> String {
    cst_date(now_ms).format("%Y-%m-%d").to_string()
}

/// 毫秒时间戳 → CST 小时（0–23）。
pub fn cst_hour(now_ms: i64) -> u32 {
    cst_datetime(now_ms).hour()
}

/// 毫秒时间戳 → CST 的 `NaiveDateTime`（用于 `2006-01-02 15:04:05` 形态的上游报文）。
pub fn cst_naive(now_ms: i64) -> NaiveDateTime {
    cst_datetime(now_ms).naive_local()
}

/// 毫秒时间戳 → CST 的**昨日** `YYYY-MM-DD`（补签卡 `target_date` 用）。
///
/// 连登断档只可能发生在「上一个自然日」（今日尚未结算），故补签判据固定盯昨日。
pub fn cst_yesterday_str(now_ms: i64) -> String {
    let today = cst_date(now_ms);
    today
        .pred_opt()
        .unwrap_or(today)
        .format("%Y-%m-%d")
        .to_string()
}

/// 夜猫窗口判定：CST 23:00–08:00。
pub fn in_night_window(now_ms: i64) -> bool {
    let hour = cst_hour(now_ms);
    !(8..23).contains(&hour)
}

/// 某 CST 自然日某个整点（`hour:00:00`）的毫秒时间戳；`hour` 越界（>23）返回 `None`。
pub fn cst_hour_ms(date: NaiveDate, hour: u32) -> Option<i64> {
    cst_hm_ms(date, hour, 0)
}

/// 某 CST 自然日某个「时:分」（`hour:minute:00`）的毫秒时间戳；任一越界返回 `None`。
///
/// 排程需要分钟粒度（issue #27）后新增：`cst_hour_ms` 只能表达整点，直接补一个 `minute`
/// 参数比在调用方手工加偏移更不容易错（手加偏移会绕过 `and_hms_opt` 的边界校验）。
pub fn cst_hm_ms(date: NaiveDate, hour: u32, minute: u32) -> Option<i64> {
    let naive = date.and_hms_opt(hour, minute, 0)?;
    cst_offset()
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp_millis())
}

/// 「次日 04:00」锚点的毫秒时间戳（CST）。
///
/// 语义与参考实现 `CooldownUntilTomorrow4AM` 一致：
/// - 当前 CST 小时 `< 4` → **当天** 04:00；
/// - 否则 → **次日** 04:00。
///
/// 用于账号池的 402（余额不足）硬冷却。跨月 / 跨年由 `NaiveDate::succ_opt` 处理，
/// 不做手工进位（手工进位是这类日期代码最常见的错源）。
pub fn next_day_4am(now_ms: i64) -> i64 {
    let local = cst_datetime(now_ms);
    let today = local.date_naive();
    let target = if local.hour() < 4 {
        today
    } else {
        today.succ_opt().unwrap_or(today)
    };
    let naive = target.and_hms_opt(4, 0, 0).expect("04:00:00 是合法时刻");
    cst_offset()
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(now_ms)
}

/// 次日 00:00（CST）的毫秒时间戳。
///
/// 用于「内容拦截降级门」的重置时点：降级期固定到次日零点结束，**不续期**
/// （同一降级期内再次触发不延长截止时间），避免被持续拦截时降级期无限滚动、
/// 永久偏离用户配置。
pub fn next_midnight(now_ms: i64) -> i64 {
    let local = cst_datetime(now_ms);
    let today = local.date_naive();
    let target = today.succ_opt().unwrap_or(today);
    let naive = target.and_hms_opt(0, 0, 0).expect("00:00:00 是合法时刻");
    cst_offset()
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(now_ms)
}

/// 构造 CST 某时刻的毫秒时间戳（仅供单测使用，避免每个测试模块各写一份）。
#[cfg(test)]
pub(crate) fn cst_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    let naive = NaiveDate::from_ymd_opt(year, month, day)
        .expect("合法日期")
        .and_hms_opt(hour, minute, 0)
        .expect("合法时刻");
    cst_offset()
        .from_local_datetime(&naive)
        .single()
        .expect("无歧义 CST 时刻")
        .timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cst_date_and_hour_use_utc8_not_local() {
        // UTC 2026-09-15 20:00 == CST 2026-09-16 04:00
        let utc_ms = Utc
            .with_ymd_and_hms(2026, 9, 15, 20, 0, 0)
            .single()
            .expect("合法 UTC")
            .timestamp_millis();
        assert_eq!(cst_date_str(utc_ms), "2026-09-16");
        assert_eq!(cst_hour(utc_ms), 4);
    }

    #[test]
    fn yesterday_str_is_one_day_before_cst_date() {
        assert_eq!(cst_yesterday_str(cst_ms(2026, 9, 16, 10, 0)), "2026-09-15");
        assert_eq!(cst_yesterday_str(cst_ms(2026, 10, 1, 10, 0)), "2026-09-30");
    }

    #[test]
    fn cst_hour_ms_rejects_out_of_range_hour() {
        let date = cst_date(cst_ms(2026, 9, 16, 0, 0));
        assert_eq!(cst_hour_ms(date, 23), Some(cst_ms(2026, 9, 16, 23, 0)));
        assert_eq!(cst_hour_ms(date, 24), None);
    }

    /// 分钟粒度：整点只是 `minute == 0` 的特例，且越界分必须被拒（而不是静默进到下一小时）。
    #[test]
    fn cst_hm_ms_honours_minute_and_rejects_out_of_range() {
        let date = cst_date(cst_ms(2026, 9, 16, 0, 0));
        assert_eq!(cst_hm_ms(date, 9, 37), Some(cst_ms(2026, 9, 16, 9, 37)));
        assert_eq!(cst_hm_ms(date, 9, 0), cst_hour_ms(date, 9));
        // 边界：23:59 合法，24:00 / 09:60 非法（后者若被放行会被 chrono 进位成 10:00）。
        assert_eq!(cst_hm_ms(date, 23, 59), Some(cst_ms(2026, 9, 16, 23, 59)));
        assert_eq!(cst_hm_ms(date, 24, 0), None);
        assert_eq!(cst_hm_ms(date, 9, 60), None);
    }

    #[test]
    fn cst_naive_formats_expected_wire_string() {
        let now = cst_ms(2026, 9, 16, 14, 26);
        assert_eq!(
            cst_naive(now).format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-09-16 14:26:00"
        );
    }

    #[test]
    fn cst_datetime_falls_back_to_epoch_on_out_of_range_input() {
        // 面向不可信输入：越界时间戳必须回落而不是 panic
        let out_of_range = i64::MAX;
        assert_eq!(cst_date_str(out_of_range), "1970-01-01");
        assert_eq!(cst_hour(out_of_range), 8);
    }

    #[test]
    fn before_4am_targets_same_day() {
        let now = cst_ms(2026, 9, 16, 3, 30);
        assert_eq!(next_day_4am(now), cst_ms(2026, 9, 16, 4, 0));
    }

    #[test]
    fn exactly_4am_targets_next_day() {
        // hour<4 为严格小于：04:00 整点归入「次日」，与参考实现一致
        let now = cst_ms(2026, 9, 16, 4, 0);
        assert_eq!(next_day_4am(now), cst_ms(2026, 9, 17, 4, 0));
    }

    #[test]
    fn after_4am_targets_next_day() {
        let now = cst_ms(2026, 9, 16, 14, 26);
        assert_eq!(next_day_4am(now), cst_ms(2026, 9, 17, 4, 0));
    }

    #[test]
    fn next_day_4am_crosses_month_and_year_boundaries() {
        assert_eq!(
            next_day_4am(cst_ms(2026, 9, 30, 23, 59)),
            cst_ms(2026, 10, 1, 4, 0)
        );
        assert_eq!(
            next_day_4am(cst_ms(2026, 12, 31, 20, 0)),
            cst_ms(2027, 1, 1, 4, 0)
        );
    }

    #[test]
    fn next_day_4am_handles_leap_day() {
        assert_eq!(
            next_day_4am(cst_ms(2028, 2, 28, 23, 30)),
            cst_ms(2028, 2, 29, 4, 0)
        );
    }

    #[test]
    fn night_window_boundaries() {
        assert!(in_night_window(cst_ms(2026, 9, 16, 23, 0)));
        assert!(in_night_window(cst_ms(2026, 9, 17, 0, 0)));
        assert!(in_night_window(cst_ms(2026, 9, 17, 7, 59)));
        assert!(!in_night_window(cst_ms(2026, 9, 17, 8, 0)));
        assert!(!in_night_window(cst_ms(2026, 9, 16, 12, 0)));
        assert!(!in_night_window(cst_ms(2026, 9, 16, 22, 59)));
    }

    #[test]
    fn next_midnight_lands_on_following_cst_day() {
        assert_eq!(
            next_midnight(cst_ms(2026, 9, 16, 14, 26)),
            cst_ms(2026, 9, 17, 0, 0)
        );
        // 已过 00:00 但仍是同一天 → 次日零点
        assert_eq!(
            next_midnight(cst_ms(2026, 9, 16, 0, 1)),
            cst_ms(2026, 9, 17, 0, 0)
        );
        // 月末跨月
        assert_eq!(
            next_midnight(cst_ms(2026, 9, 30, 23, 59)),
            cst_ms(2026, 10, 1, 0, 0)
        );
    }
}
