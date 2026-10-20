//! 进程 / 计划卡片的纯状态测试：展开、键盘选择、结算、后台更新与计划恢复。

use crate::app::state::{
    AppState, ChatMessageView, ProcessStateView, ToolCallView, ToolDetailView, ToolStatus,
};
#[allow(unused_imports)]
use crate::app::update::chat::apply_interaction_exit;
use crate::app::update::chat::*;
use crate::chat::plan::{PlanSnapshot, PlanStep, PlanStepStatus};
use crate::chat::process::{
    ProcessOwner, ProcessResult, ProcessResultKind, ProcessSnapshot, ProcessState,
};
use crate::layout::PaneId;

/// 一个助手消息（带一个工具步骤）。
fn message_with(call: ToolCallView) -> ChatMessageView {
    ChatMessageView {
        id: "msg-1".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: 0,
        steps: vec![crate::app::state::AssistantStepView::Tool { position: 0, call }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}

/// 运行中的命令详情。
fn command_detail(session_id: u64) -> ToolDetailView {
    let result = ProcessResult {
        kind: ProcessResultKind::ExecCommand,
        session_id,
        command: "cargo test".into(),
        cwd: "/workspace".into(),
        tty: false,
        state: ProcessState::Running,
        exit_code: None,
        output: "first\n".into(),
        output_start: 0,
        output_end: 6,
        dropped_bytes: 0,
        truncated: false,
        input: None,
        wrote_input: false,
        terminate: false,
        chunk_id: "abc123".into(),
        wall_time_ms: 5,
    };
    ToolDetailView::from_process(&result)
}

/// 带一张命令卡片的窗格状态。
fn state_with_command(session_id: u64) -> (AppState, PaneId) {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut call = ToolCallView::new("call-1", "exec_command", 0);
    call.status = ToolStatus::Running;
    call.detail = Some(Box::new(command_detail(session_id)));
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.chat.messages = vec![message_with(call)];
    (state, pane)
}

#[test]
fn toggle_detail_flips_and_is_keyed_by_stable_ids() {
    let (mut state, pane) = state_with_command(1);
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("msg-1", "call-1")
    );
    assert!(toggle_tool_detail(&mut state, pane, "msg-1", "call-1"));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("msg-1", "call-1")
    );
    assert!(toggle_tool_detail(&mut state, pane, "msg-1", "call-1"));
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("msg-1", "call-1")
    );
    // 未知卡片：不改任何状态。
    assert!(!toggle_tool_detail(&mut state, pane, "msg-1", "missing"));
}

#[test]
fn keyboard_toggle_selects_the_latest_expandable_card() {
    let (mut state, pane) = state_with_command(1);
    assert!(toggle_selected_tool_detail(&mut state, pane));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert_eq!(
        chat.selected_tool,
        Some(("msg-1".to_string(), "call-1".to_string())),
        "未选中时自动选最近一张可展开卡片"
    );
    assert!(chat.detail_is_expanded("msg-1", "call-1"));
}

#[test]
fn adjacent_selection_moves_within_expandable_cards() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut first = ToolCallView::new("call-1", "exec_command", 0);
    first.detail = Some(Box::new(command_detail(1)));
    let mut second = ToolCallView::new("call-2", "exec_command", 1);
    second.detail = Some(Box::new(command_detail(2)));
    let target = state.pane_mut_anywhere(pane).expect("pane");
    let mut message = message_with(first);
    message
        .steps
        .push(crate::app::state::AssistantStepView::Tool {
            position: 1,
            call: second,
        });
    target.lx.chat.messages = vec![message];

    assert!(select_adjacent_tool(&mut state, pane, true));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .selected_tool,
        Some(("msg-1".to_string(), "call-1".to_string()))
    );
    assert!(select_adjacent_tool(&mut state, pane, true));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .selected_tool,
        Some(("msg-1".to_string(), "call-2".to_string()))
    );
    // 末尾不越界。
    assert!(select_adjacent_tool(&mut state, pane, true));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .selected_tool,
        Some(("msg-1".to_string(), "call-2".to_string()))
    );
    assert!(select_adjacent_tool(&mut state, pane, false));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .selected_tool,
        Some(("msg-1".to_string(), "call-1".to_string()))
    );
}

#[test]
fn background_output_updates_the_original_card_only() {
    let (mut state, pane) = state_with_command(7);
    let snapshot = ProcessSnapshot {
        session_id: 7,
        command: "cargo test".into(),
        cwd: "/workspace".into(),
        tty: false,
        state: ProcessState::Running,
        exit_code: None,
        tail: "first\nsecond\n".into(),
        dropped_bytes: 0,
        started_at_ms: 1_000,
        ended_at_ms: None,
        origin_call_id: "call-1".into(),
    };
    assert!(apply_process_output(
        &mut state,
        7,
        "call-1",
        snapshot.clone()
    ));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    let call = chat.messages[0].tool("call-1").expect("tool");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert_eq!(view.output_preview, vec!["first", "second"]);
    assert_eq!(view.output_tail, "first\nsecond\n");
    // 幂等：无变化时不报 changed（避免无谓重绘）。
    assert!(!apply_process_output(&mut state, 7, "call-1", snapshot));
    // 别的进程/调用不影响这张卡片。
    let mut other = ProcessSnapshot {
        session_id: 8,
        command: "other".into(),
        cwd: "/workspace".into(),
        tty: false,
        state: ProcessState::Running,
        exit_code: None,
        tail: "x\n".into(),
        dropped_bytes: 0,
        started_at_ms: 1_000,
        ended_at_ms: None,
        origin_call_id: "call-1".into(),
    };
    assert!(!apply_process_output(
        &mut state,
        8,
        "call-1",
        other.clone()
    ));
    other.origin_call_id = "call-2".into();
    assert!(!apply_process_output(&mut state, 8, "call-2", other));
}

