//! 上下文压缩持久化测试：取号、幂等、revision 核对、撤销与级联。

use std::path::PathBuf;

use super::*;
use crate::db::chat::{ModelSnapshot, TurnOutcome, Usage};

fn temp(label: &str) -> (Database, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "lx-compact-db-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("workspace");
    let database = Database::open(&root.join("lx.db")).expect("db");
    (database, root)
}

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
}

fn part(position: i64, content: &str) -> crate::db::chat::ChatPart {
    crate::db::chat::ChatPart {
        position,
        kind: "text".into(),
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

/// 造一轮完整对话，返回该轮提交结果。
fn turn(
    database: &Database,
    workspace: &std::path::Path,
    session: Option<&str>,
    text: &str,
    answer: &str,
) -> crate::db::chat::TurnCommitted {
    let committed = database
        .begin_turn(workspace, session, text, &model())
        .expect("begin");
    database
        .finish(
            workspace,
            &committed.assistant_message_id,
            &[part(0, answer)],
            &TurnOutcome::Complete {
                stop_reason: Some("stop".into()),
            },
            Usage::default(),
        )
        .expect("finish");
    committed
}

/// 提交参数（范围取该会话当前全部消息）。
fn commit_of(session: &str, revision: i64, start: i64, end: i64, operation: &str) -> CompactCommit {
    CompactCommit {
        session_id: session.into(),
        expected_revision: revision,
        base_checkpoint_id: None,
        start_sequence: start,
        end_sequence: end,
        summary_text: "summary".into(),
        model_label: "gpt-5".into(),
        operation_id: operation.into(),
    }
}

#[test]
fn commit_writes_checkpoint_and_bumps_revision() {
    let (database, workspace) = temp("commit");
    let first = turn(&database, &workspace, None, "question", "answer");
    let before = database
        .session_revision(&first.session_id)
        .expect("revision");
    let committed = database
        .commit_compact(
            &workspace,
            &commit_of(&first.session_id, before, 0, 1, "op-1"),
        )
        .expect("compact");
    assert!(committed.created);
    assert_eq!(committed.checkpoint.state, COMPACT_ACTIVE);
    // 时间线序号在消息之后。
    assert!(committed.checkpoint.sequence >= 2);
    assert_eq!(
        database
            .session_revision(&first.session_id)
            .expect("revision"),
        before + 1
    );
    let active = database
        .active_checkpoint(&workspace, &first.session_id)
        .expect("active")
        .expect("some");
    assert_eq!(active.id, committed.checkpoint.id);
    assert_eq!(active.model_label, "gpt-5");
}

#[test]
fn same_operation_is_idempotent() {
    let (database, workspace) = temp("idempotent");
    let first = turn(&database, &workspace, None, "q", "a");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let commit = commit_of(&first.session_id, revision, 0, 1, "op-same");
    let one = database.commit_compact(&workspace, &commit).expect("first");
    let two = database
        .commit_compact(&workspace, &commit)
        .expect("second");
    assert!(one.created);
    assert!(!two.created, "重复回执不得再写一次");
    assert_eq!(one.checkpoint.id, two.checkpoint.id);
    assert_eq!(
        database
            .compact_checkpoints(&workspace, &first.session_id)
            .expect("list")
            .len(),
        1
    );
    assert_eq!(
        database
            .session_revision(&first.session_id)
            .expect("revision"),
        revision + 1,
        "幂等重放不得重复推进 revision"
    );
}

#[test]
fn stale_revision_fails_with_zero_writes() {
    let (database, workspace) = temp("stale");
    let first = turn(&database, &workspace, None, "q", "a");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let result = database.commit_compact(
        &workspace,
        &commit_of(&first.session_id, revision - 1, 0, 1, "op-stale"),
    );
    assert!(matches!(result, Err(DbError::RewindTargetChanged)));
    assert!(
        database
            .compact_checkpoints(&workspace, &first.session_id)
            .expect("list")
            .is_empty(),
        "失败必须零写入"
    );
    assert_eq!(
        database
            .session_revision(&first.session_id)
            .expect("revision"),
        revision
    );
}

#[test]
fn empty_summary_and_invalid_range_are_rejected() {
    let (database, workspace) = temp("invalid");
    let first = turn(&database, &workspace, None, "q", "a");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let mut commit = commit_of(&first.session_id, revision, 0, 1, "op-empty");
    commit.summary_text = "   ".into();
    assert!(matches!(
        database.commit_compact(&workspace, &commit),
        Err(DbError::InvalidDraft)
    ));
    let mut commit = commit_of(&first.session_id, revision, 5, 1, "op-range");
    commit.operation_id = "op-range".into();
    assert!(matches!(
        database.commit_compact(&workspace, &commit),
        Err(DbError::PositionConflict)
    ));
    assert!(
        database
            .compact_checkpoints(&workspace, &first.session_id)
            .expect("list")
            .is_empty()
    );
}

#[test]
fn consecutive_compacts_track_the_base_checkpoint() {
    let (database, workspace) = temp("consecutive");
    let first = turn(&database, &workspace, None, "q1", "a1");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let one = database
        .commit_compact(
            &workspace,
            &commit_of(&first.session_id, revision, 0, 1, "op-1"),
        )
        .expect("first");
    turn(&database, &workspace, Some(&first.session_id), "q2", "a2");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let mut second = commit_of(&first.session_id, revision, 2, 3, "op-2");
    second.base_checkpoint_id = Some(one.checkpoint.id.clone());
    let two = database
        .commit_compact(&workspace, &second)
        .expect("second");
    assert_eq!(
        two.checkpoint.base_checkpoint_id.as_deref(),
        Some(one.checkpoint.id.as_str())
    );
    assert_eq!(
        database
            .active_checkpoint(&workspace, &first.session_id)
            .expect("active")
            .expect("some")
            .id,
        two.checkpoint.id,
        "最新 active 成为上下文起点"
    );
    assert_eq!(
        database
            .compact_checkpoints(&workspace, &first.session_id)
            .expect("list")
            .len(),
        2
    );
}

#[test]
fn revert_only_flips_state_and_restores_previous_active() {
    let (database, workspace) = temp("revert");
    let first = turn(&database, &workspace, None, "q1", "a1");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let one = database
        .commit_compact(
            &workspace,
            &commit_of(&first.session_id, revision, 0, 1, "op-1"),
        )
        .expect("first");
    turn(&database, &workspace, Some(&first.session_id), "q2", "a2");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let mut second = commit_of(&first.session_id, revision, 2, 3, "op-2");
    second.base_checkpoint_id = Some(one.checkpoint.id.clone());
    let two = database
        .commit_compact(&workspace, &second)
        .expect("second");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let reverted = database
        .revert_compact(
            &workspace,
            &CompactRevert {
                operation_id: "rewind-op-1".into(),
                session_id: first.session_id.clone(),
                checkpoint_id: two.checkpoint.id.clone(),
                expected_revision: revision,
            },
        )
        .expect("revert");
    assert_eq!(reverted.state, COMPACT_REVERTED);
    // 原始消息保留（只切状态）。
    let messages = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("session");
    assert_eq!(messages.len(), 4, "撤销 compact 不得删除任何消息");
    // active 指针回退到上一个检查点。
    let active = database
        .active_checkpoint(&workspace, &first.session_id)
        .expect("active")
        .expect("some");
    assert_eq!(active.id, one.checkpoint.id);
}

#[test]
fn revert_rejects_stale_revision_and_out_of_order_targets() {
    let (database, workspace) = temp("revert-guard");
    let first = turn(&database, &workspace, None, "q1", "a1");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let one = database
        .commit_compact(
            &workspace,
            &commit_of(&first.session_id, revision, 0, 1, "op-1"),
        )
        .expect("first");
    turn(&database, &workspace, Some(&first.session_id), "q2", "a2");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let mut second = commit_of(&first.session_id, revision, 2, 3, "op-2");
    second.base_checkpoint_id = Some(one.checkpoint.id.clone());
    let two = database
        .commit_compact(&workspace, &second)
        .expect("second");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");

    // 期望 revision 过期：零写入。
    assert!(matches!(
        database.revert_compact(
            &workspace,
            &CompactRevert {
                operation_id: "rewind-stale".into(),
                session_id: first.session_id.clone(),
                checkpoint_id: two.checkpoint.id.clone(),
                expected_revision: revision - 1,
            }
        ),
        Err(DbError::RewindTargetChanged)
    ));
    // 更早的检查点：仍被更晚的 active 覆盖，拒绝。
    assert!(matches!(
        database.revert_compact(
            &workspace,
            &CompactRevert {
                operation_id: "rewind-early".into(),
                session_id: first.session_id.clone(),
                checkpoint_id: one.checkpoint.id.clone(),
                expected_revision: revision,
            }
        ),
        Err(DbError::RewindTargetChanged)
    ));
    // 两次失败都没有改变状态。
    assert_eq!(
        database
            .compact_checkpoints(&workspace, &first.session_id)
            .expect("list")
            .iter()
            .filter(|checkpoint| checkpoint.state == COMPACT_ACTIVE)
            .count(),
        2
    );
}

#[test]
fn revert_is_idempotent_for_the_same_checkpoint() {
    let (database, workspace) = temp("revert-idem");
    let first = turn(&database, &workspace, None, "q1", "a1");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let one = database
        .commit_compact(
            &workspace,
            &commit_of(&first.session_id, revision, 0, 1, "op-1"),
        )
        .expect("first");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let revert = CompactRevert {
        operation_id: "rewind-op".into(),
        session_id: first.session_id.clone(),
        checkpoint_id: one.checkpoint.id.clone(),
        expected_revision: revision,
    };
    let once = database.revert_compact(&workspace, &revert).expect("first");
    let twice = database
        .revert_compact(&workspace, &revert)
        .expect("second");
    assert_eq!(once.id, twice.id);
    assert_eq!(twice.state, COMPACT_REVERTED);
}

#[test]
fn deleting_the_session_removes_checkpoints() {
    let (database, workspace) = temp("cascade");
    let first = turn(&database, &workspace, None, "q1", "a1");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    database
        .commit_compact(
            &workspace,
            &commit_of(&first.session_id, revision, 0, 1, "op-1"),
        )
        .expect("compact");
    database
        .delete_chat_session(&workspace, &first.session_id)
        .expect("delete");
    assert!(
        database
            .compact_checkpoints(&workspace, &first.session_id)
            .expect("list")
            .is_empty(),
        "会话删除必须级联清理检查点"
    );
}

#[test]
fn cross_workspace_access_is_rejected() {
    let (database, workspace) = temp("cross");
    let other = workspace.join("other");
    std::fs::create_dir_all(&other).expect("other");
    database.trust_workspace(&other, "other").expect("trust");
    let first = turn(&database, &workspace, None, "q1", "a1");
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    assert!(matches!(
        database.commit_compact(
            &other,
            &commit_of(&first.session_id, revision, 0, 1, "op-cross")
        ),
        Err(DbError::SessionMissing)
    ));
    assert!(
        database
            .active_checkpoint(&other, &first.session_id)
            .expect("active")
            .is_none()
    );
}
