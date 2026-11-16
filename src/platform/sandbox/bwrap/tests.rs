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

fn args_for(roots: &[PathBuf], protected: &[PathBuf], cwd: &Path, network: bool) -> Vec<String> {
    let spec = SandboxSpec {
        cwd,
        writable_roots: roots,
        protected,
        network_access: network,
    };
    build_args(&spec, OsStr::new("/bin/sh"), &[OsString::from("-c")])
        .expect("build args")
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn workspace_write_arguments_are_ordered_and_network_is_off() {
    let base = temp_dir("args");
    let shallow = base.join("a");
    let deep = shallow.join("b");
    std::fs::create_dir_all(&deep).expect("deep root");
    std::fs::create_dir_all(shallow.join(".git")).expect("git dir");
    let mut roots = vec![deep.clone(), shallow.clone()];
    roots.sort_by_key(|root| root.components().count());
    let protected = vec![shallow.join(".git")];
    let args = args_for(&roots, &protected, &shallow, false);
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
    // 保护路径排在可写 bind 之后。
    let protect_bind = args
        .iter()
        .position(|arg| arg == "--ro-bind")
        .expect("ro bind");
    assert!(protect_bind > deep_bind);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn read_only_has_no_writable_bind_and_network_flag_follows_setting() {
    let cwd = temp_dir("readonly");
    let args = args_for(&[], &[], &cwd, true);
    assert!(!args.iter().any(|arg| arg == "--bind"));
    assert!(!args.iter().any(|arg| arg == "--unshare-net"));
    assert!(args.contains(&"--chdir".to_string()));
    assert!(args.contains(&cwd.display().to_string()));
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn relative_cwd_is_rejected() {
    let spec = SandboxSpec {
        cwd: Path::new("relative"),
        writable_roots: &[],
        protected: &[],
        network_access: false,
    };
    assert!(matches!(
        build_args(&spec, OsStr::new("/bin/sh"), &[]),
        Err(SandboxError::Invalid(_))
    ));
}
