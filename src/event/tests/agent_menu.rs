use super::test_support::*;
use super::*;

use crate::app::overlay::{MenuCommand, OverlayTarget};
use crate::app::prompt::{PromptEditing, PromptItem, PromptPage, PromptPageId, PromptTree};
use crate::app::state::{
    AgentPromptRef, AgentTitleOverride, FocusedContainer, RightTab, SidebarAgentEntry,
    SidebarAgentKind,
};
use crate::detect::{AgentKind, AgentState};
use crate::event::session::Geometry;
use crate::tiling::PaneId;

/// 同一窗格同时有 lx 会话与终端 Agent 的场景；条目顺序固定为 `[lx, terminal]`。
fn dual_agent_state() -> (AppState, Config, Geometry, PaneId) {
    let mut state = demo_terminal();
    let pane = mount_agent(&mut state, AgentKind::Claude, AgentState::Working);
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = Some("ses-agent-menu".to_string());
    }
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 100, 24));
    (state, config, geo, pane)
}

/// 侧栏 Agents 分区矩形。
fn agents_section(geo: &Geometry, state: &AppState) -> Rect {
    ui::layout::sidebar_sections(geo.view.left_panel, state.agents_collapsed)
        .expect("sections are visible")
        .agents
}

/// 投递一次鼠标事件并返回是否置脏。
fn send_mouse(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> bool {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(kind, column, row),
        state,
        &mut sessions,
        geo,
        config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    dirty
}

/// 断言当前菜单精确指向给定 Agent 条目，命令与锚点符合预期。
fn assert_agent_menu(
    state: &AppState,
    entry: SidebarAgentEntry,
    anchor: (u16, u16),
    commands: &[MenuCommand],
) {
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("agent menu overlay expected");
    };
    assert_eq!(
        menu.target,
        OverlayTarget::Agent {
            workspace: 0,
            tab: entry.tab_index,
            pane: entry.pane_id,
            kind: entry.kind,
        },
        "右键 Agents 条目必须命中 Agent 目标，不得误开工作区/Tab/Pane 菜单"
    );
    assert_eq!(menu.anchor, anchor);
    assert_eq!(menu.commands.as_slice(), commands);
    assert!(!menu.confirming, "菜单打开时不处于确认态");
}

#[test]
fn right_click_agent_first_row_opens_agent_menu() {
    let (mut state, config, geo, pane) = dual_agent_state();
    let agents = agents_section(&geo, &state);
    let expected = state.agent_entries()[0];
    assert_eq!(expected.pane_id, pane);
    assert!(matches!(expected.kind, SidebarAgentKind::Lx { .. }));
    let anchor = (agents.x + 1, agents.y);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );

    assert!(dirty);
    assert_agent_menu(
        &state,
        expected,
        anchor,
        &[MenuCommand::EditAgentTitle, MenuCommand::CloseAgent],
    );
}

#[test]
fn right_click_agent_second_row_opens_same_agent_menu() {
    let (mut state, config, geo, _pane) = dual_agent_state();
    let agents = agents_section(&geo, &state);
    let expected = state.agent_entries()[0];
    let anchor = (agents.x + 1, agents.y + 1);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );

    assert!(dirty);
    assert_agent_menu(
        &state,
        expected,
        anchor,
        &[MenuCommand::EditAgentTitle, MenuCommand::CloseAgent],
    );
}

#[test]
fn right_click_terminal_agent_opens_agent_menu_not_pane_menu() {
    let (mut state, config, _geo, pane) = dual_agent_state();
    // 拆出第二个窗格，使终端 Agent 条目可以关闭，同时验证目标仍精确落在 Agent 上。
    state
        .active_tab_mut()
        .split_pane(pane, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    let geo = geometry(&state, &config, Rect::new(0, 0, 100, 24));
    let agents = agents_section(&geo, &state);
    let expected = state.agent_entries()[1];
    assert_eq!(expected.pane_id, pane);
    assert!(matches!(
        expected.kind,
        SidebarAgentKind::Terminal(AgentKind::Claude)
    ));
    let anchor = (agents.x + 1, agents.y + 2);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );

    assert!(dirty);
    assert_agent_menu(
        &state,
        expected,
        anchor,
        &[MenuCommand::EditAgentTitle, MenuCommand::CloseAgent],
    );
    assert_eq!(state.active_workspace, 0);
    assert_eq!(state.active_workspace().active_tab, 0);
}

