//! 进程 / 计划工具的端到端：真实 DB 握手 + 真实子进程 + 假 transport。
//!
//! 覆盖 design §1 的完成标准：
//!
//! 1. 模型能启动命令、拿到输出与退出码并继续推理；
//! 2. 未退出命令返回会话 ID，后续轮次用同一 ID 读输出（不重复启动）；
//! 3. 进程退出与工具调用完成是两套状态（工具已 completed，进程仍 running）；
//! 4. `update_plan` 结果落库后才更新当前计划；
//! 5. 非零退出被识别为失败结果（不能只显示 Completed）。

use super::*;
use crate::app::state::{ProcessStateView, ToolDetailView, ToolStatus};
use crate::chat::agent_loop::AgentTools;

/// 为一次驱动构造生产执行器（同一会话进程管理器）。
fn agent_tools(fixture: &Fixture, streams: &mut ChatStreams, key: &RequestKey) -> AgentTools {
    let runtime = streams.runtimes.get_or_create(
        fixture.workspace().to_path_buf(),
        &key.session,
        tokio::sync::mpsc::unbounded_channel().0,
    );
    let turn = runtime.turn(&key.generation.to_string());
    AgentTools::new(runtime.manager().clone(), turn)
}

/// 在同一窗格里开启下一轮（生产由「Enter + DB ack」完成；这里只补状态与 DB 行）。
///
/// 返回新的 request key（generation 前进、assistant 消息换新），进程管理器不变——
/// 这正是「跨轮复用同一会话实例」的前提。
fn next_turn(
    fixture: &Fixture,
    state: &mut AppState,
    previous: &RequestKey,
    persistence: &Persistence,
    draft: &str,
) -> RequestKey {
    // 上一轮收尾：返回 Idle 才会有下一轮。
    let _ = update::on_finish_ack(state, previous, true);
    {
        let target = state.pane_mut_anywhere(previous.pane).expect("pane");
        target.lx.input = draft.into();
    }
    assert!(update::try_send(state, previous.pane), "下一轮发送受理");
    let generation = match &state
        .pane_anywhere(previous.pane)
        .expect("pane")
        .lx
        .chat
        .request
    {
        RequestState::Committing { generation, .. } => *generation,
        other => panic!("expected Committing, got {other:?}"),
    };
    let committed = fixture
        .reopen()
        .begin_turn(
            fixture.workspace(),
            Some(&previous.session),
            draft,
            &crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("begin next turn");
    assert!(update::on_turn_committed(
        state,
        previous.pane,
        generation,
        &committed
    ));
    let _ = persistence;
    let key = RequestKey {
        pane: previous.pane,
        session: committed.session_id,
        generation,
    };
    state.effects.clear();
    // 事件层冻结工作区（生产由 start_stream 写入）。
    let _ = fixture;
    key
}

/// 驱动一轮并返回（driven, 执行器）以便测试后续清理进程。
async fn drive_processes(
    fixture: &Fixture,
    state: &mut AppState,
    key: &RequestKey,
    transport: &FakeTransport,
    persistence: &Persistence,
    streams: &mut ChatStreams,
    tools: AgentTools,
) -> Driven {
    drive_with_hook(
        fixture,
        state,
        key,
        transport,
        persistence,
        streams,
        no_cancel(),
        tools,
        Hooks::default(),
    )
    .await
}

#[tokio::test]
async fn exec_command_then_read_output_across_turns() {
    let fixture = Fixture::new("exec-cross-turn");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let tools = agent_tools(&fixture, &mut streams, &key);

    // 第一轮：启动一个输出两段、短暂停留的命令，工具在等待窗口内返回 running。
    let exec_args = serde_json::json!({
        "cmd": "echo first; sleep 1; echo second",
        "yield_time_ms": 200,
        "shell": "/bin/sh"
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_exec", "exec_command", &exec_args, 0)], None),
            final_response("started", None),
        ],
        StopReason::Complete,
    );
    let driven = drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools.clone(),
    )
    .await;
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    assert_eq!(
        driven.trace,
        vec![
            Trace::Begin("call_exec".into()),
            Trace::Finish("call_exec".into(), ToolTerminalStatus::Completed),
        ],
        "等待窗口到期时工具正常完成（不是失败）"
    );

    // 卡片：类型化命令视图已写入，进程仍 running（工具完成 ≠ 进程退出）。
    let chat = &state.pane_anywhere(key.pane).expect("pane").lx.chat;
    let call = chat
        .messages
        .last()
        .expect("msg")
        .tool("call_exec")
        .expect("tool");
    assert_eq!(call.status, ToolStatus::Completed);
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert_eq!(view.state, ProcessStateView::Running);
    let session_id = view.process_session_id.expect("session id");
    let recorded = persistence
        .load_processes_for_session(fixture.workspace(), &key.session)
        .expect("load processes");
    assert_eq!(recorded.len(), 1, "进程元数据已登记");
    assert_eq!(recorded[0].process_session_id as u64, session_id);

    // 第二轮：用同一 ID 读取后续输出（不重复启动）。
    let key = next_turn(&fixture, &mut state, &key, &persistence, "read more");
    let _ = &key;
    let stdin_args = serde_json::json!({
        "session_id": session_id,
        "yield_time_ms": 3000
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_stdin", "write_stdin", &stdin_args, 0)], None),
            final_response("read later output", None),
        ],
        StopReason::Complete,
    );
    let driven = drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools.clone(),
    )
    .await;
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    assert_eq!(
        driven.executed,
        vec!["call_stdin".to_string()],
        "第二轮只执行交互，没有再次启动命令"
    );
    let recorded = persistence
        .load_processes_for_session(fixture.workspace(), &key.session)
        .expect("load processes");
    assert_eq!(recorded.len(), 1, "没有重复登记进程");

    tools.processes().shutdown().await;
}

