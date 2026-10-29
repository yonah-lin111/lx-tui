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
fn confirm_switch_cwd_click_confirm_returns_cd_command() {
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: PathBuf::from("/tmp/target"),
    }));
    let (button, geo) = confirm_switch_button(&mut state, "/tmp/target", true);

    let action = handle_overlay_click(&mut state, geo.screen, button.x, button.y);

    assert_eq!(action, Some((pane, b"cd \"/tmp/target\"\n".to_vec())));
    assert!(state.overlay.is_none());
}

#[test]
fn confirm_switch_cwd_click_cancel_closes_without_command() {
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: PathBuf::from("/tmp/target"),
    }));
    let (button, geo) = confirm_switch_button(&mut state, "/tmp/target", false);

    let action = handle_overlay_click(&mut state, geo.screen, button.x, button.y);

    assert_eq!(action, None);
    assert!(state.overlay.is_none());
}

#[test]
fn mouse_click_confirm_switch_cwd_closes_overlay() {
    let config = Config::default();
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: PathBuf::from("/tmp/target"),
    }));
    let (button, geo) = confirm_switch_button(&mut state, "/tmp/target", true);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.overlay.is_none());
    assert!(dirty);
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
fn rename_overlay_inside_click_keeps_open_close_button_and_outside_dismiss() {
    let screen = Rect::new(0, 0, 80, 24);
    let mut state = AppState::demo();
    let rename = || {
        Some(Overlay::Rename(crate::app::overlay::Rename {
            target: crate::app::overlay::RenameTarget::Workspace(0),
            input: crate::app::overlay::TextInput::new("name"),
        }))
    };
    state.overlay = rename();
    let shell = ui::overlay::rename_shell(screen).expect("shell");
    // 点击内部内容（输入行空白）：保持打开。
    handle_overlay_click(&mut state, screen, shell.inner.x + 1, shell.inner.y);
    assert!(state.overlay.is_some(), "点击内部内容不关闭");
    // 右上角 [✕]：关闭。
    let close = ui::widgets::modal::close_button_rect(shell.area).expect("close button");
    handle_overlay_click(&mut state, screen, close.x, close.y);
    assert!(state.overlay.is_none(), "[✕] 关闭");
    // 点击浮层之外：关闭。
    state.overlay = rename();
    handle_overlay_click(&mut state, screen, 0, 0);
    assert!(state.overlay.is_none(), "点击浮层之外关闭");
}

#[test]
fn trust_overlay_inside_click_keeps_open_and_close_button_refuses() {
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
    // 右上角 [✕]：拒绝信任并关闭。
    let close = ui::widgets::modal::close_button_rect(shell.area).expect("close button");
    handle_overlay_click(&mut state, screen, close.x, close.y);
    assert!(state.overlay.is_none(), "[✕] 关闭");
    assert!(state.effects.is_empty(), "拒绝不登记信任效应");
}

#[test]
fn toast_inside_click_keeps_close_button_and_outside_dismiss() {
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

    // 点击 toast 内部内容：保持显示。
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
    assert!(state.toast.is_some(), "点击 toast 内容不关闭");

    // 点击右上角 [✕]：关闭。
    let close = ui::widgets::modal::close_button_rect(area).expect("close button");
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), close.x, close.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.toast.is_none(), "[✕] 关闭 toast");

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
