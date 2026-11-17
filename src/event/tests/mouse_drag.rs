use super::test_support::*;
use super::*;

#[test]
fn mouse_drag_resizes_sidebar() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let boundary = geo.view.left_panel.right() - 1;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_sidebar, "press on boundary starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary + 8, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.sidebar_width, boundary + 9);

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), boundary + 8, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!state.resizing_sidebar);
}

#[test]
fn mouse_drag_resizes_prompt() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let boundary = geo.view.right_panel.x;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_prompt, "press on boundary starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary - 5, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_width, geo.view.right_panel.width + 5);
}

#[test]
fn swapped_divider_drag_resizes_prompt_from_left() {
    let config = Config::default();
    let mut state = demo_terminal();
    update::swap_prompt_side(&mut state, 96, 38);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    assert!(geo.view.right_panel_left);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 交换后分隔线两列为 prompt 右边框与窗格左边框；取 prompt 侧一列。
    let boundary = geo.view.right_panel.right() - 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_prompt, "press on left divider starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary + 5, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_width, geo.view.right_panel.width + 5);
}

#[test]
fn mouse_move_over_swap_button_is_not_divider_hover() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let button = ui::swap_button(&geo.view).expect("swap button visible");

    handle_terminal_event(
        mouse(MouseEventKind::Moved, button.x, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        !state.prompt_hover,
        "swap button is a control, not a divider"
    );

    handle_terminal_event(
        mouse(MouseEventKind::Moved, geo.view.right_panel.x, button.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_hover, "divider rows still highlight on hover");
}

#[test]
fn mouse_drag_reorders_workspaces() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::create_workspace(&mut state);
    crate::app::update::create_workspace(&mut state);
    let names: Vec<String> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.clone())
        .collect();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections = crate::ui::layout::sidebar_sections(geo.view.left_panel, false)
        .expect("sidebar sections are visible");
    let list_y = sections.workspaces.y;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 2, list_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_drag, Some(0));

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), 2, list_y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_drag, Some(2));

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), 2, list_y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.workspace_drag.is_none());
    let order: Vec<&str> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(
        order,
        vec![names[1].as_str(), names[2].as_str(), names[0].as_str()]
    );
    assert_eq!(state.active_workspace, 2);
}

#[test]
fn mouse_drag_resizes_pane_boundary() {
    let config = Config::default();
    let mut state = demo_terminal();
    let left = state.active_tab().layout.focus();
    let right = state
        .active_tab_mut()
        .split_pane(left, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let before = pane_rects_of(&state, &geo, &config);
    let left_before = rect_of(&before, left);
    let boundary = rect_of(&before, right).x;
    let row = left_before.y + 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.resizing_pane,
        Some(tiling::BoundaryHit {
            first: left,
            second: right,
            direction: ratatui::layout::Direction::Horizontal,
        })
    );

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary + 4, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let after = pane_rects_of(&state, &geo, &config);
    assert_eq!(rect_of(&after, left).width, left_before.width + 4);

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), boundary + 4, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_pane.is_none());
}

#[test]
fn mouse_drag_resizes_vertical_pane_boundary() {
    let config = Config::default();
    let mut state = demo_terminal();
    let top = state.active_tab().layout.focus();
    let bottom = state
        .active_tab_mut()
        .split_pane(top, ratatui::layout::Direction::Vertical)
        .expect("split succeeds");
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let before = pane_rects_of(&state, &geo, &config);
    let top_before = rect_of(&before, top);
    let boundary = rect_of(&before, bottom).y;
    let column = top_before.x + 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, boundary),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.resizing_pane,
        Some(tiling::BoundaryHit {
            first: top,
            second: bottom,
            direction: ratatui::layout::Direction::Vertical,
        })
    );

    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            column,
            boundary + 4,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let after = pane_rects_of(&state, &geo, &config);
    assert_eq!(rect_of(&after, top).height, top_before.height + 4);

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), column, boundary + 4),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_pane.is_none());
}

#[test]
fn mouse_down_on_collapsed_pane_boundary_does_not_start_resize() {
    let config = Config::default();
    let mut state = demo_terminal();
    let left = state.active_tab().layout.focus();
    let right = state
        .active_tab_mut()
        .split_pane(left, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    assert!(state.active_tab_mut().layout.set_collapsed(right, true));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let rects = pane_rects_of(&state, &geo, &config);
    let boundary = rect_of(&rects, right).x;
    let row = rect_of(&rects, left).y + 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_pane.is_none());
}

