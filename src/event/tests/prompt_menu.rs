use super::test_support::*;
use super::*;

#[test]
fn mouse_click_folders_button_opens_referenced_menu() {
    let mut state = demo_terminal();
    state.active_workspace_mut().cwd = Some(std::path::PathBuf::from("/tmp/lx-event-ref"));
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let folders = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Folders,
        false,
    )
    .expect("folders button visible");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            folders.x,
            folders.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(matches!(
        state.overlay,
        Some(Overlay::ReferencedFolderMenu(_))
    ));
    assert!(dirty);
}

#[test]
fn folders_button_no_op_without_project_key() {
    let mut state = demo_terminal();
    state.active_workspace_mut().cwd = None;
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let folders = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Folders,
        false,
    )
    .expect("folders button visible");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            folders.x,
            folders.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.overlay.is_none(), "无项目键不打开菜单");
}

#[test]
fn referenced_search_ctrl_or_cmd_z_returns_to_parent_scope() {
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
        let mut state = referenced_search_entered_state();
        let (config, geo) = toolbar_geometry(&mut state);
        let mut sessions = HashMap::new();
        let mut dirty = false;
        handle_terminal_event(
            TerminalEvent::Key(KeyEvent::new(KeyCode::Char('z'), modifiers)),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
            panic!("search stays open: {modifiers:?}");
        };
        assert_eq!(
            search.root,
            std::path::PathBuf::from("/tmp/lx-ref-z/root"),
            "回退到父搜索范围: {modifiers:?}"
        );
        assert!(search.stack.is_empty(), "{modifiers:?}");
        assert!(search.request.is_some(), "回退后登记新扫描: {modifiers:?}");
        assert!(dirty, "{modifiers:?}");
    }
}

#[test]
fn referenced_search_ctrl_z_at_root_is_noop() {
    let mut state = referenced_search_entered_state();
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.stack.clear();
    }
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search stays open");
    };
    assert!(
        search.root.ends_with("docs"),
        "初始根不再回退: {:?}",
        search.root
    );
}

#[test]
fn mouse_click_referenced_menu_row_selects_then_double_click_opens_search() {
    let mut state = referenced_menu_state();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let row = referenced_menu_layout(&state, &geo).rows[1];
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), row.x, row.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    // 单击只选中（与 prompt 列表一致）。
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderMenu(_))),
        "单击不打开内容弹窗"
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu open");
    };
    assert_eq!(menu.selected, 1, "单击选中该行");
    // 双击同一行才打开内容弹窗。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), row.x, row.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("双击打开搜索");
    };
    assert_eq!(search.folder, 2, "双击行对应的记录被打开");
    assert!(dirty);
}

#[test]
fn mouse_click_referenced_menu_add_row_opens_input() {
    let mut state = referenced_menu_state();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let rows = referenced_menu_layout(&state, &geo).rows;
    let add = rows.last().copied().expect("add row");
    // 动作项单击即打开路径输入（与草稿保存选择器一致）。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), add.x, add.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderInput(_))),
        "新建行打开路径输入"
    );
    assert!(dirty);
}

#[test]
fn mouse_click_referenced_menu_blank_keeps_open_and_outside_closes() {
    let mut state = referenced_menu_state();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let layout = referenced_menu_layout(&state, &geo);
    let rows_bottom = layout.rows.last().expect("rows").y;
    let blank = Rect::new(layout.area.x + 2, rows_bottom + 2, 1, 1);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), blank.x, blank.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderMenu(_))),
        "弹窗内空白保持打开"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.overlay.is_none(), "弹窗之外关闭");
}

#[test]
fn referenced_menu_ctrl_c_copies_and_ctrl_d_or_del_removes() {
    for code in [KeyCode::Char('d'), KeyCode::Delete] {
        let mut state = referenced_menu_state();
        let (config, geo) = toolbar_geometry(&mut state);
        let mut sessions = HashMap::new();
        let mut dirty = false;
        handle_terminal_event(
            TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::CONTROL)),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
        assert!(
            matches!(state.overlay, Some(Overlay::ReferencedFolderDelete(_))),
            "移除键打开删除确认: {code:?}"
        );
    }
    let mut state = referenced_menu_state();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderMenu(_))),
        "复制不关弹窗"
    );
    assert!(matches!(
        state.effects.as_slice(),
        [crate::app::effects::Effect::CopyToClipboard { text, .. }]
            if text.contains("lx-ref-menu/alpha")
    ));
}

#[test]
fn mouse_hover_over_referenced_menu_backgrounds_row_without_moving_highlight() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 悬停第 4 个可见行：只记录悬停行（铺悬停底色），不动 `❯` 选中标记，不动显式视口。
    let target = layout.rows[3];
    handle_terminal_event(
        mouse(MouseEventKind::Moved, target.x + 1, target.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.hover, Some(3), "悬停记录悬停行");
    assert_eq!(menu.selected, 0, "悬停不移动 `❯` 选中标记");
    assert_eq!(menu.viewport, None, "悬停不动显式视口");
    assert!(dirty);
}

