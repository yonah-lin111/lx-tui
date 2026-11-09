use super::*;
use std::path::PathBuf;

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("lx-logging-{label}-{}-{nanos}", std::process::id()))
}

#[test]
fn filter_directives_keep_dependencies_at_warn() {
    assert_eq!(filter_directives("debug"), "lx=debug,lx_tui=debug,warn");
    assert_eq!(filter_directives("off"), "lx=off,lx_tui=off,warn");
    assert_eq!(filter_directives("INFO"), "lx=info,lx_tui=info,warn");
    assert_eq!(filter_directives("bogus"), "lx=info,lx_tui=info,warn");
}

#[test]
fn normalize_level_accepts_known_values_and_falls_back() {
    for level in LEVELS {
        assert_eq!(normalize_level(level), level);
        assert_eq!(normalize_level(&level.to_ascii_uppercase()), level);
    }
    assert_eq!(normalize_level("  debug  "), "debug");
    assert_eq!(normalize_level("verbose"), "info");
}

#[test]
fn env_bool_parses_common_values() {
    for value in ["1", "true", "TRUE", "on", "yes"] {
        assert_eq!(parse_env_bool(value), Some(true), "{value}");
    }
    for value in ["0", "false", "OFF", "no"] {
        assert_eq!(parse_env_bool(value), Some(false), "{value}");
    }
    assert_eq!(parse_env_bool("maybe"), None);
}

#[test]
fn request_ids_are_unique_and_well_formed() {
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..1_000 {
        let id = new_request_id();
        assert_eq!(id.len(), 12, "{id}");
        assert!(id.starts_with("req-"), "{id}");
        assert!(id[4..].chars().all(|ch| ch.is_ascii_hexdigit()), "{id}");
        assert!(seen.insert(id));
    }
}

#[test]
fn utc_dates_round_trip() {
    assert_eq!(utc_date_from_unix_days(0), (1970, 1, 1));
    assert_eq!(days_from_civil((1970, 1, 1)), 0);
    let cases = [(2026, 11, 10), (2000, 2, 29), (1999, 12, 31)];
    for date in cases {
        assert_eq!(utc_date_from_unix_days(days_from_civil(date)), date);
    }
}

#[test]
fn cleanup_removes_only_expired_matching_files() {
    let dir = temp_dir("cleanup");
    std::fs::create_dir_all(&dir).expect("create dir");
    let today = utc_today();
    let old = utc_date_from_unix_days(days_from_civil(today) - 8);
    let recent = utc_date_from_unix_days(days_from_civil(today) - 3);
    for name in [
        format!("lx-tui.log.{}", format_date(old)),
        format!("payload-{}.jsonl", format_date(old)),
        format!("lx-tui.log.{}", format_date(recent)),
        format!("payload-{}.jsonl", format_date(today)),
        "notes.txt".to_string(),
    ] {
        std::fs::write(dir.join(name), "x").expect("write");
    }
    cleanup_old_logs(&dir, today, RETENTION_DAYS);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("read")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!names.contains(&format!("lx-tui.log.{}", format_date(old))));
    assert!(!names.contains(&format!("payload-{}.jsonl", format_date(old))));
    assert!(names.contains(&format!("lx-tui.log.{}", format_date(recent))));
    assert!(names.contains(&format!("payload-{}.jsonl", format_date(today))));
    assert!(names.contains(&"notes.txt".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_log_date_handles_both_prefixes() {
    assert_eq!(
        parse_log_date("lx-tui.log.2026-11-10"),
        Some((2026, 11, 10))
    );
    assert_eq!(
        parse_log_date("payload-2026-11-10.jsonl"),
        Some((2026, 11, 10))
    );
    assert_eq!(parse_log_date("payload-2026-11-10.json"), None);
    assert_eq!(parse_log_date("notes.txt"), None);
}
