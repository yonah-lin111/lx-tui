//! `platform::sandbox::roots` 单元测试：根去重、`.git` 两种形态与路径边界。

use std::path::{Path, PathBuf};

use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-sandbox-roots-{}-{label}-{}",
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
fn roots_are_deduped_and_include_tmp() {
    let cwd = temp_dir("dedupe");
    let roots = writable_roots(&cwd, &[cwd.clone(), cwd.clone()]);
    assert_eq!(roots.iter().filter(|root| **root == cwd).count(), 1);
    #[cfg(unix)]
    assert!(
        roots
            .iter()
            .any(|root| root == Path::new("/tmp") || root == Path::new("/private/tmp"))
    );
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn missing_extra_root_is_skipped() {
    let cwd = temp_dir("missing");
    let missing = cwd.join("does-not-exist");
    let roots = writable_roots(&cwd, std::slice::from_ref(&missing));
    assert!(!roots.contains(&missing));
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn git_directory_and_metadata_are_protected() {
    let cwd = temp_dir("git-dir");
    std::fs::create_dir_all(cwd.join(".git")).expect("git dir");
    std::fs::create_dir_all(cwd.join(".agents")).expect("agents dir");
    let protected = protected_paths(std::slice::from_ref(&cwd));
    assert!(protected.contains(&cwd.join(".git")));
    assert!(protected.iter().any(|path| path.ends_with(".agents")));
    assert!(!protected.iter().any(|path| path.ends_with(".aws")));
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn git_file_pointer_protects_target_directory() {
    let cwd = temp_dir("git-file");
    let target = cwd.join("real-git-dir");
    std::fs::create_dir_all(&target).expect("target");
    std::fs::write(cwd.join(".git"), format!("gitdir: {}\n", target.display())).expect("git file");
    let git_file = std::fs::canonicalize(cwd.join(".git")).expect("canonical git file");
    let resolved_target = std::fs::canonicalize(&target).expect("canonical target");
    let protected = protected_paths(std::slice::from_ref(&cwd));
    assert!(protected.contains(&git_file));
    assert!(protected.contains(&resolved_target));
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn write_target_rejects_parent_escape_and_resolves_missing_children() {
    let cwd = temp_dir("write-target");
    let resolved = resolve_write_target(&cwd.join("a").join("b")).expect("resolve");
    assert_eq!(resolved, cwd.join("a").join("b"));
    let escape = cwd.join("..").join("outside");
    assert_eq!(resolve_write_target(&escape), None);
    let _ = std::fs::remove_dir_all(cwd);
}