#[test]
fn mouse_hover_over_referenced_menu_after_wheel_keeps_viewport() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 先滚轮滚开视口。
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            layout.rows[0].x + 1,
            layout.rows[0].y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    // 再悬停窗口内某行：只记录悬停行，显式视口保持不变（窗口不跳）。
    let layout = referenced_menu_layout(&state, &geo);
    let target = layout.rows[2];
    let row_index = layout.start + 2;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, target.x + 1, target.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.hover, Some(row_index), "悬停按可见行命中");
    assert_eq!(menu.selected, 0, "悬停不移动 `❯` 选中标记");
    assert_eq!(menu.viewport, Some(3), "悬停不改变已滚开的视口");
}

#[test]
fn mouse_hover_over_referenced_menu_paints_background_without_marker() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let target = layout.rows[3];
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, target.x + 1, target.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        geo.screen.width,
        geo.screen.height,
    ))
    .expect("backend");
    terminal
        .draw(|frame| {
            crate::ui::render(frame, &state, &config);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    // 悬停行：左侧无 `❯` 标记、整行铺悬停底色。
    assert_eq!(
        buffer[(target.x, target.y)].symbol(),
        " ",
        "悬停行左侧无 ❯ 标记"
    );
    assert_eq!(
        buffer[(target.x, target.y)].bg,
        ratatui::style::Color::Indexed(240),
        "悬停行铺悬停底色"
    );
    // 选中行（未悬停）只有 `❯` 标记、不铺底色。
    let selected = layout.rows[0];
    assert_eq!(
        buffer[(selected.x, selected.y)].symbol(),
        "❯",
        "选中行保留 ❯ 标记"
    );
    assert_eq!(
        buffer[(selected.x, selected.y)].bg,
        ratatui::style::Color::Reset,
        "选中行不铺底色"
    );
}

#[test]
fn mouse_hover_after_opening_referenced_menu_backgrounds_row_without_marker() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let mut state = demo_terminal();
    state.active_workspace_mut().cwd = Some(std::path::PathBuf::from("/tmp/lx-event-hover"));
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 通过工具栏 [dir] 按钮真实打开弹窗。
    let folders = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Folders,
        false,
    )
    .expect("folders button visible");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            folders.x,
            folders.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    if let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_mut() {
        menu.rows = (0..8)
            .map(|id| ReferencedFolderMenuRow::Folder {
                id,
                path: std::path::PathBuf::from(format!("/tmp/lx-event-hover/f{id}")),
            })
            .chain(std::iter::once(ReferencedFolderMenuRow::Add))
            .collect();
        menu.selected = 0;
        menu.hover = None;
        menu.viewport = None;
    }
    let layout = referenced_menu_layout(&state, &geo);
    let target = layout.rows[3];
    handle_terminal_event(
        mouse(MouseEventKind::Moved, target.x + 1, target.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        geo.screen.width,
        geo.screen.height,
    ))
    .expect("backend");
    terminal
        .draw(|frame| {
            crate::ui::render(frame, &state, &config);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    // 悬停后：悬停行只铺底色、无 `❯`；选中行（第 0 行）保留 `❯` 且无底色。
    assert_eq!(
        buffer[(target.x, target.y)].symbol(),
        " ",
        "悬停行左侧无 ❯ 标记"
    );
    assert_eq!(
        buffer[(target.x, target.y)].bg,
        ratatui::style::Color::Indexed(240),
        "悬停行铺悬停底色"
    );
    assert_eq!(
        buffer[(layout.rows[0].x, layout.rows[0].y)].symbol(),
        "❯",
        "选中行保留 ❯ 标记"
    );
    assert_eq!(
        buffer[(layout.rows[0].x, layout.rows[0].y)].bg,
        ratatui::style::Color::Reset,
        "选中行不铺底色"
    );
}

#[test]
fn mouse_wheel_over_referenced_menu_scrolls_viewport_not_highlight() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    assert!(layout.scrollbar.is_some(), "记录溢出时出现滚动条");
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let prompt_scroll = state.prompt.scroll();

    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            layout.rows[0].x + 1,
            layout.rows[0].y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.viewport, Some(3), "滚轮下移视口 3 项");
    assert_eq!(menu.selected, 0, "滚轮不动高亮");
    assert_eq!(
        state.prompt.scroll(),
        prompt_scroll,
        "弹窗内滚轮不滚动 prompt"
    );
    assert!(dirty);
}

#[test]
fn mouse_click_referenced_menu_scrollbar_track_jumps_viewport() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let bar = layout.scrollbar.expect("list overflows");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert!(
        menu.viewport.is_some_and(|offset| offset > 0),
        "轨道点击跳转视口"
    );
    assert_eq!(menu.selected, 0, "滚动条不动高亮");
    assert!(
        state.referenced_menu_scroll_drag.is_none(),
        "非 thumb 不进入拖拽"
    );
    assert!(dirty);
}

#[test]
fn mouse_drag_referenced_menu_scrollbar_thumb_scrolls_viewport() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let bar = layout.scrollbar.expect("list overflows");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.thumb.x,
            bar.thumb.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.referenced_menu_scroll_drag,
        Some(0),
        "按住 thumb 进入拖拽"
    );

    let max = ui::widgets::scrollbar::offset_from_drag_row(&bar, bar.track.bottom() - 1, 0);
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.viewport, Some(max), "拖拽视口到末窗口");
    assert_eq!(menu.selected, 0, "拖拽不动高亮");

    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.referenced_menu_scroll_drag.is_none(), "松开清除拖拽");
}
