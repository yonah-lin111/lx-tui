//! 提示词正文分割的几何单元测试。

use ratatui::layout::Rect;

use super::*;
use crate::tiling::pane_inner_rect;

#[test]
fn prompt_body_split_reserves_drawer_rows() {
    let panel = Rect::new(0, 0, 40, 18);
    let body = prompt_body_rect(pane_inner_rect(panel));
    let closed = prompt_body_split(panel, false, 10);
    assert_eq!(closed.main, body_text_rect(body));
    assert!(closed.recent_divider.is_none());

    let two = prompt_body_split(panel, true, 2);
    assert_eq!(two.visible_entries, 2);
    assert_eq!(two.main.height, body.height - 5, "1 行分割线 + 2 条双行");
    assert_eq!(two.recent_text.expect("content").height, 4);
    assert_eq!(two.recent_scrollbar.expect("scrollbar").height, 4);

    let capped = prompt_body_split(panel, true, 10);
    assert_eq!(capped.visible_entries, 3, "半屏容量 3 条");
    assert_eq!(capped.main.height, body.height - 7);

    let one = prompt_body_split(panel, true, 1);
    assert_eq!(one.visible_entries, 2, "仅 1 条记录也保留 2 条高度");
    assert_eq!(one.main.height, body.height - 5);
    assert_eq!(one.recent_text.expect("content").height, 4);
}

#[test]
fn prompt_body_split_grows_to_half_height() {
    // 面板 40x30：内容区 26 行，半屏 13 行，容量 (13-1)/2 = 6 条。
    let panel = Rect::new(0, 0, 40, 30);
    let body = prompt_body_rect(pane_inner_rect(panel));
    let split = prompt_body_split(panel, true, 10);
    assert_eq!(split.visible_entries, 6);
    assert_eq!(split.main.height, body.height - 13);
    assert_eq!(split.recent_text.expect("content").height, 12);
    assert_eq!(split.recent_scrollbar.expect("scrollbar").height, 12);
}

#[test]
fn prompt_body_split_caps_at_twenty_entries() {
    // 面板足够高时，布局仍只按 MAX_RECENT_PAGES 条参与高度计算。
    let panel = Rect::new(0, 0, 40, 100);
    let split = prompt_body_split(panel, true, 25);
    assert_eq!(split.visible_entries, 20);
    assert_eq!(split.recent_text.expect("content").height, 40);
}

#[test]
fn prompt_body_split_shrinks_to_minimum_two_then_suppresses() {
    // 内容区 10 行：抽屉上限 5 行，半屏容量 2 条（1+4）。
    let panel = Rect::new(0, 0, 40, 14);
    let split = prompt_body_split(panel, true, 10);
    assert_eq!(split.visible_entries, 2);
    let body = prompt_body_rect(pane_inner_rect(panel));
    assert_eq!(split.main.height, body.height - 5);

    // 内容区 8 行：半屏 4 行连最低 2 条（5 行）都放不下，直接收起抽屉。
    let shorter = Rect::new(0, 0, 40, 12);
    let split = prompt_body_split(shorter, true, 10);
    assert!(split.recent_divider.is_none());
    assert_eq!(
        split.main,
        body_text_rect(prompt_body_rect(pane_inner_rect(shorter)))
    );

    // 内容区 4 行：同样收起。
    let tiny = Rect::new(0, 0, 40, 8);
    let split = prompt_body_split(tiny, true, 10);
    assert!(split.recent_divider.is_none());
    assert_eq!(
        split.main,
        body_text_rect(prompt_body_rect(pane_inner_rect(tiny)))
    );
}

#[test]
fn prompt_body_split_keeps_empty_state_row() {
    let panel = Rect::new(0, 0, 40, 12);
    let split = prompt_body_split(panel, true, 0);
    assert_eq!(split.visible_entries, 0);
    assert_eq!(split.recent_text.expect("empty row").height, 1);
    assert!(split.recent_scrollbar.expect("gutter").height == 1);
}
