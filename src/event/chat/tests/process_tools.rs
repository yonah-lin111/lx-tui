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

/// 非零退出落库为 **completed**，且不留失败原因。
///
/// 这是 `grep -c` 无匹配返回 1 那次误判的回归测试：工具成功跑完了，
/// 退出码 4 是它的结果，不是它坏了。
#[tokio::test]
async fn non_zero_exit_settles_completed_without_a_failure_reason() {
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
            Trace::Finish("call_fail".into(), ToolTerminalStatus::Completed),
        ],
        "非零退出落库为 completed：工具成功跑完了"
    );
    let chat = &state.pane_anywhere(key.pane).expect("pane").lx.chat;
    let call = chat
        .messages
        .last()
        .expect("msg")
        .tool("call_fail")
        .expect("tool");
    assert_eq!(call.status, ToolStatus::Completed, "不是 failed");
    assert_eq!(call.error, None, "不留失败原因：{:?}", call.error);
    // 退出码仍在结果体里，界面据此显示 `exit 4`。
    let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert_eq!(
        view.state,
        crate::app::state::ProcessStateView::Exited { code: Some(4) }
    );
    assert!(view.state.is_nonzero_exit(), "退出码信息保留");
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

/// 后台任务页端到端：列出真实后台进程，再按会话 ID 停掉它。
///
/// 这是「缺失的主动清理入口」的核心验证：此前 lx 只有在切会话/退出时才会清理，
/// 用户留在会话里时没有任何办法停掉自己起的服务。
///
/// 注意刷新时机：`^d` 只发停止请求（异步），列表由**进程终态事件**驱动移除——
/// 同步重读会读到停止前的快照，表现为「停不掉」。这里按生产路径投递终态事件。
#[tokio::test]
async fn task_page_lists_and_stops_a_real_background_process() {
    let fixture = Fixture::new("task-page");
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

    // 打开后台任务页：页面骨架 + 列表效应。
    assert!(crate::app::update::open_task_page(&mut state, key.pane));
    assert!(crate::event::chat::apply_process_effects(
        &mut state,
        &mut streams
    ));
    // 底部计数指标同步到页面外的状态行。
    assert!(crate::event::chat::sync_background_task_counts(
        &mut state, &streams
    ));
    let page = state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.task.clone())
        .expect("后台任务页已打开");
    assert_eq!(page.processes.len(), 1, "列出真实后台进程");
    assert_eq!(page.processes[0].command, "sleep 30");
    assert_eq!(
        state.pane_anywhere(key.pane).unwrap().lx.background_tasks,
        1,
        "输入框底部指标显示 1 个后台任务"
    );
    let process_session_id = page.processes[0].session_id;

    // `^d` 停止高亮进程。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::StopProcess);
    assert!(crate::event::chat::apply_process_effects(
        &mut state,
        &mut streams
    ));
    for _ in 0..100 {
        if tools.processes().running_count() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        tools.processes().running_count(),
        0,
        "进程 {process_session_id} 已被后台任务页停掉"
    );

    // 进程终态事件驱动列表移除（生产路径：运行实例在进程退出时发 Exited）。
    let snapshot = tools
        .processes()
        .snapshot(process_session_id)
        .expect("snapshot");
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
        "终态事件应刷新打开着的后台任务页"
    );
    let page = state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.task.clone())
        .expect("停完仍留在页面上，便于连停多个");
    assert!(
        page.processes.is_empty(),
        "刷新后不再列出已停止的进程：{:?}",
        page.processes
    );
    tools.processes().shutdown().await;
}

// ===== 停止反馈走 toast，不用输入框上方的 hint =====
// 面板打开时输入框已被控制面板替换，hint 的锚点是输入框，会飘在错误的位置。

#[tokio::test]
async fn stopping_a_missing_runtime_reports_via_toast() {
    // 运行实例已不在（刚切会话/刚退出）：页面还挂着这一行，必须给出反馈。
    // 反馈走 toast —— 不是输入框上方的 hint。
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut streams = ChatStreams::default();
    state.toast = None;
    state
        .effects
        .push(crate::app::effects::Effect::StopSessionProcess {
            pane,
            process_session_id: 999,
        });
    assert!(crate::event::chat::apply_process_effects(
        &mut state,
        &mut streams
    ));
    let toast = state.toast.as_ref().expect("停止失败应弹出 toast");
    assert!(
        toast.message.contains("Failed to stop"),
        "toast 文案：{:?}",
        toast.message
    );
    assert_eq!(toast.anchor, Some(pane), "toast 锚定在该窗格");
}

// ===== 后台任务面板的运行时长必须定时刷新 =====