#[test]
fn mouse_reporting_pane_does_not_steal_boundary_drag() {
    let config = Config::default();
    let mut state = demo_terminal();
    let left = state.active_tab().layout.focus();
    let right = state
        .active_tab_mut()
        .split_pane(left, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    {
        let target = state.pane_mut_anywhere(left).expect("terminal pane");
        target
            .terminal
            .feed(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
    }
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let rects = pane_rects_of(&state, &geo, &config);
    let boundary = rect_of(&rects, right).x;
    let row = rect_of(&rects, left).y + 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_pane.is_some());
}

/// 拖动工作区条目期间滚轮只滚视口；落点按滚动后的指针行立即重排。
#[test]
fn wheel_during_workspace_drag_scrolls_list_and_moves_drop_target() {
    let config = Config::default();
    let mut state = demo_terminal();
    for _ in 0..20 {
        crate::app::update::create_workspace(&mut state);
    }
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections = crate::ui::layout::sidebar_sections(geo.view.left_panel, false)
        .expect("sidebar sections are visible");
    let list_y = sections.workspaces.y;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 2, list_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_drag, Some(0));

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, 2, list_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(
        state.workspace_scroll,
        usize::try_from(crate::app::update::WHEEL_LINES).expect("wheel lines"),
        "拖动中滚轮滚动工作区列表视口"
    );
    assert_eq!(
        state.workspace_drag,
        Some(usize::try_from(crate::app::update::WHEEL_LINES).expect("wheel lines")),
        "被拖条目跟随滚动后的指针落点"
    );
    assert!(state.workspace_dragging, "重排后保持拖拽态");
}

/// 指针离开列表后滚轮被拖拽现场吞掉：不改视口，也不清拖拽。
#[test]
fn wheel_outside_workspace_list_keeps_drag_and_scroll() {
    let config = Config::default();
    let mut state = demo_terminal();
    for _ in 0..20 {
        crate::app::update::create_workspace(&mut state);
    }
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections = crate::ui::layout::sidebar_sections(geo.view.left_panel, false)
        .expect("sidebar sections are visible");
    let list_y = sections.workspaces.y;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 2, list_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_drag, Some(0));

    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            geo.view.main_content.x + 2,
            geo.view.main_content.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.workspace_scroll, 0, "列表外滚轮不改列表视口");
    assert_eq!(state.workspace_drag, Some(0), "拖拽现场保留");
    assert!(!state.workspace_dragging);
}

/// 工作区活跃列表：指针停在底部检测区，定时器在没有鼠标移动时持续滚动。
#[test]
fn workspace_drag_autoscroll_runs_from_bottom_band() {
    let config = Config::default();
    let mut state = demo_terminal();
    for _ in 0..20 {
        crate::app::update::create_workspace(&mut state);
    }
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let list = ui::workspace_list_rect(&geo.view, &state).expect("workspace list");
    state.workspace_drag = Some(0);
    state.workspace_dragging = true;
    let now = std::time::Instant::now();

    crate::event::mouse::autoscroll::arm_drag(
        &mut state,
        crate::app::state::DragScrollSurface::WorkspaceActive,
        &geo.view,
        (list.x + 1, list.bottom() - 1),
        now,
    );
    assert!(state.drag_autoscroll.is_some(), "底部检测区已登记");
    assert!(crate::event::mouse::autoscroll::tick_drag(
        &mut state,
        &geo.view,
        now + std::time::Duration::from_millis(31)
    ));
    assert!(state.workspace_scroll > 0, "定时器推进列表视口");
    assert!(state.workspace_drag.is_some(), "拖拽现场保留");
}

