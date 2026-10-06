//! 集成测试：硬件光标跟随聚焦终端窗格的仿真光标（IME 预输入）与显隐规则。

use lx_tui::app::actions::EditorCommand;
use lx_tui::app::state::AppState;
use lx_tui::app::update;
use lx_tui::config::Config;
use lx_tui::layout;
use lx_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Position, Rect};

/// 构造已同步几何的场景（100x24，右栏展开），返回终端窗格与 prompt 的内容区。
fn ready_state() -> (AppState, Config, Rect, Rect) {
    let config = Config::default();
    let mut state = AppState::demo();
    let screen = Rect::new(0, 0, 100, 24);
    let view = ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
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
    let pane = state.active_tab().layout.focus();
    let pane_rect = rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| *rect)
        .expect("focused pane is tiled");
    (
        state,
        config,
        layout::pane_inner_rect(pane_rect),
        layout::pane_inner_rect(view.prompt),
    )
}

fn draw(state: &AppState, config: &Config) -> Terminal<TestBackend> {
    let mut terminal =
        Terminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    terminal
        .draw(|frame| ui::render(frame, state, config))
        .expect("draw succeeds");
    terminal
}

/// 终端窗格聚焦时硬件光标跟随仿真光标。
#[test]
fn hardware_cursor_tracks_focused_terminal_for_ime_preedit() {
    let (mut state, config, pane_inner, _) = ready_state();
    let pane = state.active_tab().layout.focus();
    update::focus_pane(&mut state, pane);

    let terminal = draw(&state, &config);
    let (row, col) = state
        .active_tab()
        .pane(pane)
        .expect("focused pane exists")
        .terminal
        .cursor_viewport()
        .expect("terminal cursor is visible");
    assert!(terminal.backend().cursor_visible());
    assert_eq!(
        terminal.backend().cursor_position(),
        Position::new(pane_inner.x + col as u16, pane_inner.y + row as u16)
    );
}

/// 程序隐藏光标（DECTCEM off）时硬件光标一并隐藏。
#[test]
fn hidden_terminal_cursor_hides_hardware_cursor() {
    let (mut state, config, _, _) = ready_state();
    let pane = state.active_tab().layout.focus();
    update::focus_pane(&mut state, pane);
    update::feed_pane(&mut state, pane, b"\x1b[?25l");

    let terminal = draw(&state, &config);
    assert!(!terminal.backend().cursor_visible());
}

/// prompt 取回焦点后硬件光标跳到编辑器光标处，离开终端窗格。
#[test]
fn prompt_focus_moves_hardware_cursor_off_terminal() {
    let (mut state, config, _, prompt_inner) = ready_state();
    let pane = state.active_tab().layout.focus();
    update::focus_pane(&mut state, pane);
    update::apply_editor(&mut state, EditorCommand::InsertText("你好".into()));
    update::focus_prompt(&mut state);

    let terminal = draw(&state, &config);
    assert!(terminal.backend().cursor_visible());
    assert_eq!(
        terminal.backend().cursor_position(),
        Position::new(prompt_inner.x + 4, prompt_inner.y)
    );
}
