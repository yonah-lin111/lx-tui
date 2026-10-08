//! 单元测试；仅测试构建编译。

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

const LIST: Rect = Rect {
    x: 0,
    y: 0,
    width: 20,
    height: 10,
};

#[test]
fn layout_hidden_when_content_fits() {
    assert_eq!(layout(LIST, 10, 10, 0), None);
    assert_eq!(layout(LIST, 3, 10, 0), None);
    assert_eq!(layout(Rect::new(0, 0, 20, 0), 20, 10, 0), None);
    assert_eq!(layout(LIST, 20, 0, 0), None);
}

#[test]
fn layout_places_track_at_right_edge_with_proportional_thumb() {
    let bar = layout(LIST, 20, 10, 0).expect("scrollbar is needed");
    assert_eq!(bar.track, Rect::new(19, 0, 1, 10));
    assert_eq!(bar.thumb.x, 19);
    assert_eq!(bar.thumb.height, 5);
    assert_eq!(bar.thumb.y, 0);

    let bottom = layout(LIST, 20, 10, 10).expect("scrollbar is needed");
    assert_eq!(bottom.thumb.y, 5);
}

#[test]
fn layout_clamps_offset_beyond_max() {
    let bar = layout(LIST, 20, 10, 99).expect("scrollbar is needed");
    assert_eq!(bar.thumb.y, 5);
}

#[test]
fn layout_keeps_thumb_at_least_one_row() {
    let bar = layout(LIST, 1000, 10, 0).expect("scrollbar is needed");
    assert_eq!(bar.thumb.height, 1);
}

#[test]
fn thumb_grab_offset_only_hits_thumb() {
    let bar = layout(LIST, 20, 10, 0).expect("scrollbar is needed");
    assert_eq!(thumb_grab_offset(&bar, 0), Some(0));
    assert_eq!(thumb_grab_offset(&bar, 4), Some(4));
    assert_eq!(thumb_grab_offset(&bar, 5), None);
}

#[test]
fn thumb_grab_offset_ignores_track_above_thumb() {
    let bar = layout(LIST, 20, 10, 10).expect("scrollbar is needed");
    assert_eq!(bar.thumb.y, 5);
    assert_eq!(thumb_grab_offset(&bar, 0), None);
    assert_eq!(thumb_grab_offset(&bar, 4), None);
    assert_eq!(thumb_grab_offset(&bar, 5), Some(0));
}

#[test]
fn track_click_maps_row_to_offset() {
    let bar = layout(LIST, 20, 10, 0).expect("scrollbar is needed");
    assert_eq!(offset_from_track_row(&bar, 0), 0);
    assert_eq!(offset_from_track_row(&bar, 4), 4);
    assert_eq!(offset_from_track_row(&bar, 9), 10);
    // 越界行钳制到轨道末端。
    assert_eq!(offset_from_track_row(&bar, 99), 10);
}

#[test]
fn drag_row_maps_with_grab_offset() {
    let bar = layout(LIST, 20, 10, 0).expect("scrollbar is needed");
    assert_eq!(offset_from_drag_row(&bar, 0, 0), 0);
    assert_eq!(offset_from_drag_row(&bar, 4, 0), 8);
    assert_eq!(offset_from_drag_row(&bar, 9, 0), 10);
    assert_eq!(offset_from_drag_row(&bar, 9, 4), 10);
    assert_eq!(offset_from_drag_row(&bar, 5, 4), 2);
}

#[test]
fn render_draws_track_and_text_thumb() {
    let bar = layout(LIST, 20, 10, 0).expect("scrollbar is needed");
    let mut terminal = Terminal::new(TestBackend::new(20, 10)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &bar)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(buffer[(19, 0)].symbol(), THUMB_SYMBOL);
    assert_eq!(buffer[(19, 0)].modifier, Modifier::empty());
    assert_eq!(buffer[(19, 0)].fg, Color::Reset);
    assert_eq!(buffer[(19, 0)].bg, Color::Reset);
    assert_eq!(buffer[(19, 9)].symbol(), TRACK_SYMBOL);
    assert!(buffer[(19, 9)].modifier.contains(Modifier::DIM));
}

#[test]
fn thumb_style_matches_text_style_and_avoids_reversed_black_gap() {
    let bar = layout(LIST, 20, 10, 0).expect("scrollbar is needed");
    let mut terminal = Terminal::new(TestBackend::new(20, 10)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &bar)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let text_style = style::text();
    assert_eq!(buffer[(19, 0)].modifier, text_style.add_modifier);
    assert!(
        !buffer[(19, 0)].modifier.contains(Modifier::REVERSED),
        "thumb must not be reversed to prevent inverted black gap on right half-block"
    );
}