#[tokio::test]
async fn non_zero_exit_marks_the_tool_result_as_failed() {
    let fixture = Fixture::new("exec-failure");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let tools = agent_tools(&fixture, &mut streams, &key);
    let exec_args = serde_json::json!({
        "cmd": "echo boom; exit 4",
        "yield_time_ms": 5000,
        "shell": "/bin/sh"
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_fail", "exec_command", &exec_args, 0)], None),
            final_response("failed", None),
        ],
        StopReason::Complete,
    );
    let driven = drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools.clone(),
    )
    .await;
    assert_eq!(
        driven.trace,
        vec![
            Trace::Begin("call_fail".into()),
            Trace::Finish("call_fail".into(), ToolTerminalStatus::Failed),
        ],
        "非零退出落库为 failed，不是 completed"
    );
    let chat = &state.pane_anywhere(key.pane).expect("pane").lx.chat;
    let call = chat
        .messages
        .last()
        .expect("msg")
        .tool("call_fail")
        .expect("tool");
    assert_eq!(call.status, ToolStatus::Failed);
    assert!(
        call.error
            .as_deref()
            .is_some_and(|error| error.contains('4')),
        "失败原因带真实退出码：{:?}",
        call.error
    );
    tools.processes().shutdown().await;
}

#[tokio::test]
async fn update_plan_commits_only_after_persistence() {
    let fixture = Fixture::new("plan-commit");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let tools = agent_tools(&fixture, &mut streams, &key);
    let plan_args = serde_json::json!({
        "explanation": "checking",
        "plan": [
            { "step": "implement", "status": "completed" },
            { "step": "verify", "status": "in_progress" }
        ]
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_plan", "update_plan", &plan_args, 0)], None),
            final_response("plan set", None),
        ],
        StopReason::Complete,
    );
    let driven = drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools,
    )
    .await;
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    // 当前计划 = 该次提交的快照。
    let plan = state
        .pane_anywhere(key.pane)
        .and_then(|pane| pane.lx.chat.plan.clone())
        .expect("plan committed");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.completed(), 1);
    // 历史卡片保留该次快照（不放最新计划）。
    let chat = &state.pane_anywhere(key.pane).expect("pane").lx.chat;
    let call = chat
        .messages
        .last()
        .expect("msg")
        .tool("call_plan")
        .expect("tool");
    let Some(ToolDetailView::Plan(view)) = call.detail.as_deref() else {
        panic!("expected plan view");
    };
    assert_eq!(view.steps.len(), 2);
}

#[tokio::test]
async fn invalid_plan_arguments_fail_without_touching_the_current_plan() {
    let fixture = Fixture::new("plan-invalid");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let tools = agent_tools(&fixture, &mut streams, &key);
    let transport = FakeTransport::new(
        vec![
            tool_calls(
                &[("call_bad", "update_plan", r#"{"plan":"nope"}"#, 0)],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let driven = drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools,
    )
    .await;
    assert_eq!(
        driven.trace,
        vec![
            Trace::Begin("call_bad".into()),
            Trace::Finish("call_bad".into(), ToolTerminalStatus::Failed),
        ]
    );
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .chat
            .plan
            .is_none(),
        "校验失败不提交计划"
    );
}

#[tokio::test]
async fn terminated_process_reports_stopped_and_keeps_output() {
    let fixture = Fixture::new("exec-terminate");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let tools = agent_tools(&fixture, &mut streams, &key);
    let exec_args = serde_json::json!({
        "cmd": "echo up; sleep 30",
        "yield_time_ms": 200,
        "shell": "/bin/sh"
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_run", "exec_command", &exec_args, 0)], None),
            final_response("started", None),
        ],
        StopReason::Complete,
    );
    drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools.clone(),
    )
    .await;
    let session_id = {
        let chat = &state.pane_anywhere(key.pane).expect("pane").lx.chat;
        let call = chat
            .messages
            .last()
            .expect("msg")
            .tool("call_run")
            .expect("tool");
        let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
            panic!("expected command view");
        };
        view.process_session_id.expect("session id")
    };
    // 第二轮（同一会话、新 generation）才能再发工具调用。
    let key = next_turn(&fixture, &mut state, &key, &persistence, "stop it");
    let _ = &key;
    let stop_args = serde_json::json!({
        "session_id": session_id,
        "terminate": true
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_stop", "write_stdin", &stop_args, 0)], None),
            final_response("stopped", None),
        ],
        StopReason::Complete,
    );
    let driven = drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools.clone(),
    )
    .await;
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    let chat = &state.pane_anywhere(key.pane).expect("pane").lx.chat;
    let call = chat
        .messages
        .last()
        .expect("msg")
        .tool("call_stop")
        .expect("tool");
    let Some(ToolDetailView::Interaction(view)) = call.detail.as_deref() else {
        panic!("expected interaction view");
    };
    assert_eq!(
        view.action,
        crate::app::state::InteractionAction::StopCommand
    );
    assert_eq!(view.state, ProcessStateView::Stopped);
    tools.processes().shutdown().await;
}

