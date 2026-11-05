//! `exec` 事件映射与人类可读格式测试：工具调用 → `ThreadItemDetails`、
//! 头部/结算行/摘要格式。
//!
//! 工具结果夹具复用 `processor_tests` 的构造器。

use std::path::PathBuf;

use super::super::events::{
    FileChangeEntry, ItemStatus, ThreadItemDetails, format_tool_header, format_tool_outcome_line,
    format_tool_summary, item_details_for_finish, item_details_for_start,
};
use super::processor_tests::{
    agent_list_item, agent_status, call, process_result, read_call, read_result, subagent_outcome,
    web_search_result,
};
use crate::chat::agent::{AgentRole, SubagentOutcome, WaitMode};
use crate::chat::agent_loop::ToolOutcome;
use crate::chat::plan::{PlanSnapshot, PlanStep, PlanStepStatus};
use crate::chat::process::{ProcessResultKind, ProcessState};
use crate::chat::tools::{
    FileChange, FileChangeKind, McpToolResult, ToolErrorCode, ToolExecutionResult,
};

// ===== 工具 → 事件映射 =====

#[test]
fn command_execution_is_mapped_from_start_to_finish() {
    let call = call("exec_command", r#"{"cmd":"cargo check","workdir":"/work"}"#);
    match item_details_for_start(&call) {
        ThreadItemDetails::CommandExecution {
            command,
            aggregated_output,
            exit_code,
            status,
        } => {
            assert_eq!(command, "cargo check");
            assert!(aggregated_output.is_empty());
            assert_eq!(exit_code, None);
            assert_eq!(status, ItemStatus::InProgress);
        }
        other => panic!("expected command execution, got {other:?}"),
    }

    let outcome = ToolOutcome::Executed(process_result(ProcessState::Exited { code: Some(0) }));
    match item_details_for_finish(&call, &outcome) {
        ThreadItemDetails::CommandExecution {
            command,
            aggregated_output,
            exit_code,
            status,
        } => {
            assert_eq!(command, "cargo check");
            assert_eq!(aggregated_output, "ok\n");
            assert_eq!(exit_code, Some(0));
            assert_eq!(status, ItemStatus::Completed);
        }
        other => panic!("expected command execution, got {other:?}"),
    }

    let timed_out = ToolOutcome::Executed(process_result(ProcessState::TimedOut));
    match item_details_for_finish(&call, &timed_out) {
        ThreadItemDetails::CommandExecution { status, .. } => {
            assert_eq!(status, ItemStatus::Failed);
        }
        other => panic!("expected command execution, got {other:?}"),
    }
}

#[test]
fn file_changes_are_mapped_for_edit_and_write() {
    let edit = call(
        "edit",
        "{\"patch\":\"*** Begin Patch\\n*** Update File: a.rs\\n*** Add File: b.rs\\n*** End Patch\"}",
    );
    match item_details_for_start(&edit) {
        ThreadItemDetails::FileChange { changes, status } => {
            assert_eq!(status, ItemStatus::InProgress);
            assert_eq!(
                changes,
                vec![
                    FileChangeEntry {
                        path: "a.rs".to_string(),
                        kind: "updated".to_string(),
                    },
                    FileChangeEntry {
                        path: "b.rs".to_string(),
                        kind: "added".to_string(),
                    },
                ]
            );
        }
        other => panic!("expected file change, got {other:?}"),
    }

    let executed = ToolOutcome::Executed(ToolExecutionResult::Edit {
        changes: vec![FileChange {
            path: PathBuf::from("/work/a.rs"),
            added: 2,
            removed: 1,
            kind: FileChangeKind::Updated,
            diff: None,
        }],
        partial: false,
    });
    match item_details_for_finish(&edit, &executed) {
        ThreadItemDetails::FileChange { changes, status } => {
            assert_eq!(status, ItemStatus::Completed);
            assert_eq!(
                changes,
                vec![FileChangeEntry {
                    path: "/work/a.rs".to_string(),
                    kind: "updated".to_string(),
                }]
            );
        }
        other => panic!("expected file change, got {other:?}"),
    }

    let write = call("write", r#"{"path":"notes.md","content":"x"}"#);
    let executed = ToolOutcome::Executed(ToolExecutionResult::Write {
        path: PathBuf::from("/work/notes.md"),
        created: true,
        added: 1,
        removed: 0,
        diff: None,
    });
    match item_details_for_finish(&write, &executed) {
        ThreadItemDetails::FileChange { changes, status } => {
            assert_eq!(status, ItemStatus::Completed);
            assert_eq!(
                changes,
                vec![FileChangeEntry {
                    path: "/work/notes.md".to_string(),
                    kind: "added".to_string(),
                }]
            );
        }
        other => panic!("expected file change, got {other:?}"),
    }
}

#[test]
fn web_search_and_plan_are_mapped_to_their_own_shapes() {
    let search = call("web_search", r#"{"queries":["ratatui","tokio"]}"#);
    match item_details_for_start(&search) {
        ThreadItemDetails::WebSearch { query, status } => {
            assert_eq!(query, "ratatui, tokio");
            assert_eq!(status, ItemStatus::InProgress);
        }
        other => panic!("expected web search, got {other:?}"),
    }
    let executed = ToolOutcome::Executed(web_search_result(8));
    match item_details_for_finish(&search, &executed) {
        ThreadItemDetails::WebSearch { query, status } => {
            assert_eq!(query, "ratatui, tokio");
            assert_eq!(status, ItemStatus::Completed);
        }
        other => panic!("expected web search, got {other:?}"),
    }

    let plan = call(
        "update_plan",
        r#"{"plan":[{"step":"inspect","status":"completed"}]}"#,
    );
    match item_details_for_start(&plan) {
        ThreadItemDetails::TodoList { items } => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].step, "inspect");
            assert_eq!(items[0].status, "completed");
        }
        other => panic!("expected todo list, got {other:?}"),
    }
    let executed = ToolOutcome::Executed(ToolExecutionResult::Plan(PlanSnapshot {
        explanation: None,
        steps: vec![
            PlanStep {
                step: "inspect".to_string(),
                status: PlanStepStatus::Completed,
            },
            PlanStep {
                step: "patch".to_string(),
                status: PlanStepStatus::InProgress,
            },
        ],
        revision: 1,
    }));
    match item_details_for_finish(&plan, &executed) {
        ThreadItemDetails::TodoList { items } => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[1].step, "patch");
            assert_eq!(items[1].status, "in_progress");
        }
        other => panic!("expected todo list, got {other:?}"),
    }
}

