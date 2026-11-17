//! `platform::sandbox::seatbelt` 测试：profile 结构、`-D` 参数与真实读写阻断。

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

#[derive(Default)]
struct Specs {
    roots: Vec<PathBuf>,
    read_only: Vec<PathBuf>,
    deny: Vec<PathBuf>,
    globs: Vec<String>,
    network: bool,
}

fn profile_for(cwd: &Path, specs: &Specs) -> String {
    let spec = SandboxSpec {
        cwd,
        writable_roots: &specs.roots,
        read_only_paths: &specs.read_only,
        deny_paths: &specs.deny,
        deny_globs: &specs.globs,
        glob_scan_max_depth: 16,
        network_access: specs.network,
    };
    build_profile(&spec).expect("build profile").0
}

fn run(cwd: &Path, specs: &Specs, script: &str) -> std::process::ExitStatus {
    let spec = SandboxSpec {
        cwd,
        writable_roots: &specs.roots,
        read_only_paths: &specs.read_only,
        deny_paths: &specs.deny,
        deny_globs: &specs.globs,
        glob_scan_max_depth: 16,
        network_access: specs.network,
    };
    let wrapped = wrap(
        &spec,
        OsStr::new("/bin/sh"),
        &[OsString::from("-c"), OsString::from(script)],
    )
    .expect("wrap");
    std::process::Command::new(&wrapped.program)
        .args(&wrapped.args)
        .status()
        .expect("run sandbox")
}