/// 会话进程管理器在轮次结束后仍保留（正常完成一轮不清理服务）。
#[tokio::test]
async fn runtimes_survive_a_completed_turn() {
    let fixture = Fixture::new("runtime-survive");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let tools = agent_tools(&fixture, &mut streams, &key);
    let exec_args = serde_json::json!({
        "cmd": "sleep 30",
        "yield_time_ms": 100,
        "shell": "/bin/sh"
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_bg", "exec_command", &exec_args, 0)], None),
            final_response("started", None),
        ],
        StopReason::Complete,
    );
    drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools.clone(),
    )
    .await;
    // 整轮已结束：运行实例仍在，进程仍 running（下一轮可复用）。
    let runtime = streams.runtimes.get(&key.session).expect("runtime kept");
    assert_eq!(
        runtime.manager().running_count(),
        1,
        "正常结束一轮不清理服务"
    );
    tools.processes().shutdown().await;
}

/// `/stop` 端到端：列出真实后台进程，再按会话 ID 停掉它。
///
/// 这是「缺失的主动清理入口」的核心验证：此前 lx 只有在切会话/退出时才会清理，
/// 用户留在会话里时没有任何办法停掉自己起的服务。
#[tokio::test]
async fn stop_panel_lists_and_stops_a_real_background_process() {
    let fixture = Fixture::new("stop-panel");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let tools = agent_tools(&fixture, &mut streams, &key);
    let exec_args = serde_json::json!({
        "cmd": "sleep 30",
        "yield_time_ms": 100,
        "shell": "/bin/sh"
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_bg", "exec_command", &exec_args, 0)], None),
            final_response("started", None),
        ],
        StopReason::Complete,
    );
    drive_processes(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        tools.clone(),
    )
    .await;
    assert_eq!(tools.processes().running_count(), 1, "进程在跑");

    // 打开 `/stop`：面板骨架 + 列表效应。
    assert!(crate::app::update::open_stop_picker(&mut state, key.pane));
    assert!(crate::event::chat::apply_process_effects(
        &mut state,
        &mut streams
    ));
    let panel = match state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.slash.clone())
    {
        Some(crate::app::state::SlashPanel::Stop(picker)) => picker,
        other => panic!("期望 /stop 面板，实际 {other:?}"),
    };
    assert_eq!(panel.processes.len(), 1, "列出真实后台进程");
    assert_eq!(panel.processes[0].command, "sleep 30");
    let process_session_id = panel.processes[0].session_id;

    // 一级 → 二级 → 采纳：停止该进程。
    assert!(crate::app::update::apply_slash_enter(&mut state, key.pane));
    assert!(crate::app::update::apply_slash_enter(&mut state, key.pane));
    assert!(crate::event::chat::apply_process_effects(
        &mut state,
        &mut streams
    ));
    // 停止是异步的：等到进程真的不在运行。
    for _ in 0..100 {
        if tools.processes().running_count() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        tools.processes().running_count(),
        0,
        "进程 {process_session_id} 已被 /stop 停掉"
    );
    // 进程终态事件驱动面板刷新（実生产路径：运行实例在进程退出时发
    // `ProcessUiEvent::Exited`）。这里直接投递同一个事件验证面板响应。
    let snapshot = tools
        .processes()
        .snapshot(process_session_id)
        .expect("snapshot");
    // 面板列表被移除一项 → 视为一次变化（事件层据此置脏重绘）。
    assert!(
        crate::event::chat::handle_process_event(
            &mut state,
            &mut streams,
            &persistence,
            crate::chat::process::ProcessUiEvent::Exited {
                session_id: process_session_id,
                origin_call_id: snapshot.origin_call_id.clone(),
                state: snapshot.state.clone(),
                exit_code: snapshot.exit_code,
            },
        ),
        "终态事件应刷新打开着的 /stop 面板"
    );
    // 面板仍在（便于连停多个），并已刷新为空列表。
    let panel = match state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.slash.clone())
    {
        Some(crate::app::state::SlashPanel::Stop(picker)) => picker,
        other => panic!("停止后仍应留在面板里，实际 {other:?}"),
    };
    assert!(
        panel.processes.is_empty(),
        "刷新后不再列出已停止的进程：{:?}",
        panel.processes
    );
    tools.processes().shutdown().await;
}