#[test]
fn terminal_agent_in_sole_pane_offers_only_edit_title() {
    let mut state = demo_terminal();
    let pane = mount_agent(&mut state, AgentKind::Codex, AgentState::Idle);
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 100, 24));
    let agents = agents_section(&geo, &state);
    let expected = state.agent_entries()[0];
    assert_eq!(state.active_tab().layout.pane_ids(), vec![pane]);
    let anchor = (agents.x + 1, agents.y);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );

    assert!(dirty);
    assert_agent_menu(&state, expected, anchor, &[MenuCommand::EditAgentTitle]);
}

#[test]
fn clicking_each_agent_row_focuses_pane_and_switches_view() {
    let (mut state, config, geo, pane) = dual_agent_state();
    let agents = agents_section(&geo, &state);
    let (column, top) = (agents.x + 1, agents.y);

    fn set_view(state: &mut AppState, pane: PaneId, view: PaneView) {
        if let Some(target) = state.active_tab_mut().pane_mut(pane) {
            target.view = view;
        }
    }

    // 条目 0 第一行：切到 lx 视图。
    assert_eq!(view_of(&state, pane), PaneView::Terminal);
    send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Left),
        column,
        top,
    );
    assert_eq!(view_of(&state, pane), PaneView::Lx);
    assert_eq!(state.active_tab().layout.focus(), pane);

    // 条目 0 第二行（标题行）：同一条目同样切到 lx 视图。
    set_view(&mut state, pane, PaneView::Terminal);
    send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Left),
        column,
        top + 1,
    );
    assert_eq!(view_of(&state, pane), PaneView::Lx);

    // 条目 1 第一行与第二行：都切到终端视图。
    for offset in [2, 3] {
        set_view(&mut state, pane, PaneView::Lx);
        send_mouse(
            &mut state,
            &geo,
            &config,
            MouseEventKind::Down(MouseButton::Left),
            column,
            top + offset,
        );
        assert_eq!(view_of(&state, pane), PaneView::Terminal);
        assert_eq!(state.active_tab().layout.focus(), pane);
    }
    assert!(!state.prompt_focused());
}

#[test]
fn wheel_scroll_then_hover_and_right_click_use_new_row_offset() {
    let mut state = demo_terminal();
    create_agent_tabs(&mut state, 20);
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 100, 24));
    let agents = agents_section(&geo, &state);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::ScrollDown,
        agents.x,
        agents.y,
    );
    assert!(dirty);
    assert_eq!(state.agent_scroll, 3);
    assert_eq!(state.agent_hover, Some(3), "滚动后按新偏移重算悬停");

    // 第二可见条目的第二行：新偏移下应命中索引 4。
    let row = agents.y + 3;
    send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Moved,
        agents.x + 1,
        row,
    );
    assert_eq!(state.agent_hover, Some(4));

    // 同一坐标右键也应命中索引 4 的条目。
    let expected = state.agent_entries()[4];
    let anchor = (agents.x + 1, row);
    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );
    assert!(dirty);
    assert_agent_menu(&state, expected, anchor, &[MenuCommand::EditAgentTitle]);
}

// ===== Agents 标题弹窗与 Open prompt（鼠标路径） =====

/// 已入库页种子：`(page_id, page_title, content, updated_at)`；Recent 按时间倒序。
type SeededPage = (i64, &'static str, &'static str, i64);

/// 带 lx 会话、Recent 页与几何的标题弹窗测试场景。
struct TitleFixture {
    state: AppState,
    config: Config,
    geo: Geometry,
    pane: PaneId,
    index: usize,
    kind: SidebarAgentKind,
}

fn seed_recent_pages(state: &mut AppState, pages: &[SeededPage]) {
    let pages: Vec<PromptPage> = pages
        .iter()
        .map(|(id, title, content, updated_at)| {
            PromptPage::saved_at(*id, *title, *content, *updated_at)
        })
        .collect();
    let item = PromptItem {
        id: 7,
        folder_id: None,
        name: "Backend Prompt".to_string(),
        active_page: pages[0].id,
        pages,
    };
    let panel = &mut state.active_workspace_mut().prompt_panel;
    panel.tree = PromptTree::new();
    panel.tree.replace(Vec::new(), vec![item]);
}

fn title_fixture(pages: &[SeededPage]) -> TitleFixture {
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("ses-agent-title".to_string());
    seed_recent_pages(&mut state, pages);
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let index = state
        .agent_entries()
        .iter()
        .position(|entry| {
            entry.pane_id == pane && matches!(entry.kind, SidebarAgentKind::Lx { .. })
        })
        .expect("lx agent entry");
    let kind = state.agent_entries()[index].kind;
    TitleFixture {
        state,
        config,
        geo,
        pane,
        index,
        kind,
    }
}

