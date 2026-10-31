//! 待办日程纯函数日期运算（Howard Hinnant 公历算法，无第三方依赖）。
//!
//! 日期统一以本地 `YYYY-MM-DD` 文本键表示，支持按天偏移、所在周 7 日推算与最近 N 日序列。

use std::time::{SystemTime, UNIX_EPOCH};

const WEEKDAY_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAY_INITIAL: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

/// 读取当前本地日期（`YYYY-MM-DD`）；平台不支持本地时间时退回 UTC。
pub fn today_date() -> String {
    crate::platform::local_datetime()
        .map(|(date, _)| date)
        .unwrap_or_else(utc_today_date)
}

/// 读取当前 UTC 日期（`YYYY-MM-DD`）。
pub fn utc_today_date() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format_date(year, month, day)
}

/// 格式化公历年月日为 `YYYY-MM-DD`。
pub fn format_date(year: i32, month: u32, day: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

/// 解析 `YYYY-MM-DD` 字符串为 `(year, month, day)`。
pub fn parse_date(date: &str) -> Option<(i32, u32, u32)> {
    let mut parts = date.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// 公历年月日 → 自 1970-01-01 起的天数（Howard Hinnant `days_from_civil`）。
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - if month <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let d = i64::from(day);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 自 1970-01-01 起的天数 → 公历年月日（Howard Hinnant `civil_from_days`）。
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = i32::try_from(if m <= 2 { y + 1 } else { y }).unwrap_or(1970);
    (year, m, d)
}

/// 按天偏移日期字符串；若输入非法则原样返回。
pub fn shift_date(date: &str, delta_days: i32) -> String {
    let Some((year, month, day)) = parse_date(date) else {
        return date.to_string();
    };
    let days = days_from_civil(year, month, day) + i64::from(delta_days);
    let (ny, nm, nd) = civil_from_days(days);
    format_date(ny, nm, nd)
}

/// 计算星期几索引：0 = 周一 (Mon) .. 6 = 周日 (Sun)。
pub fn weekday_index(date: &str) -> usize {
    let Some((year, month, day)) = parse_date(date) else {
        return 0;
    };
    let days = days_from_civil(year, month, day);
    // 1970-01-01 (days = 0) 是周四 (index = 3)。
    usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0)
}

/// 英文三字母星期缩写（`Mon`..`Sun`）。
pub fn weekday_short(date: &str) -> &'static str {
    WEEKDAY_SHORT[weekday_index(date).min(6)]
}

/// 英文单字母星期缩写（`M`..`S`）。
pub fn weekday_initial(date: &str) -> &'static str {
    WEEKDAY_INITIAL[weekday_index(date).min(6)]
}

/// 提取月份中的日数字（1..=31）。
pub fn day_of_month(date: &str) -> u32 {
    parse_date(date).map(|(_, _, day)| day).unwrap_or(1)
}

/// 返回给定日期所在周（周一至周日）的 7 个日期键。
pub fn week_dates(selected_date: &str) -> [String; 7] {
    let dow = i32::try_from(weekday_index(selected_date)).unwrap_or(0);
    let monday = shift_date(selected_date, -dow);
    std::array::from_fn(|index| {
        let offset = i32::try_from(index).unwrap_or(0);
        shift_date(&monday, offset)
    })
}

/// 返回以 `end_date` 为终点（含）、向前连续 `count` 天的升序日期列表。
pub fn recent_dates(end_date: &str, count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|index| {
            let back = i32::try_from(count - 1 - index).unwrap_or(0);
            shift_date(end_date, -back)
        })
        .collect()
}
