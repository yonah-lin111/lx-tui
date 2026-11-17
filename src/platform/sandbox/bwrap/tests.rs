//! `platform::sandbox::bwrap` 参数快照测试（仅在 Linux 编译）。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-bwrap-{}-{label}-{}",
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

const DEFAULT_CAPS: BwrapCapabilities = BwrapCapabilities {
    available: true,
    user_ns: UserNsMode::UnshareUser,
    supports_perms: true,
};

fn wrap_for_with_caps(
    cwd: &Path,
    specs: &Specs,
    caps: BwrapCapabilities,
) -> Result<(Vec<String>, Vec<PathBuf>), SandboxError> {
    let spec = SandboxSpec {
        cwd,
        writable_roots: &specs.roots,
        read_only_paths: &specs.read_only,
        deny_paths: &specs.deny,
        deny_globs: &specs.globs,
        glob_scan_max_depth: 16,
        network_access: specs.network,
    };
    build_args_with_caps(&spec, OsStr::new("/bin/sh"), &[OsString::from("-c")], caps).map(
        |wrapped| {
            (
                wrapped
                    .args
                    .into_iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
                wrapped.synthetic_targets,
            )
        },
    )
}

fn args_for(cwd: &Path, specs: &Specs) -> Result<Vec<String>, SandboxError> {
    wrap_for_with_caps(cwd, specs, DEFAULT_CAPS).map(|(args, _)| args)
}