fn set_override(fixture: &mut TitleFixture, title: AgentTitleOverride) {
    fixture
        .state
        .pane_mut_anywhere(fixture.pane)
        .expect("pane")
        .set_agent_title_override(fixture.kind, Some(title));
}

fn bound_title(text: &str, page_id: i64) -> AgentTitleOverride {
    AgentTitleOverride {
        text: text.to_string(),
        prompt: Some(AgentPromptRef {
            item_id: 7,
            page_id: PromptPageId::Saved(page_id),
        }),
    }
}

/// 在真实 Agents 分区坐标右键目标条目；返回锚点。
fn right_click_agent(fixture: &mut TitleFixture) -> (u16, u16) {
    let agents = agents_section(&fixture.geo, &fixture.state);
    let anchor = (agents.x + 1, agents.y + fixture.index as u16 * 2);
    let dirty = send_mouse(
        &mut fixture.state,
        &fixture.geo,
        &fixture.config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );
    assert!(dirty);
    anchor
}

fn menu_commands(state: &AppState) -> Vec<MenuCommand> {
    match state.overlay.as_ref() {
        Some(Overlay::Menu(menu)) => menu.commands.clone(),
        _ => panic!("expected menu overlay"),
    }
}

/// 用菜单真实几何坐标左键点击指定命令。
fn click_menu_command(fixture: &mut TitleFixture, command: MenuCommand) {
    let item = {
        let Some(Overlay::Menu(menu)) = fixture.state.overlay.as_ref() else {
            panic!("expected menu overlay");
        };
        let layout = ui::overlay::menu_layout(&fixture.state, fixture.geo.screen, menu);
        let index = menu
            .commands
            .iter()
            .position(|candidate| *candidate == command)
            .expect("menu command offered");
        layout.item_rects[index]
    };
    let dirty = send_mouse(
        &mut fixture.state,
        &fixture.geo,
        &fixture.config,
        MouseEventKind::Down(MouseButton::Left),
        item.x + 1,
        item.y,
    );
    assert!(dirty);
}

fn open_title_dialog(fixture: &mut TitleFixture) {
    right_click_agent(fixture);
    click_menu_command(fixture, MenuCommand::EditAgentTitle);
    assert!(
        matches!(fixture.state.overlay, Some(Overlay::AgentTitle(_))),
        "Edit title 激活后必须打开标题弹窗"
    );
}

fn title_shell(fixture: &TitleFixture) -> ui::widgets::modal::ModalShell {
    let Some(Overlay::AgentTitle(dialog)) = fixture.state.overlay.as_ref() else {
        panic!("expected agent title dialog");
    };
    ui::overlay::agent_title_shell(fixture.geo.screen, dialog.rows.len()).expect("title shell")
}

/// 扫描弹窗内容区，取命中指定 Recent 行的真实屏幕坐标。
fn row_point(fixture: &TitleFixture, index: usize) -> (u16, u16) {
    let shell = title_shell(fixture);
    let Some(Overlay::AgentTitle(dialog)) = fixture.state.overlay.as_ref() else {
        panic!("expected agent title dialog");
    };
    for row in shell.inner.y..shell.inner.bottom() {
        for column in shell.inner.x..shell.inner.right() {
            if ui::overlay::agent_title_row_at(&shell, dialog, column, row) == Some(index) {
                return (column, row);
            }
        }
    }
    panic!("recent row {index} not hit");
}

/// 扫描弹窗按钮行，取指定按钮的真实屏幕坐标。
fn button_point(fixture: &TitleFixture, button: ui::overlay::AgentTitleButton) -> (u16, u16) {
    let shell = title_shell(fixture);
    for row in shell.inner.y..shell.inner.bottom() {
        for column in shell.inner.x..shell.inner.right() {
            if ui::overlay::agent_title_button_at(&shell, column, row) == Some(button) {
                return (column, row);
            }
        }
    }
    panic!("title button {button:?} not hit");
}

fn click_at(fixture: &mut TitleFixture, point: (u16, u16)) -> bool {
    send_mouse(
        &mut fixture.state,
        &fixture.geo,
        &fixture.config,
        MouseEventKind::Down(MouseButton::Left),
        point.0,
        point.1,
    )
}

