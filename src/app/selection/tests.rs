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
