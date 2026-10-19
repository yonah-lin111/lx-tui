//! 工具调用持久化测试：状态机、位置分配、归属校验、大小上限与异常恢复。

use std::path::PathBuf;

use super::*;

/// 独立临时目录中的数据库路径。
fn temp_db_path(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos());
    std::env::temp_dir().join(format!(
        "lx-tui-tool-test-{label}-{}",
        nanos.map(|value| format!("{value:x}")).unwrap_or_default()
    ))
}

fn open_test_database(label: &str) -> Database {
    Database::open(&temp_db_path(label)).expect("database opens")
}

fn workspace_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("lx-tui-tool-ws-{label}"))
}

fn model() -> super::super::ModelSnapshot {
    super::super::ModelSnapshot {
        provider: "example".into(),
        model: "example-chat".into(),
        variant: None,
    }
}

/// 建库、信任工作区并开启一轮，返回（数据库，工作区，会话 id，assistant 消息 id）。
fn session(label: &str) -> (Database, PathBuf, String, String) {
    let database = open_test_database(label);
    let workspace = workspace_path(label);
    database.trust_workspace(&workspace, "ws").expect("trust");
    let committed = database
        .begin_turn(&workspace, None, "hi", &model())
        .expect("begin turn");
    (
        database,
        workspace,
        committed.session_id,
        committed.assistant_message_id,
    )
}

/// 一次完整的 pending -> running -> completed 流程。
fn completed_call(
    database: &Database,
    workspace: &std::path::Path,
    session_id: &str,
    message_id: &str,
    call_id: &str,
) {
    let ticket = database
        .begin_tool_call(
            workspace,
            session_id,
            message_id,
            call_id,
            "read",
            r#"{"path":"a"}"#,
        )
        .expect("begin");
    database
        .mark_tool_running(workspace, &ticket)
        .expect("running");
    database
        .finish_tool_call(
            workspace,
            &ticket,
            &ToolCallResult::completed(Some(r#"{"content":"ok"}"#.into())),
        )
        .expect("finish");
}

#[test]
fn begin_records_pending_row_and_committed_ticket() {
    let (database, workspace, session_id, message_id) = session("begin");
    let ticket = database
        .begin_tool_call(
            &workspace,
            &session_id,
            &message_id,
            "call-1",
            "read",
            r#"{"path":"src/lib.rs"}"#,
        )
        .expect("begin");
    assert_eq!(ticket.session_id, session_id);
    assert_eq!(ticket.message_id, message_id);
    assert_eq!(ticket.call_id, "call-1");
    assert_eq!(ticket.position, 0, "空消息从 0 开始分配位置");
    let calls = database
        .load_tool_calls_for_session(&workspace, &session_id)
        .expect("load");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status, "pending");
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].arguments_json, r#"{"path":"src/lib.rs"}"#);
    assert_eq!(calls[0].completed_at_ms, None);
    assert_eq!(calls[0].duration_ms, None);
}

#[test]
fn state_machine_reaches_each_terminal_status() {
    let (database, workspace, session_id, message_id) = session("terminal");
    // pending -> running -> completed
    let ticket = database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-ok", "read", "{}")
        .expect("begin ok");
    database
        .mark_tool_running(&workspace, &ticket)
        .expect("running ok");
    database
        .finish_tool_call(&workspace, &ticket, &ToolCallResult::completed(None))
        .expect("completed");
    // pending -> running -> failed
    let failed = database
        .begin_tool_call(
            &workspace,
            &session_id,
            &message_id,
            "c-fail",
            "write",
            "{}",
        )
        .expect("begin fail");
    database
        .mark_tool_running(&workspace, &failed)
        .expect("running fail");
    database
        .finish_tool_call(&workspace, &failed, &ToolCallResult::failed("io error"))
        .expect("failed");
    // pending -> cancelled
    let cancelled = database
        .begin_tool_call(
            &workspace,
            &session_id,
            &message_id,
            "c-cancel",
            "edit",
            "{}",
        )
        .expect("begin cancel");
    database
        .finish_tool_call(
            &workspace,
            &cancelled,
            &ToolCallResult {
                status: ToolTerminalStatus::Cancelled,
                result_json: None,
                error_text: Some("cancelled by user".into()),
            },
        )
        .expect("cancelled");
    let calls = database
        .load_tool_calls_for_session(&workspace, &session_id)
        .expect("load");
    let statuses: Vec<(&str, &str)> = calls
        .iter()
        .map(|call| (call.call_id.as_str(), call.status.as_str()))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("c-ok", "completed"),
            ("c-fail", "failed"),
            ("c-cancel", "cancelled"),
        ]
    );
    let failed_row = calls
        .iter()
        .find(|call| call.call_id == "c-fail")
        .expect("row");
    assert_eq!(failed_row.error_text.as_deref(), Some("io error"));
    assert_eq!(failed_row.status, "failed");
    assert!(failed_row.completed_at_ms.is_some());
    assert!(failed_row.duration_ms.is_some());
}

