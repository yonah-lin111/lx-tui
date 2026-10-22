//! 端到端撤销：真实文件 + 临时 SQLite，验证恢复内容、冲突零写入与提示持久化。

use super::*;
use crate::db::chat::{
    ImageWrite, ModelSnapshot, ObservedWrite, PathMutationWrite, TurnOutcome, Usage,
};

fn fixture(label: &str) -> (Database, std::path::PathBuf, std::path::PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "lx-rewind-e2e-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("workspace");
    let db_path = root.join("lx.db");
    let database = Database::open(&db_path).expect("db");
    (database, root, db_path)
}

fn image(bytes: &[u8]) -> ImageWrite {
    ImageWrite {
        kind: "regular".into(),
        bytes: Some(bytes.to_vec()),
        mode: Some(0o644),
        link_target: None,
        git_oid: None,
        worktree_root: None,
        git_dir: None,
        head_ref: None,
        head_oid: None,
    }
}

/// 造一轮“Agent 将 old 写成 new”的完整记录。
fn turn_with_file(
    database: &Database,
    workspace: &Path,
    file: &Path,
    before: &[u8],
    after: &[u8],
) -> crate::db::chat::TurnCommitted {
    let turn = database
        .begin_turn(
            workspace,
            None,
            "edit it",
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    let ticket = database
        .prepare_mutations(
            workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[PathMutationWrite {
                operation: "update".into(),
                path: file.to_string_lossy().into_owned(),
                target_path: file.to_string_lossy().into_owned(),
                before: image(before),
                planned_after: image(after),
            }],
        )
        .expect("prepare");
    database
        .commit_mutations(
            workspace,
            &turn.session_id,
            &ticket,
            &[ObservedWrite {
                actual_after: image(after),
                success: true,
            }],
        )
        .expect("commit mutation");
    database
        .finish(
            workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");
    turn
}

fn request(
    turn: &crate::db::chat::TurnCommitted,
    workspace: &Path,
    action: RewindAction,
) -> RewindRequest {
    RewindRequest {
        pane: crate::layout::PaneId::alloc(),
        session_id: turn.session_id.clone(),
        target_user_message_id: turn.user_message_id.clone(),
        expected_revision: u64::try_from(turn.revision).expect("revision"),
        action,
        workspace: workspace.to_path_buf(),
        cwd: workspace.to_path_buf(),
        input_at_open: String::new(),
        user_text: "edit it".into(),
        operation_id: format!("op-{}", turn.user_message_id),
    }
}

#[test]
fn restores_bytes_and_deletes_latest_turn() {
    let (database, workspace, _db_path) = fixture("restore");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = turn_with_file(&database, &workspace, &file, b"old", b"new");

    let outcome = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertAndDelete),
    );
    assert!(
        matches!(
            outcome,
            RewindOutcome::Succeeded {
                restored_files: 1,
                ..
            }
        ),
        "unexpected: {outcome:?}"
    );
    assert_eq!(std::fs::read(&file).expect("file"), b"old");
    assert!(
        database
            .load_chat_session(&workspace, &turn.session_id)
            .expect("load")
            .expect("session")
            .is_empty()
    );
    let notices = database
        .load_notices(&workspace, &turn.session_id)
        .expect("notices");
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, "revert_and_delete");
    assert_eq!(notices[0].restored_file_count, 1);
}

#[test]
fn later_external_write_is_a_conflict_and_nothing_is_rewritten() {
    let (database, workspace, _db_path) = fixture("conflict");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = turn_with_file(&database, &workspace, &file, b"old", b"new");
    // 该轮之后，外部程序（或另一个 Agent）又改了同一文件。
    std::fs::write(&file, b"later").expect("external write");

    let outcome = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertAndDelete),
    );
    assert!(matches!(outcome, RewindOutcome::Failed { .. }));
    assert_eq!(
        std::fs::read(&file).expect("file"),
        b"later",
        "冲突必须零写入"
    );
    assert!(
        !database
            .load_chat_session(&workspace, &turn.session_id)
            .expect("load")
            .expect("session")
            .is_empty(),
        "冲突时不能删除对话"
    );
    assert!(
        database
            .load_notices(&workspace, &turn.session_id)
            .expect("notices")
            .is_empty(),
        "失败不插成功 tip"
    );
}

