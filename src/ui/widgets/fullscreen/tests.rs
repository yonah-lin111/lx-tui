//! 全屏覆盖几何、清底与命中。

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

#[test]
fn layout_wraps_the_whole_screen_with_a_border() {
    let screen = Rect::new(0, 0, 100, 30);
    let shell = layout(screen).expect("fits");
    assert_eq!(shell.area, screen);
    // 边框内区收缩一圈。
    assert_eq!(shell.inner, Rect::new(1, 1, 98, 28));
    assert_eq!(shell.header, Rect::new(1, 1, 98, 1));
    assert_eq!(shell.content, Rect::new(1, 2, 98, 26));
    assert_eq!(shell.footer, Rect::new(1, 28, 98, 1));
}

#[test]
fn layout_respects_screen_offset() {
    let screen = Rect::new(5, 3, 40, 12);
    let shell = layout(screen).expect("fits");
    assert_eq!(shell.inner.x, 6);
    assert_eq!(shell.inner.y, 4);
    assert_eq!(shell.header, Rect::new(6, 4, 38, 1));
    assert_eq!(shell.footer.y, 13);
}

#[test]
fn too_small_screen_has_no_shell() {
    assert!(layout(Rect::new(0, 0, 5, 5)).is_none());
    assert!(layout(Rect::new(0, 0, 40, 2)).is_none());
}

#[test]
fn close_button_sits_at_header_right_end() {
    let screen = Rect::new(0, 0, 100, 30);
    let shell = layout(screen).expect("fits");
    let close = close_button_rect(&shell).expect("close button");
    assert_eq!(close.right(), shell.header.right());
    assert_eq!(close.y, shell.header.y);
    assert!(close_button_at(&shell, close.x, close.y));
    assert!(!close_button_at(&shell, close.x.saturating_sub(1), close.y));
}

#[test]
fn render_draws_a_border_but_no_panel_background() {
    let screen = Rect::new(0, 0, 100, 30);
    let shell = layout(screen).expect("fits");
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("test backend");
    terminal
        .draw(|frame| {
            render(frame, &shell, "UI", "[esc] close");
        })
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer().clone();
    // 最外层边框符号存在（圆角）。
    assert_eq!(buffer[(0, 0)].symbol(), "╭");
    assert_eq!(buffer[(99, 29)].symbol(), "╯");
    // 边框为强调色（Cyan）。
    assert_eq!(buffer[(0, 0)].fg, Color::Cyan);
    // 全屏覆盖不铺自包含面板底色（overlay_panel 为 Indexed 236）。
    assert_ne!(buffer[(50, 15)].bg, Color::Indexed(236));
}

#[test]
fn render_paints_title_and_hint() {
    let screen = Rect::new(0, 0, 40, 10);
    let shell = layout(screen).expect("fits");
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).expect("test backend");
    terminal
        .draw(|frame| {
            render(frame, &shell, "UI", "[esc] close");
        })
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer().clone();
    let header: String = (0..40).map(|x| buffer[(x, 1)].symbol()).collect();
    let footer: String = (0..40).map(|x| buffer[(x, 8)].symbol()).collect();
    assert!(header.contains("UI"));
    assert!(header.contains(text::MODAL_CLOSE));
    assert!(footer.contains("[esc] close"));
}
