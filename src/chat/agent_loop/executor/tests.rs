//! 生产执行器测试：三类工具分派、跨轮复用、本轮停止集合与计划修订号。

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use super::*;
use crate::chat::process::{ProcessManager, ProcessOwner};
use crate::chat::tools::{TOOL_NAMES, ToolErrorCode};

/// 独立临时工作目录。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "lx-executor-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 构造执行器（共享同一进程管理器 + 本轮上下文）。
fn executor(dir: &TempDir) -> AgentTools {
    let manager = Arc::new(ProcessManager::new(
        ProcessOwner {
            workspace: dir.path.clone(),
            chat_session_id: "chat-1".into(),
            runtime_epoch: "epoch-1".into(),
        },
        None,
    ));
    AgentTools::new(manager, Arc::new(Mutex::new(TurnExecution::default())))
}

/// 构造一次调用。
fn call(name: &str, arguments: &str) -> ReadyToolCall {
    ReadyToolCall {
        call_id: format!("call-{name}"),
        name: name.to_string(),
        arguments_json: arguments.to_string(),
        ordinal: 0,
    }
}

#[tokio::test]
async fn file_tools_still_route_through_the_file_executor() {
    let dir = TempDir::new("file");
    std::fs::write(dir.path.join("a.txt"), "hello\n").expect("write");
    let tools = executor(&dir);
    let result = tools
        .execute(&dir.path, &call("read", r#"{"path":"a.txt"}"#))
        .await;
    let ToolExecutionResult::Read { content, .. } = result else {
        panic!("expected read result");
    };
    assert!(content.contains("hello"));
}

#[tokio::test]
async fn exec_command_produces_a_process_result_and_registers_the_turn() {
    let dir = TempDir::new("exec");
    let tools = executor(&dir);
    let result = tools
        .execute(
            &dir.path,
            &call("exec_command", r#"{"cmd":"echo hi","yield_time_ms":5000}"#),
        )
        .await;
    let ToolExecutionResult::Process(process) = &result else {
        panic!("expected process result, got {result:?}");
    };
    assert!(process.output.contains("hi"));
    let session_id = process.session_id;
    // 本轮新建集合记录了这个进程（Esc 停止按它取目标）。
    let turn = tools.turn().lock().await;
    assert!(turn.created.contains(&session_id));
    drop(turn);
    tools.processes().shutdown().await;
}

#[tokio::test]
async fn same_executor_keeps_the_session_id_for_later_write_stdin() {
    let dir = TempDir::new("cross");
    let tools = executor(&dir);
    let first = tools
        .execute(
            &dir.path,
            &call(
                "exec_command",
                r#"{"cmd":"echo one; sleep 1; echo two","yield_time_ms":200}"#,
            ),
        )
        .await;
    let ToolExecutionResult::Process(started) = &first else {
        panic!("expected running process");
    };
    let session_id = started.session_id;
    let arguments = format!(r#"{{"session_id":{session_id},"yield_time_ms":3000}}"#);
    let second = tools
        .execute(&dir.path, &call("write_stdin", &arguments))
        .await;
    let ToolExecutionResult::Process(followed) = &second else {
        panic!("expected write_stdin result");
    };
    assert!(
        followed.output.contains("two"),
        "跨轮读取到后续输出：{:?}",
        followed.output
    );
    tools.processes().shutdown().await;
}

#[tokio::test]
async fn update_plan_assigns_monotonic_revisions() {
    let dir = TempDir::new("plan");
    let tools = executor(&dir);
    let arguments = r#"{"plan":[{"step":"a","status":"in_progress"}]}"#;
    let first = tools
        .execute(&dir.path, &call("update_plan", arguments))
        .await;
    let second = tools
        .execute(&dir.path, &call("update_plan", arguments))
        .await;
    let revision = |result: &ToolExecutionResult| match result {
        ToolExecutionResult::Plan(snapshot) => snapshot.revision,
        other => panic!("expected plan, got {other:?}"),
    };
    assert_eq!(revision(&first), 1);
    assert_eq!(revision(&second), 2, "修订号单调递增");
}

#[tokio::test]
async fn invalid_plan_arguments_are_a_validation_failure() {
    let dir = TempDir::new("badplan");
    let tools = executor(&dir);
    let result = tools
        .execute(&dir.path, &call("update_plan", r#"{"plan":"nope"}"#))
        .await;
    assert!(result.is_failure());
    assert_eq!(result.error_code(), Some(ToolErrorCode::PlanInvalid));
}

#[tokio::test]
async fn exit_code_failures_are_reported_as_failure_results() {
    let dir = TempDir::new("nonzero");
    let tools = executor(&dir);
    let result = tools
        .execute(
            &dir.path,
            &call("exec_command", r#"{"cmd":"exit 7","yield_time_ms":5000}"#),
        )
        .await;
    assert!(result.is_failure(), "非零退出是命令失败");
    let ToolExecutionResult::Process(process) = &result else {
        panic!("expected process");
    };
    assert_eq!(process.exit_code, Some(7));
    tools.processes().shutdown().await;
}

#[tokio::test]
async fn turn_stop_targets_union_created_and_active() {
    let dir = TempDir::new("targets");
    let tools = executor(&dir);
    let first = tools
        .execute(
            &dir.path,
            &call("exec_command", r#"{"cmd":"sleep 30","yield_time_ms":100}"#),
        )
        .await;
    let ToolExecutionResult::Process(process) = &first else {
        panic!("expected process");
    };
    let session_id = process.session_id;
    let turn = tools.turn().lock().await;
    let targets = turn.stop_targets();
    assert!(targets.contains(&session_id), "本轮新建进程在停止集合内");
    drop(turn);
    tools.processes().shutdown().await;
}

/// D9：本轮停止目标 = 本轮新建 ∪ 当前活动交互；**上一轮**启动、本轮未操作的服务
/// 不在停止集合里（不会被 Esc 误杀）。
#[tokio::test]
async fn older_services_are_not_in_the_stop_targets() {
    let dir = TempDir::new("d9");
    let manager = Arc::new(ProcessManager::new(
        ProcessOwner {
            workspace: dir.path.clone(),
            chat_session_id: "chat-1".into(),
            runtime_epoch: "epoch-1".into(),
        },
        None,
    ));
    // 上一轮：启动一个长期服务（新建集合记在上一轮的上下文里）。
    let old_turn = Arc::new(Mutex::new(TurnExecution::default()));
    let old_tools = AgentTools::new(manager.clone(), old_turn);
    let old = old_tools
        .execute(
            &dir.path,
            &call("exec_command", r#"{"cmd":"sleep 30","yield_time_ms":100}"#),
        )
        .await;
    let ToolExecutionResult::Process(old_process) = &old else {
        panic!("expected process");
    };
    let old_id = old_process.session_id;

    // 本轮：全新的执行上下文，只启动另一个进程。
    let turn = Arc::new(Mutex::new(TurnExecution::default()));
    let tools = AgentTools::new(manager.clone(), turn);
    let current = tools
        .execute(
            &dir.path,
            &call("exec_command", r#"{"cmd":"sleep 30","yield_time_ms":100}"#),
        )
        .await;
    let ToolExecutionResult::Process(current_process) = &current else {
        panic!("expected process");
    };
    let current_id = current_process.session_id;

    // 本轮停止集合只含本轮进程；上一轮的服务保持运行。
    let targets = tools.turn().lock().await.stop_targets();
    assert!(targets.contains(&current_id));
    assert!(
        !targets.contains(&old_id),
        "下一轮不会因为 Esc 停止而误杀上一轮启动的服务"
    );
    manager
        .stop_many(targets.into_iter().collect::<Vec<_>>())
        .await;
    let old_snapshot = manager.snapshot(old_id).expect("still tracked");
    assert!(!old_snapshot.state.is_terminal(), "旧服务仍在运行");
    manager.shutdown().await;
}

#[tokio::test]
async fn write_stdin_marks_the_interacted_process_as_active() {
    let dir = TempDir::new("active");
    let tools = executor(&dir);
    let started = tools
        .execute(
            &dir.path,
            &call("exec_command", r#"{"cmd":"sleep 30","yield_time_ms":100}"#),
        )
        .await;
    let ToolExecutionResult::Process(process) = &started else {
        panic!("expected process");
    };
    let session_id = process.session_id;
    let arguments = format!(r#"{{"session_id":{session_id},"yield_time_ms":50}}"#);
    tools
        .execute(&dir.path, &call("write_stdin", &arguments))
        .await;
    let turn = tools.turn().lock().await;
    assert_eq!(
        turn.active_interaction,
        Some(session_id),
        "正在等待/交互的进程进入本轮停止集合"
    );
    drop(turn);
    tools.processes().shutdown().await;
}

#[tokio::test]
async fn finished_process_leaves_the_active_interaction() {
    let dir = TempDir::new("finished");
    let tools = executor(&dir);
    let result = tools
        .execute(
            &dir.path,
            &call(
                "exec_command",
                r#"{"cmd":"echo done","yield_time_ms":5000}"#,
            ),
        )
        .await;
    let ToolExecutionResult::Process(process) = &result else {
        panic!("expected process");
    };
    assert!(process.state.is_terminal());
    let turn = tools.turn().lock().await;
    assert_eq!(
        turn.active_interaction, None,
        "已退出进程不再算“正在等待”，避免后续误杀"
    );
}

#[tokio::test]
async fn unknown_tool_still_reports_the_full_tool_list() {
    let dir = TempDir::new("unknown");
    let tools = executor(&dir);
    let result = tools.execute(&dir.path, &call("nonexistent", "{}")).await;
    let ToolExecutionResult::Failed { message, .. } = result else {
        panic!("expected failure");
    };
    for name in TOOL_NAMES {
        assert!(message.contains(name), "错误消息应列出 {name}：{message}");
    }
}

#[tokio::test]
async fn invalid_process_arguments_never_spawn_a_process() {
    let dir = TempDir::new("no_spawn");
    let tools = executor(&dir);
    let result = tools
        .execute(&dir.path, &call("exec_command", r#"{"cmd":"  "}"#))
        .await;
    assert!(result.is_failure());
    assert_eq!(tools.processes().running_count(), 0, "校验失败不产生进程");
}