#[test]
fn non_zero_exit_updates_state_without_flagging_a_failure() {
    let (mut state, pane) = state_with_command(9);
    // 工具已经结算为 completed，但进程随后以非零码退出。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let call = target.lx.chat.messages[0].tool_mut("call-1").expect("tool");
        call.status = ToolStatus::Completed;
    }
    let event = crate::chat::process::ProcessUiEvent::Exited {
        session_id: 9,
        origin_call_id: "call-1".into(),
        state: ProcessState::Exited { code: Some(1) },
        exit_code: Some(1),
    };
    assert!(apply_process_exit(&mut state, &event, None));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    let call = chat.messages[0].tool("call-1").expect("tool");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert_eq!(view.state, ProcessStateView::Exited { code: Some(1) });
    // 非零退出是命令跑完后的正常结果，不写失败原因：写了会被渲染成红色失败标记。
    // 退出码仍显示在卡片标题上（`exit 1`），信息不丢。
    assert_eq!(call.error, None, "非零退出不留失败原因");
    assert!(
        view.state.is_nonzero_exit(),
        "但视图仍记录着非零退出，标题据此显示 exit 1"
    );
}

#[test]
fn timeout_exit_still_leaves_a_visible_reason() {
    // 真正的进程级失败必须仍然留原因（红色标记的来源）。
    let (mut state, pane) = state_with_command(9);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let call = target.lx.chat.messages[0].tool_mut("call-1").expect("tool");
        call.status = ToolStatus::Completed;
    }
    let event = crate::chat::process::ProcessUiEvent::Exited {
        session_id: 9,
        origin_call_id: "call-1".into(),
        state: ProcessState::TimedOut,
        exit_code: None,
    };
    assert!(apply_process_exit(&mut state, &event, None));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    let call = chat.messages[0].tool("call-1").expect("tool");
    assert!(call.error.is_some(), "超时仍留下可见原因");
}

#[test]
fn plan_commit_updates_current_plan_and_ignores_duplicates() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let snapshot = PlanSnapshot {
        explanation: Some("doing".into()),
        steps: vec![PlanStep {
            step: "a".into(),
            status: PlanStepStatus::InProgress,
        }],
        revision: 1,
    };
    assert!(commit_plan(&mut state, pane, &snapshot));
    let plan = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.plan.clone())
        .expect("plan stored");
    assert_eq!(plan.revision, 1);
    assert_eq!(plan.steps.len(), 1);
    assert!(!commit_plan(&mut state, pane, &snapshot), "重复提交无变化");
}

#[test]
fn plan_restore_picks_the_latest_successful_update_only() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let successful = |revision: u64, step: &str| PlanSnapshot {
        explanation: None,
        steps: vec![PlanStep {
            step: step.into(),
            status: PlanStepStatus::Pending,
        }],
        revision,
    };
    let mut completed = ToolCallView::new("call-1", "update_plan", 0);
    completed.status = ToolStatus::Completed;
    completed.result_json = Some(successful(2, "later").to_json().to_string());
    let mut failed = ToolCallView::new("call-2", "update_plan", 1);
    failed.status = ToolStatus::Failed;
    failed.result_json = Some(successful(9, "failed-update").to_json().to_string());
    let mut message = message_with(completed);
    message
        .steps
        .push(crate::app::state::AssistantStepView::Tool {
            position: 1,
            call: failed,
        });
    let restored = restore_plan(&mut state, pane, &[message]);
    assert!(restored);
    let plan = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.plan.clone())
        .expect("plan");
    assert_eq!(plan.revision, 2, "失败的更新不覆盖最后成功的计划");
    assert_eq!(plan.steps[0].0, "later");
}

#[test]
fn settle_tool_detail_writes_the_typed_view() {
    let (mut state, pane) = state_with_command(1);
    let key = crate::app::state::RequestKey {
        pane,
        session: "s".into(),
        generation: 1,
    };
    // 只有 Streaming 状态的轮次才接受结算。
    state.pane_mut_anywhere(pane).expect("pane").lx.chat.request =
        crate::app::state::RequestState::Streaming {
            key: key.clone(),
            assistant_id: "msg-1".into(),
        };
    let detail = Some(command_detail(5));
    assert!(settle_tool_detail(
        &mut state,
        &key,
        "call-1",
        detail.clone()
    ));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert_eq!(
        chat.messages[0]
            .tool("call-1")
            .expect("tool")
            .detail
            .as_deref(),
        detail.as_ref()
    );
    assert!(!settle_tool_detail(&mut state, &key, "call-1", detail));
}

