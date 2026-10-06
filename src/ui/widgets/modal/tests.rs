//! 单元测试；仅测试构建编译。

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

#[test]
fn layout_centers_and_reserves_content_area() {
    let shell = layout(SCREEN, 40, 3).expect("modal fits");
    assert_eq!(shell.area.width, 40);
    assert_eq!(shell.area.height, 3);
    assert_eq!(shell.area.x, 20);
    assert_eq!(shell.area.y, 10);
    assert_eq!(shell.inner.x, shell.area.x + 1);
    assert_eq!(shell.inner.y, shell.area.y + 1);
    assert_eq!(shell.inner.width, 38);
    assert_eq!(shell.inner.height, 1);
}

#[test]
fn layout_clamps_to_small_screen() {
    let shell = layout(Rect::new(0, 0, 30, 8), 40, 4).expect("modal fits");
    assert_eq!(shell.area.width, 30);
    assert_eq!(shell.area.height, 4);
    assert_eq!(shell.area.right(), 30);
}

#[test]
fn layout_rejects_screen_too_small() {
    assert_eq!(layout(Rect::new(0, 0, 3, 10), 40, 3), None);
    assert_eq!(layout(Rect::new(0, 0, 40, 2), 40, 3), None);
    assert_eq!(layout(Rect::new(0, 0, 0, 0), 40, 3), None);
}

#[test]
fn render_draws_border_and_title() {
    let shell = layout(SCREEN, 40, 3).expect("modal fits");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &shell, "rename workspace")) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(buffer[(shell.area.x, shell.area.y)].symbol(), "╭");
    assert_eq!(
        buffer[(shell.area.right() - 1, shell.area.bottom() - 1)].symbol(),
        "╯"
    );
    let title_row: String = (shell.area.x..shell.area.right())
        .map(|x| buffer[(x, shell.area.y)].symbol())
        .collect();
    assert!(title_row.contains(" rename workspace "));
}
