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
    crate::app::update::show_prompt_tree(&mut state);
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

fn draw_lx_for_drag_test(
    state: &mut AppState,
    config: &Config,
    geo: &crate::event::session::Geometry,
    cache: &mut ui::RenderCache,
) {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut terminal =
        Terminal::new(TestBackend::new(geo.screen.width, geo.screen.height)).expect("test backend");
    let rects = geo.rects.clone();
    update::resolve_lx_history(state, &rects, cache);
    update::resolve_task_output_visible(state, &rects);
    update::resolve_task_agents_visible(state, &rects, cache);
    update::resolve_task_btw_visible(state, &rects, cache);
    terminal
        .draw(|frame| ui::render_cached(frame, state, config, cache))
        .expect("draw succeeds");
}

fn push_lx_messages(state: &mut AppState, pane: PaneId, count: usize) {
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    for index in 0..count {
        target
            .lx
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                id: format!("msg-{index}"),
                sequence: index as i64,
                kind: "chat".into(),
                role: if index % 2 == 0 { "user" } else { "assistant" }.into(),
                status: "complete".into(),
                turn_index: (index / 2) as i64,
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Text,
                        content: format!("alpha beta gamma line {index}"),
                        provider_metadata_json: None,
                        started_at_ms: None,
                        ended_at_ms: None,
                        completed_at_ms: None,
                    },
                }],
                model_label: None,
                error_code: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                duration_ms: None,
                reports: Vec::new(),
                attachments: Vec::new(),
            });
    }
}

/// lx 主消息列表：顶边自动滚动保留 Word 选区粒度；在底部向下拖动不死循环。
#[test]
fn lx_history_selection_autoscroll_preserves_word_unit_and_stops_at_bottom() {
    use crate::app::selection::SelectionUnit;

    let config = Config::default();
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    push_lx_messages(&mut state, pane, 24);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 20));
    let mut cache = ui::RenderCache::default();
    draw_lx_for_drag_test(&mut state, &config, &geo, &mut cache);

    let (_, rect) = geo.rects.iter().find(|(id, _)| *id == pane).expect("rect");
    let inner = tiling::pane_inner_rect(*rect);
    let geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("geom")
    };
    let bottom_cell = geom.cell(geom.rect.x + 2, geom.rect.bottom() - 1);
    let (start, end) = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::unit_cell_range(
            inner,
            &target.lx,
            bottom_cell,
            SelectionUnit::Word,
            cache.pane(pane),
        )
    };
    update::begin_lx_selection_range(&mut state, pane, start, end, SelectionUnit::Word);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .follow_bottom
    );

    // 已经贴底时把鼠标放在底边检测区：定时器触发后返回 false 且清空，不能 33Hz 死循环。
    let now = std::time::Instant::now();
    crate::event::mouse::autoscroll::arm_lx_selection(
        &mut state,
        pane,
        &geo,
        &mut cache,
        (geom.rect.x + 2, geom.rect.bottom() - 1),
        now,
    );
    assert!(state.lx_selection_autoscroll.is_some());
    assert!(
        !crate::event::mouse::autoscroll::tick_lx_selection(
            &mut state,
            &geo,
            &mut cache,
            now + std::time::Duration::from_millis(31),
        ),
        "已贴底时向下自动滚动必须立即停止，不能无限空转"
    );
    assert!(state.lx_selection_autoscroll.is_none());

    // 拖到顶边检测区：每次 tick 向上滚动并保持 Word 选区粒度（整词区间扩展，非单格）。
    let initial_scroll = geom.scroll;
    assert!(initial_scroll > 0);
    crate::event::mouse::autoscroll::arm_lx_selection(
        &mut state,
        pane,
        &geo,
        &mut cache,
        (geom.rect.x + 4, geom.rect.y),
        now,
    );
    assert!(crate::event::mouse::autoscroll::tick_lx_selection(
        &mut state,
        &geo,
        &mut cache,
        now + std::time::Duration::from_millis(31),
    ));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert!(!chat.follow_bottom);
    assert_eq!(
        chat.history_scroll,
        initial_scroll - 1,
        "顶边自动滚动向上推进视口"
    );
    let sel = state.lx_selection.expect("selection");
    assert_eq!(sel.unit(), SelectionUnit::Word);
    assert!(
        sel.range()
            .is_some_and(|(a, b)| a.0 < b.0 && sel.cursor().1 > 0),
        "Word 粒度选区应跨词对齐而非退化成单格"
    );
}