#[test]
fn revert_only_keeps_conversation_and_is_idempotent() {
    let (database, workspace, _db_path) = fixture("revert-only");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = turn_with_file(&database, &workspace, &file, b"old", b"new");

    let first = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertOnly),
    );
    assert!(matches!(
        first,
        RewindOutcome::Succeeded {
            restored_files: 1,
            ..
        }
    ));
    assert_eq!(std::fs::read(&file).expect("file"), b"old");
    assert!(
        !database
            .load_chat_session(&workspace, &turn.session_id)
            .expect("load")
            .expect("session")
            .is_empty(),
        "仅恢复文件保留对话"
    );
    // 第二次仅恢复文件：幂等无操作，不插新 tip（revision 已随首次成功推进）。
    let revision = u64::try_from(
        database
            .session_revision(&turn.session_id)
            .expect("revision"),
    )
    .expect("u64");
    let second = run(
        &database,
        &workspace,
        &RewindRequest {
            operation_id: "op-second".into(),
            expected_revision: revision,
            ..request(&turn, &workspace, RewindAction::RevertOnly)
        },
    );
    assert!(
        matches!(
            second,
            RewindOutcome::Succeeded {
                already_restored: true,
                restored_files: 0,
                ..
            }
        ),
        "second = {second:?}"
    );
    // 首次恢复插入一条 tip；第二次幂等无操作不得再插。
    assert_eq!(
        database
            .load_notices(&workspace, &turn.session_id)
            .expect("notices")
            .len(),
        1
    );
}

#[test]
fn unreachable_message_without_snapshot_can_only_delete_conversation() {
    let (database, workspace, _db_path) = fixture("no-snapshot");
    let turn = database
        .begin_turn(
            &workspace,
            None,
            "plain",
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    // 模拟旧轮次：删除 scope，等价于升级前的历史。
    database
        .drop_turn_scope_for_tests(&turn.user_message_id)
        .expect("drop scope");
    database
        .finish(
            &workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");
    let outcome = run(
        &database,
        &workspace,
        &request(&turn, &workspace, RewindAction::RevertOnly),
    );
    assert!(matches!(outcome, RewindOutcome::Failed { .. }));
    let outcome = run(
        &database,
        &workspace,
        &RewindRequest {
            operation_id: "op-delete-only".into(),
            ..request(&turn, &workspace, RewindAction::DeleteOnly)
        },
    );
    assert!(matches!(outcome, RewindOutcome::Succeeded { .. }));
}

/// 会话 cwd 在主 checkout，实际目标在 linked worktree：只恢复实际目标路径。
#[test]
fn restores_actual_linked_worktree_target_only() {
    let (database, workspace, _db_path) = fixture("worktree");
    let repo = workspace.join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let git = |dir: &Path, args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&repo, &["init", "-q", "-b", "dev"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), b"base").expect("base");
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    let linked = workspace.join("linked");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "task",
            linked.to_str().expect("utf8"),
        ],
    );
    let target = linked.join("a.txt");
    std::fs::write(&target, b"new").expect("agent write");
    let identity = crate::git::identify(&target).expect("identity");
    let with_git = |bytes: &[u8]| ImageWrite {
        kind: "regular".into(),
        bytes: Some(bytes.to_vec()),
        mode: Some(0o644),
        link_target: None,
        git_oid: None,
        worktree_root: Some(identity.root.to_string_lossy().into_owned()),
        git_dir: Some(identity.git_dir.to_string_lossy().into_owned()),
        head_ref: identity.head_ref.clone(),
        head_oid: identity.head_oid.clone(),
    };
    let turn = database
        .begin_turn(
            &repo,
            None,
            "edit linked",
            &ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    let ticket = database
        .prepare_mutations(
            &repo,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[PathMutationWrite {
                operation: "update".into(),
                path: target.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                before: with_git(b"base"),
                planned_after: with_git(b"new"),
            }],
        )
        .expect("prepare");
    database
        .commit_mutations(
            &repo,
            &turn.session_id,
            &ticket,
            &[ObservedWrite {
                actual_after: with_git(b"new"),
                success: true,
            }],
        )
        .expect("commit");
    database
        .finish(
            &repo,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");
    // 主 checkout 同名文件也改过，但不在本轮记录里，必须保持不变。
    std::fs::write(repo.join("a.txt"), b"main-dirty").expect("main write");
    let outcome = run(
        &database,
        &repo,
        &crate::app::effects::RewindRequest {
            pane: crate::layout::PaneId::alloc(),
            session_id: turn.session_id.clone(),
            target_user_message_id: turn.user_message_id.clone(),
            expected_revision: u64::try_from(turn.revision).expect("rev"),
            action: RewindAction::RevertAndDelete,
            workspace: repo.clone(),
            cwd: repo.clone(),
            input_at_open: String::new(),
            user_text: "edit linked".into(),
            operation_id: "op-worktree".into(),
        },
    );
    assert!(
        matches!(
            outcome,
            RewindOutcome::Succeeded {
                restored_files: 1,
                ..
            }
        ),
        "unexpected: {outcome:?}"
    );
    assert_eq!(std::fs::read(&target).expect("linked"), b"base");
    assert_eq!(
        std::fs::read(repo.join("a.txt")).expect("main"),
        b"main-dirty",
        "主 checkout 同名文件不得被恢复"
    );
}
