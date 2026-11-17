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

fn args_for(cwd: &Path, specs: &Specs) -> Result<Vec<String>, SandboxError> {
    let spec = SandboxSpec {
        cwd,
        writable_roots: &specs.roots,
        read_only_paths: &specs.read_only,
        deny_paths: &specs.deny,
        deny_globs: &specs.globs,
        glob_scan_max_depth: 16,
        network_access: specs.network,
    };
    build_args(&spec, OsStr::new("/bin/sh"), &[OsString::from("-c")]).map(|args| {
        args.into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    })
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
