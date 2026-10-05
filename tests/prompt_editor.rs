//! 集成测试：prompt markdown 输入与高亮贯穿几何同步、状态更新与渲染。

use lx_tui::app::actions::EditorCommand;
use lx_tui::app::state::AppState;
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

#[test]
fn typed_markdown_renders_highlight_and_cursor() {
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

    update::focus_prompt(&mut state);
    for ch in "# 标题".chars() {
        update::apply_editor(&mut state, EditorCommand::InsertChar(ch));
    }

    let mut terminal =
        Terminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    terminal
        .draw(|frame| ui::render(frame, &state, &config))
        .expect("draw succeeds");
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
fn clicking_terminal_pane_reads_back_focus_and_keeps_editing_target() {
    let mut state = AppState::demo();
    update::focus_prompt(&mut state);
    assert!(state.prompt_focused);

    let pane = state.active_tab().layout.focus();
    update::focus_pane(&mut state, pane);
    assert!(!state.prompt_focused);
    assert_eq!(state.active_tab().layout.focus(), pane);
}