#[test]
fn illegal_transitions_are_rejected() {
    let (database, workspace, session_id, message_id) = session("illegal");
    let ticket = database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-1", "read", "{}")
        .expect("begin");
    // pending 直接 finish 是非法转移（必须先 running）。
    assert!(matches!(
        database.finish_tool_call(&workspace, &ticket, &ToolCallResult::completed(None)),
        Err(DbError::InvalidToolTransition)
    ));
    // pending 直接 failed 同样非法（只有 cancelled 允许从 pending 结算）。
    assert!(matches!(
        database.finish_tool_call(&workspace, &ticket, &ToolCallResult::failed("boom")),
        Err(DbError::InvalidToolTransition)
    ));
    database
        .mark_tool_running(&workspace, &ticket)
        .expect("running");
    // running 重复 mark 幂等。
    database
        .mark_tool_running(&workspace, &ticket)
        .expect("running idempotent");
    database
        .finish_tool_call(&workspace, &ticket, &ToolCallResult::completed(None))
        .expect("complete");
    // 已终态后再 mark running 非法。
    assert!(matches!(
        database.mark_tool_running(&workspace, &ticket),
        Err(DbError::InvalidToolTransition)
    ));
}

#[test]
fn finish_is_idempotent_for_same_result_and_rejects_different_result() {
    let (database, workspace, session_id, message_id) = session("finish-idempotent");
    let ticket = database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-1", "read", "{}")
        .expect("begin");
    database
        .mark_tool_running(&workspace, &ticket)
        .expect("running");
    let result = ToolCallResult::completed(Some(r#"{"content":"a"}"#.into()));
    for _ in 0..2 {
        database
            .finish_tool_call(&workspace, &ticket, &result)
            .expect("same result idempotent");
    }
    assert!(matches!(
        database.finish_tool_call(
            &workspace,
            &ticket,
            &ToolCallResult::completed(Some(r#"{"content":"b"}"#.into()))
        ),
        Err(DbError::ToolTerminalConflict)
    ));
    // 不同终态同样拒绝，原记录不被覆盖。
    assert!(matches!(
        database.finish_tool_call(&workspace, &ticket, &ToolCallResult::failed("boom")),
        Err(DbError::ToolTerminalConflict)
    ));
    let calls = database
        .load_tool_calls_for_session(&workspace, &session_id)
        .expect("load");
    assert_eq!(calls[0].status, "completed");
    assert_eq!(calls[0].result_json.as_deref(), Some(r#"{"content":"a"}"#));
}

#[test]
fn duplicate_call_id_is_rejected_and_unknown_ticket_is_missing() {
    let (database, workspace, session_id, message_id) = session("duplicate");
    database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-1", "read", "{}")
        .expect("begin");
    assert!(matches!(
        database.begin_tool_call(&workspace, &session_id, &message_id, "c-1", "read", "{}"),
        Err(DbError::ToolCallConflict)
    ));
    let ghost = ToolCallTicket {
        session_id: session_id.clone(),
        message_id: message_id.clone(),
        call_id: "missing".into(),
        position: 99,
        started_at_ms: 0,
    };
    assert!(matches!(
        database.mark_tool_running(&workspace, &ghost),
        Err(DbError::ToolCallMissing)
    ));
    assert!(matches!(
        database.finish_tool_call(&workspace, &ghost, &ToolCallResult::completed(None)),
        Err(DbError::ToolCallMissing)
    ));
}

#[test]
fn cross_workspace_and_foreign_message_are_rejected() {
    let database = open_test_database("cross-workspace");
    let workspace_a = workspace_path("cross-workspace-a");
    let workspace_b = workspace_path("cross-workspace-b");
    database
        .trust_workspace(&workspace_a, "a")
        .expect("trust a");
    database
        .trust_workspace(&workspace_b, "b")
        .expect("trust b");
    let committed = database
        .begin_turn(&workspace_a, None, "hi", &model())
        .expect("begin");
    // B 工作区不能给 A 的消息落工具行。
    assert!(matches!(
        database.begin_tool_call(
            &workspace_b,
            &committed.session_id,
            &committed.assistant_message_id,
            "c-1",
            "read",
            "{}",
        ),
        Err(DbError::SessionMissing)
    ));
    // A 工作区内，消息不属于该会话时拒绝。
    assert!(matches!(
        database.begin_tool_call(
            &workspace_a,
            &committed.session_id,
            "msg-missing",
            "c-1",
            "read",
            "{}",
        ),
        Err(DbError::ToolCallMissing)
    ));
    // 读该会话工具行也必须同工作区。
    assert!(matches!(
        database.load_tool_calls_for_session(&workspace_b, &committed.session_id),
        Err(DbError::SessionMissing)
    ));
}

#[test]
fn positions_interleave_with_parts_and_stay_monotonic() {
    let (database, workspace, session_id, message_id) = session("positions");
    // 先落一个 text part（position 0），再依次落工具。
    database
        .checkpoint(
            &workspace,
            &message_id,
            &[super::super::ChatPart {
                position: 0,
                kind: "text".into(),
                content: "intro".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
        )
        .expect("checkpoint");
    let first = database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-1", "read", "{}")
        .expect("begin 1");
    assert_eq!(first.position, 1, "接在既有 part 之后");
    let second = database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-2", "read", "{}")
        .expect("begin 2");
    assert_eq!(second.position, 2);
    // 写回一个占位 4 的 part 后，下一个工具接在 4 之后。
    database
        .checkpoint(
            &workspace,
            &message_id,
            &[
                super::super::ChatPart {
                    position: 0,
                    kind: "text".into(),
                    content: "intro".into(),
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                },
                super::super::ChatPart {
                    position: 4,
                    kind: "text".into(),
                    content: "later".into(),
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                },
            ],
        )
        .expect("checkpoint keeps positions");
    let third = database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-3", "read", "{}")
        .expect("begin 3");
    assert_eq!(third.position, 5);
    let calls = database
        .load_tool_calls_for_session(&workspace, &session_id)
        .expect("load");
    let positions: Vec<i64> = calls.iter().map(|call| call.position).collect();
    assert_eq!(positions, vec![1, 2, 5]);
}

#[test]
fn duplicate_positions_in_parts_are_rejected_without_half_write() {
    let (database, workspace, _session_id, message_id) = session("position-conflict");
    let base = super::super::ChatPart {
        position: 0,
        kind: "text".into(),
        content: "a".into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    };
    let duplicate = super::super::ChatPart {
        position: 0,
        content: "b".into(),
        ..base.clone()
    };
    assert!(matches!(
        database.checkpoint(&workspace, &message_id, &[base, duplicate]),
        Err(DbError::PositionConflict)
    ));
    // 事务回滚：消息内不留任何 part。
    let messages = database
        .load_chat_session(
            &workspace,
            &database
                .list_chat_sessions(&workspace, 0, 10)
                .expect("list")
                .first()
                .expect("session")
                .external_id,
        )
        .expect("load")
        .expect("exists");
    assert!(messages[1].parts.is_empty());
}

#[test]
fn part_position_colliding_with_tool_is_rejected() {
    let (database, workspace, session_id, message_id) = session("part-tool-conflict");
    database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-1", "read", "{}")
        .expect("begin");
    assert!(matches!(
        database.checkpoint(
            &workspace,
            &message_id,
            &[super::super::ChatPart {
                position: 0,
                kind: "text".into(),
                content: "x".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
        ),
        Err(DbError::PositionConflict)
    ));
}

#[test]
fn tool_position_unique_constraint_guards_each_message() {
    let (database, workspace, session_id, message_id) = session("tool-unique");
    database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-1", "read", "{}")
        .expect("begin");
    // 库层 UNIQUE(message_id, position) 兜底：绕过服务层直接写同位置必须被拒。
    let result = database.conn.execute(
        r#"INSERT INTO agent_tool_call
               (external_id, session_id, message_id, call_id, name, position, status,
                started_at_ms)
           VALUES ('tc-dup', ?1, ?2, 'c-2', 'read', 0, 'pending', 0)"#,
        rusqlite::params![session_id, message_id],
    );
    assert!(result.is_err(), "同消息同位置被 UNIQUE 拒绝");
}

#[test]
fn oversized_arguments_and_results_are_rejected_without_rows() {
    let (database, workspace, session_id, message_id) = session("size-limits");
    let oversized = "x".repeat(MAX_TOOL_ARGUMENT_BYTES + 1);
    assert!(matches!(
        database.begin_tool_call(
            &workspace,
            &session_id,
            &message_id,
            "c-big",
            "read",
            &oversized
        ),
        Err(DbError::ToolArgumentTooLarge)
    ));
    assert!(
        database
            .load_tool_calls_for_session(&workspace, &session_id)
            .expect("load")
            .is_empty(),
        "超限参数不落半行"
    );
    let ticket = database
        .begin_tool_call(&workspace, &session_id, &message_id, "c-1", "read", "{}")
        .expect("begin");
    database
        .mark_tool_running(&workspace, &ticket)
        .expect("running");
    let big_result = "x".repeat(MAX_TOOL_RESULT_BYTES + 1);
    assert!(matches!(
        database.finish_tool_call(
            &workspace,
            &ticket,
            &ToolCallResult::completed(Some(big_result))
        ),
        Err(DbError::ToolResultTooLarge)
    ));
    let calls = database
        .load_tool_calls_for_session(&workspace, &session_id)
        .expect("load");
    assert_eq!(calls[0].status, "running", "结果超限不改状态");
    assert_eq!(calls[0].result_json, None);
    assert_eq!(calls[0].completed_at_ms, None);
}

#[test]
fn recovery_marks_pending_and_running_interrupted_once() {
    let (database, workspace, session_id, message_id) = session("recover");
    let pending = database
        .begin_tool_call(
            &workspace,
            &session_id,
            &message_id,
            "c-pending",
            "read",
            "{}",
        )
        .expect("begin pending");
    let running = database
        .begin_tool_call(
            &workspace,
            &session_id,
            &message_id,
            "c-running",
            "read",
            "{}",
        )
        .expect("begin running");
    database
        .mark_tool_running(&workspace, &running)
        .expect("running");
    completed_call(&database, &workspace, &session_id, &message_id, "c-done");
    let recovered = database.recover_interrupted_tool_calls().expect("recover");
    assert_eq!(recovered, 2, "只改 pending/running 两行");
    let calls = database
        .load_tool_calls_for_session(&workspace, &session_id)
        .expect("load");
    for call in &calls {
        match call.call_id.as_str() {
            "c-pending" | "c-running" => {
                assert_eq!(call.status, "interrupted");
                assert_eq!(call.error_text.as_deref(), Some(INTERRUPTED_ERROR_CODE));
                assert!(call.completed_at_ms.is_some());
                assert_eq!(call.result_json, None, "中断不伪造成功结果");
            }
            "c-done" => assert_eq!(call.status, "completed"),
            other => panic!("unexpected call id {other}"),
        }
    }
    // 幂等：第二次恢复无行可改。
    assert_eq!(database.recover_interrupted_tool_calls().expect("again"), 0);
    // interrupted 之后不允许再推进 running。
    assert!(matches!(
        database.mark_tool_running(&workspace, &pending),
        Err(DbError::InvalidToolTransition)
    ));
}

#[test]
fn load_chat_session_returns_parts_and_tools_in_position_order() {
    let (database, workspace, session_id, message_id) = session("load-order");
    database
        .checkpoint(
            &workspace,
            &message_id,
            &[super::super::ChatPart {
                position: 0,
                kind: "reasoning".into(),
                content: "思考".into(),
                provider_metadata_json: None,
                started_at_ms: Some(1),
                ended_at_ms: Some(2),
                completed_at_ms: Some(2),
            }],
        )
        .expect("checkpoint");
    completed_call(&database, &workspace, &session_id, &message_id, "c-1");
    let messages = database
        .load_chat_session(&workspace, &session_id)
        .expect("load")
        .expect("exists");
    let assistant = messages.last().expect("assistant");
    assert_eq!(assistant.parts.len(), 1);
    assert_eq!(assistant.parts[0].completed_at_ms, Some(2));
    assert_eq!(assistant.tools.len(), 1);
    assert_eq!(assistant.tools[0].position, 1);
    assert!(messages[0].tools.is_empty(), "用户消息不挂工具行");
}

#[test]
fn checkpoint_keeps_committed_tools_and_preserves_completed_at() {
    let (database, workspace, session_id, message_id) = session("checkpoint-tools");
    completed_call(&database, &workspace, &session_id, &message_id, "c-1");
    database
        .checkpoint(
            &workspace,
            &message_id,
            &[super::super::ChatPart {
                position: 2,
                kind: "text".into(),
                content: "后续正文".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
        )
        .expect("checkpoint");
    let calls = database
        .load_tool_calls_for_session(&workspace, &session_id)
        .expect("load");
    assert_eq!(calls.len(), 1, "checkpoint 不删除工具行");
    assert_eq!(calls[0].status, "completed");
    let messages = database
        .load_chat_session(&workspace, &session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(messages[1].parts[0].position, 2, "位置不被重排");
}

#[test]
fn finish_keeps_tool_rows_and_is_idempotent() {
    let (database, workspace, session_id, message_id) = session("finish-tools");
    completed_call(&database, &workspace, &session_id, &message_id, "c-1");
    for _ in 0..2 {
        database
            .finish(
                &workspace,
                &message_id,
                &[super::super::ChatPart {
                    position: 2,
                    kind: "text".into(),
                    content: "done".into(),
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                }],
                &super::super::TurnOutcome::Complete { stop_reason: None },
                super::super::Usage::default(),
            )
            .expect("finish");
    }
    let calls = database
        .load_tool_calls_for_session(&workspace, &session_id)
        .expect("load");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status, "completed");
}
