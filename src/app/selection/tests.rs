//! 单元测试；仅测试构建编译。

use super::*;

fn selection() -> Selection {
    let mut selection = Selection::begin(PaneId::from_raw_for_test(1), 1, 2);
    selection.drag(3, 4);
    selection
}

#[test]
fn finish_ends_dragging_and_keeps_range() {
    let mut selection = selection();
    assert!(selection.is_dragging());
    selection.finish();
    assert!(!selection.is_dragging());
    assert_eq!(selection.range(), Some(((1, 2), (3, 4))));
}

#[test]
fn range_normalizes_drag_direction() {
    let forward = selection();
    assert_eq!(forward.range(), Some(((1, 2), (3, 4))));
    let mut backward = Selection::begin(PaneId::from_raw_for_test(1), 3, 4);
    backward.drag(1, 2);
    assert_eq!(backward.range(), Some(((1, 2), (3, 4))));
}

#[test]
fn click_without_drag_has_no_range() {
    let selection = Selection::begin(PaneId::from_raw_for_test(1), 1, 2);
    assert_eq!(selection.range(), None);
    assert!(!selection.contains(1, 2));
}

#[test]
fn contains_marks_reading_order_cells() {
    let selection = selection();
    assert!(selection.contains(1, 2));
    assert!(selection.contains(1, 9));
    assert!(selection.contains(2, 0));
    assert!(selection.contains(3, 4));
    assert!(selection.contains(3, 0));
    assert!(!selection.contains(1, 1));
    assert!(!selection.contains(3, 5));
    assert!(!selection.contains(0, 2));
    assert!(!selection.contains(4, 2));
}

#[test]
fn single_line_range_limits_columns() {
    let mut selection = Selection::begin(PaneId::from_raw_for_test(1), 2, 1);
    selection.drag(2, 5);
    assert!(selection.contains(2, 1));
    assert!(selection.contains(2, 5));
    assert!(!selection.contains(2, 0));
    assert!(!selection.contains(2, 6));
    assert!(!selection.contains(3, 2));
}

#[test]
fn full_selection_covers_whole_pane_range() {
    let pane = PaneId::from_raw_for_test(1);
    let selection = Selection::full(pane, 4, 9);
    assert_eq!(selection.pane(), pane);
    assert!(!selection.is_dragging());
    assert_eq!(selection.range(), Some(((0, 0), (4, 9))));
    assert!(selection.contains(0, 0));
    assert!(selection.contains(2, 5));
    assert!(selection.contains(4, 9));
    assert!(!selection.contains(5, 0));
}

#[test]
fn full_selection_with_zero_extent_has_no_range() {
    let selection = Selection::full(PaneId::from_raw_for_test(1), 0, 0);
    assert_eq!(selection.range(), None);
    assert!(!selection.contains(0, 0));
}

#[test]
fn click_count_cycles_within_window_and_resets_outside_or_moved() {
    let mut last = None;
    let t0 = Instant::now();
    assert_eq!(next_click_count(&mut last, 5, 3, t0), 1);
    assert_eq!(
        next_click_count(&mut last, 5, 3, t0 + Duration::from_millis(120)),
        2
    );
    assert_eq!(
        next_click_count(&mut last, 5, 3, t0 + Duration::from_millis(240)),
        3
    );
    assert_eq!(
        next_click_count(&mut last, 5, 3, t0 + Duration::from_millis(360)),
        1,
        "第 4 击循环回 1"
    );
    assert_eq!(
        next_click_count(&mut last, 6, 3, t0 + Duration::from_millis(400)),
        1,
        "坐标改变重置为 1"
    );
    assert_eq!(
        next_click_count(
            &mut last,
            6,
            3,
            t0 + Duration::from_millis(400) + SELECTION_CLICK_WINDOW
        ),
        1,
        "超过 400ms 重置为 1"
    );
}

#[test]
fn selection_unit_byte_range_and_drag_range_expand_by_word_and_line() {
    let text = "hello foo_bar_123 你好世界\nsecond line";
    let (w0, w1) = SelectionUnit::Word.byte_range(text, 8);
    assert_eq!(&text[w0..w1], "foo_bar_123");
    let cjk_offset = text.find('好').expect("cjk");
    let (c0, c1) = SelectionUnit::Word.byte_range(text, cjk_offset);
    assert_eq!(&text[c0..c1], "你好世界");
    let (l0, l1) = SelectionUnit::Line.byte_range(text, 8);
    assert_eq!(&text[l0..l1], "hello foo_bar_123 你好世界");

    let pane = PaneId::from_raw_for_test(1);
    let mut word_sel = Selection::begin_range(pane, (0, 6), (0, 16), SelectionUnit::Word);
    assert_eq!(word_sel.range(), Some(((0, 6), (0, 16))));
    // 向左拖拽到首词：保留初始词右端点 (0, 16)，左端点扩到 (0, 0)。
    word_sel.drag_range((0, 0), (0, 4));
    assert_eq!(word_sel.range(), Some(((0, 0), (0, 16))));
}
