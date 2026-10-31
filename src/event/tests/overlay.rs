use super::test_support::*;
use super::*;

#[test]
fn mouse_right_click_tab_opens_tab_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::create_tab(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let (_, first) = bar.tabs[0];
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), first.x, first.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("tab menu overlay expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::Tab {
            workspace: 0,
            tab: 0
        }
    );
    assert!(dirty);
}

#[test]
fn mouse_right_click_workspace_item_opens_workspace_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let (column, row) = (sections.workspaces.x, sections.workspaces.y);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("workspace menu overlay expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::Workspace(0)
    );
    assert_eq!(menu.anchor, (column, row));
    assert!(dirty);
}

#[test]
fn mouse_right_click_workspace_list_blank_opens_new_workspace_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let (column, row) = (sections.workspaces.x, sections.workspaces.y + 1);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("workspace list menu overlay expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::WorkspaceList
    );
    assert_eq!(
        menu.commands,
        vec![crate::app::overlay::MenuCommand::NewWorkspace]
    );
    assert_eq!(menu.anchor, (column, row));
    assert!(dirty);
}

#[test]
fn mouse_right_click_workspace_blank_retargets_open_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.overlay.is_some(), "窗格菜单先打开");

    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let (column, row) = (sections.workspaces.x, sections.workspaces.y + 1);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("workspace list menu overlay expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::WorkspaceList,
        "菜单改挂到工作区列表空白目标"
    );
    assert_eq!(menu.anchor, (column, row));
}

#[test]
fn mouse_right_click_agents_blank_does_not_open_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let (column, row) = (sections.agents.x, sections.agents.y + 1);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.overlay.is_none());
    assert!(!dirty);
}

#[test]
fn mouse_right_click_pane_opens_pane_menu_at_pointer() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let (column, row) = (inner.x + 1, inner.y + 1);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("pane menu overlay expected");
    };
    assert_eq!(menu.anchor, (column, row));
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::Pane {
            workspace: 0,
            tab: 0,
            pane
        }
    );
    assert!(dirty);
}

#[test]
fn mouse_right_click_prompt_does_not_open_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let inner = layout::pane_inner_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.overlay.is_none());
    assert!(!dirty);
}

#[test]
fn mouse_right_click_other_pane_retargets_open_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let first = state.active_tab().layout.focus();
    let second = state
        .active_tab_mut()
        .split_pane(first, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for pane in [first, second] {
        let inner = pane_inner(&geo, pane);
        handle_terminal_event(
            mouse(
                MouseEventKind::Down(MouseButton::Right),
                inner.x + 1,
                inner.y + 1,
            ),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("pane menu overlay expected");
        };
        assert_eq!(
            menu.target,
            crate::app::overlay::OverlayTarget::Pane {
                workspace: 0,
                tab: 0,
                pane
            }
        );
    }
}

#[test]
fn mouse_right_click_prompt_closes_open_pane_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.overlay.is_some());

    let prompt_inner = layout::pane_inner_rect(geo.view.prompt);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            prompt_inner.x + 1,
            prompt_inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.overlay.is_none());
}

#[test]
fn mouse_click_pane_menu_split_item_creates_pane() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("pane menu overlay expected");
    };
    let menu_layout = ui::overlay::menu_layout(&state, geo.screen, menu);
    let item = menu_layout.item_rects[0];
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), item.x + 1, item.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.overlay.is_none());
    assert_eq!(state.active_tab().layout.pane_ids().len(), 2);
}

#[test]
fn switch_cwd_menu_item_first_click_arms_confirm_second_click_returns_cd_command() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.workspaces[0].cwd = Some(PathBuf::from("/tmp/target"));
    let pane = state.active_tab().layout.focus();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    crate::app::update::open_pane_menu(&mut state, pane, (10, 5));

    let index = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("pane menu expected");
        };
        menu.commands
            .iter()
            .position(|cmd| *cmd == crate::app::overlay::MenuCommand::SwitchToWorkspaceCwd)
            .expect("switch to ws path offered")
    };
    let item = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("pane menu expected");
        };
        ui::overlay::menu_layout(&state, geo.screen, menu).item_rects[index]
    };

    // 第一次点击：不关闭菜单、不返回命令，切换为行内待确认态。
    let first = handle_overlay_click(&mut state, geo.screen, item.x + 1, item.y);
    assert_eq!(first, None);
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu stays open for inline confirmation");
    };
    assert!(menu.confirming);
    assert_eq!(menu.selected, index);

    // 第二次点击同一项：关闭菜单并返回写入窗格的 cd 命令。
    let item2 = ui::overlay::menu_layout(&state, geo.screen, menu).item_rects[index];
    let second = handle_overlay_click(&mut state, geo.screen, item2.x + 1, item2.y);
    assert_eq!(second, Some((pane, b"cd \"/tmp/target\"\n".to_vec())));
    assert!(state.overlay.is_none());
}

