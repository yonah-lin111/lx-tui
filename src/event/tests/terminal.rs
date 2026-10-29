use super::test_support::*;
use super::*;

#[test]
fn mouse_wheel_over_terminal_pane_scrolls_backlog() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    scrolled_pane(&mut state, pane);
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    let offset = target.terminal.display_offset();
    assert!(offset > 0, "wheel up scrolls into backlog");
    assert!(dirty);

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert!(target.terminal.display_offset() < offset);
}

#[test]
fn mouse_wheel_extends_in_progress_selection() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    for i in 0..40 {
        target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
    }
    // 本地选区只存在于非鼠标上报窗格；选中滚动优先于本地回滚。
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 2,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.terminal_selection, Some(pane));

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x + 2, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 3);
    let content = target.terminal.renderable_content();
    let range = content.selection.expect("selection range");
    // 锚点钉在网格行 0（鼠标按下的内容），终点随视口滚到网格行 -3。
    assert_eq!(range.start.line.0, -3);
    assert_eq!(range.end.line.0, 0);
    assert_eq!(range.start.column.0, 2);
    assert_eq!(range.end.column.0, 2);
    assert!(dirty);
}

#[test]
fn mouse_drag_to_pane_edge_arms_autoscroll() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        for i in 0..80 {
            target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
        }
    }
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 2,
            inner.y + 5,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            inner.x + 2,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state.selection_autoscroll.is_some(),
        "edge drag arms autoscroll"
    );

    let before = state
        .pane_anywhere(pane)
        .expect("terminal pane")
        .terminal
        .display_offset();
    assert!(crate::app::update::tick(
        &mut state,
        Instant::now() + Duration::from_millis(31)
    ));
    let after = state
        .pane_anywhere(pane)
        .expect("terminal pane")
        .terminal
        .display_offset();
    assert!(after > before, "autoscroll tick scrolls the viewport");
}

#[test]
fn mouse_wheel_in_mouse_report_mode_leaves_local_view_at_bottom() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    scrolled_pane(&mut state, pane);
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    target.terminal.feed(b"\x1b[?1000h\x1b[?1006h");
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 0);
    assert!(dirty);
}

#[test]
fn terminal_scrollbar_thumb_drag_moves_viewport() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        for i in 0..80 {
            target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
        }
    }
    let target = state.pane_anywhere(pane).expect("terminal pane");
    let history = target.terminal.history_size();
    assert!(history > 0);
    let inner = pane_inner(&geo, pane);
    let bar = ui::main_content::scrollbar(inner, target).expect("scrollbar");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.track.x,
            bar.thumb.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.terminal_scroll_drag.is_some());
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 0);

    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            bar.track.x,
            bar.track.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), history);

    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            bar.track.x,
            bar.track.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.terminal_scroll_drag.is_none());
}

#[test]
fn mouse_drag_on_mouse_report_pane_skips_local_selection() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        target
            .terminal
            .feed(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
    }
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_terminal_event(
            mouse(kind, inner.x + 1, inner.y + 1),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }

    assert!(
        state.terminal_selection.is_none(),
        "mouse-report app owns selection"
    );
    assert!(state.selection.is_none());
}

#[test]
fn mouse_down_on_plain_pane_starts_local_selection() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
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

    assert_eq!(state.terminal_selection, Some(pane));
    assert!(dirty);
}

#[test]
fn mouse_wheel_on_alternate_screen_consumes_without_local_scroll() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    target.terminal.feed(b"\x1b[?1049h\x1b[?1007h");
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 0);
    assert!(dirty);
}

