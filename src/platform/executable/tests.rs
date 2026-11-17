//! PATH 查找与候选顺序回归：Unix 权限位、候选优先级与空路径。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::find_executable_in;

/// 隔离临时目录：按测试名、进程号与时间戳唯一；Drop 时清理。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lx-executable-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("mkdir tempdir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
fn write_script(dir: &Path, name: &str, executable: bool) {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(name);
    std::fs::write(&file, "#!/bin/sh\n").expect("write script");
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// 用多个目录拼出 PATH 值。
fn path_value(dirs: &[&Path]) -> OsString {
    std::env::join_paths(dirs).expect("join paths")
}

/// `find_executable_in` 接受 `&OsStr` 的便捷包装。
fn find_in(names: &[&str], path: &OsStr) -> Option<(String, PathBuf)> {
    find_executable_in(names, path)
}

#[cfg(unix)]
#[test]
fn requires_executable_bit_and_skips_plain_files() {
    let dir = TempDir::new("perm");
    write_script(dir.path(), "not-exec", false);
    write_script(dir.path(), "agent", true);
    let path = path_value(&[dir.path()]);

    assert_eq!(
        find_in(&["agent"], &path).map(|(name, _)| name),
        Some("agent".to_string())
    );
    assert!(
        find_in(&["not-exec"], &path).is_none(),
        "缺少执行位的同名文件不得被当作已安装"
    );
}

#[cfg(unix)]
#[test]
fn returns_first_candidate_hit() {
    let first = TempDir::new("order-a");
    let second = TempDir::new("order-b");
    write_script(second.path(), "alias", true);
    write_script(second.path(), "primary", true);
    let path = path_value(&[first.path(), second.path()]);

    assert_eq!(
        find_in(&["missing", "primary", "alias"], &path).map(|(name, _)| name),
        Some("primary".to_string()),
        "同一目录内按候选顺序命中"
    );
}

#[cfg(unix)]
#[test]
fn searches_directories_in_path_order() {
    let first = TempDir::new("dir-a");
    let second = TempDir::new("dir-b");
    write_script(second.path(), "agent", true);
    write_script(first.path(), "agent", true);
    let path = path_value(&[first.path(), second.path()]);

    let (_, resolved) = find_in(&["agent"], &path).expect("agent");
    assert_eq!(resolved, first.path().join("agent"));
}

#[test]
fn empty_or_missing_path_returns_none() {
    assert!(find_in(&["agent"], OsStr::new("")).is_none());
    assert!(find_executable_in(&["agent"], OsStr::new("")).is_none());
}
