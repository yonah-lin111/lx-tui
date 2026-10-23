//! 本地命令提交、回执与队列的纯状态测试。

use crate::app::effects::Effect;
use crate::app::state::{AppState, ChatMessageView, QueuedSubmission, RequestState};
use crate::app::update::chat::{self, user_shell};
use crate::db::chat::UserShellCommitted;

use super::{focus_pane, ready_state};

fn shell_state(text: &str) -> AppState {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .import_full_text(text);
    state
}

fn committed(session: &str, message: &str) -> UserShellCommitted {
    UserShellCommitted {
        session_id: session.into(),
        message_id: message.into(),
        turn_index: 0,
        sequence: 0,
        created_at_ms: 1_000,
    }
}

fn begin_shell(state: &mut AppState, pane: crate::layout::PaneId, message: &str, command: &str) {
    let committed = committed("sess", message);
    user_shell::on_user_shell_begun(
        state,
        pane,
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .generation_counter,
        &committed.session_id,
        &committed.message_id,
        committed.turn_index,
        command,
        "/tmp",
        "/bin/zsh",
    );
}

#[test]
fn shell_enter_freezes_command_without_model_request() {
    let mut state = shell_state("  !echo hi");
    let pane = focus_pane(&state);
    assert!(chat::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.pending_shells.len(), 1);
    assert_eq!(target.lx.chat.pending_shells[0].command, "echo hi");
    assert_eq!(
        target.lx.chat.pending_shells[0].revision,
        target.lx.revision()
    );
    assert!(matches!(target.lx.chat.request, RequestState::Idle));
    let effects: Vec<&Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::BeginUserShell { .. } | Effect::BeginChatTurn { .. }
            )
        })
        .collect();
    assert_eq!(effects.len(), 1);
    match effects[0] {
        Effect::BeginUserShell { command, cwd, .. } => {
            assert_eq!(command, "echo hi");
            assert!(cwd.is_absolute());
        }
        other => panic!("必须是本地命令效应: {other:?}"),
    }
}

#[test]
fn empty_shell_command_does_not_submit() {
    let mut state = shell_state("!   ");
    let pane = focus_pane(&state);
    assert!(chat::try_send(&mut state, pane), "给了可见提示");
    assert!(state.effects.is_empty());
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .pending_shells
            .is_empty()
    );
}

#[test]
fn spawn_ack_clears_matching_revision_and_records_history_once() {
    let mut state = shell_state("!pwd");
    let pane = focus_pane(&state);
    chat::try_send(&mut state, pane);
    let generation = state.pane_anywhere(pane).unwrap().lx.chat.pending_shells[0].generation;
    user_shell::on_user_shell_begun(
        &mut state, pane, generation, "sess", "msg-1", 0, "pwd", "/tmp", "/bin/zsh",
    );
    assert!(user_shell::on_user_shell_spawned(
        &mut state,
        pane,
        generation,
        "msg-1",
        7,
        Some("epoch")
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, "", "spawn 成功清空草稿");
    assert!(!target.lx.shell_mode(), "回到 Chat 模式");
    assert!(target.lx.chat.pending_shells.is_empty());
    assert_eq!(
        target
            .lx
            .chat
            .messages
            .last()
            .unwrap()
            .shell
            .as_ref()
            .unwrap()
            .process_id,
        Some(7)
    );
    assert!(state.prompt_history.entries().contains(&"!pwd".to_string()));
    // 重复 ACK 不再追加历史。
    let before = state.prompt_history.entries().to_vec();
    user_shell::on_user_shell_spawned(&mut state, pane, generation, "msg-1", 7, Some("epoch"));
    assert_eq!(state.prompt_history.entries(), before.as_slice());
}

#[test]
fn stale_spawn_ack_does_not_clear_new_draft() {
    let mut state = shell_state("!pwd");
    let pane = focus_pane(&state);
    chat::try_send(&mut state, pane);
    let generation = state.pane_anywhere(pane).unwrap().lx.chat.pending_shells[0].generation;
    user_shell::on_user_shell_begun(
        &mut state, pane, generation, "sess", "msg-1", 0, "pwd", "/tmp", "/bin/zsh",
    );
    // spawn 前改过草稿（修订号前进）：旧 ACK 不得清空。
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .import_full_text("!ls");
    user_shell::on_user_shell_spawned(&mut state, pane, generation, "msg-1", 7, Some("epoch"));
    assert_eq!(state.pane_anywhere(pane).unwrap().lx.input, "ls");
    assert!(state.pane_anywhere(pane).unwrap().lx.shell_mode());
}

#[test]
fn spawn_failure_keeps_draft_and_does_not_record_history() {
    let mut state = shell_state("!pwd");
    let pane = focus_pane(&state);
    chat::try_send(&mut state, pane);
    let generation = state.pane_anywhere(pane).unwrap().lx.chat.pending_shells[0].generation;
    user_shell::on_user_shell_begun(
        &mut state, pane, generation, "sess", "msg-1", 0, "pwd", "/tmp", "/bin/zsh",
    );
    user_shell::on_user_shell_spawn_failed(&mut state, pane, generation, "msg-1", "resource_limit");
    assert_eq!(state.pane_anywhere(pane).unwrap().lx.input, "pwd");
    assert!(!state.prompt_history.entries().contains(&"!pwd".to_string()));
    let message = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .messages
        .last()
        .unwrap()
        .clone();
    assert_eq!(message.status, "failed");
    assert!(message.shell.as_ref().unwrap().is_failed());
}

#[test]
fn shell_tab_queues_only_while_busy() {
    let mut state = shell_state("!pwd");
    let pane = focus_pane(&state);
    assert!(
        !user_shell::queue_shell(&mut state, pane),
        "空闲 Tab 不执行"
    );
    assert_eq!(state.pane_anywhere(pane).unwrap().lx.input, "pwd");
    // 让模型处于忙碌态。
    state.pane_mut_anywhere(pane).unwrap().lx.chat.request = RequestState::Committing {
        generation: 1,
        draft: "hi".into(),
    };
    assert!(user_shell::queue_shell(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.input.is_empty());
    assert!(matches!(
        target.lx.chat.queue.as_slice(),
        [QueuedSubmission::Shell { .. }]
    ));
}

#[test]
fn chat_enter_queues_while_shell_runs_then_dispatches_after_terminal() {
    let mut state = ready_state("解释一下");
    let pane = focus_pane(&state);
    begin_shell(&mut state, pane, "msg-1", "sleep 5");
    // 卡片初始为 Starting（运行中）。
    assert!(chat::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(
        target
            .lx
            .chat
            .queue
            .iter()
            .any(|item| matches!(item, QueuedSubmission::Chat { .. }))
    );
    assert!(target.lx.input.is_empty());
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BeginChatTurn { .. }))
    );
    // 命令结束：队列分派普通聊天。
    let mut view = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .messages
        .last()
        .unwrap()
        .shell
        .clone()
        .unwrap();
    view.state = crate::app::state::ProcessStateView::Exited { code: Some(0) };
    view.revision = 1;
    user_shell::apply_user_shell_snapshot(&mut state, pane, "msg-1", view);
    assert!(user_shell::on_user_shell_terminal(&mut state, pane));
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BeginChatTurn { .. }))
    );
}

