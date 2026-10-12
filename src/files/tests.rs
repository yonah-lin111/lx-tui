//! 单元测试；仅测试构建编译。

use std::fs;
use std::path::{Path, PathBuf};

use super::*;

/// 建临时 fixture 目录；同名旧目录先清空，保证重复运行可复现。
fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lx-tui-files-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("fixture dir");
    dir
}

/// 写入 fixture 文件，自动创建父目录。
fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent");
    }
    fs::write(path, content).expect("fixture file");
}

/// 相对路径集合。
fn paths(entries: &[MentionEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.path.as_str()).collect()
}

/// 指定路径的条目。
fn entry<'a>(entries: &'a [MentionEntry], path: &str) -> Option<&'a MentionEntry> {
    entries.iter().find(|entry| entry.path == path)
}

#[test]
fn lists_files_and_directories_with_relative_paths() {
    let root = fixture("lists");
    write(&root, "src/app.rs", "fn main() {}");
    write(&root, "README.md", "# readme");

    let entries = scan(&root);
    assert_eq!(
        entry(&entries, "src").map(|entry| entry.is_directory),
        Some(true)
    );
    assert_eq!(
        entry(&entries, "src/app.rs").map(|entry| entry.is_directory),
        Some(false)
    );
    assert_eq!(
        entry(&entries, "README.md").map(|entry| entry.is_directory),
        Some(false)
    );
    assert!(paths(&entries).contains(&"src/app.rs"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn skips_hidden_and_ignored_directories_but_keeps_hidden_files() {
    let root = fixture("skips");
    write(&root, ".git/config", "git");
    write(&root, ".cache/entry", "cache");
    write(&root, "node_modules/pkg/index.js", "module");
    write(&root, "dist/bundle.js", "bundle");
    write(&root, "src/main.rs", "fn main() {}");
    write(&root, ".gitignore", "");

    let entries = scan(&root);
    let listed = paths(&entries);
    for skipped in [".git", ".cache", "node_modules", "dist"] {
        assert!(!listed.contains(&skipped), "{skipped} should be skipped");
    }
    assert!(listed.contains(&"src/main.rs"));
    assert!(listed.contains(&".gitignore"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn respects_gitignore_outside_git_repository() {
    let root = fixture("gitignore");
    write(&root, ".gitignore", "target/\n*.log\n");
    write(&root, "target/debug/build", "artifact");
    write(&root, "server.log", "log");
    write(&root, "keep.rs", "fn main() {}");

    let entries = scan(&root);
    let listed = paths(&entries);
    assert!(!listed.contains(&"target"));
    assert!(!listed.contains(&"target/debug/build"));
    assert!(!listed.contains(&"server.log"));
    assert!(listed.contains(&"keep.rs"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn stops_at_entry_limit() {
    let root = fixture("limit");
    for index in 0..5 {
        write(&root, &format!("file{index}.rs"), "");
    }

    let entries = scan_with_limit(&root, 2);
    assert_eq!(entries.len(), 2);

    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn skips_symbolic_links() {
    let root = fixture("symlink");
    write(&root, "real.rs", "fn main() {}");
    std::os::unix::fs::symlink(root.join("real.rs"), root.join("link.rs")).expect("symlink");

    let entries = scan(&root);
    let listed = paths(&entries);
    assert!(listed.contains(&"real.rs"));
    assert!(!listed.contains(&"link.rs"));

    let _ = fs::remove_dir_all(&root);
}