#[test]
fn switch_cwd_menu_item_click_outside_cancels_without_command() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.workspaces[0].cwd = Some(PathBuf::from("/tmp/target"));
    let pane = state.active_tab().layout.focus();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    crate::app::update::open_pane_menu(&mut state, pane, (10, 5));

    let index = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("pane menu expected");
        };
        menu.commands
            .iter()
            .position(|cmd| *cmd == crate::app::overlay::MenuCommand::SwitchToWorkspaceCwd)
            .expect("switch to ws path offered")
    };
    let item = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("pane menu expected");
        };
        ui::overlay::menu_layout(&state, geo.screen, menu).item_rects[index]
    };

    assert_eq!(
        handle_overlay_click(&mut state, geo.screen, item.x + 1, item.y),
        None
    );
    assert!(state.overlay.is_some());

    // 点击菜单外空白关闭菜单，不产生 cd 命令。
    let action = handle_overlay_click(&mut state, geo.screen, 0, 0);
    assert_eq!(action, None);
    assert!(state.overlay.is_none());
}

#[test]
fn mouse_click_close_tab_menu_item_twice_closes_tab_inline() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::create_tab(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    crate::app::update::open_tab_menu(&mut state, 0, (10, 5));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let close_rect = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("tab menu open");
        };
        let layout = ui::overlay::menu_layout(&state, geo.screen, menu);
        *layout.item_rects.last().expect("close item")
    };

    // 第一次点击：进入行内待确认态，标签尚未关闭。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            close_rect.x + 1,
            close_rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.active_workspace().tabs.len(), 2);
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu remains open in confirming state");
    };
    assert!(menu.confirming);

    // 第二次点击：真正关闭标签并收起菜单。
    let close_rect2 = *ui::overlay::menu_layout(&state, geo.screen, menu)
        .item_rects
        .last()
        .expect("confirm close item");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            close_rect2.x + 1,
            close_rect2.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.overlay.is_none());
    assert_eq!(state.active_workspace().tabs.len(), 1);
    assert!(dirty);
}

#[test]
fn hover_in_widened_confirm_region_clears_hover_when_menu_shrinks_back() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.workspaces[0].cwd = Some(PathBuf::from("/tmp/target"));
    let pane = state.active_tab().layout.focus();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    crate::app::update::open_pane_menu(&mut state, pane, (10, 5));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let (initial_right, switch_idx) = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("pane menu open");
        };
        let initial = ui::overlay::menu_layout(&state, geo.screen, menu);
        let switch_idx = menu
            .commands
            .iter()
            .position(|c| *c == crate::app::overlay::MenuCommand::SwitchToWorkspaceCwd)
            .expect("switch command");
        (initial.area.right(), switch_idx)
    };
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = switch_idx;
        menu.confirming = true;
    }
    state.menu_hover = true;
    let (wide_right, row0_y) = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("pane menu open");
        };
        let wide = ui::overlay::menu_layout(&state, geo.screen, menu);
        (wide.item_rects[0].right(), wide.item_rects[0].y)
    };
    assert!(
        wide_right > initial_right,
        "确认态使菜单宽度向右拓宽: initial_right={initial_right}, wide_right={wide_right}"
    );

    // 鼠标移到第 0 行的拓宽列区（在确认态拓宽框内，但在常规宽度框外）。
    let col_in_expanded_zone = initial_right;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, col_in_expanded_zone, row0_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu remains open");
    };
    assert_ne!(menu.selected, switch_idx);
    assert!(!menu.confirming, "切换菜单项后复位 confirming");
    assert!(
        !state.menu_hover,
        "菜单缩回后鼠标位于新菜单框外，不得残留虚假的 menu_hover 高亮"
    );
}

#[test]
fn first_click_on_confirm_item_does_not_yank_sidebar_scroll_to_active_workspace() {
    let config = Config::default();
    let mut state = demo_terminal();
    for _ in 0..15 {
        crate::app::update::create_workspace(&mut state);
    }
    crate::app::update::switch_workspace(&mut state, 0);
    // 手工把侧栏滚到下方（查看非当前激活的工作区 #15），并打开 #15 的右键菜单。
    state.workspace_scroll = 10;
    crate::app::update::open_workspace_menu(&mut state, 15, (5, 10));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 14));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let close_rect = {
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("workspace menu open");
        };
        *ui::overlay::menu_layout(&state, geo.screen, menu)
            .item_rects
            .last()
            .expect("close item")
    };

    // 第一次点击 Close：仅进入菜单行内确认态，不得把侧栏滚动条强行拉回 active_workspace(0)。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            close_rect.x + 1,
            close_rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.workspace_scroll, 10,
        "首次点击进入行内确认态时不得重置侧栏滚动位置"
    );
}

#[test]
fn mouse_leaving_overlay_menu_clears_hover_feedback() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    crate::app::update::open_tab_menu(&mut state, 0, (10, 4));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu open");
    };
    let layout = ui::overlay::menu_layout(&state, geo.screen, menu);
    let item = layout.item_rects[1];

    // 悬停某行：高亮跟随。
    handle_terminal_event(
        mouse(MouseEventKind::Moved, item.x + 1, item.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.menu_hover);
    assert_eq!(
        state.overlay.as_ref().and_then(|overlay| match overlay {
            Overlay::Menu(menu) => Some(menu.selected),
            _ => None,
        }),
        Some(1)
    );

    // 移出容器：清除悬停高亮反馈（选中项保持不变）。
    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.screen.x,
            geo.screen.bottom().saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!state.menu_hover, "离开容器移除悬停效果");
    assert!(dirty);
}

