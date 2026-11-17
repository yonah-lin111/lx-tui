//! `chat::sandbox::permissions` 测试：基准、具体度、deny 优先、glob 与请求校验。

use std::path::PathBuf;

use super::*;
use crate::config::sandbox::SandboxMode;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-permissions-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonical temp dir")
}

#[test]
fn baseline_is_read_all_and_write_workspace() {
    let cwd = temp_dir("baseline");
    std::fs::create_dir_all(cwd.join(".git")).expect("git dir");
    let policy = FileSystemPolicy::baseline(SandboxMode::WorkspaceWrite, &cwd, &[]);
    assert_eq!(
        policy.access_for(Path::new("/etc/hosts")),
        FileSystemAccessMode::Read
    );
    assert_eq!(
        policy.access_for(&cwd.join("src/main.rs")),
        FileSystemAccessMode::Write
    );
    assert_eq!(
        policy.access_for(&cwd.join(".git/config")),
        FileSystemAccessMode::Read
    );
    assert!(policy.writable_roots().contains(&cwd));
    assert!(policy.protected_paths().contains(&cwd.join(".git")));
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn read_only_has_no_writable_roots() {
    let cwd = temp_dir("readonly");
    let policy = FileSystemPolicy::baseline(SandboxMode::ReadOnly, &cwd, &[]);
    assert_eq!(
        policy.access_for(&cwd.join("x")),
        FileSystemAccessMode::Read
    );
    assert!(policy.writable_roots().is_empty());
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn literal_path_beats_glob_and_deny_beats_write_on_ties() {
    let cwd = temp_dir("specificity");
    let policy = FileSystemPolicy::baseline(SandboxMode::WorkspaceWrite, &cwd, &[])
        .with_request(&FileSystemRequest {
            read: vec![format!("{}/secret", cwd.display())],
            write: vec![format!("{}/secret/deep", cwd.display())],
            deny: vec![format!("{}/secret/deep/blocked.txt", cwd.display())],
        })
        .expect("request");

    // 具体字面 deny 优先于上位 write。
    assert_eq!(
        policy.access_for(&cwd.join("secret/deep/blocked.txt")),
        FileSystemAccessMode::Deny
    );
    // 更具体的 write 覆盖较浅的 read 条目。
    assert_eq!(
        policy.access_for(&cwd.join("secret/deep/ok.txt")),
        FileSystemAccessMode::Write
    );
    // 其余仍由 read 条目限制。
    assert_eq!(
        policy.access_for(&cwd.join("secret/other.txt")),
        FileSystemAccessMode::Read
    );
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn glob_deny_matches_without_crossing_separators() {
    let cwd = temp_dir("glob");
    let policy = FileSystemPolicy::baseline(SandboxMode::WorkspaceWrite, &cwd, &[])
        .with_request(&FileSystemRequest {
            deny: vec![format!("{}/**/*.log", cwd.display())],
            ..FileSystemRequest::default()
        })
        .expect("request");
    assert_eq!(policy.deny_globs().len(), 1);
    let matcher = compile_glob(&policy.deny_globs()[0]).expect("matcher");
    assert!(matcher.is_match(cwd.join("a.log")));
    assert!(matcher.is_match(cwd.join("nested/a.log")));
    assert!(!matcher.is_match(cwd.join("a.log.txt")));
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn entries_require_absolute_paths_and_reject_home_and_control() {
    for raw in ["relative", "~/x", "/tmp/a\nb", ""] {
        assert!(parse_entry(raw).is_err(), "entry {raw:?}");
    }
    assert!(matches!(
        parse_entry("/tmp/**/*.md").expect("glob"),
        FileSystemPath::Glob(_)
    ));
    assert!(matches!(
        parse_entry("/tmp/notes.md").expect("path"),
        FileSystemPath::Path(_)
    ));
}

#[test]
fn request_limits_are_enforced() {
    let too_many = FileSystemRequest {
        read: (0..=MAX_REQUEST_ENTRIES)
            .map(|index| format!("/tmp/read-{index}"))
            .collect(),
        ..FileSystemRequest::default()
    };
    assert!(too_many.validate().is_err());

    let too_long = FileSystemRequest {
        write: vec![format!("/tmp/{}", "a".repeat(MAX_REQUEST_BYTES))],
        ..FileSystemRequest::default()
    };
    assert!(too_long.validate().is_err());

    let write_glob = FileSystemRequest {
        write: vec!["/tmp/out/**".into()],
        ..FileSystemRequest::default()
    };
    assert!(write_glob.validate().is_err(), "write 不接受 glob");
    let read_glob = FileSystemRequest {
        read: vec!["/tmp/src/**".into()],
        ..FileSystemRequest::default()
    };
    assert!(read_glob.validate().is_err(), "read 不接受 glob");

    let denies_only = FileSystemRequest {
        deny: vec!["/tmp/secret".into()],
        ..FileSystemRequest::default()
    };
    assert!(denies_only.denies_only());
    assert!(
        AdditionalPermissions {
            network: None,
            file_system: Some(denies_only),
        }
        .denies_only()
    );
    assert!(
        !AdditionalPermissions {
            network: Some(true),
            file_system: None,
        }
        .denies_only()
    );
}

#[test]
fn additional_deny_shrinks_existing_write_access() {
    let cwd = temp_dir("override");
    let base = FileSystemPolicy::baseline(SandboxMode::WorkspaceWrite, &cwd, &[]);
    let deny = format!("{}/secret", cwd.display());
    let policy = base
        .with_request(&FileSystemRequest {
            deny: vec![deny.clone()],
            ..FileSystemRequest::default()
        })
        .expect("request");
    assert_eq!(
        policy.access_for(&cwd.join("secret/x")),
        FileSystemAccessMode::Deny
    );
    assert_eq!(
        base.access_for(&cwd.join("secret/x")),
        FileSystemAccessMode::Write
    );
    assert!(!policy.writable_roots().is_empty());
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn deny_is_absolute_over_deeper_write() {
    let cwd = temp_dir("deny-absolute");
    let policy = FileSystemPolicy::baseline(SandboxMode::WorkspaceWrite, &cwd, &[])
        .with_request(&FileSystemRequest {
            write: vec![format!("{}/out", cwd.display())],
            deny: vec![format!("{}/**", cwd.display())],
            ..FileSystemRequest::default()
        })
        .expect("request");
    assert_eq!(
        policy.access_for(&cwd.join("out/x.txt")),
        FileSystemAccessMode::Deny,
        "Deny 命中的路径不能被更具体的 Write 重新打开"
    );
    let _ = std::fs::remove_dir_all(cwd);
}
