//! 撤销持久化：scope 封印、序号、线性检查、最终事务与提示隔离。

use super::*;
use crate::db::chat::{ModelSnapshot, TurnOutcome, Usage};

fn temp(label: &str) -> (Database, std::path::PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "lx-rewind-db-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("workspace");
    let path = root.join("lx.db");
    let database = Database::open(&path).expect("db");
    (database, root)
}

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
}

fn regular(bytes: &[u8]) -> ImageWrite {
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

fn change(path: &str, before: &[u8], after: &[u8]) -> PathMutationWrite {
    PathMutationWrite {
        operation: "update".into(),
        path: path.into(),
        target_path: path.into(),
        before: regular(before),
        planned_after: regular(after),
    }
}

fn started(database: &Database, workspace: &std::path::Path) -> crate::db::chat::TurnCommitted {
    database
        .begin_turn(workspace, None, "hello", &model())
        .expect("turn")
}

fn sealed(database: &Database, workspace: &std::path::Path, turn: &crate::db::chat::TurnCommitted) {
    database
        .finish(
            workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");
}

#[test]
fn begin_turn_creates_capturing_scope_and_finish_seals_it() {
    let (database, workspace) = temp("scope");
    let turn = started(&database, &workspace);
    let scope = database
        .turn_scope(&turn.user_message_id)
        .expect("scope")
        .expect("present");
    assert_eq!(scope.state, SCOPE_CAPTURING);
    sealed(&database, &workspace, &turn);
    let scope = database
        .turn_scope(&turn.user_message_id)
        .expect("scope")
        .expect("present");
    assert_eq!(scope.state, SCOPE_COMPLETE);
}

#[test]
fn prepare_with_prepared_mutation_keeps_scope_unsealed() {
    let (database, workspace) = temp("unsealed");
    let turn = started(&database, &workspace);
    let ticket = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[change("/tmp/a", b"a", b"b")],
        )
        .expect("prepare");
    assert_eq!(ticket.ids.len(), 1);
    sealed(&database, &workspace, &turn);
    let scope = database
        .turn_scope(&turn.user_message_id)
        .expect("scope")
        .expect("present");
    assert_eq!(
        scope.state, SCOPE_CAPTURING,
        "存在未结算 mutation 时不能把轮次标成完整"
    );
}

/// 在已有会话上追加一轮并封口。
fn next_turn(
    database: &Database,
    workspace: &std::path::Path,
    session_id: &str,
) -> crate::db::chat::TurnCommitted {
    let turn = database
        .begin_turn(workspace, Some(session_id), "latest", &model())
        .expect("turn");
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

fn commit_delete(
    database: &Database,
    workspace: &std::path::Path,
    turn: &crate::db::chat::TurnCommitted,
    operation_id: &str,
) -> RewindCommitted {
    database
        .begin_operation(
            workspace,
            operation_id,
            &turn.session_id,
            &turn.user_message_id,
            turn.revision,
            "delete_only",
            "hello",
        )
        .expect("operation");
    database
        .commit_rewind(
            workspace,
            &RewindCommit {
                operation_id: operation_id.into(),
                session_id: turn.session_id.clone(),
                target_user_message_id: turn.user_message_id.clone(),
                expected_revision: turn.revision,
                kind: "delete_only".into(),
                delete_conversation: true,
                revert_files: false,
                restored_file_count: 0,
                files_already_restored: false,
            },
        )
        .expect("commit")
}

#[test]
fn rewind_deletes_latest_turn_and_persists_one_notice() {
    let (database, workspace) = temp("commit");
    let first = started(&database, &workspace);
    sealed(&database, &workspace, &first);
    let turn = next_turn(&database, &workspace, &first.session_id);
    let result = commit_delete(&database, &workspace, &turn, "op-1");
    assert!(result.executed);
    assert!(!result.session_deleted, "仍有上一轮，会话必须保留");
    assert_eq!(result.user_text, "latest");
    let messages = database
        .load_chat_session(&workspace, &turn.session_id)
        .expect("load")
        .expect("session");
    assert_eq!(messages.len(), 2, "只删除最新轮次，保留上一轮");
    assert_eq!(messages[0].external_id, first.user_message_id);
    let notices = database
        .load_notices(&workspace, &turn.session_id)
        .expect("notices");
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, "delete_only");
    // 重复提交同一 operation 幂等：不插第二条 tip。
    let replay = database
        .commit_rewind(
            &workspace,
            &RewindCommit {
                operation_id: "op-1".into(),
                session_id: turn.session_id.clone(),
                target_user_message_id: turn.user_message_id.clone(),
                expected_revision: turn.revision,
                kind: "delete_only".into(),
                delete_conversation: true,
                revert_files: false,
                restored_file_count: 0,
                files_already_restored: false,
            },
        )
        .expect("replay");
    assert!(!replay.executed);
    assert_eq!(
        database
            .load_notices(&workspace, &turn.session_id)
            .expect("notices")
            .len(),
        1
    );
}

/// 只剩对话提示的空会话必须整体移除，不保留“仅 Rewind”历史。
#[test]
fn rewinding_only_turn_removes_session_and_related_data() {
    let (database, workspace) = temp("only-turn");
    let turn = started(&database, &workspace);
    sealed(&database, &workspace, &turn);
    let result = commit_delete(&database, &workspace, &turn, "op-only");
    assert!(result.executed);
    assert!(result.session_deleted);
    assert_eq!(result.user_text, "hello");
    assert!(
        database
            .load_chat_session(&workspace, &turn.session_id)
            .expect("load")
            .is_none(),
        "空会话必须删除"
    );
    assert!(
        database
            .load_notices(&workspace, &turn.session_id)
            .expect("notices")
            .is_empty()
    );
    assert!(
        database
            .turn_scope(&turn.user_message_id)
            .expect("scope")
            .is_none(),
        "scope 随会话级联清理"
    );
    assert!(
        database.operation("op-only").expect("operation").is_none(),
        "操作日志随会话级联清理"
    );
}

#[test]
fn next_turn_sequence_is_after_persisted_notice() {
    let (database, workspace) = temp("sequence");
    let first = started(&database, &workspace);
    sealed(&database, &workspace, &first);
    let turn = next_turn(&database, &workspace, &first.session_id);
    commit_delete(&database, &workspace, &turn, "op-seq");
    let notices = database
        .load_notices(&workspace, &turn.session_id)
        .expect("notices");
    assert_eq!(notices.len(), 1);
    let next = database
        .begin_turn(&workspace, Some(&turn.session_id), "again", &model())
        .expect("next turn");
    assert!(
        next.user_sequence > notices[0].sequence,
        "新消息序号必须在提示之后，不能复用旧位置"
    );
}

#[test]
fn interleaved_same_path_marks_turn_incomplete() {
    let (database, workspace) = temp("linear");
    let turn = started(&database, &workspace);
    let first = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "a1",
            &[change("/tmp/a", b"zero", b"a")],
        )
        .expect("prepare a1");
    database
        .commit_mutations(
            &workspace,
            &turn.session_id,
            &first,
            &[ObservedWrite {
                actual_after: regular(b"a"),
                success: true,
            }],
        )
        .expect("commit a1");
    // B 在 A1 与 A2 之间写入：A2 的 before 不等于 A1 的 after。
    let second = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "a2",
            &[change("/tmp/a", b"ab", b"abc")],
        )
        .expect("prepare a2");
    database
        .commit_mutations(
            &workspace,
            &turn.session_id,
            &second,
            &[ObservedWrite {
                actual_after: regular(b"abc"),
                success: true,
            }],
        )
        .expect("commit a2");
    sealed(&database, &workspace, &turn);
    let plan = database
        .restore_plan(&workspace, &turn.session_id, &turn.user_message_id)
        .expect("plan");
    assert_eq!(plan.availability, FileAvailability::Incomplete);
    assert!(plan.paths.is_empty(), "交错写入不得生成整轮恢复计划");
}

