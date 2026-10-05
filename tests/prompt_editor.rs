//! 集成测试：prompt markdown 输入、点击定位、滚轮滚动贯穿几何同步、状态更新与渲染。

use lx_tui::app::actions::EditorCommand;
use lx_tui::app::state::AppState;
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};

/// 构造已同步几何的 prompt 场景（100x24，右栏展开）。
fn ready_state() -> (AppState, Config, ui::layout::ViewLayout) {
    let config = Config::default();
    let mut state = AppState::demo();
    let screen = Rect::new(0, 0, 100, 24);
    let view = ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let mut rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
    );
    rects.push((state.prompt.id(), view.prompt));
    update::resize_panes(&mut state, &rects);
    (state, config, view)
}

fn draw(state: &AppState, config: &Config) -> Terminal<TestBackend> {
    let mut terminal =
        Terminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    terminal
        .draw(|frame| ui::render(frame, state, config))
        .expect("draw succeeds");
    terminal
}

#[test]
fn typed_markdown_renders_highlight_and_cursor() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    for ch in "# 标题".chars() {
        update::apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }

    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    let inner = layout::pane_inner_rect(view.prompt);
    let marker = &buffer[(inner.x, inner.y)];
    assert_eq!(marker.symbol(), "#");
    assert!(marker.modifier.contains(Modifier::DIM));

    let heading = &buffer[(inner.x + 2, inner.y)];
    assert_eq!(heading.symbol(), "标");
    assert_eq!(heading.fg, Color::Yellow);
    assert!(heading.modifier.contains(Modifier::BOLD));

    assert!(
        buffer[(inner.x + 6, inner.y)]
            .modifier
            .contains(Modifier::REVERSED)
    );
}

#[test]
fn click_places_cursor_and_wheel_scrolls_viewport() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    let text = (0..30)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    update::apply_editor(&mut state, EditorCommand::InsertText(text));

    state.prompt.scroll_by(-100);
    update::place_prompt_cursor(&mut state, 0, 0);
    assert_eq!(state.prompt.scroll(), 0);
    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    let inner = layout::pane_inner_rect(view.prompt);
    assert_eq!(buffer[(inner.x, inner.y)].symbol(), "0");
    assert!(
        buffer[(inner.x, inner.y)]
            .modifier
            .contains(Modifier::REVERSED)
    );

    update::scroll_prompt(&mut state, 1);
    assert_eq!(state.prompt.scroll(), 3);
    let terminal = draw(&state, &config);
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(inner.x, inner.y)].symbol(), "3");
    assert!(
        !buffer[(inner.x, inner.y)]
            .modifier
            .contains(Modifier::REVERSED)
    );
}

#[test]
fn hardware_cursor_tracks_focused_prompt_for_ime_preedit() {
    let (mut state, config, view) = ready_state();
    update::focus_prompt(&mut state);
    update::apply_editor(&mut state, EditorCommand::InsertText("你好".into()));

    let terminal = draw(&state, &config);
    let inner = layout::pane_inner_rect(view.prompt);
    assert!(terminal.backend().cursor_visible());
    assert_eq!(
        terminal.backend().cursor_position(),
        Position::new(inner.x + 4, inner.y)
    );

    let pane = state.active_tab().layout.focus();
    update::focus_pane(&mut state, pane);
    let terminal = draw(&state, &config);
    assert!(!terminal.backend().cursor_visible());
}

#[test]
fn clicking_terminal_pane_reads_back_focus_and_keeps_editing_target() {
    let mut state = AppState::demo();
    update::focus_prompt(&mut state);
    assert!(state.prompt_focused);

    let pane = state.active_tab().layout.focus();
    update::focus_pane(&mut state, pane);
    assert!(!state.prompt_focused);
    assert_eq!(state.active_tab().layout.focus(), pane);
}
