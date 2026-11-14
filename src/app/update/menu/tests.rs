//! Agents 条目右键菜单、行内确认、关闭分派、标题弹窗与 Open prompt 测试。

use super::*;

use crate::app::actions::OverlayKey;
use crate::app::prompt::{PromptItem, PromptPage, PromptPageId, PromptTree};
use crate::app::state::{AgentPromptRef, AgentTitleOverride, AiWebSite, ChatMessageView, LxEntry};
use crate::app::update::{
    apply_overlay_key, clear_agent_title_input, new_session, update_pane_agent,
};
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

fn custom(text: &str) -> AgentTitleOverride {
    AgentTitleOverride::custom(text)
}

fn bound(text: &str, item_id: i64, page_id: i64) -> AgentTitleOverride {
    AgentTitleOverride {
        text: text.to_string(),
        prompt: Some(AgentPromptRef {
            item_id,
            page_id: PromptPageId::Saved(page_id),
        }),
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

fn set_override(
    state: &mut AppState,
    pane: PaneId,
    kind: SidebarAgentKind,
    title: AgentTitleOverride,
) {
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .set_agent_title_override(kind, Some(title));
}

fn seed_recent_page(
    state: &mut AppState,
    item_id: i64,
    prompt_name: &str,
    page_id: i64,
    page_title: &str,
    content: &str,
) {
    let page = PromptPage::saved_at(page_id, page_title, content, 100);
    let item = PromptItem {
        id: item_id,
        folder_id: None,
        name: prompt_name.to_string(),
        active_page: page.id,
        pages: vec![page],
    };
    let panel = &mut state.active_workspace_mut().prompt_panel;
    panel.tree = PromptTree::new();
    panel.tree.replace(Vec::new(), vec![item]);
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

fn open_edit_title(state: &mut AppState, index: usize) {
    open_agent_menu(state, index, (0, 0));
    select_command(state, MenuCommand::EditAgentTitle);
    assert_eq!(activate_menu(state), None);
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
fn lx_agent_menu_offers_edit_and_close_without_binding() {
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
fn bound_title_adds_open_prompt_after_edit_title() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), bound("Auth Flow", 7, 9));

    open_agent_menu(&mut state, 0, (0, 0));

    assert_eq!(
        menu_commands(&state),
        vec![
            MenuCommand::EditAgentTitle,
            MenuCommand::OpenPrompt,
            MenuCommand::UnbindAgentPrompt,
            MenuCommand::CloseAgent
        ]
    );
}

#[test]
fn terminal_bound_title_adds_open_prompt_and_opens_page() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    update_pane_agent(
        &mut state,
        pane,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("claude"),
        )),
    );
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(
        &mut state,
        pane,
        SidebarAgentKind::Terminal(AgentKind::Claude),
        bound("Auth Flow", 7, 9),
    );
    let index = agent_entry_index(&state, pane, SidebarAgentKind::Terminal(AgentKind::Claude));

    open_agent_menu(&mut state, index, (0, 0));
    assert_eq!(
        menu_commands(&state),
        vec![
            MenuCommand::EditAgentTitle,
            MenuCommand::OpenPrompt,
            MenuCommand::UnbindAgentPrompt
        ],
        "唯一窗格的终端 Agent 不提供 Close，但绑定后提供 Open prompt 与解绑"
    );
    select_command(&mut state, MenuCommand::OpenPrompt);

    assert_eq!(activate_menu(&mut state), None);

    let panel = &state.prompt_owner().prompt_panel;
    assert_eq!(
        panel.editing,
        Some(crate::app::prompt::PromptEditing::Item(7))
    );
    assert_eq!(
        panel.tree.item(7).map(|item| item.active_page),
        Some(PromptPageId::Saved(9))
    );
}

#[test]
fn custom_title_does_not_add_open_prompt() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), custom("My title"));

    open_agent_menu(&mut state, 0, (0, 0));

    assert_eq!(
        menu_commands(&state),
        vec![MenuCommand::EditAgentTitle, MenuCommand::CloseAgent]
    );
}

