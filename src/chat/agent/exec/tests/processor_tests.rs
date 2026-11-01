//! `exec` 输出层测试：人类模式 stdout/stderr 分流、JSONL 事件序列与工具 → 事件映射。

use std::path::PathBuf;

use super::super::events::{
    ExecUsage, FileChangeEntry, ItemStatus, ThreadEvent, ThreadItemDetails, format_tool_header,
    format_tool_outcome_line, item_details_for_finish, item_details_for_start,
};
use super::super::processor::{
    ExecEventProcessor, HumanOutputProcessor, JsonlOutputProcessor, SessionHeader,
};
use crate::chat::agent::{AgentRole, SubagentOutcome};
use crate::chat::agent_loop::{ReadyToolCall, ToolOutcome, TurnSummary};
use crate::chat::plan::{PlanSnapshot, PlanStep, PlanStepStatus};
use crate::chat::process::{ProcessResult, ProcessResultKind, ProcessState};
use crate::chat::tools::{
    FileChange, FileChangeKind, McpToolResult, ReadKind, SearchDepth, ToolErrorCode,
    ToolExecutionResult, WebSearchProvider, WebSearchToolResult,
};
use crate::db::chat::Usage;

// ===== 夹具 =====

fn call(name: &str, arguments: &str) -> ReadyToolCall {
    ReadyToolCall {
        call_id: format!("call-{name}"),
        name: name.to_string(),
        arguments_json: arguments.to_string(),
        ordinal: 0,
    }
}

fn read_call() -> ReadyToolCall {
    call("read", r#"{"path":"Cargo.toml"}"#)
}

fn read_result() -> ToolExecutionResult {
    ToolExecutionResult::Read {
        path: PathBuf::from("/work/Cargo.toml"),
        kind: ReadKind::File,
        content: "1\t[package]\n".to_string(),
        start_line: 1,
        end_line: 57,
        next_offset: None,
        truncated: false,
        total_lines: 57,
        version: Some("v1".to_string()),
        image: None,
    }
}

fn process_result(state: ProcessState) -> ToolExecutionResult {
    ToolExecutionResult::Process(Box::new(ProcessResult {
        kind: ProcessResultKind::ExecCommand,
        session_id: 7,
        command: "cargo check".to_string(),
        cwd: "/work".to_string(),
        tty: false,
        exit_code: state.exit_code(),
        state,
        output: "ok\n".to_string(),
        output_start: 0,
        output_end: 3,
        dropped_bytes: 0,
        truncated: false,
        input: None,
        wrote_input: false,
        terminate: false,
        chunk_id: "0a1b2c".to_string(),
        wall_time_ms: 420,
    }))
}

fn usage() -> Usage {
    Usage {
        input: Some(1_980),
        output: Some(160),
        cache_read: Some(1_024),
        cache_write: Some(32),
    }
}

fn header<'a>(workdir: &'a std::path::Path) -> SessionHeader<'a> {
    SessionHeader {
        version: "0.1.0",
        workdir,
        model: "claude-sonnet-4",
        provider: "anthropic",
        session_id: "sess-1",
        prompt: "check the version",
    }
}

fn parse_events(bytes: &[u8]) -> Vec<ThreadEvent> {
    std::str::from_utf8(bytes)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("thread event"))
        .collect()
}

/// 每个测试独立的临时目录（仓库未引入 `tempfile` 依赖）。
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "lx-exec-processor-{}-{label}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ===== HumanOutputProcessor =====

/// 管道模式下 `stdout` 只能有最终回复，全部过程信息留在 `stderr`。
#[test]
fn human_piped_stdout_carries_only_the_final_message() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor.on_reasoning("checking the manifest");
        let call = read_call();
        processor.on_tool_started(&call);
        processor.on_tool_finished(&call, &ToolOutcome::Executed(read_result()), 12);
        processor.on_assistant_text("Final answer");
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    assert_eq!(stdout, b"Final answer\n");
    let stderr = String::from_utf8(stderr).expect("utf8");
    assert!(stderr.contains("lx-tui exec v0.1.0"));
    assert!(stderr.contains("workdir: /work"));
    assert!(stderr.contains("model: claude-sonnet-4"));
    assert!(stderr.contains("provider: anthropic"));
    assert!(stderr.contains("session id: sess-1"));
    assert!(stderr.contains("user\ncheck the version"));
    assert!(stderr.contains("● Thinking..."));
    assert!(stderr.contains("  checking the manifest"));
    assert!(stderr.contains("■ read Cargo.toml"));
    assert!(stderr.contains("  └─ Read 57 lines (12ms)"));
    assert!(stderr.contains("tokens used: 2,140 (in 1,980 · cached 1,024 · out 160)"));
}

