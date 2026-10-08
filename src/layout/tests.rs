//! 单元测试；仅测试构建编译。

use super::*;

impl PaneId {
    /// 测试用构造。
    pub(crate) fn from_raw_for_test(raw: u32) -> Self {
        Self(raw)
    }
}

fn area(width: u16, height: u16) -> Rect {
    Rect::new(0, 0, width, height)
}

fn demo_layout() -> (TileLayout, PaneId, PaneId, PaneId) {
    let mut layout = TileLayout::new();
    let left = layout.focus();
    let right = layout.split_focused(Direction::Horizontal, 0.5);
    let bottom = layout.split_focused(Direction::Vertical, 0.6);
    (layout, left, right, bottom)
}

#[test]
fn single_pane_fills_area() {
    let layout = TileLayout::new();
    let rects = pane_rects(&layout, area(80, 24), 10, 3);
    assert_eq!(rects.len(), 1);
    assert_eq!(rects[0].0, layout.focus());
    assert_eq!(rects[0].1, area(80, 24));
}

#[test]
fn nested_splits_produce_expected_rects() {
    let (layout, left, right, bottom) = demo_layout();
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    let rect_of = |id: PaneId| rects.iter().find(|(pane, _)| *pane == id).map(|(_, r)| *r);
    assert_eq!(rect_of(left), Some(Rect::new(0, 0, 40, 20)));
    assert_eq!(rect_of(right), Some(Rect::new(40, 0, 40, 12)));
    assert_eq!(rect_of(bottom), Some(Rect::new(40, 12, 40, 8)));
}

#[test]
fn focus_cycles_through_panes() {
    let (mut layout, left, right, bottom) = demo_layout();
    assert_eq!(layout.focus(), bottom);
    layout.focus_next();
    assert_eq!(layout.focus(), left);
    layout.focus_prev();
    assert_eq!(layout.focus(), bottom);
    layout.focus_pane(right);
    layout.focus_next();
    assert_eq!(layout.focus(), bottom);
}

#[test]
fn focus_pane_rejects_unknown_id() {
    let mut layout = TileLayout::new();
    let foreign = PaneId(9999);
    assert!(!layout.focus_pane(foreign));
}

#[test]
fn pane_in_direction_finds_neighbors() {
    let (layout, left, right, bottom) = demo_layout();
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(
        pane_in_direction(&rects, left, NavDirection::Right),
        Some(right)
    );
    assert_eq!(
        pane_in_direction(&rects, right, NavDirection::Down),
        Some(bottom)
    );
    assert_eq!(
        pane_in_direction(&rects, bottom, NavDirection::Up),
        Some(right)
    );
    assert_eq!(pane_in_direction(&rects, left, NavDirection::Left), None);
    assert_eq!(pane_in_direction(&rects, right, NavDirection::Right), None);
}

fn two_pane_layout(direction: Direction) -> (TileLayout, PaneId, PaneId) {
    let mut layout = TileLayout::new();
    let first = layout.focus();
    let second = layout.split_focused(direction, 0.5);
    (layout, first, second)
}

fn rect_of(rects: &[(PaneId, Rect)], id: PaneId) -> Option<Rect> {
    rects.iter().find(|(pane, _)| *pane == id).map(|(_, r)| *r)
}

fn horizontal_hit(first: PaneId, second: PaneId) -> BoundaryHit {
    BoundaryHit {
        first,
        second,
        direction: Direction::Horizontal,
    }
}

fn vertical_hit(first: PaneId, second: PaneId) -> BoundaryHit {
    BoundaryHit {
        first,
        second,
        direction: Direction::Vertical,
    }
}

#[test]
fn collapsed_right_pane_becomes_edge_strip() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(layout.set_collapsed(right, true));
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 76, 20)));
    assert_eq!(rect_of(&rects, right), Some(Rect::new(76, 0, 4, 20)));
}

#[test]
fn collapsed_left_pane_becomes_left_strip() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(layout.set_collapsed(left, true));
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 4, 20)));
    assert_eq!(rect_of(&rects, right), Some(Rect::new(4, 0, 76, 20)));
}

#[test]
fn collapsed_bottom_pane_becomes_bottom_strip() {
    let (mut layout, top, bottom) = two_pane_layout(Direction::Vertical);
    assert!(layout.set_collapsed(bottom, true));
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, top), Some(Rect::new(0, 0, 80, 16)));
    assert_eq!(rect_of(&rects, bottom), Some(Rect::new(0, 16, 80, 4)));
}