#[test]
fn task_page_clock_advances_even_when_the_list_is_unchanged() {
    // `now_ms` 原来只在列表回投时更新：进程不变就冻住，时长永远停在第一次的值。
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-1".into());
        target.lx.task = Some(crate::app::state::TaskPage::default());
    }
    // 把 now_ms 拨回过去，模拟"已经过了一秒以上"。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task.as_mut().expect("page").now_ms = 0;
    }
    assert!(
        crate::event::chat::tick_task_pages(&mut state),
        "经过一秒后应当推进 now_ms 并要求重绘"
    );
    let now = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .task
        .as_ref()
        .unwrap()
        .now_ms;
    assert!(now > 0, "now_ms 已被推进");
    // 同一秒内不重复重绘。
    assert!(
        !crate::event::chat::tick_task_pages(&mut state),
        "同一秒内不重复置脏"
    );
}

#[test]
fn task_page_clock_is_ignored_without_a_page() {
    let mut state = AppState::demo();
    assert!(
        !crate::event::chat::tick_task_pages(&mut state),
        "没有面板时不动作"
    );
}

// ===== 面板打开后才启动的进程也必须出现 =====

#[tokio::test]
async fn a_process_started_after_opening_the_panel_shows_up() {
    let fixture = Fixture::new("panel-late-process");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let tools = agent_tools(&fixture, &mut streams, &key);

    // 先把面板打开（此时还没有任何进程）。
    let pane = key.pane;
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(key.session.clone());
        target.lx.task = Some(crate::app::state::TaskPage::default());
    }
    // 手动压入一次列表请求（等价于每轮刷新做的那一步）。
    state
        .effects
        .push(crate::app::effects::Effect::ListSessionProcesses { pane });
    crate::event::chat::apply_process_effects(&mut state, &mut streams);
    let listed = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .task
        .clone()
        .expect("page");
    assert!(listed.processes.is_empty(), "此刻确实还没有进程");

    // 现在启动一个长驻进程。
    let exec_args = serde_json::json!({
        "cmd": "sleep 30",
        "yield_time_ms": 100,
        "shell": "/bin/sh"
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call_late", "exec_command", &exec_args, 0)], None),
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

    // 关键：不需要重开面板，下一次**每轮刷新**就要把它拉进列表。
    assert!(
        crate::event::chat::refresh_open_task_pages(&mut state),
        "面板开着时应每轮请求刷新"
    );
    assert!(crate::event::chat::apply_process_effects(
        &mut state,
        &mut streams
    ));
    let page = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .task
        .clone()
        .expect("page");
    assert!(
        !page.processes.is_empty(),
        "面板打开之后启动的进程必须出现在列表里：{:?}",
        page.processes
    );
    assert!(
        page.processes
            .iter()
            .any(|p| p.command.contains("sleep 30")),
        "列表里应是刚启动的那个进程：{:?}",
        page.processes
    );
    tools.processes().shutdown().await;
}

#[test]
fn refresh_is_a_noop_without_an_open_panel() {
    let mut state = AppState::demo();
    assert!(
        !crate::event::chat::refresh_open_task_pages(&mut state),
        "没有打开的面板时不发请求"
    );
}

// ===== 重开会话：卡片与后台面板不许给出相反答案 =====

/// 登记首帧不该给仍在运行的进程盖 `ended_at_ms`。
///
/// 回归用户报的现象：重开 session 后消息列表显示 Running，而后台任务面板是空的。
/// 根因之一：`register_process` 用 `terminal = true` 写首帧，`finish_process`
/// 无条件写 `ended_at_ms`，库里出现 "state=running 但已结束" 的矛盾行。
#[tokio::test]
async fn registering_a_running_process_does_not_stamp_its_end_time() {
    let fixture = Fixture::new("register-running");
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
            tool_calls(&[("call_alive", "exec_command", &exec_args, 0)], None),
            final_response("running", None),
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

    let recorded = persistence
        .load_processes_for_session(fixture.workspace(), &key.session)
        .expect("load processes");
    assert_eq!(recorded.len(), 1, "进程元数据已登记");
    assert_eq!(recorded[0].state, "running", "进程仍在跑");
    assert_eq!(
        recorded[0].ended_at_ms, None,
        "还在跑的进程不该有结束时刻（否则「运行中但已结束」自相矛盾）"
    );
    tools.processes().shutdown().await;
}

/// 关闭会话（运行实例被回收）后必须把终态写回库。
///
/// 这是用户报的那条完整链路：关掉 session → 重新打开 → 卡片 Running，
/// 但后台任务面板空着。面板读内存运行实例（已随会话回收而空），
/// 卡片读 DB 记录——若回收时只杀进程不落终态，两处就永久不一致。
#[tokio::test]
async fn reaping_the_runtime_persists_the_terminal_state() {
    let fixture = Fixture::new("reap-terminal");
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
            tool_calls(&[("call_reap", "exec_command", &exec_args, 0)], None),
            final_response("running", None),
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

    let before = persistence
        .load_processes_for_session(fixture.workspace(), &key.session)
        .expect("load");
    assert_eq!(before[0].state, "running", "回收前仍在跑");

    // 模拟关闭拥有该运行实例的窗格：解绑会话 → 下一轮事件循环回收运行实例。
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.chat.session_id = None;
    }
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    assert!(
        crate::event::chat::pump_chat_streams(&mut state, &mut streams, &sender, &persistence),
        "解绑后回收运行实例"
    );
    // 回收是后台任务：等它停止完成并回投终态快照。
    let (workspace, epoch, snapshots) =
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(AppEvent::ProcessesReaped {
                    workspace,
                    epoch,
                    snapshots,
                }) = receiver.recv().await
                {
                    return (workspace, epoch, snapshots);
                }
            }
        })
        .await
        .expect("回收任务回投终态快照");
    // 生产由 handle_app_event 的 ProcessesReaped 分支完成，这里调用同一入口。
    crate::event::chat::persist_terminal_snapshots(&persistence, &workspace, &epoch, &snapshots);

    let after = persistence
        .load_processes_for_session(fixture.workspace(), &key.session)
        .expect("load after reap");
    assert!(
        after[0].state != "running",
        "回收后库里不能再留 running 行：{:?}",
        after[0]
    );
    assert!(
        after[0].ended_at_ms.is_some(),
        "终态必须有结束时刻：{:?}",
        after[0]
    );
}

