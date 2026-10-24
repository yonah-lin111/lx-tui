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

/// 读后版本 token：与执行层同一口径（`before` 镜像重算）。
fn token(before: &[u8]) -> String {
    crate::platform::version_token_from_parts("regular", Some(before), Some(0o644), None)
        .expect("token")
}

fn change(path: &str, before: &[u8], after: &[u8]) -> PathMutationWrite {
    PathMutationWrite {
        operation: "update".into(),
        path: path.into(),
        target_path: path.into(),
        anchor_path: None,
        anchor_canonical: None,
        before: regular(before),
        planned_after: regular(after),
        expected_version: Some(token(before)),
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

/// 删除轮次时同步清掉该轮的 scope/mutation，避免快照数据永久残留。
#[test]
fn deleting_turn_cleans_scope_mutations_and_images() {
    let (database, workspace) = temp("cleanup");
    let first = started(&database, &workspace);
    sealed(&database, &workspace, &first);
    let turn = database
        .begin_turn(&workspace, Some(&first.session_id), "latest", &model())
        .expect("turn");
    let ticket = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[change("/tmp/a", b"a", b"b")],
        )
        .expect("prepare");
    database
        .commit_mutations(
            &workspace,
            &turn.session_id,
            &ticket,
            &[ObservedWrite {
                actual_after: regular(b"b"),
                success: true,
            }],
        )
        .expect("commit mutation");
    sealed(&database, &workspace, &turn);
    let result = commit_delete(&database, &workspace, &turn, "op-cleanup");
    assert!(result.executed);
    assert!(!result.session_deleted, "上一轮仍在");
    assert!(
        database
            .turn_scope(&turn.user_message_id)
            .expect("scope")
            .is_none(),
        "被删轮次的 scope 必须清理"
    );
    let mutations: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM agent_file_mutation WHERE user_message_id = ?1",
            params![turn.user_message_id],
            |row| row.get(0),
        )
        .expect("count mutations");
    assert_eq!(mutations, 0);
    database
        .prune_orphan_images(&turn.session_id)
        .expect("prune");
    let images: i64 = database
        .conn
        .query_row(
            "SELECT COUNT(*) FROM agent_file_image WHERE session_id = ?1",
            params![turn.session_id],
            |row| row.get(0),
        )
        .expect("count images");
    assert_eq!(images, 0, "失去引用的镜像必须回收");
}

/// 面板预检只读元数据：不加载 BLOB，也能区分空轮次、未结算与已恢复。
#[test]
fn availability_is_metadata_only() {
    let (database, workspace) = temp("availability");
    let turn = started(&database, &workspace);
    sealed(&database, &workspace, &turn);
    let (availability, revision, text) = database
        .rewind_availability(&workspace, &turn.session_id, &turn.user_message_id)
        .expect("availability");
    assert_eq!(availability, FileAvailability::KnownEmpty);
    assert_eq!(revision, turn.revision);
    assert_eq!(text, "hello");

    let next = database
        .begin_turn(&workspace, Some(&turn.session_id), "latest", &model())
        .expect("turn");
    let _ticket = database
        .prepare_mutations(
            &workspace,
            &next.session_id,
            &next.user_message_id,
            "call",
            &[change("/tmp/a", b"a", b"b")],
        )
        .expect("prepare");
    let (availability, _, _) = database
        .rewind_availability(&workspace, &next.session_id, &next.user_message_id)
        .expect("availability");
    assert_eq!(
        availability,
        FileAvailability::Incomplete,
        "未结算的 mutation 不能报可恢复"
    );
}

/// actual-after 镜像也计入本轮快照字节，后续 prepare 的限额才有意义。
#[test]
fn commit_accounts_actual_image_bytes() {
    let (database, workspace) = temp("actual-bytes");
    let turn = started(&database, &workspace);
    database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call",
            &[change("/tmp/a", b"a", b"bb")],
        )
        .expect("prepare");
    let ticket = database
        .prepare_mutations(
            &workspace,
            &turn.session_id,
            &turn.user_message_id,
            "call-2",
            &[change("/tmp/b", b"", b"ccc")],
        )
        .expect("prepare second");
    let prepared = database
        .turn_scope(&turn.user_message_id)
        .expect("scope")
        .expect("present")
        .snapshot_bytes;
    database
        .commit_mutations(
            &workspace,
            &turn.session_id,
            &ticket,
            &[ObservedWrite {
                actual_after: regular(b"ccc"),
                success: true,
            }],
        )
        .expect("commit");
    let after = database
        .turn_scope(&turn.user_message_id)
        .expect("scope")
        .expect("present")
        .snapshot_bytes;
    assert_eq!(
        after - prepared,
        3,
        "actual-after 的 3 字节必须计入本轮用量"
    );
}

/// 本地命令记录在数据库里也是 `role='user'`，但不能成为撤销目标。
#[test]
fn latest_user_turn_skips_shell_command_records() {
    let (database, workspace) = temp("shell-latest");
    let turn = started(&database, &workspace);
    sealed(&database, &workspace, &turn);
    database
        .begin_user_shell(
            &workspace,
            Some(&turn.session_id),
            &crate::db::chat::UserShellRecord::starting("pwd", "/tmp", "/bin/sh"),
        )
        .expect("shell record");
    let latest = database
        .latest_user_turn(&workspace, &turn.session_id)
        .expect("latest")
        .expect("turn");
    assert_eq!(latest.user_message_id, turn.user_message_id);
    assert_eq!(latest.text, "hello");
}