#[test]
fn expand_restores_full_rect() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(layout.set_collapsed(right, true));
    assert!(layout.set_collapsed(right, false));
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 40, 20)));
    assert_eq!(rect_of(&rects, right), Some(Rect::new(40, 0, 40, 20)));
}

#[test]
fn collapse_moves_focus_off_pane_and_blocks_it() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(layout.focus_pane(right));
    assert!(layout.set_collapsed(right, true));
    assert_eq!(layout.focus(), left);
    assert!(!layout.focus_pane(right));
    assert_eq!(layout.visible_pane_ids(), vec![left]);
}

#[test]
fn set_collapsed_rejects_unknown_id() {
    let (mut layout, ..) = two_pane_layout(Direction::Horizontal);
    let foreign = PaneId(9999);
    assert!(!layout.set_collapsed(foreign, true));
    assert_eq!(layout.collapsed(), None);
}

#[test]
fn focus_cycle_skips_collapsed_pane() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(layout.set_collapsed(right, true));
    assert_eq!(layout.focus(), left);
    layout.focus_next();
    assert_eq!(layout.focus(), left);
}

#[test]
fn pane_rects_clamps_split_to_min_width() {
    let (layout, left, right) = two_pane_layout(Direction::Horizontal);
    let mut narrow = layout.clone();
    assert!(narrow.resize_boundary(horizontal_hit(left, right), area(80, 20), 0, 10, 3));
    let rects = pane_rects(&narrow, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 10, 20)));
    assert_eq!(rect_of(&rects, right), Some(Rect::new(10, 0, 70, 20)));
}

#[test]
fn pane_rects_falls_back_when_area_below_twice_min() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(!layout.resize_boundary(horizontal_hit(left, right), area(15, 20), 7, 10, 3));
    let rects = pane_rects(&layout, area(15, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 8, 20)));
    assert_eq!(rect_of(&rects, right), Some(Rect::new(8, 0, 7, 20)));
}

#[test]
fn pane_rects_clamps_vertical_split_to_min_height() {
    let (layout, top, bottom) = two_pane_layout(Direction::Vertical);
    let mut squashed = layout.clone();
    assert!(squashed.resize_boundary(vertical_hit(top, bottom), area(80, 20), 0, 10, 3));
    let rects = pane_rects(&squashed, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, top), Some(Rect::new(0, 0, 80, 3)));
    assert_eq!(rect_of(&rects, bottom), Some(Rect::new(0, 3, 80, 17)));
}

#[test]
fn resize_boundary_clamps_to_both_minimums() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(layout.resize_boundary(horizontal_hit(left, right), area(80, 20), 200, 10, 3));
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 70, 20)));
    assert_eq!(rect_of(&rects, right), Some(Rect::new(70, 0, 10, 20)));
}

#[test]
fn resize_boundary_adjusts_vertical_split_and_clamps() {
    let (mut layout, top, bottom) = two_pane_layout(Direction::Vertical);
    assert!(layout.resize_boundary(vertical_hit(top, bottom), area(80, 20), 3, 10, 3));
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, top), Some(Rect::new(0, 0, 80, 3)));
    assert_eq!(rect_of(&rects, bottom), Some(Rect::new(0, 3, 80, 17)));

    assert!(layout.resize_boundary(vertical_hit(top, bottom), area(80, 20), 200, 10, 3));
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, top), Some(Rect::new(0, 0, 80, 17)));
    assert_eq!(rect_of(&rects, bottom), Some(Rect::new(0, 17, 80, 3)));
}

#[test]
fn resize_boundary_rejects_unknown_or_mismatched_pair() {
    let mut layout = TileLayout::new();
    let only = layout.focus();
    assert!(!layout.resize_boundary(horizontal_hit(only, only), area(80, 20), 40, 10, 3));
    let (mut vertical, top, bottom) = two_pane_layout(Direction::Vertical);
    assert!(!vertical.resize_boundary(horizontal_hit(top, bottom), area(80, 20), 40, 10, 3));
    assert!(!vertical.resize_boundary(vertical_hit(top, bottom), area(80, 4), 2, 10, 3));
}