#[test]
fn key_and_paste_snap_scrolled_pane_back_to_bottom() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    scrolled_pane(&mut state, pane);
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(
        target.terminal.display_offset(),
        0,
        "typing returns to bottom"
    );

    scrolled_pane(&mut state, pane);
    handle_terminal_event(
        TerminalEvent::Paste("x".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(
        target.terminal.display_offset(),
        0,
        "paste returns to bottom"
    );
}

#[test]
fn terminal_selection_retains_on_mouse_up_scrolls_and_copies_on_ctrl_c() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 20));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        for idx in 0..35 {
            target
                .terminal
                .feed(format!("term-line-{idx:02}\r\n").as_bytes());
        }
    }
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;

    // 1. 终端窗格拖拽选择后松开左键：不自动复制，保留仿真器选区高亮。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), inner.x, inner.y),
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
            inner.x + 11,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), inner.x + 11, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.terminal_selection.is_none(),
        "松开鼠标后结束终端选区拖拽态"
    );
    assert!(state.toast.is_none(), "松开鼠标不自动触发复制 toast");
    assert_eq!(
        update::terminal_selection_text(&state, pane)
            .as_deref()
            .map(str::len),
        Some(12),
        "松开鼠标后终端选区保留在仿真器中"
    );

    // 2. 松开鼠标后滚轮向上回滚：选区不延伸、不清除，随内容行一起保留。
    let selected_before = update::terminal_selection_text(&state, pane).expect("selected");
    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x + 5, inner.y + 3),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        update::terminal_selection_text(&state, pane).as_deref(),
        Some(selected_before.as_str()),
        "松开后滚轮滚动不改变已选中的终端内容文本"
    );

    // 3. 按下 Ctrl+C 复制：弹出全局顶部居中 toast（anchor = None）并显示复制字符数，选区继续保留。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let toast = state
        .toast
        .as_ref()
        .expect("终端选区按 Ctrl+C 弹出全局复制 toast");
    assert_eq!(toast.anchor, None, "全局复制 toast 固定在屏幕顶部居中");
    assert_eq!(toast.message, "Copied 12 chars");
    assert!(
        update::terminal_selection_text(&state, pane).is_some(),
        "复制后终端选区高亮继续保留"
    );

    // 4. 在终端输入普通按键：自动清除终端选区。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        update::terminal_selection_text(&state, pane).is_none(),
        "终端输入其它按键后自动清除选区"
    );
}

#[test]
fn double_and_triple_click_select_word_and_line_across_all_views() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.insert_str("alpha beta_gamma delta");
        target
            .lx
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                sequence: 0,
                id: "m0".into(),
                role: "assistant".into(),
                status: "complete".into(),
                turn_index: 0,
                kind: "chat".into(),
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Text,
                        content: "firstWord secondWord thirdWord".into(),
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
    update::resize_panes(&mut state, &geo.rects);
    state.prompt.set_text("prompt_one prompt_two\nsecond_line");

    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;

    // 1. lx 主内容区：双击选中 "second_word"，三击选中整行。
    let hist_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("hist geom")
    };
    let word_x = hist_geom.rect.x + 15;
    let word_y = hist_geom.rect.y;
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), word_x, word_y),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), word_x, word_y),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    let range = state
        .lx_selection
        .and_then(|sel| sel.range())
        .expect("双击历史消息选中词");
    let copied_word = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::copy_text(inner, &target.lx, range, cache.pane(pane))
    };
    assert_eq!(copied_word, "secondWord");

    // 第 3 击选中整行。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), word_x, word_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), word_x, word_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let line_range = state
        .lx_selection
        .and_then(|sel| sel.range())
        .expect("三击历史消息选中整行");
    let copied_line = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::copy_text(inner, &target.lx, line_range, cache.pane(pane))
    };
    assert!(
        copied_line.contains("firstWord secondWord thirdWord"),
        "三击复制整行：{copied_line}"
    );

    // 2. lx 底部输入框：双击选中 "beta_gamma"，松开后保留高亮，Ctrl+C 复制且不清空草稿。
    let input_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::input_geom(inner, &target.lx).expect("input geom")
    };
    let (ix, iy) = input_geom.text_origin();
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), ix + 8, iy),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), ix + 8, iy),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|t| t.lx.selected_text())
            .as_deref(),
        Some("beta_gamma"),
        "双击 lx 输入框选中词并保留高亮"
    );
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        state.toast.as_ref().map(|t| t.message.as_str()),
        Some("Copied 10 chars")
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "alpha beta_gamma delta",
        "输入框有选区时按 Ctrl+C 不清空草稿"
    );

    // 3. 右侧 Prompt 编辑器：双击选词、三击选整逻辑行。
    let prompt_area = super::paste::prompt_text_area(&geo.view);
    let px = prompt_area.x + 12;
    let py = prompt_area.y;
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), px, py),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), px, py),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    assert_eq!(
        update::prompt_selection_text(&state).as_deref(),
        Some("prompt_two"),
        "双击 Prompt 选中单词"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), px, py),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), px, py),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        update::prompt_selection_text(&state).as_deref(),
        Some("prompt_one prompt_two"),
        "三击 Prompt 选中整条逻辑行"
    );

    // 4. 终端窗格：双击选词、三击选整行。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Terminal;
        target.terminal.feed(b"term_alpha term_beta\r\n");
    }
    let tx = inner.x + 12;
    let ty = inner.y;
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), tx, ty),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), tx, ty),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    assert_eq!(
        update::terminal_selection_text(&state, pane).as_deref(),
        Some("term_beta"),
        "双击终端窗格选中词"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), tx, ty),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), tx, ty),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        update::terminal_selection_text(&state, pane).as_deref(),
        Some("term_alpha term_beta\n"),
        "三击终端窗格选中整行"
    );
}
