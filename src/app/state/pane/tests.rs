//! 侧栏 Agent 标题解析与内存覆盖生命周期测试。

use super::*;

use crate::app::state::{AiWebSite, AppState, LxEntry};
use crate::app::update::update_pane_agent;
use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};

fn lx_kind() -> SidebarAgentKind {
    SidebarAgentKind::Lx {
        entry: LxEntry::Native,
        ai_web_site: AiWebSite::Deepseek,
    }
}

fn terminal_kind(kind: AgentKind) -> SidebarAgentKind {
    SidebarAgentKind::Terminal(kind)
}

fn snapshot(kind: AgentKind, state: AgentState, title: Option<&str>) -> PaneAgentSnapshot {
    PaneAgentSnapshot {
        kind,
        state,
        title: title.map(str::to_string),
    }
}

fn active_lx_pane() -> Pane {
    let mut pane = Pane::new();
    pane.lx.chat.session_id = Some("session-1".to_string());
    pane
}

#[test]
fn pane_new_has_no_agent_title_overrides() {
    let pane = Pane::new();
    assert_eq!(pane.terminal_agent_title, None);
    assert_eq!(pane.lx.chat.agent_title_override, None);
}

#[test]
fn lx_default_title_uses_session_title() {
    let mut pane = active_lx_pane();
    assert_eq!(pane.agent_display_title(lx_kind()), None);

    pane.lx.chat.session_title = Some("Fix login bug".to_string());
    assert_eq!(pane.agent_display_title(lx_kind()), Some("Fix login bug"));
}

#[test]
fn lx_override_wins_and_clear_falls_back() {
    let mut pane = active_lx_pane();
    pane.lx.chat.session_title = Some("Auto title".to_string());

    pane.set_agent_title_override(lx_kind(), Some("  Manual title  ".to_string()));
    assert_eq!(
        pane.agent_display_title(lx_kind()),
        Some("Manual title"),
        "覆盖优先且首尾空白被裁掉"
    );
    assert_eq!(
        pane.lx.chat.session_title.as_deref(),
        Some("Auto title"),
        "设置覆盖不得修改会话真实标题"
    );

    pane.set_agent_title_override(lx_kind(), None);
    assert_eq!(pane.agent_display_title(lx_kind()), Some("Auto title"));

    pane.set_agent_title_override(lx_kind(), Some("Manual title".to_string()));
    pane.set_agent_title_override(lx_kind(), Some("   ".to_string()));
    assert_eq!(
        pane.agent_display_title(lx_kind()),
        Some("Auto title"),
        "空白覆盖按清除处理"
    );
}

#[test]
fn lx_override_requires_active_session() {
    let mut pane = Pane::new();
    pane.lx.chat.session_title = Some("Auto title".to_string());

    pane.set_agent_title_override(lx_kind(), Some("Manual title".to_string()));

    assert_eq!(pane.lx.chat.agent_title_override, None);
    assert_eq!(pane.agent_display_title(lx_kind()), Some("Auto title"));
}

#[test]
fn terminal_default_title_uses_snapshot_title_and_blank_is_none() {
    let mut pane = Pane::new();
    let kind = terminal_kind(AgentKind::Claude);

    assert_eq!(pane.agent_display_title(kind), None);
    pane.agent = Some(snapshot(
        AgentKind::Claude,
        AgentState::Idle,
        Some("Refactor parser"),
    ));
    assert_eq!(pane.agent_display_title(kind), Some("Refactor parser"));

    pane.agent = Some(snapshot(AgentKind::Claude, AgentState::Idle, Some("")));
    assert_eq!(pane.agent_display_title(kind), None);

    pane.agent = Some(snapshot(AgentKind::Claude, AgentState::Idle, None));
    assert_eq!(pane.agent_display_title(kind), None);
}

#[test]
fn terminal_override_only_applies_to_matching_kind() {
    let mut pane = Pane::new();
    pane.agent = Some(snapshot(
        AgentKind::Claude,
        AgentState::Idle,
        Some("Claude OSC"),
    ));

    pane.set_agent_title_override(terminal_kind(AgentKind::Claude), Some("Manual".to_string()));
    assert_eq!(
        pane.agent_display_title(terminal_kind(AgentKind::Claude)),
        Some("Manual")
    );
    assert_eq!(
        pane.agent_display_title(terminal_kind(AgentKind::Codex)),
        None,
        "没有对应快照时不得借用别的 Agent 覆盖"
    );
    assert_eq!(
        pane.terminal_agent_title,
        Some((AgentKind::Claude, "Manual".to_string()))
    );

    // 当前窗格是 Claude 时，写入 Codex 的覆盖无效。
    pane.set_agent_title_override(terminal_kind(AgentKind::Codex), Some("Wrong".to_string()));
    assert_eq!(
        pane.terminal_agent_title,
        Some((AgentKind::Claude, "Manual".to_string()))
    );

    // 覆盖按 AgentKind 存储：查询另一种 Agent 不借用 Claude 的覆盖。
    assert_eq!(
        pane.agent_display_title(terminal_kind(AgentKind::Codex)),
        None
    );
}

#[test]
fn update_pane_agent_clears_override_on_kind_change() {
    let mut state = AppState::demo();
    let pane_id = state.active_tab().layout.focus();
    update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("Claude OSC"),
        )),
    );
    state
        .pane_mut_anywhere(pane_id)
        .expect("pane")
        .set_agent_title_override(terminal_kind(AgentKind::Claude), Some("Manual".to_string()));

    assert!(update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Codex,
            AgentState::Idle,
            Some("Codex OSC"),
        )),
    ));

    assert_eq!(
        state
            .pane_anywhere(pane_id)
            .expect("pane")
            .terminal_agent_title,
        None
    );
}

#[test]
fn update_pane_agent_clears_override_on_agent_exit() {
    let mut state = AppState::demo();
    let pane_id = state.active_tab().layout.focus();
    update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("Claude OSC"),
        )),
    );
    state
        .pane_mut_anywhere(pane_id)
        .expect("pane")
        .set_agent_title_override(terminal_kind(AgentKind::Claude), Some("Manual".to_string()));

    assert!(update_pane_agent(&mut state, pane_id, None));

    assert_eq!(
        state
            .pane_anywhere(pane_id)
            .expect("pane")
            .terminal_agent_title,
        None
    );
}

#[test]
fn update_pane_agent_keeps_override_for_same_kind() {
    let mut state = AppState::demo();
    let pane_id = state.active_tab().layout.focus();
    update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("First OSC"),
        )),
    );
    state
        .pane_mut_anywhere(pane_id)
        .expect("pane")
        .set_agent_title_override(terminal_kind(AgentKind::Claude), Some("Manual".to_string()));

    assert!(update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Working,
            Some("Second OSC"),
        )),
    ));

    let pane = state.pane_anywhere(pane_id).expect("pane");
    assert_eq!(
        pane.terminal_agent_title,
        Some((AgentKind::Claude, "Manual".to_string()))
    );
    assert_eq!(
        pane.agent_display_title(terminal_kind(AgentKind::Claude)),
        Some("Manual")
    );
}

#[test]
fn reset_session_view_clears_lx_override() {
    let mut pane = active_lx_pane();
    pane.lx.chat.session_title = Some("Auto title".to_string());
    pane.set_agent_title_override(lx_kind(), Some("Manual title".to_string()));
    assert_eq!(
        pane.lx.chat.agent_title_override.as_deref(),
        Some("Manual title")
    );

    pane.lx.chat.reset_session_view();

    assert_eq!(pane.lx.chat.agent_title_override, None);
    assert_eq!(pane.agent_display_title(lx_kind()), None);
}