/// 用户报的完整链路：关闭会话 → 重新打开 → 卡片与后台面板不许说两套话。
///
/// 修复前：回收运行实例只杀进程不落终态，库里留着 `running` 行。重开后
/// `reconcile_process_records` 按该行把卡片判活显示 Running，而后台任务面板
/// 读的是内存运行实例（已随会话回收而空）——于是"卡片在跑、面板全空"。
#[tokio::test]
async fn closing_then_reopening_a_session_settles_the_card_to_a_terminal_state() {
    let fixture = Fixture::new("close-reopen");
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
            tool_calls(&[("call_reopen", "exec_command", &exec_args, 0)], None),
            final_response("running", None),
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

    // 关闭会话：解绑窗格 → 下一轮事件循环回收运行实例。
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.chat.session_id = None;
    }
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    crate::event::chat::pump_chat_streams(&mut state, &mut streams, &sender, &persistence);
    let (workspace, epoch, snapshots) =
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(AppEvent::ProcessesReaped {
                    workspace,
                    epoch,
                    snapshots,
                }) = receiver.recv().await
                {
                    return (workspace, epoch, snapshots);
                }
            }
        })
        .await
        .expect("回收任务回投终态快照");
    crate::event::chat::persist_terminal_snapshots(&persistence, &workspace, &epoch, &snapshots);

    // 重新打开同一会话：卡片按库里的记录校正。
    let records = persistence
        .load_processes_for_session(fixture.workspace(), &key.session)
        .expect("load processes");
    assert!(
        records.iter().all(|record| record.state != "running"),
        "关闭会话后库里不能留 running 行：{:?}",
        records
    );
    let messages = fixture
        .reopen()
        .load_chat_session(fixture.workspace(), &key.session)
        .expect("load session")
        .unwrap_or_default();
    crate::app::update::on_session_loaded(&mut state, key.pane, &key.session, &messages, None);
    crate::app::update::reconcile_process_records(&mut state, key.pane, &records);

    let chat = &state.pane_anywhere(key.pane).expect("pane").lx.chat;
    let call = chat
        .messages
        .iter()
        .flat_map(|message| message.tools())
        .find(|tool| tool.call_id == "call_reopen")
        .expect("tool card");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert!(
        !view.state.is_running(),
        "重开会话后卡片不能还显示 Running（进程早已被回收）：{:?}",
        view.state
    );
}

// ===== review 回归：输出快照不得落后于最新输出 =====

#[tokio::test]
async fn tail_snapshot_keeps_up_past_the_model_budget() {
    // 真实缺陷：`tail_text()` 从保留区间**起点**读 64 KiB，输出超过预算后
    // 卡片与后台任务面板的预览会永久停在最早那一段。
    // 这里直接验证 `OutputBuffer` 的尾部读取口径（进程层已按同一公式取数）。
    let mut buffer = crate::chat::process::OutputBuffer::new(256 * 1024);
    buffer.push(
        crate::chat::process::OutputSource::Stdout,
        &vec![b'x'; 100 * 1024],
    );
    buffer.push(crate::chat::process::OutputSource::Stdout, b"\nNEWEST\n");
    let from = buffer
        .end()
        .saturating_sub(crate::chat::process::MAX_MODEL_OUTPUT_BYTES as u64);
    let read = buffer.read_from(from, crate::chat::process::MAX_MODEL_OUTPUT_BYTES, true);
    assert!(read.text.contains("NEWEST"), "尾部快照必须包含最新输出");
}
