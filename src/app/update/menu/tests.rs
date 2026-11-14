//! Agents 条目右键菜单、行内确认、关闭分派与标题编辑提交测试。

use super::*;

use crate::app::actions::OverlayKey;
use crate::app::state::{AiWebSite, ChatMessageView, LxEntry};
use crate::app::update::{apply_overlay_key, new_session, update_pane_agent};
use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};

fn lx_kind() -> SidebarAgentKind {
    SidebarAgentKind::Lx {
        entry: LxEntry::Native,
        ai_web_site: AiWebSite::Deepseek,
    }
}

fn snapshot(kind: AgentKind, state: AgentState, title: Option<&str>) -> PaneAgentSnapshot {
    PaneAgentSnapshot {
        kind,
        state,
        title: title.map(str::to_string),
    }
}

fn set_lx_session(state: &mut AppState, pane: PaneId, session: &str) {
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some(session.to_string());
}

fn split_pane(state: &mut AppState, source: PaneId) -> PaneId {
    state
        .active_tab_mut()
        .split_pane(source, Direction::Horizontal)
        .expect("split pane")
}

fn agent_entry_index(state: &AppState, pane: PaneId, kind: SidebarAgentKind) -> usize {
    state
        .agent_entries()
        .iter()
        .position(|entry| entry.pane_id == pane && entry.kind == kind)
        .expect("agent entry")
}

fn menu_commands(state: &AppState) -> Vec<MenuCommand> {
    match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => menu.commands.clone(),
        _ => panic!("expected menu overlay"),
    }
}

fn select_command(state: &mut AppState, command: MenuCommand) {
    let selected = match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => menu
            .commands
            .iter()
            .position(|candidate| *candidate == command)
            .expect("menu command"),
        _ => panic!("expected menu overlay"),
    };
    assert!(set_menu_selection(state, selected));
}

fn rename_input(state: &mut AppState, text: &str) {
    let Some(Overlay::Rename(rename)) = state.overlay.as_mut() else {
        panic!("expected rename overlay");
    };
    rename.input = TextInput::new(text);
}

fn chat_message(id: &str) -> ChatMessageView {
    ChatMessageView {
        id: id.to_string(),
        sequence: 0,
        kind: "chat".to_string(),
        role: "user".to_string(),
        status: "complete".to_string(),
        turn_index: 0,
        shell: None,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

#[test]
fn lx_agent_menu_offers_edit_and_close() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    state.agent_scroll_drag = Some(3);

    open_agent_menu(&mut state, 0, (3, 4));

    assert_eq!(
        menu_commands(&state),
        vec![MenuCommand::EditAgentTitle, MenuCommand::CloseAgent]
    );
    assert_eq!(state.agent_scroll_drag, None, "打开菜单必须结束滚动条拖拽");
    match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => assert_eq!(
            menu.target,
            OverlayTarget::Agent {
                workspace: 0,
                tab: 0,
                pane,
                kind: lx_kind(),
            }
        ),
        _ => panic!("expected menu overlay"),
    }
}

#[test]
fn terminal_agent_menu_offers_close_only_with_multiple_panes() {
    let mut state = AppState::demo();
    let first = state.active_tab().layout.focus();
    let second = split_pane(&mut state, first);
    update_pane_agent(
        &mut state,
        second,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("claude"),
        )),
    );
    let index = agent_entry_index(
        &state,
        second,
        SidebarAgentKind::Terminal(AgentKind::Claude),
    );

    open_agent_menu(&mut state, index, (1, 1));

    assert_eq!(
        menu_commands(&state),
        vec![MenuCommand::EditAgentTitle, MenuCommand::CloseAgent]
    );
}

#[test]
fn terminal_agent_menu_omits_close_for_last_pane() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    update_pane_agent(
        &mut state,
        pane,
        Some(snapshot(AgentKind::Codex, AgentState::Idle, Some("codex"))),
    );
    let index = agent_entry_index(&state, pane, SidebarAgentKind::Terminal(AgentKind::Codex));

    open_agent_menu(&mut state, index, (0, 0));

    assert_eq!(menu_commands(&state), vec![MenuCommand::EditAgentTitle]);
}

#[test]
fn edit_agent_title_opens_rename_prefilled_with_display_title() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_title = Some("Auto title".to_string());
        target.set_agent_title_override(lx_kind(), Some("Manual title".to_string()));
    }
    open_agent_menu(&mut state, 0, (0, 0));

    assert_eq!(activate_menu(&mut state), None);

    match state.overlay.as_ref() {
        Some(Overlay::Rename(rename)) => {
            assert_eq!(
                rename.target,
                RenameTarget::AgentTitle {
                    workspace: 0,
                    tab: 0,
                    pane,
                    kind: lx_kind(),
                }
            );
            assert_eq!(rename.input.text(), "Manual title");
        }
        _ => panic!("expected rename overlay"),
    }
}