fn dialog(fixture: &TitleFixture) -> &crate::app::overlay::AgentTitleDialog {
    match fixture.state.overlay.as_ref() {
        Some(Overlay::AgentTitle(dialog)) => dialog,
        _ => panic!("expected agent title dialog"),
    }
}

#[test]
fn right_click_edit_title_opens_dialog_with_prefilled_title_and_recent_rows() {
    let mut fixture = title_fixture(&[(9, "Auth Flow", "Auth content", 100)]);
    set_override(&mut fixture, AgentTitleOverride::custom("Manual title"));

    open_title_dialog(&mut fixture);

    let dialog = dialog(&fixture);
    assert_eq!(
        (dialog.workspace, dialog.tab, dialog.pane, dialog.kind),
        (0, 0, fixture.pane, fixture.kind)
    );
    assert_eq!(dialog.input.text(), "Manual title");
    assert_eq!(dialog.binding, None);
    assert_eq!(dialog.selected, None);
    assert_eq!(dialog.rows.len(), 1);
    assert_eq!(dialog.rows[0].item_id, 7);
    assert_eq!(dialog.rows[0].page_title, "Auth Flow");
    assert_eq!(dialog.rows[0].prompt_name, "Backend Prompt");
}

#[test]
fn clicking_recent_rows_fills_title_and_binds_page() {
    // 三条页保证弹窗高度可容纳两行可见 Recent 条目。
    let mut fixture = title_fixture(&[
        (9, "Auth Flow", "Auth content", 100),
        (10, "Deploy Plan", "Deploy content", 200),
        (11, "Hotfix", "Hotfix content", 50),
    ]);
    open_title_dialog(&mut fixture);

    let first = row_point(&fixture, 0);
    assert!(click_at(&mut fixture, first));
    {
        let dialog = dialog(&fixture);
        assert_eq!(dialog.input.text(), "Deploy Plan");
        assert_eq!(dialog.selected, Some(0));
        assert_eq!(
            dialog.binding,
            Some(AgentPromptRef {
                item_id: 7,
                page_id: PromptPageId::Saved(10),
            })
        );
    }

    let second = row_point(&fixture, 1);
    assert!(click_at(&mut fixture, second));
    let dialog = dialog(&fixture);
    assert_eq!(dialog.input.text(), "Auth Flow");
    assert_eq!(dialog.selected, Some(1));
    assert_eq!(
        dialog.binding,
        Some(AgentPromptRef {
            item_id: 7,
            page_id: PromptPageId::Saved(9),
        })
    );
    assert!(
        matches!(fixture.state.overlay, Some(Overlay::AgentTitle(_))),
        "点选 Recent 行只回填输入，不关闭弹窗"
    );
}

#[test]
fn save_commits_title_and_menu_offers_open_prompt() {
    let mut fixture = title_fixture(&[
        (9, "Auth Flow", "Auth content", 100),
        (10, "Deploy Plan", "Deploy content", 200),
        (11, "Hotfix", "Hotfix content", 50),
    ]);
    open_title_dialog(&mut fixture);
    let first = row_point(&fixture, 0);
    assert!(click_at(&mut fixture, first));

    let save = button_point(&fixture, ui::overlay::AgentTitleButton::Save);
    assert!(click_at(&mut fixture, save));

    assert!(fixture.state.overlay.is_none());
    let target = fixture.state.pane_anywhere(fixture.pane).expect("pane");
    let title = target
        .agent_title_override(fixture.kind)
        .expect("committed title");
    assert_eq!(title.text, "Deploy Plan");
    assert_eq!(
        title.prompt,
        Some(AgentPromptRef {
            item_id: 7,
            page_id: PromptPageId::Saved(10),
        })
    );
    assert_eq!(
        target.agent_display_title(fixture.kind),
        Some("Deploy Plan")
    );

    let anchor = right_click_agent(&mut fixture);
    let entry = fixture.state.agent_entries()[fixture.index];
    assert_agent_menu(
        &fixture.state,
        entry,
        anchor,
        &[
            MenuCommand::EditAgentTitle,
            MenuCommand::OpenPrompt,
            MenuCommand::UnbindAgentPrompt,
            MenuCommand::CloseAgent,
        ],
    );
}

