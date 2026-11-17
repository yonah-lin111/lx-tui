//! New agent 弹窗状态与创建路径测试。

use std::time::{Duration, Instant};

use super::*;

use crate::app::overlay::Overlay;
use crate::app::state::PaneView;
use crate::detect::{AgentKind, InstalledAgent};

fn agent(kind: AgentKind, command: &str) -> InstalledAgent {
    InstalledAgent {
        kind,
        command: command.to_string(),
    }
}

fn installed() -> Vec<InstalledAgent> {
    vec![
        agent(AgentKind::Claude, "claude"),
        agent(AgentKind::OpenCode, "opencode2"),
        agent(AgentKind::Kimi, "kimi"),
    ]
}

fn with_dialog(state: &mut AppState) {
    state.installed_agents = installed();
    open_new_agent_dialog(state, 0);
}

fn dialog_workspace(state: &AppState) -> Option<usize> {
    match state.overlay.as_ref() {
        Some(Overlay::NewAgent(dialog)) => Some(dialog.workspace),
        _ => None,
    }
}

fn selected(state: &AppState) -> Option<usize> {
    match state.overlay.as_ref() {
        Some(Overlay::NewAgent(dialog)) => Some(dialog.selected),
        _ => None,
    }
}

#[test]
fn open_snapshots_installed_agents_and_defaults_to_first_row() {
    let mut state = AppState::demo();
    with_dialog(&mut state);

    assert_eq!(dialog_workspace(&state), Some(0));
    assert_eq!(selected(&state), Some(0));
    match state.overlay.as_ref() {
        Some(Overlay::NewAgent(dialog)) => assert_eq!(dialog.agents, installed()),
        _ => panic!("new agent dialog expected"),
    }
    assert_eq!(state.installed_agents, installed());
}

#[test]
fn open_ignores_missing_workspace() {
    let mut state = AppState::demo();
    state.installed_agents = installed();
    open_new_agent_dialog(&mut state, 99);
    assert!(state.overlay.is_none(), "目标工作区不存在时不得打开弹窗");
}

#[test]
fn selection_moves_within_bounds_and_ignores_unknown_index() {
    let mut state = AppState::demo();
    with_dialog(&mut state);

    assert!(move_new_agent_selection(&mut state, 1));
    assert_eq!(selected(&state), Some(1));
    assert!(move_new_agent_selection(&mut state, 10));
    assert_eq!(selected(&state), Some(2), "下移钳到末项");
    assert!(!move_new_agent_selection(&mut state, 1), "已在末项不变化");
    assert!(move_new_agent_selection(&mut state, -2));
    assert_eq!(selected(&state), Some(0));
    assert!(!move_new_agent_selection(&mut state, -1), "已在首项不变化");

    assert!(set_new_agent_selection(&mut state, 2));
    assert_eq!(selected(&state), Some(2));
    assert!(!set_new_agent_selection(&mut state, 9), "越界忽略");
    assert_eq!(selected(&state), Some(2));
}

#[test]
fn enter_creates_terminal_tab_with_pending_launch() {
    let mut state = AppState::demo();
    with_dialog(&mut state);
    move_new_agent_selection(&mut state, 1);
    let before = Instant::now();

    apply_new_agent_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none(), "创建后弹窗关闭");
    assert_eq!(state.workspaces[0].tabs.len(), 2);
    assert_eq!(state.active_workspace().active_tab, 1);
    let pane = state.active_tab().layout.focus();
    let target = state.pane_anywhere(pane).expect("new pane");
    assert_eq!(target.view, PaneView::Terminal);
    assert!(
        target.cwd.is_none(),
        "启动命令由事件循环写入，创建阶段不探测 cwd"
    );
    let launch = target.pending_launch.as_ref().expect("pending launch");
    assert_eq!(launch.command, "opencode2");
    assert!(
        launch.at >= before + crate::app::update::AGENT_LAUNCH_DELAY,
        "启动延时用于等待 shell 就绪"
    );
}

#[test]
fn enter_with_empty_list_is_noop_and_esc_closes() {
    let mut state = AppState::demo();
    state.installed_agents = Vec::new();
    open_new_agent_dialog(&mut state, 0);

    apply_new_agent_key(&mut state, OverlayKey::Enter);
    assert!(
        matches!(state.overlay, Some(Overlay::NewAgent(_))),
        "空列表回车不得创建或关闭"
    );
    assert_eq!(state.workspaces[0].tabs.len(), 1);

    apply_new_agent_key(&mut state, OverlayKey::Esc);
    assert!(state.overlay.is_none());
}

#[test]
fn vertical_keys_are_ignored_when_list_is_empty() {
    let mut state = AppState::demo();
    state.installed_agents = Vec::new();
    open_new_agent_dialog(&mut state, 0);

    assert!(!move_new_agent_selection(&mut state, 1));
    assert!(!move_new_agent_selection(&mut state, -1));
    assert_eq!(selected(&state), Some(0));
}

#[test]
fn create_is_noop_when_workspace_disappeared_after_open() {
    let mut state = AppState::demo();
    with_dialog(&mut state);
    match state.overlay.as_mut() {
        Some(Overlay::NewAgent(dialog)) => dialog.workspace = 9,
        _ => panic!("new agent dialog expected"),
    }

    apply_new_agent_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    assert_eq!(state.workspaces[0].tabs.len(), 1, "不得创建半成品标签");
}

#[test]
fn double_click_requires_same_row_within_window() {
    let mut state = AppState::demo();
    with_dialog(&mut state);
    let t0 = Instant::now();

    assert!(!new_agent_double_click(&mut state, 1, t0));
    assert!(
        new_agent_double_click(&mut state, 1, t0 + Duration::from_millis(120)),
        "同一行窗口内第二次点击算双击"
    );
    assert!(
        !new_agent_double_click(&mut state, 1, t0 + Duration::from_millis(140)),
        "双击触发后现场清空，避免三击连发"
    );

    assert!(
        !new_agent_double_click(&mut state, 2, t0 + Duration::from_millis(150)),
        "不同行不算双击"
    );
    assert!(
        !new_agent_double_click(
            &mut state,
            2,
            t0 + Duration::from_millis(150)
                + crate::app::update::PROMPT_DOUBLE_CLICK_WINDOW
                + Duration::from_millis(1)
        ),
        "超出窗口不算双击"
    );
}