#[test]
fn linear_writes_derive_first_before_and_last_after() {
    let (database, workspace) = temp("plan");
    let turn = started(&database, &workspace);
    let first = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "a1",
            &[change("/tmp/a", b"zero", b"a")],
        )
        .expect("prepare a1");
    database
        .commit_mutations(
            &workspace,
            &turn.session_id,
            &first,
            &[ObservedWrite {
                actual_after: regular(b"a"),
                success: true,
            }],
        )
        .expect("commit a1");
    let second = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "a2",
            &[change("/tmp/a", b"a", b"ab")],
        )
        .expect("prepare a2");
    database
        .commit_mutations(
            &workspace,
            &turn.session_id,
            &second,
            &[ObservedWrite {
                actual_after: regular(b"ab"),
                success: true,
            }],
        )
        .expect("commit a2");
    sealed(&database, &workspace, &turn);
    let plan = database
        .restore_plan(&workspace, &turn.session_id, &turn.user_message_id)
        .expect("plan");
    assert_eq!(plan.availability, FileAvailability::Ready { paths: 1 });
    assert_eq!(
        plan.paths[0].before.bytes.as_deref(),
        Some(b"zero".as_slice())
    );
    assert_eq!(
        plan.paths[0].expected_after.bytes.as_deref(),
        Some(b"ab".as_slice())
    );
}

#[test]
fn known_empty_turn_is_not_unavailable() {
    let (database, workspace) = temp("empty");
    let turn = started(&database, &workspace);
    sealed(&database, &workspace, &turn);
    let plan = database
        .restore_plan(&workspace, &turn.session_id, &turn.user_message_id)
        .expect("plan");
    assert_eq!(plan.availability, FileAvailability::KnownEmpty);
}
