//! 启动信任目标单元测试：路径自身、仓库主 checkout 与 worktree 目录的归位判定。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::state::AppState;
use crate::db::Database;
use crate::db::service::Persistence;

use super::startup_trust_target;

/// 独立临时目录（建好）。
fn temp_dir(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("lx-tui-startup-{label}-{nanos:x}-{sequence:x}"));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

/// 独立临时库上的持久化服务。
fn service_with(label: &str) -> Persistence {
    let database = Database::open(&temp_dir(label).join("lx.db")).expect("db opens");
    Persistence::with_database(database)
}

/// demo 状态并把活动工作区 cwd 指向给定路径。
fn state_with_cwd(cwd: &Path) -> AppState {
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(cwd.to_path_buf());
    state
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

/// 真实仓库 + linked worktree；git 不可用时 None。
fn repo_with_linked_worktree(label: &str) -> Option<(PathBuf, PathBuf, PathBuf)> {
    if !git_available() {
        return None;
    }
    let base = temp_dir(label);
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
    Some((base, repo, linked))
}

#[test]
fn trusted_workspace_needs_no_startup_gate() {
    let service = service_with("trusted");
    let dir = std::fs::canonicalize(temp_dir("trusted-ws")).expect("canonical");
    assert!(service.trust_path(&dir, "ws"));
    let state = state_with_cwd(&dir);
    assert_eq!(startup_trust_target(&service, &state), None);
}

#[test]
fn untrusted_plain_dir_targets_itself() {
    let service = service_with("plain");
    let dir = temp_dir("plain-ws");
    let state = state_with_cwd(&dir);
    assert_eq!(
        startup_trust_target(&service, &state),
        Some(std::fs::canonicalize(&dir).expect("canonical"))
    );
}

#[test]
fn trusted_repo_root_covers_linked_worktree_cwd() {
    let Some((base, repo, linked)) = repo_with_linked_worktree("wt-trusted") else {
        return;
    };
    let service = service_with("wt-trusted-db");
    assert!(service.trust_path(&repo, "repo"), "只信任仓库主 checkout");
    let state = state_with_cwd(&linked);
    assert_eq!(
        startup_trust_target(&service, &state),
        None,
        "主 checkout 已信任时 worktree 目录直接放行"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn untrusted_repo_root_targets_main_checkout() {
    let Some((base, repo, linked)) = repo_with_linked_worktree("wt-untrusted") else {
        return;
    };
    let service = service_with("wt-untrusted-db");
    let state = state_with_cwd(&linked);
    assert_eq!(
        startup_trust_target(&service, &state),
        Some(repo.clone()),
        "信任目标归位到仓库主 checkout"
    );
    // 路径自身已信任（历史数据）时直接放行，不要求仓库根。
    assert!(service.trust_path(&linked, "linked"));
    assert_eq!(startup_trust_target(&service, &state), None);
    let _ = std::fs::remove_dir_all(&base);
}
