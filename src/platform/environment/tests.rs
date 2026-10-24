//! 环境事实采集的测试：白名单化、UTC 兜底与有界网络探测。

use super::*;

/// UTC 兜底必须给出稳定格式的日期时间（不依赖本机时区）。
#[test]
fn utc_fallback_formats_date_and_time() {
    let (date, time) = utc_datetime();
    assert_eq!(date.len(), 10, "日期格式必须是 YYYY-MM-DD：{date}");
    assert_eq!(time.len(), 5, "时间格式必须是 HH:MM：{time}");
    let parts: Vec<&str> = date.split('-').collect();
    assert_eq!(parts.len(), 3);
    assert!(
        parts[0].parse::<i32>().is_ok_and(|year| year >= 1970),
        "年份必须可解析：{date}"
    );
    let hour: u32 = time[..2].parse().expect("hour");
    assert!(hour < 24, "小时越界：{time}");
}

/// `civil_from_days` 的固定样本（含闰年与世纪边界）。
#[test]
fn civil_from_days_matches_known_dates() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
}

/// 终端类型只保留安全字符并限长；非法 TERM 被过滤为空即视为未知。
#[test]
fn terminal_kind_filters_unsafe_characters() {
    assert_eq!(
        super::terminal_kind_with(Some("xterm-256color".to_string())),
        Some("xterm-256color".to_string())
    );
    // 控制字符/空格/引号被丢弃，不会伪造结构。
    assert_eq!(
        super::terminal_kind_with(Some("bad\u{1b}[31m term".to_string())),
        Some("bad31mterm".to_string())
    );
    let long = "a".repeat(MAX_TERM_CHARS + 20);
    assert_eq!(
        super::terminal_kind_with(Some(long))
            .map(|value| value.chars().count())
            .unwrap_or(0),
        MAX_TERM_CHARS
    );
    assert_eq!(super::terminal_kind_with(Some(String::new())), None);
    assert_eq!(super::terminal_kind_with(None), None);
}

/// 采集入口必须给出非空 OS/架构/shell，且不会 panic。
#[test]
fn capture_reports_core_facts() {
    let facts = EnvironmentFacts::capture();
    assert!(!facts.os.is_empty());
    assert!(!facts.arch.is_empty());
    assert!(!facts.shell.is_empty());
    assert_eq!(facts.os, os_name());
    assert_eq!(facts.date.len(), 10);
    assert_eq!(facts.time.len(), 5);
}

/// 网络探测必须返回确定结论（有界），不阻塞超过探测上限太多。
#[test]
fn network_probe_is_bounded() {
    let started = Instant::now();
    let available = network_available();
    assert!(available.is_some(), "探测必须给出确定结论");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "网络探测必须是有界的：{:?}",
        started.elapsed()
    );
}