/// 关闭颜色时不得出现任何 ANSI 转义序列（stdout 与 stderr 都不行）。
#[test]
fn human_without_color_emits_no_ansi_sequences() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, true, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor.on_reasoning("thinking");
        let call = read_call();
        processor.on_tool_started(&call);
        processor.on_tool_finished(&call, &ToolOutcome::Executed(read_result()), 3);
        processor.on_warning("auto compacted");
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    assert!(!stdout.windows(2).any(|window| window == b"\x1b["));
    assert!(!stderr.windows(2).any(|window| window == b"\x1b["));
    let stderr = String::from_utf8(stderr).expect("utf8");
    assert!(stderr.contains("warning: auto compacted"));
}

/// 开启颜色时进度行必须带 ANSI 样式（纯文本模式的反向保护）。
#[test]
fn human_with_color_emits_ansi_styles() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, true, false, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor.on_reasoning("thinking");
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    assert!(stderr.windows(2).any(|window| window == b"\x1b["));
}

/// 交互终端：最终回复补 `lx` 标题写在 stderr，stdout 保持干净。
#[test]
fn human_terminal_prints_the_final_message_to_stderr() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, true, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    assert!(stdout.is_empty());
    let stderr = String::from_utf8(stderr).expect("utf8");
    assert!(stderr.contains("lx\nFinal answer"));
}

/// 工具前的正文属于中间步骤：写 stderr，不能混进 stdout。
#[test]
fn human_intermediate_text_goes_to_stderr() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor.on_assistant_text("Let me check the manifest first.");
        let call = read_call();
        processor.on_tool_started(&call);
        processor.on_tool_finished(&call, &ToolOutcome::Executed(read_result()), 12);
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    assert_eq!(stdout, b"Final answer\n");
    let stderr = String::from_utf8(stderr).expect("utf8");
    assert!(stderr.contains("lx\nLet me check the manifest first."));
}

/// `-o` 落盘：自动创建父目录，写入纯文本（无额外换行）。
#[test]
fn human_writes_last_message_file_atomically() {
    let dir = TestDir::new("last-message");
    let path = dir.path.join("nested/deeper/final.txt");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor =
            HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, Some(path.clone()));
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "Final answer"
    );
}

/// 失败轮次：stderr 报稳定错误码，stdout 不写任何内容。
#[test]
fn human_failed_turn_reports_the_error_code() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor
            .on_turn_finished(
                &TurnSummary::failed("model_error", usage()),
                Some("partial"),
            )
            .expect("turn finished");
    }

    assert!(stdout.is_empty());
    let stderr = String::from_utf8(stderr).expect("utf8");
    assert!(stderr.contains("turn failed: model_error"));
}

#[test]
fn human_interrupted_turn_reports_interruption() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor
            .on_turn_finished(&TurnSummary::interrupted(usage()), None)
            .expect("turn finished");
    }

    assert!(stdout.is_empty());
    let stderr = String::from_utf8(stderr).expect("utf8");
    assert!(stderr.contains("turn interrupted"));
}

// ===== JsonlOutputProcessor =====