#[test]
fn resize_boundary_adjusts_nested_split() {
    let (mut layout, left, right, bottom) = demo_layout();
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 40, 20)),);
    assert!(layout.resize_boundary(horizontal_hit(left, bottom), area(80, 20), 60, 10, 3));
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 60, 20)));
    assert_eq!(rect_of(&rects, right), Some(Rect::new(60, 0, 20, 12)));
    assert_eq!(rect_of(&rects, bottom), Some(Rect::new(60, 12, 20, 8)));
}

#[test]
fn resize_boundary_at_hits_border_columns_only() {
    let (_, left, right) = two_pane_layout(Direction::Horizontal);
    let rects = vec![
        (left, Rect::new(0, 0, 40, 20)),
        (right, Rect::new(40, 0, 40, 20)),
    ];
    assert_eq!(
        resize_boundary_at(&rects, 39, 5),
        Some(horizontal_hit(left, right))
    );
    assert_eq!(
        resize_boundary_at(&rects, 40, 5),
        Some(horizontal_hit(left, right))
    );
    assert_eq!(resize_boundary_at(&rects, 38, 5), None);
    assert_eq!(resize_boundary_at(&rects, 41, 5), None);
}

#[test]
fn resize_boundary_at_requires_shared_row() {
    let (_, left, right) = two_pane_layout(Direction::Horizontal);
    let rects = vec![
        (left, Rect::new(0, 0, 40, 20)),
        (right, Rect::new(40, 0, 40, 10)),
    ];
    assert_eq!(
        resize_boundary_at(&rects, 39, 5),
        Some(horizontal_hit(left, right))
    );
    assert_eq!(resize_boundary_at(&rects, 39, 15), None);
}

#[test]
fn resize_boundary_at_hits_row_borders_only() {
    let (_, top, bottom) = two_pane_layout(Direction::Vertical);
    let rects = vec![
        (top, Rect::new(0, 0, 80, 10)),
        (bottom, Rect::new(0, 10, 80, 10)),
    ];
    assert_eq!(
        resize_boundary_at(&rects, 5, 9),
        Some(vertical_hit(top, bottom))
    );
    assert_eq!(
        resize_boundary_at(&rects, 5, 10),
        Some(vertical_hit(top, bottom))
    );
    assert_eq!(resize_boundary_at(&rects, 5, 8), None);
    assert_eq!(resize_boundary_at(&rects, 5, 11), None);
}

#[test]
fn resize_boundary_at_requires_shared_column() {
    let (_, top, bottom) = two_pane_layout(Direction::Vertical);
    let rects = vec![
        (top, Rect::new(0, 0, 40, 10)),
        (bottom, Rect::new(0, 10, 80, 10)),
    ];
    assert_eq!(
        resize_boundary_at(&rects, 5, 10),
        Some(vertical_hit(top, bottom))
    );
    assert_eq!(resize_boundary_at(&rects, 50, 10), None);
}

#[test]
fn prompt_text_rect_reserves_scrollbar_column() {
    let panel = Rect::new(10, 5, 10, 6);
    assert_eq!(prompt_text_rect(panel), Rect::new(11, 8, 7, 2));
    assert_eq!(prompt_scrollbar_rect(panel), Some(Rect::new(18, 8, 1, 2)));
    assert_eq!(prompt_inner_size(panel), (7, 2));
}

#[test]
fn prompt_geometry_degrades_when_too_narrow() {
    let panel = Rect::new(0, 0, 3, 4);
    assert_eq!(prompt_text_rect(panel), Rect::new(1, 1, 1, 2));
    assert_eq!(prompt_scrollbar_rect(panel), None);
    assert_eq!(prompt_inner_size(panel), (1, 2));
}

#[test]
fn prompt_header_shifts_body_and_scrollbar() {
    let panel = Rect::new(10, 5, 20, 8);
    assert_eq!(prompt_header_rect(panel), Some(Rect::new(11, 6, 18, 2)));
    assert_eq!(prompt_toolbar_rect(panel), Some(Rect::new(11, 6, 18, 1)));
    assert_eq!(prompt_divider_rect(panel), Some(Rect::new(11, 7, 18, 1)));
    assert_eq!(prompt_text_rect(panel), Rect::new(11, 8, 17, 4));
    assert_eq!(prompt_scrollbar_rect(panel), Some(Rect::new(28, 8, 1, 4)));
    assert_eq!(prompt_inner_size(panel), (17, 4));
}

