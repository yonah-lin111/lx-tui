//! 新建 Agent 标签的启动命令排队：到期门槛、失败清理与写入字节。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::app::state::PendingAgentLaunch;
use crate::app::toast::ToastKind;
use crate::event::session::{agent_launch_bytes, flush_pending_agent_launches_at};

use super::test_support::demo_terminal;

fn pane_with_launch(state: &mut crate::app::state::AppState, at: Instant, command: &str) {
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.pending_launch = Some(PendingAgentLaunch {
        command: command.to_string(),
        at,
    });
}

fn pending(state: &crate::app::state::AppState) -> Option<&PendingAgentLaunch> {
    let pane = state.active_tab().layout.focus();
    state
        .pane_anywhere(pane)
        .and_then(|target| target.pending_launch.as_ref())
}

#[test]
fn launch_before_deadline_keeps_queue_untouched() {
    let mut state = demo_terminal();
    let now = Instant::now();
    pane_with_launch(&mut state, now + Duration::from_millis(150), "claude");
    let mut sessions = HashMap::new();

    assert!(!flush_pending_agent_launches_at(
        &mut state,
        &mut sessions,
        now
    ));

    assert_eq!(
        pending(&state).map(|launch| launch.command.as_str()),
        Some("claude"),
        "未到点的命令必须继续排队"
    );
    assert!(state.toast.is_none());
}

#[test]
fn launch_without_session_clears_queue_and_toasts() {
    let mut state = demo_terminal();
    let now = Instant::now();
    pane_with_launch(&mut state, now, "claude");
    let pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();

    assert!(flush_pending_agent_launches_at(
        &mut state,
        &mut sessions,
        now
    ));

    assert!(pending(&state).is_none(), "失败后必须清掉队列");
    let toast = state.toast.as_ref().expect("launch failure toast");
    assert_eq!(toast.kind, ToastKind::Danger);
    assert_eq!(toast.message, crate::ui::text::TOAST_AGENT_LAUNCH_FAILED);
    assert_eq!(toast.anchor, Some(pane), "反馈锚定在目标窗格");
}

#[test]
fn already_fired_launch_is_not_replayed() {
    let mut state = demo_terminal();
    let now = Instant::now();
    pane_with_launch(&mut state, now, "claude");
    let mut sessions = HashMap::new();

    assert!(flush_pending_agent_launches_at(
        &mut state,
        &mut sessions,
        now
    ));
    assert!(!flush_pending_agent_launches_at(
        &mut state,
        &mut sessions,
        now + Duration::from_secs(1)
    ));
}

#[test]
fn launch_bytes_end_with_carriage_return() {
    assert_eq!(agent_launch_bytes("opencode2"), b"opencode2\r".to_vec());
    assert_eq!(agent_launch_bytes(""), b"\r".to_vec());
}