#[test]
fn process_owner_is_not_confused_with_the_chat_session() {
    // `process_session_id` 是模型见到的 ID，与聊天会话/OS PID 都不是同一个东西。
    let owner = ProcessOwner {
        workspace: std::path::PathBuf::from("/ws"),
        chat_session_id: "chat-abc".into(),
        runtime_epoch: "epoch-1".into(),
    };
    assert_ne!(owner.chat_session_id, "1");
    let detail = command_detail(1);
    let ToolDetailView::Command(view) = detail else {
        panic!("expected command");
    };
    assert_eq!(view.process_session_id, Some(1));
}

// ===== 本轮结束后的后台终态（用户报的 `Running · 4m 44s` 停不下来）=====

/// 把助手消息改成**已结束**（`complete`），模拟"这一轮早说完了，进程还在后台跑"。
fn finish_the_turn(state: &mut AppState, pane: PaneId) {
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.chat.messages[0].status = "complete".into();
}

#[test]
fn background_exit_reaches_a_card_in_a_finished_turn() {
    // 真实场景：模型交出进程控制权后本轮结束，用户继续别的对话；
    // 进程过一会儿退出，卡片必须跟着变成终态。
    // 旧实现只在「最后一条 streaming 助手消息」里找卡片，轮次一结束就再也找不到，
    // 于是标题一直显示 `Running · 4m 44s`，直到进程被杀。
    let (mut state, pane) = state_with_command(3);
    finish_the_turn(&mut state, pane);
    let event = crate::chat::process::ProcessUiEvent::Exited {
        session_id: 3,
        origin_call_id: "call-1".into(),
        state: ProcessState::Stopped,
        exit_code: None,
    };
    assert!(
        apply_process_exit(&mut state, &event, None),
        "已结束的轮次里的卡片也必须收到终态"
    );
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    let call = chat.messages[0].tool("call-1").expect("tool");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert_eq!(
        view.state,
        ProcessStateView::Stopped,
        "卡片必须收敛到终态，不能停在 Running"
    );
}

#[test]
fn background_output_reaches_a_card_in_a_finished_turn() {
    // 输出同理：轮次结束后进程继续输出，预览应当继续更新。
    let (mut state, pane) = state_with_command(4);
    finish_the_turn(&mut state, pane);
    let snapshot = ProcessSnapshot {
        session_id: 4,
        command: "cargo test".into(),
        cwd: "/workspace".into(),
        tty: false,
        state: ProcessState::Running,
        exit_code: None,
        tail: "later line\n".into(),
        dropped_bytes: 0,
        started_at_ms: 1_000,
        ended_at_ms: None,
        origin_call_id: "call-1".into(),
    };
    assert!(
        apply_process_output(&mut state, 4, "call-1", snapshot),
        "已结束的轮次里的卡片也必须继续接收输出"
    );
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    let call = chat.messages[0].tool("call-1").expect("tool");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        panic!("expected command view");
    };
    assert_eq!(view.output_tail, "later line\n");
}

#[test]
fn interaction_cards_settle_when_the_process_exits() {
    // `Send input` 卡片记的是"当次交互时进程在跑"。进程退出后它也必须收敛，
    // 否则那几张卡片会永远显示 `Running`（截图里的三张 `Send input`）。
    let (mut state, pane) = state_with_command(5);
    let result = ProcessResult {
        kind: ProcessResultKind::WriteStdin,
        session_id: 5,
        command: "while true; do read -r line; done".into(),
        cwd: "/workspace".into(),
        tty: true,
        state: ProcessState::Running,
        exit_code: None,
        output: "got: hi\n".into(),
        output_start: 0,
        output_end: 8,
        dropped_bytes: 0,
        truncated: false,
        input: Some("hi".into()),
        wrote_input: true,
        terminate: false,
        chunk_id: "c1".into(),
        wall_time_ms: 3,
    };
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let mut call = ToolCallView::new("call-input", "write_stdin", 1);
        call.status = ToolStatus::Completed;
        call.detail = Some(Box::new(ToolDetailView::from_process(&result)));
        let message = &mut target.lx.chat.messages[0];
        message
            .steps
            .push(crate::app::state::AssistantStepView::Tool { position: 1, call });
    }
    assert!(apply_interaction_exit(
        &mut state,
        5,
        &ProcessState::Stopped
    ));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    let call = chat.messages[0].tool("call-input").expect("tool");
    let Some(ToolDetailView::Interaction(view)) = call.detail.as_deref() else {
        panic!("expected interaction view");
    };
    assert_eq!(
        view.state,
        ProcessStateView::Stopped,
        "交互卡片也必须收敛到终态"
    );
    // 输出正文不动：那是一次交互的事实记录。
    assert_eq!(view.output, "got: hi\n");
    // 已终态的卡片不会被再次改动。
    assert!(!apply_interaction_exit(
        &mut state,
        5,
        &ProcessState::Stopped
    ));
}