#[test]
fn stale_binding_does_not_add_open_prompt() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), bound("Auth Flow", 7, 9));
    state.active_workspace_mut().prompt_panel.tree = PromptTree::new();

    open_agent_menu(&mut state, 0, (0, 0));

    assert_eq!(
        menu_commands(&state),
        vec![
            MenuCommand::EditAgentTitle,
            MenuCommand::UnbindAgentPrompt,
            MenuCommand::CloseAgent
        ],
        "绑定页已删除时不再提供 Open prompt，但保留解绑入口"
    );
}

#[test]
fn edit_agent_title_opens_dialog_with_custom_title_and_recent_rows() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), custom("Manual title"));

    open_edit_title(&mut state, 0);

    match state.overlay.as_ref() {
        Some(Overlay::AgentTitle(dialog)) => {
            assert_eq!(
                (dialog.workspace, dialog.tab, dialog.pane, dialog.kind),
                (0, 0, pane, lx_kind())
            );
            assert_eq!(dialog.input.text(), "Manual title");
            assert_eq!(dialog.binding, None);
            assert_eq!(dialog.selected, None);
            assert_eq!(dialog.rows.len(), 1);
            assert_eq!(dialog.rows[0].page_title, "Auth Flow");
            assert_eq!(dialog.rows[0].prompt_name, "Backend Prompt");
        }
        _ => panic!("expected agent title dialog"),
    }
}

#[test]
fn edit_agent_title_prefills_bound_title_and_selects_recent_row() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), bound("Auth Flow", 7, 9));

    open_edit_title(&mut state, 0);

    match state.overlay.as_ref() {
        Some(Overlay::AgentTitle(dialog)) => {
            assert_eq!(dialog.input.text(), "Auth Flow");
            assert_eq!(
                dialog.binding,
                Some(AgentPromptRef {
                    item_id: 7,
                    page_id: PromptPageId::Saved(9)
                })
            );
            assert_eq!(dialog.selected, Some(0));
        }
        _ => panic!("expected agent title dialog"),
    }
}

#[test]
fn down_selects_recent_row_and_fills_title() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), custom("Manual title"));
    open_edit_title(&mut state, 0);

    apply_overlay_key(&mut state, OverlayKey::Down);

    match state.overlay.as_ref() {
        Some(Overlay::AgentTitle(dialog)) => {
            assert_eq!(dialog.input.text(), "Auth Flow");
            assert_eq!(dialog.selected, Some(0));
            assert_eq!(
                dialog.binding,
                Some(AgentPromptRef {
                    item_id: 7,
                    page_id: PromptPageId::Saved(9)
                })
            );
        }
        _ => panic!("expected agent title dialog"),
    }
}

#[test]
fn typing_after_recent_selection_clears_binding() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), custom("Manual title"));
    open_edit_title(&mut state, 0);

    apply_overlay_key(&mut state, OverlayKey::Down);

    apply_overlay_key(&mut state, OverlayKey::Char('!'));

    match state.overlay.as_ref() {
        Some(Overlay::AgentTitle(dialog)) => {
            assert_eq!(dialog.input.text(), "Auth Flow!");
            assert_eq!(dialog.binding, None, "直接编辑输入框转为自定义标题");
            assert_eq!(dialog.selected, None);
        }
        _ => panic!("expected agent title dialog"),
    }
}

#[test]
fn enter_commits_custom_title_and_closes_dialog() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_title = Some("Auto title".to_string());
    open_edit_title(&mut state, 0);
    if let Some(Overlay::AgentTitle(dialog)) = state.overlay.as_mut() {
        dialog.input = TextInput::new("Custom title");
    }

    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    let target = state.pane_anywhere(pane).expect("pane");
    let title = target
        .agent_title_override(lx_kind())
        .expect("title override");
    assert_eq!(title.text, "Custom title");
    assert_eq!(title.prompt, None);
    assert_eq!(target.agent_display_title(lx_kind()), Some("Custom title"));
    assert_eq!(
        target.lx.chat.session_title.as_deref(),
        Some("Auto title"),
        "标题覆盖不得修改会话真实标题"
    );
}