#[test]
fn prompt_header_hidden_when_body_too_short() {
    // 内容区高度 == 2：隐藏表头，全部留给文本编辑区。
    let panel = Rect::new(0, 0, 20, 4);
    assert_eq!(prompt_header_rect(panel), None);
    assert_eq!(prompt_toolbar_rect(panel), None);
    assert_eq!(prompt_divider_rect(panel), None);
    assert_eq!(prompt_text_rect(panel), Rect::new(1, 1, 17, 2));
    assert_eq!(prompt_scrollbar_rect(panel), Some(Rect::new(18, 1, 1, 2)));
}

#[test]
fn prompt_toolbar_buttons_hit_expected_cells() {
    let panel = Rect::new(0, 0, 40, 8);
    let bar = prompt_toolbar_rect(panel).expect("toolbar visible");
    let hit = |column: u16, row: u16| prompt_toolbar_button_at(panel, column, row);
    assert_eq!(hit(bar.x, bar.y), Some(PromptToolbarButton::Undo));
    assert_eq!(hit(bar.x + 7, bar.y), Some(PromptToolbarButton::Redo));
    let select_all =
        prompt_toolbar_button_rect(panel, PromptToolbarButton::SelectAll).expect("select all fits");
    assert_eq!(
        hit(select_all.x, bar.y),
        Some(PromptToolbarButton::SelectAll)
    );
    assert_eq!(
        hit(bar.right() - 1, bar.y),
        Some(PromptToolbarButton::Save),
        "最右一列为保存点"
    );
    assert_eq!(hit(bar.x + 13, bar.y), None, "空白区不命中");
    assert_eq!(
        hit(bar.x, bar.y + PROMPT_TOOLBAR_HEIGHT),
        None,
        "分割线行不命中按钮"
    );
}

#[test]
fn prompt_toolbar_hides_select_all_when_narrow() {
    // 内容区宽度 26 < 28：隐藏 select all，保留 undo/redo 与保存点。
    let panel = Rect::new(0, 0, 28, 8);
    assert_eq!(
        prompt_toolbar_button_rect(panel, PromptToolbarButton::SelectAll),
        None
    );
    assert!(prompt_toolbar_button_rect(panel, PromptToolbarButton::Undo).is_some());
    assert!(prompt_toolbar_button_rect(panel, PromptToolbarButton::Redo).is_some());
    assert!(prompt_toolbar_button_rect(panel, PromptToolbarButton::Save).is_some());
    assert_eq!(
        prompt_toolbar_button_at(
            panel,
            prompt_toolbar_rect(panel).expect("toolbar").right() - 1,
            prompt_toolbar_rect(panel).expect("toolbar").y,
        ),
        Some(PromptToolbarButton::Save)
    );
}

#[test]
fn remove_pane_promotes_sibling_to_parent_area() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(layout.remove_pane(right));
    assert_eq!(layout.pane_ids(), vec![left]);
    let rects = pane_rects(&layout, area(80, 20), 10, 3);
    assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 80, 20)));
}

#[test]
fn remove_pane_moves_focus_to_promoted_sibling_subtree() {
    let (mut layout, left, right, bottom) = demo_layout();
    assert_eq!(layout.focus(), bottom);
    assert!(layout.remove_pane(bottom));
    assert_eq!(layout.focus(), right);

    assert!(layout.focus_pane(right));
    assert!(layout.remove_pane(right));
    assert_eq!(layout.focus(), left);
    assert_eq!(layout.pane_ids(), vec![left]);
}

#[test]
fn remove_pane_keeps_focus_when_other_pane_removed() {
    let (mut layout, left, right, _) = demo_layout();
    assert!(layout.focus_pane(left));
    assert!(layout.remove_pane(right));
    assert_eq!(layout.focus(), left);
}

#[test]
fn remove_pane_rejects_last_or_unknown_pane() {
    let mut single = TileLayout::new();
    let only = single.focus();
    assert!(!single.remove_pane(only));
    assert_eq!(single.pane_ids(), vec![only]);

    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    let foreign = PaneId(9999);
    assert!(!layout.remove_pane(foreign));
    assert_eq!(layout.pane_ids(), vec![left, right]);
}

#[test]
fn remove_pane_clears_collapse_when_collapsed_pane_removed() {
    let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
    assert!(layout.set_collapsed(right, true));
    assert_eq!(layout.focus(), left);
    assert!(layout.remove_pane(right));
    assert_eq!(layout.collapsed(), None);
    assert_eq!(layout.pane_ids(), vec![left]);
}
