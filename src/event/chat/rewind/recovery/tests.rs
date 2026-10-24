//! 启动恢复：prepared mutation 结算与未完成操作回滚。

use std::path::{Path, PathBuf};

use super::*;
use crate::db::chat::rewind::{
    ImageWrite, OP_FAILED, OP_RECOVERY_REQUIRED, OP_RESTORING, ObservedWrite, PathMutationWrite,
};
use crate::db::chat::{TurnOutcome, Usage};

fn fixture(label: &str) -> (Database, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "lx-rewind-recovery-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("workspace");
    let database = Database::open(&root.join("lx.db")).expect("db");
    (database, root)
}

fn model() -> crate::db::chat::ModelSnapshot {
    crate::db::chat::ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
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

/// 读后版本 token：与执行层同一口径（`before` 镜像重算）。
fn token(before: &[u8]) -> String {
    crate::platform::version_token_from_parts("regular", Some(before), Some(0o644), None)
        .expect("token")
}

fn change(path: &Path, before: &[u8], after: &[u8]) -> PathMutationWrite {
    PathMutationWrite {
        operation: "update".into(),
        path: path.to_string_lossy().into_owned(),
        target_path: path.to_string_lossy().into_owned(),
        anchor_path: None,
        anchor_canonical: None,
        before: image(before),
        planned_after: image(after),
        expected_version: Some(token(before)),
    }
}

fn scope_state(database: &Database, user_message_id: &str) -> String {
    database
        .turn_scope(user_message_id)
        .expect("scope")
        .expect("present")
        .state
}

/// prepared 且当前等于 before：结算为“未生效”，随后 scope 可以封口。
#[test]
fn settles_prepared_mutation_that_never_applied() {
    let (database, workspace) = fixture("before");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"old").expect("seed");
    let turn = database
        .begin_turn(&workspace, None, "edit", &model())
        .expect("turn");
    database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[change(&file, b"old", b"new")],
        )
        .expect("prepare");
    // 进程在这里被杀：没有 finish、没有 Commit ACK。
    recover_session_for_tests(&database, &turn.session_id).expect("recover");
    assert!(
        database
            .pending_mutations(&turn.session_id)
            .expect("pending")
            .is_empty(),
        "prepared 必须被结算"
    );
    assert_eq!(scope_state(&database, &turn.user_message_id), "complete");
}

/// prepared 且当前等于 planned：说明已写但缺 ACK，按 planned 重建实际结果。
#[test]
fn settles_prepared_mutation_that_already_applied() {
    let (database, workspace) = fixture("planned");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("written");
    let turn = database
        .begin_turn(&workspace, None, "edit", &model())
        .expect("turn");
    database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[change(&file, b"old", b"new")],
        )
        .expect("prepare");
    recover_session_for_tests(&database, &turn.session_id).expect("recover");
    assert!(
        database
            .pending_mutations(&turn.session_id)
            .expect("pending")
            .is_empty()
    );
    assert_eq!(scope_state(&database, &turn.user_message_id), "complete");
}

/// 第三种状态：证据不足，保持 prepared，scope 仍是 capturing（该轮不可恢复）。
#[test]
fn leaves_unknown_mutation_unsettled() {
    let (database, workspace) = fixture("unknown");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"someone else").expect("written");
    let turn = database
        .begin_turn(&workspace, None, "edit", &model())
        .expect("turn");
    database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[change(&file, b"old", b"new")],
        )
        .expect("prepare");
    recover_session_for_tests(&database, &turn.session_id).expect("recover");
    assert_eq!(
        database
            .pending_mutations(&turn.session_id)
            .expect("pending")
            .len(),
        1,
        "不能猜实际结果"
    );
    assert_eq!(scope_state(&database, &turn.user_message_id), "capturing");
}

/// 已完成的一轮 + 崩溃在 restoring：文件恢复回操作前状态，operation 结算为 failed。
#[test]
fn rolls_back_interrupted_restore() {
    let (database, workspace) = fixture("rollback");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = database
        .begin_turn(&workspace, None, "edit", &model())
        .expect("turn");
    let ticket = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[change(&file, b"old", b"new")],
        )
        .expect("prepare");
    database
        .commit_mutations(
            &workspace,
            &turn.session_id,
            &ticket,
            &[ObservedWrite {
                actual_after: image(b"new"),
                success: true,
            }],
        )
        .expect("commit");
    database
        .finish(
            &workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");

    // 模拟恢复到一半被杀：磁盘已是 before，operation 停在 restoring。
    std::fs::write(&file, b"old").expect("restored");
    database
        .begin_operation(
            &workspace,
            "op-crash",
            &turn.session_id,
            &turn.user_message_id,
            turn.revision,
            "revert_only",
            "",
        )
        .expect("operation");
    database
        .set_operation_phase("op-crash", OP_RESTORING, None)
        .expect("phase");

    recover_session_for_tests(&database, &turn.session_id).expect("recover");
    assert_eq!(
        std::fs::read(&file).expect("read"),
        b"new",
        "回到操作前状态"
    );
    assert_eq!(
        database
            .operation("op-crash")
            .expect("operation")
            .expect("present")
            .phase,
        OP_FAILED
    );
}

/// 恢复途中文件被第三方改成第三种状态：转 recovery_required，不动磁盘。
#[test]
fn third_state_blocks_interrupted_restore() {
    let (database, workspace) = fixture("third-state");
    let file = workspace.join("a.txt");
    std::fs::write(&file, b"new").expect("agent write");
    let turn = database
        .begin_turn(&workspace, None, "edit", &model())
        .expect("turn");
    let ticket = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[change(&file, b"old", b"new")],
        )
        .expect("prepare");
    database
        .commit_mutations(
            &workspace,
            &turn.session_id,
            &ticket,
            &[ObservedWrite {
                actual_after: image(b"new"),
                success: true,
            }],
        )
        .expect("commit");
    database
        .finish(
            &workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Interrupted,
            Usage::default(),
        )
        .expect("finish");
    std::fs::write(&file, b"third").expect("external write");
    database
        .begin_operation(
            &workspace,
            "op-third",
            &turn.session_id,
            &turn.user_message_id,
            turn.revision,
            "revert_only",
            "",
        )
        .expect("operation");
    database
        .set_operation_phase("op-third", OP_RESTORING, None)
        .expect("phase");

    recover_session_for_tests(&database, &turn.session_id).expect("recover");
    assert_eq!(std::fs::read(&file).expect("read"), b"third");
    assert_eq!(
        database
            .operation("op-third")
            .expect("operation")
            .expect("present")
            .phase,
        OP_RECOVERY_REQUIRED
    );
}