#[test]
fn profile_has_base_read_write_and_hardening() {
    let root = temp_dir("profile");
    let protected = vec![root.join("nested/.git")];
    let specs = Specs {
        roots: vec![root.clone()],
        read_only: protected,
        ..Specs::default()
    };
    let profile = profile_for(&root, &specs);
    assert!(profile.contains("(deny default)"));
    assert!(profile.contains("(allow file-read*)"));
    assert!(profile.contains("(subpath (param \"WRITE"));
    assert!(profile.contains("(require-not (literal (param \"READONLY"));
    assert!(profile.contains("(require-not (subpath (param \"READONLY"));
    assert!(profile.contains("(deny file-write-unlink"));
    assert!(profile.contains("(deny mach-lookup (xpc-service-name-prefix \"\"))"));
    assert!(!profile.contains("network-outbound"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn deny_paths_and_globs_have_read_and_write_rules() {
    let root = temp_dir("deny-profile");
    let specs = Specs {
        roots: vec![root.clone()],
        deny: vec![root.join("secret")],
        globs: vec![format!("{}/**/.env", root.display())],
        ..Specs::default()
    };
    let profile = profile_for(&root, &specs);
    assert!(profile.contains("(deny file-read* (literal (param \"DENY"));
    assert!(profile.contains("(deny file-write* (literal (param \"DENY"));
    assert!(profile.contains("(deny file-read* (regex #\""));
    assert!(profile.contains("(deny file-write* (regex #\""));
    assert!(profile.contains("(require-not (regex #\""));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_only_profile_has_no_write_rule_and_network_is_opt_in() {
    let root = temp_dir("network");
    let specs = Specs {
        network: true,
        ..Specs::default()
    };
    let profile = profile_for(&root, &specs);
    assert!(!profile.contains("(allow file-write* (require-all"));
    assert!(profile.contains("(allow network-outbound)"));
    assert!(profile.contains("(allow network-inbound)"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn path_with_spaces_is_passed_via_dash_d() {
    let root = temp_dir("space").join("with space");
    std::fs::create_dir_all(&root).expect("space dir");
    let specs = Specs {
        roots: vec![root.clone()],
        ..Specs::default()
    };
    let spec = SandboxSpec {
        cwd: &root,
        writable_roots: &specs.roots,
        read_only_paths: &[],
        deny_paths: &[],
        deny_globs: &[],
        glob_scan_max_depth: 16,
        network_access: false,
    };
    let wrapped = wrap(&spec, OsStr::new("/bin/sh"), &[]).expect("wrap");
    let param = wrapped
        .args
        .iter()
        .find(|arg| arg.to_string_lossy().starts_with("-DWRITE"))
        .expect("write param");
    assert!(param.to_string_lossy().ends_with("with space"));
    let _ = std::fs::remove_dir_all(root.parent().expect("parent"));
}

#[test]
fn control_characters_are_rejected() {
    let root = PathBuf::from("/tmp/bad\npath");
    let specs = Specs {
        roots: vec![root],
        ..Specs::default()
    };
    let spec = SandboxSpec {
        cwd: Path::new("/"),
        writable_roots: &specs.roots,
        read_only_paths: &[],
        deny_paths: &[],
        deny_globs: &[],
        glob_scan_max_depth: 16,
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
    let specs = Specs {
        roots: vec![root.clone()],
        ..Specs::default()
    };
    let script = format!(
        "printf ok > '{}' ; printf bad > '{}'",
        root.join("inside.txt").display(),
        outside.display()
    );
    let status = run(&root, &specs, &script);
    assert!(root.join("inside.txt").exists());
    assert!(!outside.exists());
    assert!(!status.success());
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(target_os = "macos")]
#[test]
fn read_only_profile_blocks_workspace_writes() {
    let root = temp_dir("readonly");
    let specs = Specs::default();
    let script = format!("printf nope > '{}'", root.join("blocked.txt").display());
    let status = run(&root, &specs, &script);
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
    let specs = Specs {
        roots: vec![root.clone()],
        read_only: vec![root.join(".git")],
        ..Specs::default()
    };
    let script = format!(
        "printf hacked > '{}'",
        root.join(".git").join("config").display()
    );
    let status = run(&root, &specs, &script);
    assert_eq!(
        std::fs::read_to_string(root.join(".git/config")).expect("read"),
        "original"
    );
    assert!(!status.success());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(target_os = "macos")]
#[test]
fn deny_literal_blocks_read_and_write_but_keeps_siblings() {
    let root = temp_dir("deny-real");
    let secret = root.join("secret");
    std::fs::create_dir_all(&secret).expect("secret dir");
    std::fs::write(secret.join("key.txt"), "hunter2").expect("seed");
    let specs = Specs {
        roots: vec![root.clone()],
        deny: vec![secret.clone()],
        ..Specs::default()
    };
    let script = format!(
        "cat '{secret}/key.txt' > '{root}/leak.txt'; printf x > '{root}/ok.txt'",
        secret = secret.display(),
        root = root.display()
    );
    let _status = run(&root, &specs, &script);
    assert_eq!(
        std::fs::read_to_string(root.join("leak.txt")).expect("redirection file"),
        "",
        "deny 读必须生效"
    );
    assert!(root.join("ok.txt").exists(), "兄弟路径仍可写");
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(target_os = "macos")]
#[test]
fn deny_glob_blocks_env_files_across_directories() {
    let root = temp_dir("deny-glob");
    std::fs::create_dir_all(root.join("a")).expect("nested");
    std::fs::write(root.join(".env"), "secret").expect("root env");
    std::fs::write(root.join("a/.env"), "secret").expect("nested env");
    let specs = Specs {
        roots: vec![root.clone()],
        globs: vec![format!("{}/**/.env", root.display())],
        ..Specs::default()
    };
    let script = format!(
        "cat '{root}/.env' > '{root}/leak1.txt'; cat '{root}/a/.env' > '{root}/leak2.txt'; printf ok > '{root}/a/ok.txt'",
        root = root.display()
    );
    let _status = run(&root, &specs, &script);
    assert_eq!(
        std::fs::read_to_string(root.join("leak1.txt")).expect("leak1"),
        ""
    );
    assert_eq!(
        std::fs::read_to_string(root.join("leak2.txt")).expect("leak2"),
        ""
    );
    assert!(root.join("a/ok.txt").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(target_os = "macos")]
#[test]
fn read_carveout_blocks_writes_but_allows_reads() {
    let root = temp_dir("carveout");
    std::fs::create_dir_all(root.join("sealed")).expect("sealed");
    std::fs::write(root.join("sealed/data.txt"), "readable").expect("seed");
    let specs = Specs {
        roots: vec![root.clone()],
        read_only: vec![root.join("sealed")],
        ..Specs::default()
    };
    let script = format!(
        "cat '{root}/sealed/data.txt' > '{root}/copy.txt'; printf bad > '{root}/sealed/data.txt'",
        root = root.display()
    );
    let status = run(&root, &specs, &script);
    assert!(root.join("copy.txt").exists(), "carveout 仍可读");
    assert_eq!(
        std::fs::read_to_string(root.join("sealed/data.txt")).expect("read"),
        "readable"
    );
    assert!(!status.success());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sandbox_exec_path_is_fixed() {
    assert_eq!(SANDBOX_EXEC, "/usr/bin/sandbox-exec");
    assert!(Path::new(SANDBOX_EXEC).exists());
}
