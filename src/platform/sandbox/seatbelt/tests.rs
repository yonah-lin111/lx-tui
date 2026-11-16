//! `platform::sandbox::seatbelt` 测试：profile 结构、`-D` 参数与真实写入阻断。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-seatbelt-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonical temp dir")
}

fn profile_for(roots: &[PathBuf], protected: &[PathBuf], network: bool) -> String {
    let spec = SandboxSpec {
        cwd: Path::new("/"),
        writable_roots: roots,
        protected,
        network_access: network,
    };
    build_profile(&spec).expect("build profile").0
}

#[test]
fn profile_has_base_read_write_and_hardening() {
    let root = temp_dir("profile");
    let protected = vec![root.join(".git")];
    let profile = profile_for(std::slice::from_ref(&root), &protected, false);
    assert!(profile.contains("(deny default)"));
    assert!(profile.contains("(allow file-read*)"));
    assert!(profile.contains("(subpath (param \"WRITE0\"))"));
    assert!(profile.contains("(require-not (literal (param \"PROTECT0\")))"));
    assert!(profile.contains("(require-not (subpath (param \"PROTECT0\")))"));
    assert!(profile.contains("(deny file-write-unlink"));
    assert!(profile.contains("(deny mach-lookup (xpc-service-name-prefix \"\"))"));
    assert!(profile.contains("(deny system-fcntl (fcntl-command 80 110))"));
    assert!(!profile.contains("network-outbound"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_only_profile_has_no_write_rule_and_network_is_opt_in() {
    let profile = profile_for(&[], &[], true);
    assert!(!profile.contains("(allow file-write* (require-all"));
    assert!(profile.contains("(allow network-outbound)"));
    assert!(profile.contains("(allow network-inbound)"));
}

#[test]
fn path_with_spaces_is_passed_via_dash_d() {
    let root = temp_dir("space").join("with space");
    std::fs::create_dir_all(&root).expect("space dir");
    let spec = SandboxSpec {
        cwd: &root,
        writable_roots: std::slice::from_ref(&root),
        protected: &[],
        network_access: false,
    };
    let wrapped = wrap(&spec, OsStr::new("/bin/sh"), &[]).expect("wrap");
    let param = wrapped
        .args
        .iter()
        .find(|arg| arg.to_string_lossy().starts_with("-DWRITE0="))
        .expect("write param");
    assert!(param.to_string_lossy().ends_with("with space"));
    let _ = std::fs::remove_dir_all(root.parent().expect("parent"));
}

#[test]
fn control_characters_are_rejected() {
    let root = PathBuf::from("/tmp/bad\npath");
    let spec = SandboxSpec {
        cwd: &root,
        writable_roots: std::slice::from_ref(&root),
        protected: &[],
        network_access: false,
    };
    assert!(matches!(
        build_profile(&spec),
        Err(SandboxError::Invalid(_))
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn sandbox_exec_blocks_writes_outside_the_root() {
    let base = temp_dir("real");
    let root = base.join("ws");
    std::fs::create_dir_all(&root).expect("workspace");
    let outside = base.join("outside.txt");
    let spec = SandboxSpec {
        cwd: &root,
        writable_roots: std::slice::from_ref(&root),
        protected: &[],
        network_access: false,
    };
    let script = format!(
        "printf ok > '{}' ; printf bad > '{}'",
        root.join("inside.txt").display(),
        outside.display()
    );
    let wrapped = wrap(
        &spec,
        OsStr::new("/bin/sh"),
        &[OsString::from("-c"), OsString::from(script)],
    )
    .expect("wrap");
    let status = std::process::Command::new(&wrapped.program)
        .args(&wrapped.args)
        .status()
        .expect("run sandbox");
    assert!(root.join("inside.txt").exists());
    assert!(!outside.exists());
    assert!(!status.success());
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(target_os = "macos")]
#[test]
fn read_only_profile_blocks_workspace_writes() {
    let root = temp_dir("readonly");
    let spec = SandboxSpec {
        cwd: &root,
        writable_roots: &[],
        protected: &[],
        network_access: false,
    };
    let script = format!("printf nope > '{}'", root.join("blocked.txt").display());
    let wrapped = wrap(
        &spec,
        OsStr::new("/bin/sh"),
        &[OsString::from("-c"), OsString::from(script)],
    )
    .expect("wrap");
    let status = std::process::Command::new(&wrapped.program)
        .args(&wrapped.args)
        .status()
        .expect("run sandbox");
    assert!(!root.join("blocked.txt").exists());
    assert!(!status.success());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(target_os = "macos")]
#[test]
fn protected_git_path_cannot_be_modified() {
    let root = temp_dir("protected");
    std::fs::create_dir_all(root.join(".git")).expect("git dir");
    std::fs::write(root.join(".git/config"), "original").expect("seed");
    let protected = vec![root.join(".git")];
    let spec = SandboxSpec {
        cwd: &root,
        writable_roots: std::slice::from_ref(&root),
        protected: &protected,
        network_access: false,
    };
    let script = format!(
        "printf hacked > '{}'",
        root.join(".git").join("config").display()
    );
    let wrapped = wrap(
        &spec,
        OsStr::new("/bin/sh"),
        &[OsString::from("-c"), OsString::from(script)],
    )
    .expect("wrap");
    let status = std::process::Command::new(&wrapped.program)
        .args(&wrapped.args)
        .status()
        .expect("run sandbox");
    assert_eq!(
        std::fs::read_to_string(root.join(".git/config")).expect("read"),
        "original"
    );
    assert!(!status.success());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sandbox_exec_path_is_fixed() {
    assert_eq!(SANDBOX_EXEC, "/usr/bin/sandbox-exec");
    assert!(Path::new(SANDBOX_EXEC).exists());
}