#[test]
fn mouse_leaving_page_menu_clears_hover_feedback() {
    let config = Config::default();
    let mut state = demo_terminal();
    assert!(crate::app::update::open_temp_prompt(&mut state));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    assert!(crate::app::update::open_prompt_page_menu(
        &mut state,
        (10, 4)
    ));
    assert!(state.menu_hover, "页码菜单打开时指针位于容器内");

    // 移出容器：清除悬停高亮反馈。
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.screen.x,
            geo.screen.bottom().saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!state.menu_hover, "离开容器移除悬停效果");

    // 回到边框按钮：恢复悬停并命中对应行。
    let Some(Overlay::PageMenu(menu)) = state.overlay.as_ref() else {
        panic!("page menu open");
    };
    let layout2 = ui::overlay::page_menu_layout(&state, geo.screen, menu);
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            layout2.new_rect.x,
            layout2.new_rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.menu_hover);
    assert_eq!(
        state.overlay.as_ref().and_then(|overlay| match overlay {
            Overlay::PageMenu(menu) => Some(menu.selected),
            _ => None,
        }),
        Some(0),
        "悬停 [add] 按钮"
    );
}

#[test]
fn rename_overlay_inside_click_keeps_open_and_outside_dismiss() {
    let screen = Rect::new(0, 0, 80, 24);
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Rename(crate::app::overlay::Rename {
        target: crate::app::overlay::RenameTarget::Workspace(0),
        input: crate::app::overlay::TextInput::new("name"),
    }));
    let shell = ui::overlay::rename_shell(screen).expect("shell");
    // 点击内部内容（输入行空白）：保持打开。
    handle_overlay_click(&mut state, screen, shell.inner.x + 1, shell.inner.y);
    assert!(state.overlay.is_some(), "点击内部内容不关闭");
    // 点击浮层之外：关闭。
    handle_overlay_click(&mut state, screen, 0, 0);
    assert!(state.overlay.is_none(), "点击浮层之外关闭");
}

#[test]
fn trust_overlay_inside_click_keeps_open_and_outside_refuses() {
    let screen = Rect::new(0, 0, 80, 24);
    let mut state = AppState::demo();
    let untrusted = PathBuf::from("/tmp/untrusted-close-test");
    state.overlay = Some(Overlay::TrustWorkspace(crate::app::trust::TrustWorkspace {
        path: untrusted.clone(),
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace { cwd: untrusted },
    }));
    let shell = ui::overlay::trust_shell(screen).expect("shell");
    // 点击问题文本（内部内容）：保持打开且不授予信任。
    handle_overlay_click(&mut state, screen, shell.inner.x + 1, shell.inner.y);
    assert!(
        matches!(state.overlay, Some(Overlay::TrustWorkspace(_))),
        "内部点击不关闭"
    );
    assert!(state.effects.is_empty(), "未登记信任效应");
    // 点击浮层之外：拒绝信任并关闭。
    handle_overlay_click(&mut state, screen, 0, 0);
    assert!(state.overlay.is_none(), "浮层之外点击关闭");
    assert!(state.effects.is_empty(), "拒绝不登记信任效应");
}

#[test]
fn toast_click_inside_or_outside_dismisses() {
    let mut state = demo_terminal();
    let show = |state: &mut AppState| {
        update::show_toast(
            state,
            Toast::new(ToastKind::Default, "toast body", None, Instant::now()),
        );
    };
    show(&mut state);
    let (config, geo) = toolbar_geometry(&mut state);
    let area =
        ui::toast::rect(&state, &geo.view, &geo.rects, geo.screen, &config).expect("toast visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点击 toast 本身：关闭。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            area.x + 1,
            area.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.toast.is_none(), "点击 toast 关闭");

    // 点击 toast 之外：关闭 toast 并继续处理点击。
    show(&mut state);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.toast.is_none(), "点击 toast 之外关闭");
}

#[test]
fn clicking_toast_while_modal_open_dismisses_toast_without_closing_modal() {
    let mut state = demo_terminal();
    update::open_new_workspace_dialog(&mut state);
    update::show_toast(
        &mut state,
        Toast::new(
            ToastKind::Danger,
            ui::text::TOAST_DIRECTORY_NOT_FOUND,
            None,
            Instant::now(),
        ),
    );
    let (config, geo) = toolbar_geometry(&mut state);
    let toast_area =
        ui::toast::rect(&state, &geo.view, &geo.rects, geo.screen, &config).expect("toast visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            toast_area.x + 2,
            toast_area.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.toast.is_none(), "点击全局报错 toast 应关闭 toast");
    assert!(
        matches!(state.overlay, Some(Overlay::NewWorkspace(_))),
        "点击 toast 本身不得误关底层正在编辑的模态弹窗"
    );
}