#[test]
fn double_escape_stops_shell_only_runs() {
    let mut state = shell_state("!sleep 60");
    let pane = focus_pane(&state);
    // 先提交并进入运行态。
    chat::try_send(&mut state, pane);
    let generation = state.pane_anywhere(pane).unwrap().lx.chat.pending_shells[0].generation;
    user_shell::on_user_shell_begun(
        &mut state, pane, generation, "sess", "msg-1", 0, "sleep 60", "/tmp", "/bin/zsh",
    );
    let mut view = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .messages
        .last()
        .unwrap()
        .shell
        .clone()
        .unwrap();
    view.state = crate::app::state::ProcessStateView::Running;
    view.process_id = Some(9);
    user_shell::apply_user_shell_snapshot(&mut state, pane, "msg-1", view);
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .queue
        .push(QueuedSubmission::Chat {
            draft: "next".into(),
        });
    assert!(chat::escape_stop(&mut state, pane), "第一次只待命");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .stop_armed_at
            .is_some()
    );
    assert!(chat::escape_stop(&mut state, pane), "第二次停止");
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CancelUserShells { .. }))
    );
    assert!(state.pane_anywhere(pane).unwrap().lx.chat.queue.is_empty());
}

#[test]
fn snapshot_does_not_revive_terminal_card() {
    let mut state = shell_state("!pwd");
    let pane = focus_pane(&state);
    begin_shell(&mut state, pane, "msg-1", "pwd");
    let mut terminal = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .messages
        .last()
        .unwrap()
        .shell
        .clone()
        .unwrap();
    terminal.state = crate::app::state::ProcessStateView::Exited { code: Some(0) };
    terminal.revision = 5;
    assert!(user_shell::apply_user_shell_snapshot(
        &mut state, pane, "msg-1", terminal
    ));
    let mut late = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .messages
        .last()
        .unwrap()
        .shell
        .clone()
        .unwrap();
    late.state = crate::app::state::ProcessStateView::Running;
    late.revision = 6;
    assert!(!user_shell::apply_user_shell_snapshot(
        &mut state, pane, "msg-1", late
    ));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .messages
            .last()
            .unwrap()
            .status,
        "complete"
    );
}

#[test]
fn shell_message_is_not_a_chat_user_draft() {
    let mut state = ready_state("draft");
    let pane = focus_pane(&state);
    begin_shell(&mut state, pane, "msg-1", "pwd");
    let messages: Vec<ChatMessageView> =
        state.pane_anywhere(pane).unwrap().lx.chat.messages.clone();
    assert!(messages.last().unwrap().is_user_shell());
    let draft = crate::event::chat::latest_user_draft_for_test(&messages);
    assert_eq!(draft, "", "最后一条 shell 不得被当成普通用户草稿");
}

#[test]
fn queued_dispatch_never_overwrites_an_active_draft() {
    let mut state = shell_state("!pwd");
    let pane = focus_pane(&state);
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .queue
        .push(QueuedSubmission::Shell {
            full_text: "!echo queued".into(),
            cwd: std::path::PathBuf::from("/tmp"),
        });
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .import_full_text("!echo mine");
    assert!(!user_shell::on_user_shell_terminal(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, "echo mine", "正在输入的草稿不得被队列覆盖");
    assert_eq!(target.lx.chat.queue.len(), 1, "队首保留到下次结算");
}

#[test]
fn shell_terminal_keeps_model_stop_arm() {
    let mut state = shell_state("!pwd");
    let pane = focus_pane(&state);
    let key = crate::app::state::RequestKey {
        pane,
        session: "sess".into(),
        generation: 1,
    };
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.chat.request = RequestState::Streaming {
        key,
        assistant_id: "a1".into(),
    };
    target.lx.chat.stop_armed_at = Some(std::time::Instant::now());
    // 模型仍在流式：命令结束不能取消模型轮的停止待命。
    user_shell::on_user_shell_terminal(&mut state, pane);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .stop_armed_at
            .is_some(),
        "模型未结束时必须保留待命"
    );
}