#[test]
fn clear_then_save_removes_title_override_and_binding() {
    let mut fixture = title_fixture(&[
        (9, "Auth Flow", "Auth content", 100),
        (10, "Deploy Plan", "Deploy content", 200),
    ]);
    set_override(&mut fixture, bound_title("Auth Flow", 9));
    open_title_dialog(&mut fixture);
    assert_eq!(dialog(&fixture).input.text(), "Auth Flow");

    let clear = button_point(&fixture, ui::overlay::AgentTitleButton::Clear);
    assert!(click_at(&mut fixture, clear));
    {
        let dialog = dialog(&fixture);
        assert_eq!(dialog.input.text(), "");
        assert_eq!(dialog.binding, None);
        assert_eq!(dialog.selected, None);
    }

    let save = button_point(&fixture, ui::overlay::AgentTitleButton::Save);
    assert!(click_at(&mut fixture, save));

    assert!(fixture.state.overlay.is_none());
    let target = fixture.state.pane_anywhere(fixture.pane).expect("pane");
    assert_eq!(target.agent_title_override(fixture.kind), None);
    assert_eq!(
        target.agent_display_title(fixture.kind),
        None,
        "空标题必须回落到 UI 的 (no title) 展示"
    );
}

#[test]
fn outside_click_dismisses_title_dialog_without_committing() {
    let mut fixture = title_fixture(&[(9, "Auth Flow", "Auth content", 100)]);
    set_override(&mut fixture, AgentTitleOverride::custom("Manual title"));
    open_title_dialog(&mut fixture);

    assert!(click_at(&mut fixture, (0, 0)));

    assert!(fixture.state.overlay.is_none());
    let target = fixture.state.pane_anywhere(fixture.pane).expect("pane");
    let title = target
        .agent_title_override(fixture.kind)
        .expect("title unchanged");
    assert_eq!(title.text, "Manual title");
    assert_eq!(title.prompt, None);
}

#[test]
fn open_prompt_menu_item_opens_bound_page_in_prompt_panel() {
    let mut fixture = title_fixture(&[(9, "Auth Flow", "Auth content", 100)]);
    set_override(&mut fixture, bound_title("Auth Flow", 9));

    right_click_agent(&mut fixture);
    assert_eq!(
        menu_commands(&fixture.state),
        vec![
            MenuCommand::EditAgentTitle,
            MenuCommand::OpenPrompt,
            MenuCommand::UnbindAgentPrompt,
            MenuCommand::CloseAgent,
        ]
    );

    click_menu_command(&mut fixture, MenuCommand::OpenPrompt);

    assert!(fixture.state.overlay.is_none());
    let panel = &fixture.state.prompt_owner().prompt_panel;
    assert_eq!(panel.editing, Some(PromptEditing::Item(7)));
    assert_eq!(
        panel.tree.item(7).map(|item| item.active_page),
        Some(PromptPageId::Saved(9))
    );
    assert_eq!(fixture.state.prompt.text(), "Auth content");
    assert_eq!(fixture.state.right_tab, RightTab::Prompt);
    assert_eq!(fixture.state.focused, FocusedContainer::Prompt);
}

#[test]
fn unbind_recent_menu_item_clears_title_and_binding() {
    let mut fixture = title_fixture(&[(9, "Auth Flow", "Auth content", 100)]);
    set_override(&mut fixture, bound_title("Auth Flow", 9));

    right_click_agent(&mut fixture);
    assert_eq!(
        menu_commands(&fixture.state),
        vec![
            MenuCommand::EditAgentTitle,
            MenuCommand::OpenPrompt,
            MenuCommand::UnbindAgentPrompt,
            MenuCommand::CloseAgent,
        ]
    );

    click_menu_command(&mut fixture, MenuCommand::UnbindAgentPrompt);

    let Some(Overlay::Menu(menu)) = fixture.state.overlay.as_ref() else {
        panic!("first unbind click must keep the menu open");
    };
    assert!(menu.confirming, "第一次点击只进入行内确认态");
    assert!(
        fixture
            .state
            .pane_anywhere(fixture.pane)
            .and_then(|target| target.agent_title_override(fixture.kind))
            .is_some(),
        "第一次点击不得解绑"
    );

    click_menu_command(&mut fixture, MenuCommand::UnbindAgentPrompt);

    assert!(fixture.state.overlay.is_none());
    let target = fixture.state.pane_anywhere(fixture.pane).expect("pane");
    assert_eq!(target.agent_title_override(fixture.kind), None);
    assert_eq!(
        target.agent_display_title(fixture.kind),
        None,
        "解绑后第二行回到 (no title)"
    );

    let entry = fixture.state.agent_entries()[fixture.index];
    let anchor = right_click_agent(&mut fixture);
    assert_agent_menu(
        &fixture.state,
        entry,
        anchor,
        &[MenuCommand::EditAgentTitle, MenuCommand::CloseAgent],
    );
}
