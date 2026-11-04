//! `exec` 输出层测试：人类模式 stdout/stderr 分流与 JSONL 事件序列。
//!
//! 工具结果夹具与 `events_tests` 共用（后者复用本文件的构造器）。

use std::path::PathBuf;

use super::super::events::{ExecUsage, ItemStatus, ThreadEvent, ThreadItemDetails};
use super::super::processor::{
    ExecEventProcessor, HumanOutputProcessor, JsonlOutputProcessor, SessionHeader,
};
use crate::chat::agent::{AgentListItemView, AgentRole, AgentStatusView, SubagentOutcome};
use crate::chat::agent_loop::{ReadyToolCall, ToolOutcome, TurnSummary};
use crate::chat::process::{ProcessResult, ProcessResultKind, ProcessState};
use crate::chat::tools::{
    ReadKind, SearchDepth, ToolExecutionResult, WebSearchProvider, WebSearchToolResult,
};
use crate::db::chat::Usage;

// ===== 夹具 =====

pub(super) fn call(name: &str, arguments: &str) -> ReadyToolCall {
    ReadyToolCall {
        call_id: format!("call-{name}"),
        name: name.to_string(),
        arguments_json: arguments.to_string(),
        ordinal: 0,
    }
}

pub(super) fn read_call() -> ReadyToolCall {
    call("read", r#"{"path":"Cargo.toml"}"#)
}

pub(super) fn read_result() -> ToolExecutionResult {
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

pub(super) fn process_result(state: ProcessState) -> ToolExecutionResult {
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

pub(super) fn web_search_result(num_results: usize) -> ToolExecutionResult {
    ToolExecutionResult::WebSearch(Box::new(WebSearchToolResult {
        queries: vec!["ratatui".to_string(), "tokio".to_string()],
        num_results,
        search_type: SearchDepth::Auto,
        provider: WebSearchProvider::Exa,
        outcomes: Vec::new(),
        text: String::new(),
        truncated: false,
        duration_ms: 840,
    }))
}

pub(super) fn agent_status(
    agent_id: &str,
    role: AgentRole,
    title: &str,
    status: &str,
) -> AgentStatusView {
    AgentStatusView {
        agent_id: agent_id.to_string(),
        nickname: None,
        title: title.to_string(),
        role,
        status: status.to_string(),
        report: None,
    }
}

pub(super) fn agent_list_item(agent_id: &str, role: AgentRole, title: &str) -> AgentListItemView {
    AgentListItemView {
        agent_id: agent_id.to_string(),
        nickname: None,
        role,
        title: title.to_string(),
        status: "completed".to_string(),
        queued_count: 0,
    }
}

pub(super) fn subagent_outcome(outcome: SubagentOutcome) -> ToolOutcome {
    ToolOutcome::Executed(ToolExecutionResult::Subagent(Box::new(outcome)))
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

/// 事件项的工具身份标签（起始与结算用同一份口径比较）。
fn tool_label(details: &ThreadItemDetails) -> String {
    match details {
        ThreadItemDetails::ToolCall { name, .. } => name.clone(),
        ThreadItemDetails::CommandExecution { command, .. } => command.clone(),
        other => format!("{other:?}"),
    }
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

/// 连续两段正文：前一段已闭合、只可能是中间说明，先落 stderr；最终回复不重复。
#[test]
fn human_consecutive_text_parts_flush_the_earlier_one() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        processor.on_assistant_text("Step 1 note");
        processor.on_assistant_text("Final answer");
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), Some("Final answer"))
            .expect("turn finished");
    }

    assert_eq!(stdout, b"Final answer\n");
    let stderr = String::from_utf8(stderr).expect("utf8");
    assert_eq!(stderr.matches("Step 1 note").count(), 1, "{stderr}");
    assert!(!stderr.contains("Final answer"), "{stderr}");
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
            // 同一工具调用的 started/completed 必须共用同一个 item.id。
            assert_eq!(item.id, "item_1");
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

/// 并发工具调用：即使完成顺序与启动顺序相反，各调用仍保持自己的 `item.id`。
#[test]
fn jsonl_concurrent_tool_calls_keep_their_item_ids() {
    let mut stdout = Vec::new();
    {
        let mut processor = JsonlOutputProcessor::new(&mut stdout, None);
        let workdir = PathBuf::from("/work");
        processor.on_session_started(&header(&workdir));
        let mut read = read_call();
        read.call_id = "call-a".to_string();
        let mut command = call("exec_command", r#"{"cmd":"cargo check"}"#);
        command.call_id = "call-b".to_string();
        processor.on_tool_started(&read);
        processor.on_tool_started(&command);
        processor.on_tool_finished(
            &command,
            &ToolOutcome::Executed(process_result(ProcessState::Exited { code: Some(0) })),
            30,
        );
        processor.on_tool_finished(&read, &ToolOutcome::Executed(read_result()), 12);
        processor
            .on_turn_finished(&TurnSummary::complete(usage()), None)
            .expect("turn finished");
    }

    let events = parse_events(&stdout);
    let mut started = Vec::new();
    let mut completed = Vec::new();
    for event in &events {
        match event {
            ThreadEvent::ItemStarted { item } => {
                started.push((item.id.clone(), tool_label(&item.details)))
            }
            ThreadEvent::ItemCompleted { item } => {
                completed.push((item.id.clone(), tool_label(&item.details)))
            }
            _ => {}
        }
    }
    assert_eq!(
        started,
        vec![
            ("item_0".to_string(), "read".to_string()),
            ("item_1".to_string(), "cargo check".to_string())
        ]
    );
    assert_eq!(
        completed,
        vec![
            ("item_1".to_string(), "cargo check".to_string()),
            ("item_0".to_string(), "read".to_string())
        ]
    );
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