#[test]
fn enter_commits_recent_binding_and_enables_open_prompt() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    open_edit_title(&mut state, 0);
    apply_overlay_key(&mut state, OverlayKey::Down);

    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    let title = state
        .pane_anywhere(pane)
        .and_then(|target| target.agent_title_override(lx_kind()))
        .expect("title override");
    assert_eq!(title.text, "Auth Flow");
    assert_eq!(
        title.prompt,
        Some(AgentPromptRef {
            item_id: 7,
            page_id: PromptPageId::Saved(9)
        })
    );

    open_agent_menu(&mut state, 0, (0, 0));
    assert_eq!(
        menu_commands(&state),
        vec![
            MenuCommand::EditAgentTitle,
            MenuCommand::OpenPrompt,
            MenuCommand::UnbindAgentPrompt,
            MenuCommand::CloseAgent
        ]
    );
}

#[test]
fn unbind_recent_clears_title_and_binding() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), bound("Auth Flow", 7, 9));
    open_agent_menu(&mut state, 0, (0, 0));
    select_command(&mut state, MenuCommand::UnbindAgentPrompt);

    assert_eq!(activate_menu(&mut state), None);
    match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => {
            assert_eq!(
                menu.commands.get(menu.selected),
                Some(&MenuCommand::UnbindAgentPrompt)
            );
            assert!(menu.confirming, "第一次触发只进入行内确认态");
        }
        _ => panic!("first activation must keep the menu open"),
    }
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.agent_title_override(lx_kind()))
            .is_some(),
        "第一次触发不得解绑"
    );

    assert_eq!(activate_menu(&mut state), None);

    assert!(state.overlay.is_none());
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.agent_title_override(lx_kind()),
        None,
        "解绑必须同时清空标题覆盖"
    );
    assert_eq!(target.agent_display_title(lx_kind()), None);

    open_agent_menu(&mut state, 0, (0, 0));
    assert_eq!(
        menu_commands(&state),
        vec![MenuCommand::EditAgentTitle, MenuCommand::CloseAgent],
        "解绑后不再提供 Open prompt / Unbind recent"
    );
}

#[test]
fn empty_title_clears_override_without_falling_back_to_session_title() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_title = Some("Auto title".to_string());
        target.set_agent_title_override(lx_kind(), Some(custom("Manual title")));
    }
    open_edit_title(&mut state, 0);
    assert!(clear_agent_title_input(&mut state));

    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.agent_title_override, None);
    assert_eq!(target.agent_display_title(lx_kind()), None);
}

#[test]
fn open_prompt_opens_bound_page_in_prompt_panel() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), bound("Auth Flow", 7, 9));
    open_agent_menu(&mut state, 0, (0, 0));
    select_command(&mut state, MenuCommand::OpenPrompt);

    assert_eq!(activate_menu(&mut state), None);

    assert!(state.overlay.is_none());
    let panel = &state.prompt_owner().prompt_panel;
    assert_eq!(
        panel.editing,
        Some(crate::app::prompt::PromptEditing::Item(7))
    );
    assert_eq!(
        panel.tree.item(7).map(|item| item.active_page),
        Some(PromptPageId::Saved(9))
    );
    assert_eq!(state.prompt.text(), "Auth content");
    assert_eq!(state.focused, crate::app::state::FocusedContainer::Prompt);
    assert_eq!(state.right_tab, crate::app::state::RightTab::Prompt);
}

#[test]
fn open_prompt_is_noop_when_page_disappeared_after_menu_opened() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    set_lx_session(&mut state, pane, "session-1");
    seed_recent_page(
        &mut state,
        7,
        "Backend Prompt",
        9,
        "Auth Flow",
        "Auth content",
    );
    set_override(&mut state, pane, lx_kind(), bound("Auth Flow", 7, 9));
    open_agent_menu(&mut state, 0, (0, 0));
    select_command(&mut state, MenuCommand::OpenPrompt);
    state.active_workspace_mut().prompt_panel.tree = PromptTree::new();

    assert_eq!(activate_menu(&mut state), None);

    assert!(state.overlay.is_none());
    let panel = &state.prompt_owner().prompt_panel;
    assert_ne!(
        panel.editing,
        Some(crate::app::prompt::PromptEditing::Item(7)),
        "目标页已删除时不得切换编辑目标"
    );
    assert_ne!(state.prompt.text(), "Auth content");
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
    assert!(MenuCommand::UnbindAgentPrompt.requires_confirm());
    assert!(!MenuCommand::EditAgentTitle.requires_confirm());
    assert!(!MenuCommand::OpenPrompt.requires_confirm());
}
