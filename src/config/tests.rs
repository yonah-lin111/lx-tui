use super::*;

use std::time::{SystemTime, UNIX_EPOCH};

/// 隔离临时目录：按测试名、进程号与时间戳唯一。
fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-config-tests-{}-{}-{}",
        name,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

#[test]
fn default_round_trips_through_json() {
    let config = Config::default();
    let text = config.to_json();
    let parsed = Config::from_json(&text).expect("default json parses");
    assert_eq!(parsed, config);
}

#[test]
fn missing_keys_fall_back_to_defaults() {
    let parsed = Config::from_json(r#"{"sidebar_width": 30}"#).expect("partial json parses");
    assert_eq!(parsed.sidebar_width, 30);
    assert_eq!(parsed.sidebar_min_width, 18);
    assert_eq!(parsed.min_height, 10);
}

#[test]
fn unknown_keys_are_ignored() {
    let parsed = Config::from_json(r#"{"future_option": true, "min_width": 50}"#)
        .expect("unknown key ignored");
    assert_eq!(parsed.min_width, 50);
}

#[test]
fn invalid_values_are_rejected() {
    // 非数字、越界、负值都可能出自手写文件。
    assert!(Config::from_json(r#"{"sidebar_width": "wide"}"#).is_err());
    assert!(Config::from_json(r#"{"sidebar_width": -1}"#).is_err());
    assert!(Config::from_json(r#"{"sidebar_width": 70000}"#).is_err());
    assert!(Config::from_json(r#"{"min_width": 1.5}"#).is_err());
    assert!(Config::from_json(r#"{"sidebar_width": 4}"#).is_err());
    assert!(Config::from_json("{").is_err());
    assert!(Config::from_json("[]").is_err());
}

#[test]
fn validate_enforces_width_relationships() {
    assert!(
        Config {
            sidebar_width: 99,
            ..Config::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        Config {
            sidebar_min_width: 41,
            ..Config::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        Config {
            sidebar_min_width: 0,
            ..Config::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        Config {
            min_pane_width: 0,
            ..Config::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn load_missing_file_returns_default() {
    let dir = temp_dir("load-missing");
    let path = dir.join("settings.json");
    let config = load_or_default(&path).expect("missing file is not an error");
    assert_eq!(config, Config::default());
}

#[test]
fn load_malformed_file_reports_error() {
    let dir = temp_dir("load-malformed");
    let path = dir.join("settings.json");
    std::fs::write(&path, "{ not json").expect("write");
    assert!(load_or_default(&path).is_err());
}

#[test]
fn save_then_load_round_trip_and_no_temp_left() {
    let dir = temp_dir("save-round-trip");
    let path = dir.join("settings.json");
    let config = Config {
        sidebar_width: 32,
        narrow_width: 90,
        ..Config::default()
    };
    save(&path, &config).expect("save ok");
    let loaded = load_or_default(&path).expect("load ok");
    assert_eq!(loaded, config);
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "temp files cleaned: {leftovers:?}");
}

#[test]
fn save_rejects_invalid_config_before_writing() {
    let dir = temp_dir("save-invalid");
    let path = dir.join("settings.json");
    let config = Config {
        sidebar_min_width: 99,
        ..Config::default()
    };
    assert!(save(&path, &config).is_err());
    assert!(!path.exists(), "invalid config must not be written");
}

#[test]
fn settings_path_shares_user_data_root() {
    // HOME 存在时路径以 `.lx-t` 结尾；不依赖具体 home 值。
    let Ok(path) = settings_path() else {
        return;
    };
    assert!(path.ends_with("settings.json"));
    let root = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str());
    assert!(root == Some(".lx-t") || root == Some(".lx-t-dev"));
}