/// 完整事件序列、顺序与内容必须与文档一致。
#[test]
fn jsonl_emits_events_in_the_documented_order() {
    let mut stdout = Vec::new();
    {
        let mut processor = JsonlOutputProcessor::new(&mut stdout, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor.on_reasoning("make a plan");
        let call = read_call();
        processor.on_tool_started(&call);
        processor.on_tool_finished(&call, &ToolOutcome::Executed(read_result()), 12);
        processor.on_assistant_text("Final answer");
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    let events = parse_events(&stdout);
    assert_eq!(events.len(), 7, "events: {events:?}");
    assert!(matches!(
        &events[0],
        ThreadEvent::ThreadStarted { thread_id } if thread_id == "sess-1"
    ));
    assert!(matches!(events[1], ThreadEvent::TurnStarted));
    match &events[2] {
        ThreadEvent::ItemCompleted { item } => {
            assert_eq!(item.id, "item_0");
            assert!(matches!(
                &item.details,
                ThreadItemDetails::Reasoning { text } if text == "make a plan"
            ));
        }
        other => panic!("expected reasoning item, got {other:?}"),
    }
    match &events[3] {
        ThreadEvent::ItemStarted { item } => {
            assert_eq!(item.id, "item_1");
            assert!(matches!(
                &item.details,
                ThreadItemDetails::ToolCall {
                    name,
                    status: ItemStatus::InProgress,
                    ..
                } if name == "read"
            ));
        }
        other => panic!("expected started tool item, got {other:?}"),
    }
    match &events[4] {
        ThreadEvent::ItemCompleted { item } => {
            assert_eq!(item.id, "item_2");
            assert!(matches!(
                &item.details,
                ThreadItemDetails::ToolCall {
                    status: ItemStatus::Completed,
                    summary: Some(summary),
                    ..
                } if summary == "Read 57 lines"
            ));
        }
        other => panic!("expected completed tool item, got {other:?}"),
    }
    match &events[5] {
        ThreadEvent::ItemCompleted { item } => {
            assert!(matches!(
                &item.details,
                ThreadItemDetails::AgentMessage { text } if text == "Final answer"
            ));
        }
        other => panic!("expected agent message, got {other:?}"),
    }
    match &events[6] {
        ThreadEvent::TurnCompleted { usage } => assert_eq!(
            usage,
            &ExecUsage {
                input_tokens: 1_980,
                cached_input_tokens: 1_024,
                cache_write_input_tokens: 32,
                output_tokens: 160,
            }
        ),
        other => panic!("expected turn completion, got {other:?}"),
    }
}

#[test]
fn jsonl_failed_turn_emits_turn_failed() {
    let mut stdout = Vec::new();
    {
        let mut processor = JsonlOutputProcessor::new(&mut stdout, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor
            .on_turn_finished(&TurnSummary::failed("model_error", usage()), None)
            .expect("turn finished");
    }

    let events = parse_events(&stdout);
    assert!(matches!(
        events.last(),
        Some(ThreadEvent::TurnFailed { error }) if error.message == "turn failed: model_error"
    ));
}

#[test]
fn jsonl_interrupted_turn_emits_turn_failed() {
    let mut stdout = Vec::new();
    {
        let mut processor = JsonlOutputProcessor::new(&mut stdout, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor
            .on_turn_finished(&TurnSummary::interrupted(usage()), None)
            .expect("turn finished");
    }

    let events = parse_events(&stdout);
    assert!(matches!(
        events.last(),
        Some(ThreadEvent::TurnFailed { error }) if error.message == "turn interrupted"
    ));
}

#[test]
fn jsonl_writes_last_message_file() {
    let dir = TestDir::new("jsonl-last-message");
    let path = dir.path.join("nested/final.txt");
    let mut stdout = Vec::new();
    {
        let mut processor = JsonlOutputProcessor::new(&mut stdout, Some(path.clone()));
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor.on_assistant_text("Final answer");
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "Final answer"
    );
}

#[test]
fn jsonl_warning_emits_error_event() {
    let mut stdout = Vec::new();
    {
        let mut processor = JsonlOutputProcessor::new(&mut stdout, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor.on_warning("context compacted");
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), None)
            .expect("turn finished");
    }

    let events = parse_events(&stdout);
    assert!(matches!(
        &events[2],
        ThreadEvent::Error { message } if message == "context compacted"
    ));
}

/// 未通过 `on_assistant_text` 送达的最终回复也必须出现在 `turn.completed` 之前。
#[test]
fn jsonl_final_message_is_emitted_when_no_assistant_block_was_streamed() {
    let mut stdout = Vec::new();
    {
        let mut processor = JsonlOutputProcessor::new(&mut stdout, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("only at the end"))
            .expect("turn finished");
    }

    let events = parse_events(&stdout);
    assert_eq!(events.len(), 4);
    assert!(matches!(
        &events[2],
        ThreadEvent::ItemCompleted { item }
            if matches!(&item.details, ThreadItemDetails::AgentMessage { text } if text == "only at the end")
    ));
    assert!(matches!(events[3], ThreadEvent::TurnCompleted { .. }));
}

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
    let executed = ToolOutcome::Executed(ToolExecutionResult::WebSearch(Box::new(
        WebSearchToolResult {
            queries: vec!["ratatui".to_string(), "tokio".to_string()],
            num_results: 8,
            search_type: SearchDepth::Auto,
            provider: WebSearchProvider::Exa,
            outcomes: Vec::new(),
            text: String::new(),
            truncated: false,
            duration_ms: 840,
        },
    )));
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
            role: AgentRole::Explorer,
            title: "inspect cli".to_string(),
        },
    )));
    match item_details_for_finish(&call, &executed) {
        ThreadItemDetails::CollabToolCall {
            tool,
            agent_id,
            role,
            title,
            status,
        } => {
            assert_eq!(tool, "spawn_agent");
            assert_eq!(agent_id.as_deref(), Some("sub-1"));
            assert_eq!(role.as_deref(), Some("explorer"));
            assert_eq!(title.as_deref(), Some("inspect cli"));
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