/// lx 主消息列表：越出顶边立即按距离跳滚且保留 Line 选区；横向移动不重置 next_at。
#[test]
fn lx_history_selection_autoscroll_outside_keeps_line_unit_and_non_starving_deadline() {
    use crate::app::selection::SelectionUnit;

    let config = Config::default();
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    push_lx_messages(&mut state, pane, 24);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 20));
    let mut cache = ui::RenderCache::default();
    draw_lx_for_drag_test(&mut state, &config, &geo, &mut cache);

    let (_, rect) = geo.rects.iter().find(|(id, _)| *id == pane).expect("rect");
    let inner = tiling::pane_inner_rect(*rect);
    let geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("geom")
    };
    let anchor_cell = geom.cell(geom.rect.x + 1, geom.rect.bottom() - 1);
    let (start, end) = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::unit_cell_range(
            inner,
            &target.lx,
            anchor_cell,
            SelectionUnit::Line,
            cache.pane(pane),
        )
    };
    update::begin_lx_selection_range(&mut state, pane, start, end, SelectionUnit::Line);

    let before = geom.scroll;
    assert!(before >= 6);
    let t0 = std::time::Instant::now();
    crate::event::mouse::autoscroll::arm_lx_selection(
        &mut state,
        pane,
        &geo,
        &mut cache,
        (geom.rect.x + 2, geom.rect.y.saturating_sub(2)),
        t0,
    );
    let after_jump = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .history_scroll;
    assert_eq!(before - after_jump, 6, "越出 2 行立即向上滚 6 行");
    let expected_next = t0 + std::time::Duration::from_millis(30);
    assert_eq!(
        state.lx_selection_autoscroll.expect("armed").next_at,
        expected_next
    );
    let (range_start, _) = state.lx_selection.expect("sel").range().expect("range");
    assert_eq!(range_start.1, 0, "Line 选区首列仍对齐行首");

    // 10ms 后同距离横向移动：既不再次暴跳 6 行，也不推迟 next_at。
    crate::event::mouse::autoscroll::arm_lx_selection(
        &mut state,
        pane,
        &geo,
        &mut cache,
        (geom.rect.x + 8, geom.rect.y.saturating_sub(2)),
        t0 + std::time::Duration::from_millis(10),
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .history_scroll,
        after_jump,
        "同距离横向晃动不得重复跳滚"
    );
    assert_eq!(
        state.lx_selection_autoscroll.expect("still armed").next_at,
        expected_next,
        "横向移动不得饥饿推迟 next_at"
    );
}

/// 后台任务页 Processes 输出：即使未先同步 output_visible，自动滚动前也会先解析可见行数并推进选区。
#[test]
fn lx_task_output_selection_autoscroll_resolves_visible_and_extends() {
    let config = Config::default();
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.task = Some(crate::app::state::TaskPage {
            tab: crate::app::state::TaskTab::Processes,
            output: (0..40)
                .map(|index| format!("output line {index}"))
                .collect(),
            output_visible: 0,
            output_scroll: 20,
            ..Default::default()
        });
    }
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let mut cache = ui::RenderCache::default();
    let (_, rect) = geo.rects.iter().find(|(id, _)| *id == pane).expect("rect");
    let inner = tiling::pane_inner_rect(*rect);
    let geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("geom")
    };
    update::begin_lx_selection(&mut state, pane, 22, 0);
    update::drag_lx_selection(&mut state, pane, 22, 4);

    let now = std::time::Instant::now();
    crate::event::mouse::autoscroll::arm_lx_selection(
        &mut state,
        pane,
        &geo,
        &mut cache,
        (geom.rect.x + 2, geom.rect.y),
        now,
    );
    assert!(crate::event::mouse::autoscroll::tick_lx_selection(
        &mut state,
        &geo,
        &mut cache,
        now + std::time::Duration::from_millis(31),
    ));
    let page = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .expect("task page");
    assert!(page.output_visible > 0, "滚动前已同步 output_visible");
    assert_eq!(page.output_scroll, 19);
    assert_eq!(state.lx_selection.expect("sel").cursor().0, 19);
}

