//! 全屏覆盖几何、清底、边框内嵌标题/提示与命中。

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

/// 渲染一行文本。
fn row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
    (buffer.area.x..buffer.area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn layout_wraps_the_whole_screen_with_a_border() {
    let screen = Rect::new(0, 0, 100, 30);
    let shell = layout(screen).expect("fits");
    assert_eq!(shell.area, screen);
    // 内容区为边框内区，标题/提示不额外占用内容行。
    assert_eq!(shell.content, Rect::new(1, 1, 98, 28));
}

#[test]
fn layout_respects_screen_offset() {
    let screen = Rect::new(5, 3, 40, 12);
    let shell = layout(screen).expect("fits");
    assert_eq!(shell.content, Rect::new(6, 4, 38, 10));
}

#[test]
fn too_small_screen_has_no_shell() {
    assert!(layout(Rect::new(0, 0, 5, 5)).is_none());
    assert!(layout(Rect::new(0, 0, 40, 2)).is_none());
}

#[test]
fn close_button_sits_at_top_border_right_end() {
    let screen = Rect::new(0, 0, 100, 30);
    let shell = layout(screen).expect("fits");
    let close = close_button_rect(&shell).expect("close button");
    assert_eq!(close.y, shell.area.y);
    assert_eq!(close.right(), shell.area.right() - 1);
    assert!(close_button_at(&shell, close.x, close.y));
    assert!(!close_button_at(&shell, close.x.saturating_sub(1), close.y));
}

#[test]
fn title_and_hint_are_embedded_in_the_border() {
    let screen = Rect::new(0, 0, 60, 12);
    let shell = layout(screen).expect("fits");
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("test backend");
    terminal
        .draw(|frame| render(frame, &shell, "UI", "[esc] close"))
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer().clone();
    let top = row(&buffer, 0);
    let bottom = row(&buffer, 11);
    // 顶边框：标题与关闭按钮都在边框线上（圆角起点仍保留）。
    assert_eq!(top.chars().next(), Some('╭'));
    assert!(top.contains("UI"), "{top}");
    assert!(top.contains(text::MODAL_CLOSE), "{top}");
    // 底边框：快捷键提示在边框线上（圆角起点仍保留）。
    assert_eq!(bottom.chars().next(), Some('╰'));
    assert!(bottom.contains("[esc] close"), "{bottom}");
}

#[test]
fn render_draws_a_border_but_no_panel_background() {
    let screen = Rect::new(0, 0, 100, 30);
    let shell = layout(screen).expect("fits");
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("test backend");
    terminal
        .draw(|frame| render(frame, &shell, "UI", "[esc] close"))
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(buffer[(0, 0)].symbol(), "╭");
    assert_eq!(buffer[(99, 29)].symbol(), "╯");
    // 边框为强调色（Cyan）。
    assert_eq!(buffer[(0, 0)].fg, Color::Cyan);
    // 全屏覆盖不铺自包含面板底色（overlay_panel 为 Indexed 236）。
    assert_ne!(buffer[(50, 15)].bg, Color::Indexed(236));
}