#[test]
fn close_agent_requires_inline_confirmation() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    open_agent_menu(&mut state, 0, (0, 0));
    select_command(&mut state, MenuCommand::CloseAgent);

    assert_eq!(activate_menu(&mut state), None);
    match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => {
            assert_eq!(
                menu.commands.get(menu.selected),
                Some(&MenuCommand::CloseAgent)
            );
            assert!(menu.confirming);
        }
        _ => panic!("first activation must keep the menu open"),
    }
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .session_id
            .as_deref(),
        Some("session-1"),
        "第一次触发不得关闭会话"
    );

    assert_eq!(activate_menu(&mut state), None);
    assert!(state.overlay.is_none());
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.chat.session_id,
        None
    );
}

#[test]
fn changing_menu_selection_resets_confirmation() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    open_agent_menu(&mut state, 0, (0, 0));
    select_command(&mut state, MenuCommand::CloseAgent);
    assert_eq!(activate_menu(&mut state), None);

    move_menu_selection(&mut state, 1);

    match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => {
            assert_eq!(menu.selected, 0);
            assert!(!menu.confirming, "键盘切换选中项必须复位确认态");
        }
        _ => panic!("expected menu overlay"),
    }

    select_command(&mut state, MenuCommand::CloseAgent);
    assert_eq!(activate_menu(&mut state), None);
    assert!(set_menu_selection(&mut state, 0));
    match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => {
            assert_eq!(menu.selected, 0);
            assert!(!menu.confirming, "指针切换选中项必须复位确认态");
        }
        _ => panic!("expected menu overlay"),
    }
}

#[test]
fn lx_close_agent_resets_session_view_without_deleting_db_session() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .push(chat_message("msg-1"));
    open_agent_menu(&mut state, 0, (0, 0));
    select_command(&mut state, MenuCommand::CloseAgent);

    activate_menu(&mut state);
    activate_menu(&mut state);

    let target = state.pane_anywhere(pane).expect("pane must be kept");
    assert_eq!(target.lx.chat.session_id, None);
    assert!(target.lx.chat.messages.is_empty());
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::DeleteChatSession { .. })),
        "关闭条目不得删除 DB 会话"
    );
    assert!(state.active_tab().pane(pane).is_some());
}

#[test]
fn terminal_close_agent_removes_only_target_pane() {
    let mut state = AppState::demo();
    let first = state.active_tab().layout.focus();
    let second = split_pane(&mut state, first);
    update_pane_agent(
        &mut state,
        second,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("claude"),
        )),
    );
    let index = agent_entry_index(
        &state,
        second,
        SidebarAgentKind::Terminal(AgentKind::Claude),
    );
    open_agent_menu(&mut state, index, (0, 0));
    select_command(&mut state, MenuCommand::CloseAgent);

    activate_menu(&mut state);
    activate_menu(&mut state);

    let tab = state.active_tab();
    assert!(tab.pane(second).is_none());
    assert!(tab.pane(first).is_some());
}

#[test]
fn close_agent_skips_target_that_disappeared_after_menu_opened() {
    let mut state = AppState::demo();
    let first = state.active_tab().layout.focus();
    let second = split_pane(&mut state, first);
    set_lx_session(&mut state, first, "session-1");
    set_lx_session(&mut state, second, "session-2");
    let index = agent_entry_index(&state, second, lx_kind());
    open_agent_menu(&mut state, index, (0, 0));

    assert!(new_session(&mut state, second));
    assert!(
        !state
            .agent_entries()
            .iter()
            .any(|entry| entry.pane_id == second),
        "目标条目应已消失"
    );
    select_command(&mut state, MenuCommand::CloseAgent);

    activate_menu(&mut state);
    activate_menu(&mut state);

    assert!(state.pane_anywhere(second).is_some());
    assert!(state.pane_anywhere(first).is_some());
    assert_eq!(
        state
            .pane_anywhere(first)
            .expect("pane")
            .lx
            .chat
            .session_id
            .as_deref(),
        Some("session-1"),
        "不得误关其他 Agent"
    );
}

#[test]
fn agent_title_rename_sets_override_and_closes_overlay() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_title = Some("Auto title".to_string());
    open_agent_menu(&mut state, 0, (0, 0));
    activate_menu(&mut state);
    rename_input(&mut state, "Custom title");

    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.agent_title_override.as_deref(),
        Some("Custom title")
    );
    assert_eq!(target.agent_display_title(lx_kind()), Some("Custom title"));
    assert_eq!(
        target.lx.chat.session_title.as_deref(),
        Some("Auto title"),
        "标题覆盖不得修改会话真实标题"
    );
}

#[test]
fn agent_title_empty_rename_clears_override_and_closes_overlay() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_title = Some("Auto title".to_string());
        target.set_agent_title_override(lx_kind(), Some("Manual title".to_string()));
    }
    open_agent_menu(&mut state, 0, (0, 0));
    activate_menu(&mut state);
    rename_input(&mut state, "");

    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.agent_title_override, None);
    assert_eq!(target.agent_display_title(lx_kind()), Some("Auto title"));
}

#[test]
fn workspace_empty_rename_keeps_overlay_open() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new(""),
    }));

    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(
        matches!(state.overlay, Some(Overlay::Rename(_))),
        "其他重命名目标空输入时保持浮层打开"
    );
}

#[test]
fn close_agent_requires_confirmation_but_edit_title_does_not() {
    assert!(MenuCommand::CloseAgent.requires_confirm());
    assert!(!MenuCommand::EditAgentTitle.requires_confirm());
}