/// 后台任务页 Agents 详情与 BTW 对话：rows_dirty / visible == 0 时自动滚动会先解析行数再正常滚动。
#[test]
fn lx_task_agents_and_btw_selection_autoscroll_resolve_dirty_rows() {
    use crate::app::state::{SubagentRun, TaskPage, TaskTab};
    use crate::chat::agent::AgentRole;

    let config = Config::default();
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();

    let mut run = SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: "agent-1".into(),
        nickname: None,
        agent_path: "/root/agent-1".into(),
        child_session_id: "child-1".into(),
        parent_session_id: "parent-1".into(),
        spawn_message_id: None,
        role: AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "completed".into(),
        title: "scan".into(),
        created_at_ms: 0,
        completed_at_ms: Some(100),
        report_json: None,
        report_delivered: false,
    });
    run.loaded = true;
    for index in 0..24 {
        run.chat.messages.push(crate::app::state::ChatMessageView {
            id: format!("sub-{index}"),
            sequence: index as i64,
            kind: "chat".into(),
            role: "assistant".into(),
            status: "complete".into(),
            turn_index: 0,
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: crate::app::state::ChatPartView {
                    kind: crate::chat::events::PartKind::Text,
                    content: format!("subagent line {index}"),
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                },
            }],
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
            reports: Vec::new(),
            attachments: Vec::new(),
        });
    }
    run.rows_dirty = true;
    run.total_rows = 0;
    run.visible = 0;

    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.agents.runs.push(run);
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Agents,
            ..Default::default()
        });
    }

    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let mut cache = ui::RenderCache::default();
    let (_, rect) = geo.rects.iter().find(|(id, _)| *id == pane).expect("rect");
    let inner = tiling::pane_inner_rect(*rect);
    let geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("geom")
    };
    let anchor = geom.cell(geom.rect.x + 1, geom.rect.bottom() - 1);
    update::begin_lx_selection(&mut state, pane, anchor.0, anchor.1);

    let now = std::time::Instant::now();
    crate::event::mouse::autoscroll::arm_lx_selection(
        &mut state,
        pane,
        &geo,
        &mut cache,
        (geom.rect.x + 1, geom.rect.y),
        now,
    );
    assert!(crate::event::mouse::autoscroll::tick_lx_selection(
        &mut state,
        &geo,
        &mut cache,
        now + std::time::Duration::from_millis(31),
    ));
    let run_after = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
    assert!(!run_after.follow);
    assert!(run_after.total_rows > run_after.visible && run_after.visible > 0);

    // 切换到 BTW 对话且 rows_dirty/visible==0：自动滚动同样先解析行数并向上滚动。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let messages = target.lx.agents.runs[0].chat.messages.clone();
        let thread = target.lx.btw.ensure_thread("qa-1");
        thread.loaded = true;
        thread.chat.messages = messages;
        thread.rows_dirty = true;
        thread.total_rows = 0;
        thread.visible = 0;
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Btw,
            ..Default::default()
        });
    }
    let btw_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("btw geom")
    };
    let btw_anchor = btw_geom.cell(btw_geom.rect.x + 1, btw_geom.rect.bottom() - 1);
    update::begin_lx_selection(&mut state, pane, btw_anchor.0, btw_anchor.1);
    crate::event::mouse::autoscroll::arm_lx_selection(
        &mut state,
        pane,
        &geo,
        &mut cache,
        (btw_geom.rect.x + 1, btw_geom.rect.y),
        now,
    );
    assert!(crate::event::mouse::autoscroll::tick_lx_selection(
        &mut state,
        &geo,
        &mut cache,
        now + std::time::Duration::from_millis(31),
    ));
    let thread_after = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert!(!thread_after.follow);
    assert!(thread_after.total_rows > thread_after.visible && thread_after.visible > 0);
}

