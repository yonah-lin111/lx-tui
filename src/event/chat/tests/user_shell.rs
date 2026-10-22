//! 本地命令端到端：真实临时 DB + 真实 ProcessManager + 受控本地脚本。
//!
//! 不请求模型、不访问开发者数据；只验证「预登记 → spawn → 输出/终态 → 落库」的顺序。

use std::time::Duration;

use tokio::sync::mpsc;

use crate::app::state::{AppState, PaneView};
use crate::app::update;
use crate::db::service::Persistence;
use crate::event::AppEvent;

use super::super::{ChatStreams, handle_process_event, pump_user_shells};
use super::Fixture;

const TIMEOUT: Duration = Duration::from_secs(10);

fn shell_ready(fixture: &Fixture, text: &str) -> (AppState, Persistence, crate::layout::PaneId) {
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(fixture.workspace().to_path_buf());
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    assert_eq!(target.view, PaneView::Lx);
    target.lx.import_full_text(text);
    (state, Persistence::with_database(fixture.database()), pane)
}

fn terminal(state: &AppState, pane: crate::layout::PaneId) -> bool {
    state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.messages.last())
        .is_some_and(|message| message.shell.is_some() && message.status != "streaming")
}

/// 驱动事件直到命令卡片进入终态（或用尽超时）。
async fn drive_until_terminal(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    receiver: &mut mpsc::UnboundedReceiver<AppEvent>,
    process_receiver: &mut mpsc::UnboundedReceiver<crate::chat::process::ProcessUiEvent>,
    pane: crate::layout::PaneId,
) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while !terminal(state, pane) {
        assert!(tokio::time::Instant::now() < deadline, "命令未在超时内结束");
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Some(AppEvent::UserShellSpawned { pane, generation, message_id, process_id }) => {
                        crate::event::chat::user_shell::handle_spawned(
                            state, streams, persistence, pane, generation, &message_id, process_id,
                        );
                    }
                    Some(AppEvent::UserShellSpawnFailed { pane, generation, message_id, code }) => {
                        crate::event::chat::user_shell::handle_spawn_failed(
                            state, streams, persistence, pane, generation, &message_id, &code,
                        );
                    }
                    Some(_) => {}
                    None => break,
                }
            }
            event = process_receiver.recv() => {
                if let Some(event) = event {
                    handle_process_event(state, streams, persistence, event);
                }
            }
            () = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
    let _ = sender;
}

fn channels() -> (
    mpsc::UnboundedSender<AppEvent>,
    mpsc::UnboundedReceiver<AppEvent>,
    mpsc::UnboundedSender<crate::chat::process::ProcessUiEvent>,
    mpsc::UnboundedReceiver<crate::chat::process::ProcessUiEvent>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let (process_sender, process_receiver) = mpsc::unbounded_channel();
    (sender, receiver, process_sender, process_receiver)
}

#[tokio::test]
async fn short_script_runs_persists_output_without_model() {
    let fixture = Fixture::new("shell-short");
    let (mut state, persistence, pane) =
        shell_ready(&fixture, "!printf 'hello\\nworld\\n' | wc -l");
    assert!(update::try_send(&mut state, pane), "shell Enter 立即受理");
    let (sender, mut receiver, process_sender, mut process_receiver) = channels();
    let mut streams = ChatStreams::default();
    streams.set_process_sender(process_sender);
    assert!(pump_user_shells(
        &mut state,
        &mut streams,
        &sender,
        &persistence
    ));
    let session = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
        .expect("命令创建会话");
    drive_until_terminal(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        &mut receiver,
        &mut process_receiver,
        pane,
    )
    .await;
    let message = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .messages
        .last()
        .unwrap()
        .clone();
    assert_eq!(message.status, "complete");
    let shell = message.shell.as_ref().expect("shell record");
    assert_eq!(shell.exit_code, Some(0));
    assert_eq!(shell.output.trim(), "2", "管道由登录 shell 解释");
    // 落库：重新打开连接读到同一事实。
    let database = fixture.reopen();
    let messages = database
        .load_chat_session(fixture.workspace(), &session)
        .expect("load")
        .expect("session");
    let loaded = messages
        .last()
        .unwrap()
        .user_shell
        .as_ref()
        .expect("record");
    assert_eq!(loaded.output.trim(), "2");
    assert_eq!(loaded.state, "exited");
    assert_eq!(loaded.exit_code, Some(0));
}

#[tokio::test]
async fn nonzero_exit_is_recorded_as_completed_command() {
    let fixture = Fixture::new("shell-nonzero");
    let (mut state, persistence, pane) = shell_ready(&fixture, "!exit 3");
    assert!(update::try_send(&mut state, pane));
    let (sender, mut receiver, process_sender, mut process_receiver) = channels();
    let mut streams = ChatStreams::default();
    streams.set_process_sender(process_sender);
    pump_user_shells(&mut state, &mut streams, &sender, &persistence);
    drive_until_terminal(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        &mut receiver,
        &mut process_receiver,
        pane,
    )
    .await;
    let message = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .messages
        .last()
        .unwrap()
        .clone();
    assert_eq!(message.status, "complete", "非零退出不是失败");
    assert_eq!(message.shell.as_ref().unwrap().exit_code, Some(3));
}

