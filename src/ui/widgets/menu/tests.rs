//! 单元测试；仅测试构建编译。

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

#[test]
fn layout_sizes_to_longest_label_with_minimum_width() {
    let menu = layout(SCREEN, (0, 0), &["Rename", "Close"]);
    assert_eq!(menu.area.width, MIN_WIDTH);
    assert_eq!(menu.area.height, 4);
    assert_eq!(menu.item_rects.len(), 2);
    assert_eq!(menu.item_rects[0].y, 1);
    assert_eq!(menu.item_rects[1].y, 2);
    assert_eq!(menu.item_rects[0].width, MIN_WIDTH - 2);

    let wide = layout(SCREEN, (0, 0), &["a very long menu entry label"]);
    assert_eq!(wide.area.width, 32);
}

#[test]
fn layout_clamps_anchor_to_screen() {
    let menu = layout(SCREEN, (79, 23), &["Rename", "Close"]);
    assert_eq!(menu.area.right(), SCREEN.right());
    assert_eq!(menu.area.bottom(), SCREEN.bottom());
    assert_eq!(menu.area.x, SCREEN.width - menu.area.width);
    assert_eq!(menu.area.y, SCREEN.height - menu.area.height);
}

#[test]
fn layout_shrinks_to_small_screen() {
    let screen = Rect::new(0, 0, 10, 3);
    let menu = layout(screen, (9, 2), &["Rename", "Close"]);
    assert_eq!(menu.area.width, 10);
    assert_eq!(menu.area.height, 3);
    assert_eq!(menu.item_rects.len(), 2);
    assert!(menu.area.bottom() <= screen.bottom());
}

#[test]
fn item_at_hits_rows_inside_border_only() {
    let menu = layout(SCREEN, (10, 5), &["Rename", "Close"]);
    assert_eq!(item_at(&menu, 11, 6), Some(0));
    assert_eq!(item_at(&menu, 11, 7), Some(1));
    assert_eq!(item_at(&menu, 10, 6), None);
    assert_eq!(item_at(&menu, 11, 5), None);
    assert_eq!(item_at(&menu, 11, 8), None);
    assert_eq!(item_at(&menu, 100, 6), None);
}

#[test]
fn render_draws_border_labels_and_reversed_selection() {
    let menu = layout(SCREEN, (10, 5), &["Rename", "Close"]);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &menu, &["Rename", "Close"], Some(1))) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let text_at = |x: u16, y: u16| buffer[(x, y)].symbol().to_string();
    assert_eq!(text_at(menu.area.x, menu.area.y), "╭");
    assert_eq!(text_at(menu.area.right() - 1, menu.area.y), "╮");
    let row: String = (menu.item_rects[0].x..menu.item_rects[0].right())
        .map(|x| text_at(x, menu.item_rects[0].y))
        .collect();
    assert_eq!(row.trim_end(), "Rename");
    let close: String = (menu.item_rects[1].x..menu.item_rects[1].right())
        .map(|x| text_at(x, menu.item_rects[1].y))
        .collect();
    assert_eq!(close.trim_end(), "Close");
    for x in menu.item_rects[1].x..menu.item_rects[1].right() {
        let cell = &buffer[(x, menu.item_rects[1].y)];
        assert!(cell.modifier.contains(Modifier::REVERSED), "x={x}");
    }
    for x in menu.item_rects[0].x..menu.item_rects[0].right() {
        let cell = &buffer[(x, menu.item_rects[0].y)];
        assert!(!cell.modifier.contains(Modifier::REVERSED), "x={x}");
    }
}