#[test]
fn workspace_write_arguments_are_ordered_and_network_is_off() {
    let base = temp_dir("args");
    let shallow = base.join("a");
    let deep = shallow.join("b");
    std::fs::create_dir_all(&deep).expect("deep root");
    std::fs::create_dir_all(shallow.join(".git")).expect("git dir");
    let specs = Specs {
        roots: vec![deep.clone(), shallow.clone()],
        read_only: vec![shallow.join(".git")],
        ..Specs::default()
    };
    let args = args_for(&shallow, &specs).expect("args");
    assert_eq!(args.first().map(String::as_str), Some("--new-session"));
    assert!(args.windows(2).any(|pair| pair == ["--ro-bind", "/"]));
    assert!(args.iter().any(|arg| arg == "--unshare-net"));
    assert!(args.iter().any(|arg| arg == "--unshare-user"));
    assert!(args.iter().any(|arg| arg == "--cap-drop"));

    let shallow_bind = args
        .iter()
        .position(|arg| arg == "--bind")
        .expect("has bind");
    assert_eq!(args[shallow_bind + 1], shallow.display().to_string());
    let deep_bind = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| *arg == "--bind")
        .map(|(index, _)| index)
        .nth(1)
        .expect("second bind");
    assert_eq!(args[deep_bind + 1], deep.display().to_string());

    // 只读 carveout 排在全部可写 bind 之后。
    let carveout = args
        .iter()
        .enumerate()
        .position(|(index, arg)| arg == "--ro-bind" && index > deep_bind)
        .expect("ro carveout");
    assert_eq!(
        args[carveout + 1],
        shallow.join(".git").display().to_string()
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn read_only_has_no_writable_bind_and_network_flag_follows_setting() {
    let cwd = temp_dir("readonly");
    let specs = Specs {
        network: true,
        ..Specs::default()
    };
    let args = args_for(&cwd, &specs).expect("args");
    assert!(!args.iter().any(|arg| arg == "--bind"));
    assert!(!args.iter().any(|arg| arg == "--unshare-net"));
    assert!(args.contains(&"--chdir".to_string()));
    assert!(args.contains(&cwd.display().to_string()));
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn deny_directory_is_masked_with_000_tmpfs() {
    let root = temp_dir("deny-dir");
    let secret = root.join("secret");
    std::fs::create_dir_all(&secret).expect("secret dir");
    let specs = Specs {
        roots: vec![root.clone()],
        deny: vec![secret.clone()],
        ..Specs::default()
    };
    let args = args_for(&root, &specs).expect("args");
    let index = args
        .iter()
        .position(|arg| arg == "--tmpfs")
        .expect("tmpfs mask");
    assert_eq!(args[index - 3], "--perms");
    assert_eq!(args[index - 2], "000");
    assert_eq!(args[index + 1], secret.display().to_string());
    assert_eq!(args[index + 2], "--remount-ro");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn deny_file_is_masked_with_dev_null() {
    let root = temp_dir("deny-file");
    let file = root.join("secret.txt");
    std::fs::write(&file, "x").expect("seed");
    let specs = Specs {
        roots: vec![root.clone()],
        deny: vec![file.clone()],
        ..Specs::default()
    };
    let args = args_for(&root, &specs).expect("args");
    let dev_null = args
        .iter()
        .position(|arg| arg == "/dev/null")
        .expect("dev null");
    assert_eq!(args[dev_null + 1], file.display().to_string());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn deny_glob_expands_inside_the_writable_root() {
    let root = temp_dir("deny-glob");
    std::fs::create_dir_all(root.join("a")).expect("nested");
    std::fs::write(root.join("a/.env"), "x").expect("seed");
    let specs = Specs {
        roots: vec![root.clone()],
        globs: vec![format!("{}/**/.env", root.display())],
        ..Specs::default()
    };
    let args = args_for(&root, &specs).expect("args");
    let expected = root.join("a/.env").display().to_string();
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "/dev/null" && pair[1] == expected),
        "glob 命中必须挂为空文件: {args:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_deny_path_masks_the_first_missing_component() {
    let root = temp_dir("deny-missing");
    let missing = root.join("not-yet").join("deep.txt");
    let specs = Specs {
        roots: vec![root.clone()],
        deny: vec![missing],
        ..Specs::default()
    };
    let args = args_for(&root, &specs).expect("args");
    let expected = root.join("not-yet").display().to_string();
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "/dev/null" && pair[1] == expected),
        "缺失路径必须占位最早缺失组件: {args:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn deny_path_crossing_a_writable_symlink_is_rejected() {
    let root = temp_dir("deny-symlink");
    let target = root.join("target");
    std::fs::create_dir_all(&target).expect("target");
    std::os::unix::fs::symlink(&target, root.join("link")).expect("symlink");
    let specs = Specs {
        roots: vec![root.clone()],
        deny: vec![root.join("link").join("x")],
        ..Specs::default()
    };
    assert!(matches!(
        args_for(&root, &specs),
        Err(SandboxError::Invalid(_))
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn relative_cwd_is_rejected() {
    let specs = Specs::default();
    assert!(matches!(
        args_for(Path::new("relative"), &specs),
        Err(SandboxError::Invalid(_))
    ));
}

#[test]
fn setuid_fallback_omits_unshare_user_and_unavailable_fails_closed() {
    let cwd = temp_dir("caps");
    let setuid_caps = BwrapCapabilities {
        available: true,
        user_ns: UserNsMode::SetuidFallback,
        supports_perms: true,
    };
    let (args, _) =
        wrap_for_with_caps(&cwd, &Specs::default(), setuid_caps).expect("setuid fallback");
    assert!(!args.iter().any(|arg| arg == "--unshare-user"));
    assert!(args.iter().any(|arg| arg == "--unshare-pid"));
    assert!(args.iter().any(|arg| arg == "--unshare-ipc"));

    let unavailable_caps = BwrapCapabilities {
        available: false,
        user_ns: UserNsMode::Unavailable,
        supports_perms: false,
    };
    assert!(matches!(
        wrap_for_with_caps(&cwd, &Specs::default(), unavailable_caps),
        Err(SandboxError::Unavailable(_))
    ));
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn missing_perms_flag_fails_closed_for_deny_dir_but_allows_deny_file() {
    let root = temp_dir("no-perms");
    let secret_dir = root.join("secret-dir");
    let secret_file = root.join("secret.txt");
    std::fs::create_dir_all(&secret_dir).expect("dir");
    std::fs::write(&secret_file, "x").expect("file");
    let no_perms = BwrapCapabilities {
        available: true,
        user_ns: UserNsMode::SetuidFallback,
        supports_perms: false,
    };
    let dir_specs = Specs {
        roots: vec![root.clone()],
        deny: vec![secret_dir],
        ..Specs::default()
    };
    assert!(matches!(
        wrap_for_with_caps(&root, &dir_specs, no_perms),
        Err(SandboxError::Unavailable(_))
    ));

    let file_specs = Specs {
        roots: vec![root.clone()],
        deny: vec![secret_file.clone()],
        ..Specs::default()
    };
    let (args, _) = wrap_for_with_caps(&root, &file_specs, no_perms).expect("deny file allowed");
    let expected = secret_file.display().to_string();
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "/dev/null" && pair[1] == expected)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_only_and_outside_existing_deny_targets_are_masked() {
    let base = temp_dir("ro-deny");
    let ws = base.join("ws");
    let outside = base.join("outside");
    let outside_dir = outside.join("vault");
    let outside_file = outside.join("key.txt");
    let missing_outside = outside.join("not-here.txt");
    std::fs::create_dir_all(&ws).expect("ws");
    std::fs::create_dir_all(&outside_dir).expect("outside dir");
    std::fs::write(&outside_file, "secret").expect("outside file");
    std::fs::write(outside.join(".env"), "env").expect("outside env");

    // read-only 模式（writable_roots 为空）：已存在文件/目录/glob 都必须遮盖，缺失路径跳过。
    let ro_specs = Specs {
        deny: vec![
            outside_dir.clone(),
            outside_file.clone(),
            missing_outside.clone(),
        ],
        globs: vec![format!("{}/**/.env", outside.display())],
        ..Specs::default()
    };
    let ro_args = args_for(&ws, &ro_specs).expect("ro deny args");
    assert!(
        ro_args
            .windows(2)
            .any(|pair| pair[0] == "--tmpfs" && pair[1] == outside_dir.display().to_string())
    );
    assert!(
        ro_args
            .windows(2)
            .any(|pair| pair[0] == "/dev/null" && pair[1] == outside_file.display().to_string())
    );
    assert!(ro_args.windows(2).any(
        |pair| pair[0] == "/dev/null" && pair[1] == outside.join(".env").display().to_string()
    ));
    assert!(!ro_args.contains(&missing_outside.display().to_string()));

    // workspace-write 模式下写根外已存在 deny 同样生效。
    let ww_specs = Specs {
        roots: vec![ws.clone()],
        deny: vec![outside_file.clone()],
        ..Specs::default()
    };
    let ww_args = args_for(&ws, &ww_specs).expect("ww outside deny");
    assert!(
        ww_args
            .windows(2)
            .any(|pair| pair[0] == "/dev/null" && pair[1] == outside_file.display().to_string())
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn nested_write_read_write_order_and_missing_root_and_symlink_carveout() {
    let base = temp_dir("layered");
    let ws = base.join("ws");
    let ro = ws.join("a");
    let deep_rw = ro.join("b");
    let missing_root = base.join("missing-root");
    std::fs::create_dir_all(&deep_rw).expect("deep rw");

    let specs = Specs {
        roots: vec![deep_rw.clone(), ws.clone(), missing_root.clone()],
        read_only: vec![ro.clone(), missing_root.join(".git")],
        ..Specs::default()
    };
    let args = args_for(&deep_rw, &specs).expect("layered args");
    assert!(!args.contains(&missing_root.display().to_string()));
    assert!(!args.contains(&missing_root.join(".git").display().to_string()));

    let bind_ws = args
        .windows(2)
        .position(|pair| pair[0] == "--bind" && pair[1] == ws.display().to_string())
        .expect("bind ws");
    let ro_a = args
        .windows(2)
        .position(|pair| pair[0] == "--ro-bind" && pair[1] == ro.display().to_string())
        .expect("ro a");
    let bind_b = args
        .windows(2)
        .position(|pair| pair[0] == "--bind" && pair[1] == deep_rw.display().to_string())
        .expect("bind b");
    assert!(bind_ws < ro_a && ro_a < bind_b, "args: {args:?}");

    let chdir_pos = args.iter().position(|arg| arg == "--chdir").expect("chdir");
    assert_eq!(args[chdir_pos + 1], deep_rw.display().to_string());

    // read_only 跨可写符号链接必须 fail-closed。
    std::os::unix::fs::symlink(&ro, ws.join("link")).expect("symlink");
    let bad_ro = Specs {
        roots: vec![ws.clone()],
        read_only: vec![ws.join("link/sub")],
        ..Specs::default()
    };
    assert!(matches!(
        args_for(&ws, &bad_ro),
        Err(SandboxError::Invalid(_))
    ));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn nested_deny_under_dir_is_deduped_and_tmp_git_is_skipped() {
    let base = temp_dir("dedupe-deny");
    let sec = base.join("sec");
    let child = sec.join(".env");
    std::fs::create_dir_all(&sec).expect("sec");
    std::fs::write(&child, "SECRET=1").expect("env");

    let specs = Specs {
        roots: vec![base.clone()],
        read_only: vec![PathBuf::from("/tmp/.git"), base.join(".git")],
        deny: vec![child.clone(), sec.clone()],
        ..Specs::default()
    };
    let (args, synthetic) = wrap_for_with_caps(&base, &specs, DEFAULT_CAPS).expect("args");
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--tmpfs" && pair[1] == sec.display().to_string())
    );
    assert!(
        !args.contains(&child.display().to_string()),
        "已被 000 tmpfs 覆盖的子路径不应再单独挂载: {args:?}"
    );
    assert!(
        !args.contains(&"/tmp/.git".to_string()),
        "/tmp/.git 不应向共享 /tmp 建占位文件: {args:?}"
    );
    assert!(
        synthetic.contains(&base.join(".git")),
        "工作区缺失的 .git 记入 synthetic_targets 以便退出后清理: {synthetic:?}"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn real_bwrap_leaves_no_tmp_git_residue_when_available() {
    if !is_available() {
        return;
    }
    let cwd = temp_dir("real-bwrap");
    let had_tmp_git = Path::new("/tmp/.git").exists();
    let specs = Specs {
        roots: vec![cwd.clone(), PathBuf::from("/tmp")],
        read_only: vec![PathBuf::from("/tmp/.git"), cwd.join(".git")],
        ..Specs::default()
    };
    let wrapped = wrap(
        &SandboxSpec {
            cwd: &cwd,
            writable_roots: &specs.roots,
            read_only_paths: &specs.read_only,
            deny_paths: &specs.deny,
            deny_globs: &specs.globs,
            glob_scan_max_depth: 16,
            network_access: false,
        },
        OsStr::new("/bin/sh"),
        &[OsString::from("-c"), OsString::from("true")],
    )
    .expect("wrap");
    let status = Command::new(&wrapped.program)
        .args(&wrapped.args)
        .status()
        .expect("run bwrap");
    assert!(status.success());
    if !had_tmp_git {
        assert!(!Path::new("/tmp/.git").exists());
    }
    for path in &wrapped.synthetic_targets {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_dir_all(cwd);
}
