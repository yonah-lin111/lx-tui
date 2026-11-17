//! `config::sandbox` 单元测试：默认值、往返、绝对路径校验、`~` 展开与 0600 权限。

use std::path::PathBuf;

use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-sandbox-cfg-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn short_labels_are_compact_english_names() {
    assert_eq!(SandboxMode::ReadOnly.short_label(), "Read-only");
    assert_eq!(SandboxMode::WorkspaceWrite.short_label(), "Workspace");
    assert_eq!(SandboxMode::DangerFullAccess.short_label(), "Full");
}

#[test]
fn defaults_are_workspace_write_without_network_and_ask_escalation() {
    let settings = SandboxSettings::default();
    assert_eq!(settings.mode, SandboxMode::WorkspaceWrite);
    assert!(!settings.network_access);
    assert_eq!(settings.escalation, EscalationPolicy::Ask);
    assert!(settings.writable_roots.is_empty());
    assert_eq!(settings, parse_sandbox("{}").expect("empty object"));
}

#[test]
fn missing_file_returns_none() {
    let dir = temp_dir("missing");
    assert_eq!(load_sandbox(&dir.join("sandbox.json")).expect("load"), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn malformed_and_unknown_values_are_rejected() {
    assert!(parse_sandbox("{").is_err());
    assert!(parse_sandbox(r#"{"mode":"readonly"}"#).is_err());
    assert!(parse_sandbox(r#"{"escalation":"always"}"#).is_err());
    // 未知键忽略，与其它 config 文件一致。
    let parsed = parse_sandbox(r#"{"mode":"read-only","future":true}"#).expect("unknown ignored");
    assert_eq!(parsed.mode, SandboxMode::ReadOnly);
}

#[test]
fn round_trip_preserves_values_and_sets_0600() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("config").join("sandbox.json");
    let settings = SandboxSettings {
        mode: SandboxMode::DangerFullAccess,
        network_access: true,
        escalation: EscalationPolicy::Never,
        writable_roots: vec![PathBuf::from("/tmp"), PathBuf::from("/private/tmp")],
    };
    save_sandbox(&path, &settings).expect("save");
    let loaded = load_sandbox(&path).expect("load").expect("file");
    assert_eq!(loaded, settings);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn roots_must_be_absolute_and_are_deduped() {
    assert!(parse_sandbox(r#"{"writable_roots":["relative/path"]}"#).is_err());
    assert!(
        normalize_roots(&[PathBuf::from("/a"), PathBuf::from("/a")])
            .expect("dedupe")
            .len()
            == 1
    );
    let too_many: Vec<PathBuf> = (0..=MAX_WRITABLE_ROOTS)
        .map(|index| PathBuf::from(format!("/tmp/root-{index}")))
        .collect();
    assert!(normalize_roots(&too_many).is_err());
}

#[test]
fn home_expansion_is_absolute() {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(home) = home else {
        return;
    };
    assert_eq!(expand_home(Path::new("~")).expect("home"), home);
    assert_eq!(
        expand_home(Path::new("~/x")).expect("home child"),
        home.join("x")
    );
    assert_eq!(
        expand_home(Path::new("/abs")).expect("abs"),
        PathBuf::from("/abs")
    );
    let parsed = parse_sandbox(r#"{"writable_roots":["~/work"]}"#).expect("expand");
    assert!(parsed.writable_roots[0].is_absolute());
}

#[test]
fn control_characters_are_rejected() {
    let settings = SandboxSettings {
        writable_roots: vec![PathBuf::from("/tmp/a\nb")],
        ..SandboxSettings::default()
    };
    assert!(settings.normalized().is_err());
}

#[test]
fn save_sandbox_expands_tilde_before_validating() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let dir = temp_dir("tilde-save");
    let path = dir.join("config").join("sandbox.json");
    let settings = SandboxSettings {
        writable_roots: vec![PathBuf::from("~/extra")],
        ..SandboxSettings::default()
    };
    save_sandbox(&path, &settings).expect("save tilde root");
    let loaded = load_sandbox(&path).expect("load").expect("file");
    assert_eq!(loaded.writable_roots, vec![home.join("extra")]);
    let _ = std::fs::remove_dir_all(dir);
}
