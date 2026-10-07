//! git 模块单元测试：porcelain 解析与真实仓库查询。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-git-tests-{}-{}-{}",
        name,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn parse_reads_main_linked_detached_and_bare() {
    let text = "\
worktree /repo/main
HEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
branch refs/heads/main

worktree /repo/linked
HEAD bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
branch refs/heads/feature/x

worktree /repo/detached
HEAD cccccccccccccccccccccccccccccccccccccccc
detached

worktree /repo/bare.git
bare
";
    let entries = parse_worktree_list(text);
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].path, PathBuf::from("/repo/main"));
    assert_eq!(entries[0].branch.as_deref(), Some("main"));
    assert!(!entries[0].is_bare);
    assert_eq!(entries[1].branch.as_deref(), Some("feature/x"));
    assert_eq!(entries[2].branch, None);
    assert!(!entries[2].is_bare);
    assert_eq!(entries[3].branch, None);
    assert!(entries[3].is_bare);
}

#[test]
fn parse_returns_empty_for_empty_or_unrelated_output() {
    assert!(parse_worktree_list("").is_empty());
    assert!(parse_worktree_list("HEAD abc\nbranch refs/heads/main\n").is_empty());
}

#[test]
fn parse_keeps_non_heads_reference_verbatim() {
    let entries = parse_worktree_list("worktree /repo\nbranch refs/tags/v1\n");
    assert_eq!(entries[0].branch.as_deref(), Some("refs/tags/v1"));
}

#[test]
fn checkout_prefers_longest_matching_prefix() {
    let list = WorktreeList {
        repo_root: PathBuf::from("/repo"),
        entries: vec![
            WorktreeEntry {
                path: PathBuf::from("/repo"),
                branch: Some("main".to_string()),
                is_bare: false,
            },
            WorktreeEntry {
                path: PathBuf::from("/repo/nested"),
                branch: Some("nested".to_string()),
                is_bare: false,
            },
        ],
    };
    let found = checkout(&list, Path::new("/repo/nested/src")).expect("nested checkout");
    assert_eq!(found.checkout_path, PathBuf::from("/repo/nested"));
    assert!(found.is_linked);
    assert_eq!(found.branch.as_deref(), Some("nested"));
    assert_eq!(found.repo_root, PathBuf::from("/repo"));

    let found = checkout(&list, Path::new("/repo/src")).expect("main checkout");
    assert_eq!(found.checkout_path, PathBuf::from("/repo"));
    assert!(!found.is_linked);
}

#[test]
fn checkout_ignores_sibling_with_shared_prefix() {
    let list = WorktreeList {
        repo_root: PathBuf::from("/repo"),
        entries: vec![WorktreeEntry {
            path: PathBuf::from("/repo"),
            branch: None,
            is_bare: false,
        }],
    };
    assert!(checkout(&list, Path::new("/repository")).is_none());
}

#[test]
fn list_worktrees_reads_real_repo_with_linked_worktree() {
    if !git_available() {
        return;
    }
    let base = temp_dir("real-repo");
    let repo = base.join("repo");
    let linked = base.join("linked");
    std::fs::create_dir_all(&repo).expect("repo dir");
    run_git(&repo, &["init", "--quiet", "-b", "main"]);
    run_git(
        &repo,
        &[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=Test",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature/x",
            linked.to_str().expect("utf-8 path"),
        ],
    );

    let repo = std::fs::canonicalize(&repo).expect("canonical repo");
    let linked = std::fs::canonicalize(&linked).expect("canonical linked");
    let list = list_worktrees(&repo).expect("repo lists worktrees");
    assert_eq!(list.entries.len(), 2);
    assert_eq!(list.repo_root, repo);
    assert_eq!(list.entries[0].branch.as_deref(), Some("main"));
    assert_eq!(list.entries[1].path, linked);
    assert_eq!(list.entries[1].branch.as_deref(), Some("feature/x"));

    let found = checkout(&list, &linked.join("src")).expect("linked checkout");
    assert!(found.is_linked);
    assert_eq!(found.checkout_path, linked);

    let found = checkout(&list, &repo).expect("main checkout");
    assert!(!found.is_linked);
    assert_eq!(found.checkout_path, repo);

    std::fs::remove_dir_all(&base).expect("cleanup");
}

#[test]
fn list_worktrees_returns_none_outside_repo() {
    if !git_available() {
        return;
    }
    let base = temp_dir("not-a-repo");
    assert!(list_worktrees(&base).is_none());
    std::fs::remove_dir_all(&base).expect("cleanup");
}