/// 工作区历史列表：底部检测区滚动后落点按新视口重算。
#[test]
fn history_drag_autoscroll_runs_from_bottom_band() {
    let config = Config::default();
    let mut state = demo_terminal();
    let entries = (0..20)
        .map(|index| crate::app::state::WorkspaceHistoryEntry {
            name: format!("history {index}"),
            path: std::env::temp_dir().join(format!("lx-autoscroll-history-{index}")),
            exists: true,
            name_is_manual: true,
            position: index,
            trusted: true,
            git: None,
        })
        .collect();
    crate::app::update::apply_workspace_history(&mut state, entries);
    state.workspace_history.mode = crate::app::state::WorkspaceMode::History;
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let list = ui::workspace_history_list_rect(&geo.view, &state).expect("history list");
    state.workspace_history.drag = Some(0);
    state.workspace_history.dragging = true;
    let now = std::time::Instant::now();

    crate::event::mouse::autoscroll::arm_drag(
        &mut state,
        crate::app::state::DragScrollSurface::WorkspaceHistory,
        &geo.view,
        (list.x + 1, list.bottom() - 1),
        now,
    );
    assert!(crate::event::mouse::autoscroll::tick_drag(
        &mut state,
        &geo.view,
        now + std::time::Duration::from_millis(31)
    ));
    assert!(state.workspace_history.scroll > 0);
    assert_ne!(
        state.workspace_history.drag,
        Some(0),
        "落点跟随滚动后的指针行"
    );
}

/// Todo：底部检测区滚动视口并刷新合并目标。
#[test]
fn todo_drag_autoscroll_runs_from_bottom_band() {
    let config = Config::default();
    let mut state = demo_terminal();
    let mut records: Vec<crate::db::todo::TodoRecord> = (0..60)
        .map(|index| {
            crate::db::todo::TodoRecord::new(
                format!("todo {index}"),
                crate::db::todo::TodoStatus::Todo,
                index as i64,
            )
        })
        .collect();
    state.todo.replace(records.clone());
    assert!(crate::app::update::select_right_tab(
        &mut state,
        crate::app::state::RightTab::Todo
    ));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let list = crate::ui::right_panel::todo::effective_list_rect(geo.view.right_panel, &state);
    state.todo.set_viewport_rows(usize::from(list.height));
    let first = records.remove(0);
    state.todo_drag = Some(crate::app::state::TodoDrag {
        source_id: first.id,
        from_display: 0,
        pressed_at: std::time::Instant::now(),
        dragging: true,
        target_index: None,
    });
    let now = std::time::Instant::now();

    crate::event::mouse::autoscroll::arm_drag(
        &mut state,
        crate::app::state::DragScrollSurface::Todo,
        &geo.view,
        (list.x + 1, list.bottom() - 1),
        now,
    );
    assert!(crate::event::mouse::autoscroll::tick_drag(
        &mut state,
        &geo.view,
        now + std::time::Duration::from_millis(31)
    ));
    assert!(state.todo.scroll() > 0, "定时器推进 Todo 视口");
    assert!(
        state
            .todo_drag
            .as_ref()
            .is_some_and(|drag| drag.target_index.is_some()),
        "落点按滚动后的指针行重算"
    );
}

/// Prompt 树：底部检测区滚动视口。
#[test]
fn prompt_drag_autoscroll_runs_from_bottom_band() {
    use crate::app::prompt::PromptItem;

    let config = Config::default();
    let mut state = demo_terminal();
    let items: Vec<PromptItem> = (0..30)
        .map(|index| PromptItem::single(100 + index, None, format!("item {index}"), "body"))
        .collect();
    state.active_workspace_mut().prompt_panel.tree = crate::app::prompt::PromptTree::new();
    state
        .active_workspace_mut()
        .prompt_panel
        .tree
        .replace(Vec::new(), items);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area =
        crate::ui::right_panel::prompt::frame::prompt_main_rect(geo.view.right_panel, &state);
    state.prompt_drag = Some(crate::app::state::PromptDrag {
        workspace: 0,
        source: Some(crate::app::state::PromptDragSource::Item(100)),
        from_row: 0,
        pressed_at: std::time::Instant::now(),
        dragging: true,
        drop_mark: crate::app::state::PromptDropMark::None,
        enabled: true,
    });
    let now = std::time::Instant::now();

    crate::event::mouse::autoscroll::arm_drag(
        &mut state,
        crate::app::state::DragScrollSurface::PromptTree,
        &geo.view,
        (area.x + 1, area.bottom().saturating_sub(1)),
        now,
    );
    assert!(crate::event::mouse::autoscroll::tick_drag(
        &mut state,
        &geo.view,
        now + std::time::Duration::from_millis(31)
    ));
    assert!(
        state.prompt_owner().prompt_panel.scroll > 0,
        "定时器推进 Prompt 树视口"
    );
}