#[test]
fn mcp_calls_are_split_into_server_and_tool() {
    let call = call("mcp__exa__search", r#"{"query":"rust"}"#);
    match item_details_for_start(&call) {
        ThreadItemDetails::McpToolCall {
            server,
            tool,
            status,
            error,
            ..
        } => {
            assert_eq!(server, "exa");
            assert_eq!(tool, "search");
            assert_eq!(status, ItemStatus::InProgress);
            assert!(error.is_none());
        }
        other => panic!("expected MCP tool call, got {other:?}"),
    }

    let executed = ToolOutcome::Executed(ToolExecutionResult::Mcp(Box::new(McpToolResult {
        server: "exa".to_string(),
        tool: "search".to_string(),
        text: "result".to_string(),
        images: Vec::new(),
        images_omitted: 0,
        duration_ms: Some(12),
        truncated: false,
    })));
    match item_details_for_finish(&call, &executed) {
        ThreadItemDetails::McpToolCall {
            server,
            tool,
            status,
            error,
            ..
        } => {
            assert_eq!(server, "exa");
            assert_eq!(tool, "search");
            assert_eq!(status, ItemStatus::Completed);
            assert!(error.is_none());
        }
        other => panic!("expected MCP tool call, got {other:?}"),
    }

    let failed = ToolOutcome::Executed(ToolExecutionResult::Failed {
        code: ToolErrorCode::McpError,
        message: "server unavailable".to_string(),
        changes: Vec::new(),
    });
    match item_details_for_finish(&call, &failed) {
        ThreadItemDetails::McpToolCall { status, error, .. } => {
            assert_eq!(status, ItemStatus::Failed);
            assert_eq!(error.as_deref(), Some("server unavailable"));
        }
        other => panic!("expected MCP tool call, got {other:?}"),
    }
}

#[test]
fn collab_calls_carry_the_agent_identity() {
    let call = call(
        "spawn_agent",
        r#"{"role":"explorer","title":"inspect cli"}"#,
    );
    match item_details_for_start(&call) {
        ThreadItemDetails::CollabToolCall {
            tool,
            role,
            title,
            status,
            ..
        } => {
            assert_eq!(tool, "spawn_agent");
            assert_eq!(role.as_deref(), Some("explorer"));
            assert_eq!(title.as_deref(), Some("inspect cli"));
            assert_eq!(status, ItemStatus::InProgress);
        }
        other => panic!("expected collab tool call, got {other:?}"),
    }

    let executed = ToolOutcome::Executed(ToolExecutionResult::Subagent(Box::new(
        SubagentOutcome::Spawned {
            agent_id: "sub-1".to_string(),
            nickname: None,
            agent_path: None,
            role: AgentRole::Explorer,
            title: "inspect cli".to_string(),
            model: None,
            reasoning_effort: None,
            fork_context: false,
            fork_turns: None,
            max_tokens: None,
        },
    )));
    match item_details_for_finish(&call, &executed) {
        ThreadItemDetails::CollabToolCall {
            tool,
            agent_id,
            role,
            title,
            status,
            nickname,
            model,
            fork_context,
        } => {
            assert_eq!(tool, "spawn_agent");
            assert_eq!(agent_id.as_deref(), Some("sub-1"));
            assert_eq!(role.as_deref(), Some("explorer"));
            assert_eq!(title.as_deref(), Some("inspect cli"));
            assert_eq!(status, ItemStatus::Completed);
            assert_eq!(nickname, None);
            assert_eq!(model, None);
            assert_eq!(fork_context, None);
        }
        other => panic!("expected collab tool call, got {other:?}"),
    }

    let rich = ToolOutcome::Executed(ToolExecutionResult::Subagent(Box::new(
        SubagentOutcome::Spawned {
            agent_id: "sub-2".to_string(),
            nickname: Some("Dijkstra".to_string()),
            agent_path: Some("/root/dijkstra".to_string()),
            role: AgentRole::parse("reviewer").expect("role"),
            title: "audit auth".to_string(),
            model: Some("openai/gpt-5".to_string()),
            reasoning_effort: Some("high".to_string()),
            fork_context: true,
            fork_turns: Some(3),
            max_tokens: Some(50_000),
        },
    )));
    match item_details_for_finish(&call, &rich) {
        ThreadItemDetails::CollabToolCall {
            nickname,
            model,
            fork_context,
            ..
        } => {
            assert_eq!(nickname.as_deref(), Some("Dijkstra"));
            assert_eq!(model.as_deref(), Some("openai/gpt-5 (high)"));
            assert_eq!(fork_context, Some(true));
        }
        other => panic!("expected collab tool call, got {other:?}"),
    }
}

/// 7 个子 agent 控制工具的 schema 目标字段各不相同，起始标题与事件项都要认得。
#[test]
fn collab_start_identity_follows_each_tool_schema() {
    let cases = [
        (
            call(
                "spawn_agent",
                r#"{"role":"explorer","title":"inspect cli"}"#,
            ),
            "spawn_agent explorer · inspect cli",
            None,
        ),
        (
            call("send_input", r#"{"target":"sub-1"}"#),
            "send_input sub-1",
            Some("sub-1"),
        ),
        (
            call("resume_agent", r#"{"id":"sub-2"}"#),
            "resume_agent sub-2",
            Some("sub-2"),
        ),
        (
            call("close_agent", r#"{"target":"sub-3"}"#),
            "close_agent sub-3",
            Some("sub-3"),
        ),
        (
            call("wait_agent", r#"{"targets":["sub-1","sub-2"]}"#),
            "wait_agent sub-1, sub-2",
            Some("sub-1, sub-2"),
        ),
        (call("wait_agent", "{}"), "wait_agent (all)", None),
        (
            call("cancel_agent_input", r#"{"submission_id":"sub-9"}"#),
            "cancel_agent_input sub-9",
            Some("sub-9"),
        ),
    ];
    for (call, header, agent_id) in cases {
        assert_eq!(format_tool_header(&call), header, "{call:?}");
        match item_details_for_start(&call) {
            ThreadItemDetails::CollabToolCall {
                agent_id: actual,
                status,
                ..
            } => {
                assert_eq!(actual.as_deref(), agent_id, "{header}");
                assert_eq!(status, ItemStatus::InProgress, "{header}");
            }
            other => panic!("expected collab tool call, got {other:?}"),
        }
    }
}

/// 多目标等待/列表必须聚合全部标识，而不是只保留第一个。
#[test]
fn collab_finish_aggregates_every_settled_agent() {
    let wait_call = call("wait_agent", r#"{"targets":["sub-1","sub-2"]}"#);
    let waited = subagent_outcome(SubagentOutcome::Waited {
        timed_out: false,
        agents: vec![
            agent_status("sub-1", AgentRole::Explorer, "inspect cli", "completed"),
            agent_status("sub-2", AgentRole::Explorer, "scan docs", "completed"),
        ],
        all: false,
        mode: WaitMode::All,
    });
    match item_details_for_finish(&wait_call, &waited) {
        ThreadItemDetails::CollabToolCall {
            agent_id,
            role,
            title,
            status,
            ..
        } => {
            assert_eq!(agent_id.as_deref(), Some("sub-1, sub-2"));
            assert_eq!(role.as_deref(), Some("explorer"));
            assert_eq!(title.as_deref(), Some("inspect cli, scan docs"));
            assert_eq!(status, ItemStatus::Completed);
        }
        other => panic!("expected collab tool call, got {other:?}"),
    }

    // 角色不一致时不能谎报单一角色；空标题不参与拼接。
    let mixed = subagent_outcome(SubagentOutcome::Waited {
        timed_out: false,
        agents: vec![
            agent_status("sub-1", AgentRole::Explorer, "inspect cli", "completed"),
            agent_status("sub-2", AgentRole::Executor, "", "failed"),
        ],
        all: false,
        mode: WaitMode::All,
    });
    match item_details_for_finish(&wait_call, &mixed) {
        ThreadItemDetails::CollabToolCall {
            agent_id,
            role,
            title,
            ..
        } => {
            assert_eq!(agent_id.as_deref(), Some("sub-1, sub-2"));
            assert_eq!(role, None);
            assert_eq!(title.as_deref(), Some("inspect cli"));
        }
        other => panic!("expected collab tool call, got {other:?}"),
    }

    let list = call("list_agents", "{}");
    let listed = subagent_outcome(SubagentOutcome::Listed {
        agents: vec![
            agent_list_item("sub-1", AgentRole::Explorer, "inspect cli"),
            agent_list_item("sub-2", AgentRole::Executor, "scan docs"),
        ],
    });
    match item_details_for_finish(&list, &listed) {
        ThreadItemDetails::CollabToolCall { agent_id, .. } => {
            assert_eq!(agent_id.as_deref(), Some("sub-1, sub-2"));
        }
        other => panic!("expected collab tool call, got {other:?}"),
    }
}

/// `wait_agent` 超时且无人结束才算失败；拿到部分报告的超时仍算正常结算。
#[test]
fn wait_agent_timeout_is_marked_failed() {
    let call = call("wait_agent", r#"{"targets":["sub-1"]}"#);
    let timed_out = subagent_outcome(SubagentOutcome::Waited {
        timed_out: true,
        agents: Vec::new(),
        all: false,
        mode: WaitMode::All,
    });
    let (line, is_error) = format_tool_outcome_line(&call, &timed_out, 500);
    assert_eq!(line, "wait timed out (500ms)");
    assert!(is_error);
    match item_details_for_finish(&call, &timed_out) {
        ThreadItemDetails::CollabToolCall { status, .. } => {
            assert_eq!(status, ItemStatus::Failed);
        }
        other => panic!("expected collab tool call, got {other:?}"),
    }

    let partial = subagent_outcome(SubagentOutcome::Waited {
        timed_out: true,
        agents: vec![agent_status(
            "sub-1",
            AgentRole::Explorer,
            "inspect",
            "failed",
        )],
        all: false,
        mode: WaitMode::All,
    });
    let (line, is_error) = format_tool_outcome_line(&call, &partial, 500);
    assert_eq!(line, "wait timed out (500ms)");
    assert!(!is_error);
    match item_details_for_finish(&call, &partial) {
        ThreadItemDetails::CollabToolCall { status, .. } => {
            assert_eq!(status, ItemStatus::Completed);
        }
        other => panic!("expected collab tool call, got {other:?}"),
    }
}

#[test]
fn generic_tools_carry_a_summary_and_cancelled_status() {
    let read = read_call();
    let executed = ToolOutcome::Executed(read_result());
    match item_details_for_finish(&read, &executed) {
        ThreadItemDetails::ToolCall {
            name,
            summary,
            status,
            ..
        } => {
            assert_eq!(name, "read");
            assert_eq!(summary.as_deref(), Some("Read 57 lines"));
            assert_eq!(status, ItemStatus::Completed);
        }
        other => panic!("expected generic tool call, got {other:?}"),
    }

    match item_details_for_finish(&read, &ToolOutcome::Cancelled) {
        ThreadItemDetails::ToolCall {
            status, summary, ..
        } => {
            assert_eq!(status, ItemStatus::Cancelled);
            assert!(summary.is_none());
        }
        other => panic!("expected generic tool call, got {other:?}"),
    }

    let cancelled = call("exec_command", r#"{"cmd":"sleep 10"}"#);
    match item_details_for_finish(&cancelled, &ToolOutcome::Cancelled) {
        ThreadItemDetails::CommandExecution {
            command, status, ..
        } => {
            assert_eq!(command, "sleep 10");
            assert_eq!(status, ItemStatus::Cancelled);
        }
        other => panic!("expected command execution, got {other:?}"),
    }
}

// ===== 人类可读单行格式 =====

#[test]
fn headers_name_the_tool_and_its_primary_argument() {
    assert_eq!(format_tool_header(&read_call()), "read Cargo.toml");
    assert_eq!(
        format_tool_header(&call(
            "exec_command",
            r#"{"cmd":"cargo check","workdir":"/work"}"#
        )),
        "exec_command `cargo check` in /work"
    );
    assert_eq!(
        format_tool_header(&call(
            "edit",
            r#"{"patch":"*** Update File: a.rs\n*** End Patch"}"#
        )),
        "edit a.rs"
    );
    assert_eq!(
        format_tool_header(&call("mcp__exa__search", "{}")),
        "mcp exa/search"
    );
}

#[test]
fn outcome_lines_report_duration_and_failure() {
    let call = read_call();
    let (line, is_error) =
        format_tool_outcome_line(&call, &ToolOutcome::Executed(read_result()), 12);
    assert_eq!(line, "Read 57 lines (12ms)");
    assert!(!is_error);

    let failed = ToolOutcome::Executed(ToolExecutionResult::Failed {
        code: ToolErrorCode::Io,
        message: "disk full".to_string(),
        changes: Vec::new(),
    });
    let (line, is_error) = format_tool_outcome_line(&call, &failed, 5);
    assert_eq!(line, "io: disk full (5ms)");
    assert!(is_error);

    let (line, is_error) = format_tool_outcome_line(
        &call,
        &ToolOutcome::Executed(process_result(ProcessState::Exited { code: Some(0) })),
        420,
    );
    assert_eq!(line, "succeeded in 420ms");
    assert!(!is_error);

    let (line, is_error) = format_tool_outcome_line(&call, &ToolOutcome::Cancelled, 0);
    assert_eq!(line, "cancelled");
    assert!(!is_error);
}

/// `write_stdin` 只读回后台进程的一段输出：耗时取本次交互，而非进程总生存时间。
#[test]
fn write_stdin_reports_the_interaction_duration() {
    let call = call("write_stdin", r#"{"session_id":7}"#);
    let mut result = process_result(ProcessState::Running);
    match &mut result {
        ToolExecutionResult::Process(process) => process.kind = ProcessResultKind::WriteStdin,
        other => panic!("expected process result, got {other:?}"),
    }
    // 夹具的 `wall_time_ms` 为 420，必须被本次调用的 9ms 覆盖。
    let (line, is_error) = format_tool_outcome_line(&call, &ToolOutcome::Executed(result), 9);
    assert_eq!(line, "running in background (9ms)");
    assert!(!is_error);
}

/// `web_search` 结算行与 JSON 摘要都要带命中条数。
#[test]
fn web_search_outcome_reports_the_result_count() {
    let call = call("web_search", r#"{"queries":["ratatui","tokio"]}"#);
    let many = ToolOutcome::Executed(web_search_result(8));
    let (line, is_error) = format_tool_outcome_line(&call, &many, 840);
    assert_eq!(line, "Searched \"ratatui, tokio\" · 8 results (840ms)");
    assert!(!is_error);
    assert_eq!(
        format_tool_summary(&call, &many).as_deref(),
        Some("Searched \"ratatui, tokio\" · 8 results")
    );

    let one = ToolOutcome::Executed(web_search_result(1));
    let (line, _) = format_tool_outcome_line(&call, &one, 120);
    assert_eq!(line, "Searched \"ratatui, tokio\" · 1 result (120ms)");
}
