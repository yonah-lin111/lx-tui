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

#[test]
fn button_row_centers_buttons_with_gap() {
    let shell = layout(SCREEN, 40, 5).expect("modal fits");
    let labels = ["[save enter]", "[clear ^c]", "[cancel esc]"];
    let rects = button_row(shell.inner, &labels, 2, 2);
    assert_eq!(rects.len(), 3);
    let widths: Vec<u16> = labels
        .iter()
        .map(|label| label.chars().count() as u16)
        .collect();
    let total: u16 = widths.iter().sum::<u16>() + 4;
    assert_eq!(rects[0].x, shell.inner.x + (shell.inner.width - total) / 2);
    assert_eq!(rects[0].y, shell.inner.y + 2);
    assert_eq!(rects[1].x, rects[0].right() + 2);
    assert_eq!(rects[2].x, rects[1].right() + 2);
    assert_eq!(rects[2].right(), rects[0].x + total);
    for (rect, width) in rects.iter().zip(widths) {
        assert_eq!(rect.width, width);
    }
}

#[test]
fn button_row_clamps_row_offset_inside_inner() {
    let shell = layout(SCREEN, 40, 4).expect("modal fits");
    let rects = button_row(shell.inner, &["[ok]"], 0, 9);
    assert_eq!(rects[0].y, shell.inner.bottom() - 1);
}

#[test]
fn button_at_hits_button_cells_only() {
    let shell = layout(SCREEN, 40, 5).expect("modal fits");
    let labels = ["[save enter]", "[cancel esc]"];
    let rects = button_row(shell.inner, &labels, 2, 2);
    assert_eq!(button_at(&rects, rects[0].x, rects[0].y), Some(0));
    assert_eq!(button_at(&rects, rects[1].right() - 1, rects[1].y), Some(1));
    assert_eq!(
        button_at(&rects, rects[0].x.saturating_sub(1), rects[0].y),
        None
    );
    assert_eq!(button_at(&rects, rects[0].x, rects[0].y + 1), None);
}
