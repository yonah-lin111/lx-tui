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

#[cfg(target_os = "macos")]
#[test]
fn uncanonicalized_tmp_symlink_paths_enforce_carveout_and_deny() {
    let raw_base = PathBuf::from(format!(
        "/tmp/lx-seatbelt-raw-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    let raw_sealed = raw_base.join("sealed");
    let raw_secret = raw_base.join("secret");
    let raw_env_dir = raw_base.join("nested");
    std::fs::create_dir_all(&raw_sealed).expect("sealed");
    std::fs::create_dir_all(&raw_secret).expect("secret");
    std::fs::create_dir_all(&raw_env_dir).expect("nested");
    std::fs::write(raw_sealed.join("data.txt"), "readable").expect("seed sealed");
    std::fs::write(raw_secret.join("key.txt"), "hunter2").expect("seed secret");
    std::fs::write(raw_env_dir.join(".env"), "TOKEN=1").expect("seed env");

    let canonical_base = std::fs::canonicalize(&raw_base).expect("canonical");
    let specs = Specs {
        // 写根故意给规范路径，而 carveout / deny / glob 给未规范化的 `/tmp/...`。
        roots: vec![canonical_base.clone()],
        read_only: vec![raw_sealed.clone()],
        deny: vec![raw_secret.clone()],
        globs: vec![format!("{}/**/.env", raw_base.display())],
        ..Specs::default()
    };
    let script = format!(
        "cat '{raw}/sealed/data.txt' > '{raw}/copy.txt'; \
         printf bad > '{raw}/sealed/data.txt' 2>/dev/null; \
         cat '{raw}/secret/key.txt' > '{raw}/leak1.txt' 2>/dev/null; \
         cat '{raw}/nested/.env' > '{raw}/leak2.txt' 2>/dev/null; \
         printf ok > '{raw}/ok.txt'",
        raw = raw_base.display()
    );
    let _status = run(&canonical_base, &specs, &script);
    assert_eq!(
        std::fs::read_to_string(raw_base.join("copy.txt")).expect("copy"),
        "readable"
    );
    assert_eq!(
        std::fs::read_to_string(raw_sealed.join("data.txt")).expect("sealed intact"),
        "readable"
    );
    assert_eq!(
        std::fs::read_to_string(raw_base.join("leak1.txt")).unwrap_or_default(),
        ""
    );
    assert_eq!(
        std::fs::read_to_string(raw_base.join("leak2.txt")).unwrap_or_default(),
        ""
    );
    assert!(raw_base.join("ok.txt").exists());
    let _ = std::fs::remove_dir_all(&raw_base);
}

#[cfg(target_os = "macos")]
#[test]
fn protect_anchor_and_glob_ancestor_unlink_and_directory_glob_descendants() {
    let base = temp_dir("p2-4");
    let ws = base.join("ws");
    let sec = ws.join("sec");
    let brace_a = ws.join("a");
    let env_dir = ws.join("sub/.env");
    std::fs::create_dir_all(&sec).expect("sec");
    std::fs::create_dir_all(&brace_a).expect("brace_a");
    std::fs::create_dir_all(&env_dir).expect("env_dir");
    std::fs::write(sec.join("app.env"), "SECRET=1").expect("sec env");
    std::fs::write(brace_a.join("key.txt"), "BRACE=1").expect("brace key");
    std::fs::write(env_dir.join("nested.txt"), "NESTED=1").expect("dir env");

    let specs = Specs {
        // 同时把 `base` 和 `ws` 设为可写根：若未保护写根锚点，可 `mv ws ws2`。
        roots: vec![base.clone(), ws.clone()],
        globs: vec![
            format!("{}/sec/*.env", ws.display()),
            format!("{}/{{a,b}}", ws.display()),
            format!("{}/**/.env", ws.display()),
        ],
        ..Specs::default()
    };
    let script = format!(
        "mv '{ws}' '{base}/ws-moved' 2>/dev/null; \
         mv '{sec}' '{ws}/sec-moved' 2>/dev/null; \
         cat '{brace}/key.txt' > '{ws}/leak-brace.txt' 2>/dev/null; \
         cat '{env_dir}/nested.txt' > '{ws}/leak-envdir.txt' 2>/dev/null",
        ws = ws.display(),
        base = base.display(),
        sec = sec.display(),
        brace = brace_a.display(),
        env_dir = env_dir.display(),
    );
    let _status = run(&ws, &specs, &script);
    assert!(ws.exists(), "writable root dir 自身禁止被重命名");
    assert!(!base.join("ws-moved").exists());
    assert!(sec.exists(), "deny_glob 静态前缀目录禁止被重命名");
    assert!(!ws.join("sec-moved").exists());
    assert_eq!(
        std::fs::read_to_string(ws.join("leak-brace.txt")).unwrap_or_default(),
        "",
        "{{a,b}} 目录形态必须拦截后代读取"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("leak-envdir.txt")).unwrap_or_default(),
        "",
        "/**/.env 目录形态必须拦截后代读取"
    );
    let _ = std::fs::remove_dir_all(base);
}