/// Todo 与 Prompt 树：边缘 2 行内原地抖动（未跨行、未出界）不触发误自动滚动；
/// 拖出滚动条列/纵向上下缘可保底落点并继续滚动，拖出右栏水平范围则取消落点与滚动。
#[test]
fn todo_and_prompt_drag_autoscroll_anti_jitter_scrollbar_clamp_and_left_cancel() {
    use crate::app::prompt::PromptItem;

    let config = Config::default();
    let mut state = demo_terminal();
    let records: Vec<crate::db::todo::TodoRecord> = (0..40)
        .map(|index| {
            crate::db::todo::TodoRecord::new(
                format!("todo {index}"),
                crate::db::todo::TodoStatus::Todo,
                index as i64,
            )
        })
        .collect();
    state.todo.replace(records);
    assert!(update::select_right_tab(
        &mut state,
        crate::app::state::RightTab::Todo
    ));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let list = ui::right_panel::todo::effective_list_rect(geo.view.right_panel, &state);
    state.todo.set_viewport_rows(usize::from(list.height));
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;

    // 1. Todo：在底边第 2 行内按下并横向移动 1 列（未跨行，distance == 0）不得触发自动滚动。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            list.x + 1,
            list.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            list.x + 2,
            list.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.drag_autoscroll.is_none(),
        "未跨行的原地抖动不得启动自动滚动"
    );

    // 2. 跨行拖拽后滑到滚动条列并超出底边 1 行：仍保持自动滚动与合并目标；拖入主区左侧则取消。
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            list.right(),
            list.bottom(),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.drag_autoscroll.is_some(),
        "滚动条列与下缘越界应保持自动滚动"
    );
    assert!(
        state
            .todo_drag
            .as_ref()
            .is_some_and(|d| d.target_index.is_some()),
        "纵向越出下缘时仍钳到可见末行作为落点"
    );

    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            geo.view.right_panel.x.saturating_sub(2),
            list.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.drag_autoscroll.is_none(),
        "拖出右栏进入主区停止自动滚动"
    );
    assert!(
        state
            .todo_drag
            .as_ref()
            .is_some_and(|d| d.target_index.is_none()),
        "拖出右栏清空合并目标"
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            geo.view.right_panel.x.saturating_sub(2),
            list.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );

    // 3. Prompt 树：Temp 不可拖拽条目在边缘两行抖动不得触发自动滚动。
    assert!(update::select_right_tab(
        &mut state,
        crate::app::state::RightTab::Prompt
    ));
    let items: Vec<PromptItem> = (0..30)
        .map(|index| PromptItem::single(100 + index, None, format!("item {index}"), "body"))
        .collect();
    state.active_workspace_mut().prompt_panel.tree = crate::app::prompt::PromptTree::new();
    state
        .active_workspace_mut()
        .prompt_panel
        .tree
        .replace(Vec::new(), items);
    update::show_prompt_tree(&mut state);
    let area = ui::right_panel::prompt::frame::prompt_main_rect(geo.view.right_panel, &state);
    state.prompt_drag = Some(crate::app::state::PromptDrag {
        workspace: 0,
        source: None,
        from_row: 0,
        pressed_at: std::time::Instant::now(),
        dragging: false,
        drop_mark: crate::app::state::PromptDropMark::None,
        enabled: false,
    });
    crate::event::mouse::autoscroll::arm_drag(
        &mut state,
        crate::app::state::DragScrollSurface::PromptTree,
        &geo.view,
        (area.x + 1, area.y + 2),
        std::time::Instant::now(),
    );
    assert!(
        state.drag_autoscroll.is_none(),
        "enabled == false 禁止自动滚动"
    );
}

/// 右键与浮层关闭后，在途拖拽与自动滚动状态不得残留。
#[test]
fn right_click_clears_active_drag_and_autoscroll() {
    let config = Config::default();
    let mut state = demo_terminal();
    for _ in 0..20 {
        update::create_workspace(&mut state);
    }
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let list = ui::workspace_list_rect(&geo.view, &state).expect("workspace list");
    state.workspace_drag = Some(0);
    state.workspace_dragging = true;
    crate::event::mouse::autoscroll::arm_drag(
        &mut state,
        crate::app::state::DragScrollSurface::WorkspaceActive,
        &geo.view,
        (list.x + 1, list.bottom() - 1),
        std::time::Instant::now(),
    );
    assert!(state.drag_autoscroll.is_some());

    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), list.x + 1, list.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(state.workspace_drag.is_none());
    assert!(state.drag_autoscroll.is_none());
}