#[tokio::test]
async fn db_pre_register_failure_never_spawns() {
    let fixture = Fixture::new("shell-db-fail");
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .import_full_text("!touch should-not-exist");
    // 没有数据库：预登记失败必须保留草稿且不执行命令。
    let persistence = Persistence::without_database();
    assert!(update::try_send(&mut state, pane));
    let (sender, _receiver, process_sender, _process_receiver) = channels();
    let mut streams = ChatStreams::default();
    streams.set_process_sender(process_sender);
    assert!(pump_user_shells(
        &mut state,
        &mut streams,
        &sender,
        &persistence
    ));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.input, "touch should-not-exist");
    assert!(!fixture.exists("should-not-exist"));
}

#[tokio::test]
async fn cancel_stops_running_command_and_keeps_output() {
    let fixture = Fixture::new("shell-cancel");
    let (mut state, persistence, pane) = shell_ready(&fixture, "!printf before; sleep 30");
    assert!(update::try_send(&mut state, pane));
    let (sender, mut receiver, process_sender, mut process_receiver) = channels();
    let mut streams = ChatStreams::default();
    streams.set_process_sender(process_sender);
    pump_user_shells(&mut state, &mut streams, &sender, &persistence);
    // 等到 spawn 成功（进程句柄已登记）。
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        tokio::select! {
            event = receiver.recv() => {
                if let Some(AppEvent::UserShellSpawned { pane: p, generation, message_id, process_id }) = event {
                    crate::event::chat::user_shell::handle_spawned(
                        &mut state, &mut streams, &persistence, p, generation, &message_id, process_id,
                    );
                    break;
                }
            }
            event = process_receiver.recv() => {
                if let Some(event) = event {
                    handle_process_event(&mut state, &mut streams, &persistence, event);
                }
            }
            () = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
        assert!(tokio::time::Instant::now() < deadline, "未等到 spawn");
    }
    // 给首行输出一点时间进入进程缓冲，再停止（停止必须保留已产生的输出）。
    tokio::time::sleep(Duration::from_millis(300)).await;
    state
        .effects
        .push(crate::app::effects::Effect::CancelUserShells { pane });
    assert!(pump_user_shells(
        &mut state,
        &mut streams,
        &sender,
        &persistence
    ));
    drive_until_terminal(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        &mut receiver,
        &mut process_receiver,
        pane,
    )
    .await;
    let message = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .messages
        .last()
        .unwrap()
        .clone();
    assert_eq!(message.status, "interrupted");
    assert!(
        message.shell.as_ref().unwrap().output.contains("before"),
        "停止保留已产生输出"
    );
}

fn terminal_record(exit: i32, output: &str) -> crate::db::chat::UserShellRecord {
    let mut record = crate::db::chat::UserShellRecord::starting("pwd", "/tmp", "/bin/zsh");
    record.state = "exited".into();
    record.exit_code = Some(exit);
    record.output = output.into();
    record.output_end = output.len() as u64;
    record.revision = 2;
    record
}

#[tokio::test]
async fn injection_skips_initial_terminal_and_absorbs_later_completion_once() {
    let fixture = Fixture::new("shell-inject");
    let persistence = Persistence::with_database(fixture.database());
    let database = fixture.reopen();
    let first = database
        .begin_user_shell(
            fixture.workspace(),
            None,
            &crate::db::chat::UserShellRecord::starting("pwd", "/tmp", "/bin/zsh"),
        )
        .expect("first shell");
    // 第一条在轮次开始时仍在运行，第二条刚开始采样后才完成。
    let mut running = crate::db::chat::UserShellRecord::starting("ls", "/tmp", "/bin/zsh");
    running.state = "running".into();
    running.revision = 1;
    database
        .update_user_shell(fixture.workspace(), &first.message_id, &running)
        .expect("running");
    let second = database
        .begin_user_shell(
            fixture.workspace(),
            Some(&first.session_id),
            &crate::db::chat::UserShellRecord::starting("echo hi", "/tmp", "/bin/zsh"),
        )
        .expect("second shell");
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.active_workspace_mut().cwd = Some(fixture.workspace().to_path_buf());
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some(first.session_id.clone());
    let key = crate::app::state::RequestKey {
        pane,
        session: first.session_id.clone(),
        generation: 1,
    };
    let mut streams = ChatStreams::default();
    crate::event::chat::user_shell::mark_initial_seen(&state, &mut streams, &key);
    // 第一条仍在运行：顺序屏障暂缓后续结果。
    assert!(
        crate::event::chat::user_shell::poll_updates(&state, &mut streams, &persistence, &key)
            .is_empty()
    );
    // 第一条完成 → 两条按提交序一次注入。
    database
        .update_user_shell(
            fixture.workspace(),
            &first.message_id,
            &terminal_record(0, "first"),
        )
        .expect("first terminal");
    database
        .update_user_shell(
            fixture.workspace(),
            &second.message_id,
            &terminal_record(0, "second"),
        )
        .expect("second terminal");
    let updates =
        crate::event::chat::user_shell::poll_updates(&state, &mut streams, &persistence, &key);
    assert_eq!(updates.len(), 2);
    assert!(updates[0].contains("first"));
    assert!(updates[1].contains("second"));
    // 已见标记：不会重复注入。
    assert!(
        crate::event::chat::user_shell::poll_updates(&state, &mut streams, &persistence, &key)
            .is_empty()
    );
}