/// 本地命令与聊天共用 sequence 编号空间：撤销提示的高水位必须被命令看到。
#[test]
fn shell_sequence_is_after_persisted_notice() {
    let (database, workspace) = temp("shell-sequence");
    let first = started(&database, &workspace);
    sealed(&database, &workspace, &first);
    let turn = next_turn(&database, &workspace, &first.session_id);
    commit_delete(&database, &workspace, &turn, "op-shell-seq");
    let notices = database
        .load_notices(&workspace, &turn.session_id)
        .expect("notices");
    let notice_sequence = notices[0].sequence;
    let mut last = 0;
    // 连续三条命令覆盖旧实现的 2/3/4：第三条会撞上提示占用的序号。
    for _ in 0..3 {
        let committed = database
            .begin_user_shell(
                &workspace,
                Some(&turn.session_id),
                &crate::db::chat::UserShellRecord::starting("pwd", "/tmp", "/bin/sh"),
            )
            .expect("shell");
        last = committed.sequence;
    }
    assert!(
        last > notice_sequence,
        "命令序号必须在提示之后：{last} vs {notice_sequence}"
    );
}

/// 读后版本保护（持久层）：缺 token 或 token 与 before 不符时整批零写入。
#[test]
fn prepare_rejects_missing_or_stale_versions_without_rows() {
    let (database, workspace) = temp("stale-read");
    // 仍处于 capturing：prepare 的读版本校验发生在写入前，与 scope 封口无关。
    let committed = started(&database, &workspace);

    // 缺 token：非 create 一律拒绝。
    let mut missing = change("a.txt", b"before", b"after");
    missing.expected_version = None;
    assert!(matches!(
        database.prepare_mutations(
            &workspace,
            &committed.session_id,
            &committed.user_message_id,
            "call-1",
            &[missing],
        ),
        Err(DbError::StaleRead)
    ));
    // token 与 before 不符：同样拒绝。
    let mut stale = change("a.txt", b"before", b"after");
    stale.expected_version = Some(token(b"different"));
    assert!(matches!(
        database.prepare_mutations(
            &workspace,
            &committed.session_id,
            &committed.user_message_id,
            "call-2",
            &[stale],
        ),
        Err(DbError::StaleRead)
    ));
    // 两次拒绝都零写入：没有 mutation 行、scope 用量未增加。
    let mutations: i64 = database
        .query_count("SELECT COUNT(*) FROM agent_file_mutation")
        .expect("count");
    assert_eq!(mutations, 0, "拒绝必须零写入");
    let bytes: i64 = database
        .query_count(&format!(
            "SELECT snapshot_bytes FROM agent_turn_file_scope WHERE user_message_id = '{}'",
            committed.user_message_id
        ))
        .expect("scope");
    assert_eq!(bytes, 0, "拒绝不得计入快照用量");
}

/// create 不要求 token（前置条件是目标不存在），其余操作必须携带。
#[test]
fn prepare_allows_create_without_version() {
    let (database, workspace) = temp("create-version");
    let committed = started(&database, &workspace);
    let mut created = change("new.txt", b"", b"content");
    created.operation = "create".into();
    created.before = crate::db::chat::ImageWrite {
        kind: "missing".into(),
        bytes: None,
        mode: None,
        link_target: None,
        git_oid: None,
        worktree_root: None,
        git_dir: None,
        head_ref: None,
        head_oid: None,
    };
    created.expected_version = None;
    assert!(
        database
            .prepare_mutations(
                &workspace,
                &committed.session_id,
                &committed.user_message_id,
                "call-create",
                &[created],
            )
            .is_ok(),
        "新建文件无需 token"
    );
}

/// `move_dst` 与 create 同类：目标原本不存在，没有可读版本，不得按“缺 token”拒绝。
#[test]
fn prepare_allows_move_destination_without_version() {
    let (database, workspace) = temp("move-dst-version");
    let committed = started(&database, &workspace);
    let missing_image = || crate::db::chat::ImageWrite {
        kind: "missing".into(),
        bytes: None,
        mode: None,
        link_target: None,
        git_oid: None,
        worktree_root: None,
        git_dir: None,
        head_ref: None,
        head_oid: None,
    };
    let mut source = change("old.txt", b"old", b"");
    source.operation = "move_src".into();
    source.planned_after = missing_image();
    let destination = PathMutationWrite {
        operation: "move_dst".into(),
        path: "old.txt".into(),
        target_path: "new.txt".into(),
        anchor_path: None,
        anchor_canonical: None,
        before: missing_image(),
        planned_after: regular(b"old"),
        expected_version: None,
    };
    let ticket = database
        .prepare_mutations(
            &workspace,
            &committed.session_id,
            &committed.user_message_id,
            "call-move",
            &[source, destination],
        )
        .expect("移动目标不需要读版本");
    assert_eq!(ticket.ids.len(), 2);
}
