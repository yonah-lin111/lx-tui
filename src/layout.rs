//! BSP 平铺布局模型与几何计算：不涉及渲染，可在无终端环境下测试。

use std::sync::atomic::{AtomicU32, Ordering};

use ratatui::layout::{Direction, Rect};

/// 折叠窗格在父分割中占用的窄条宽度（列或行），需容纳折叠按钮 `[+]`。
pub const COLLAPSED_STRIP: u16 = 3;

/// 窗格唯一标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneId(u32);

static NEXT_PANE_ID: AtomicU32 = AtomicU32::new(1);

impl PaneId {
    /// 分配全局唯一的窗格标识。
    fn alloc() -> Self {
        Self(NEXT_PANE_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// 返回原始数值，用于展示。
    pub fn raw(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
impl PaneId {
    /// 测试用构造。
    pub(crate) fn from_raw_for_test(raw: u32) -> Self {
        Self(raw)
    }
}

/// 窗格焦点移动方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

/// BSP 树节点。
#[derive(Debug, Clone)]
enum Node {
    Pane(PaneId),
    Split {
        direction: Direction,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// 平铺布局：记录 BSP 树、焦点窗格与折叠窗格。
#[derive(Debug, Clone)]
pub struct TileLayout {
    root: Node,
    focus: PaneId,
    collapsed: Option<PaneId>,
}

impl TileLayout {
    /// 创建只含一个窗格的布局。
    pub fn new() -> Self {
        let id = PaneId::alloc();
        Self {
            root: Node::Pane(id),
            focus: id,
            collapsed: None,
        }
    }

    /// 在焦点窗格处按方向与比例分割，焦点移到新窗格并返回其标识。
    pub fn split_focused(&mut self, direction: Direction, ratio: f32) -> PaneId {
        let new_id = PaneId::alloc();
        let ratio = ratio.clamp(0.1, 0.9);
        split_node(&mut self.root, self.focus, direction, ratio, new_id);
        self.focus = new_id;
        new_id
    }

    /// 当前焦点窗格。
    pub fn focus(&self) -> PaneId {
        self.focus
    }

    /// 聚焦指定窗格；窗格不存在或已折叠时返回 false。
    pub fn focus_pane(&mut self, id: PaneId) -> bool {
        if self.collapsed == Some(id) || !self.pane_ids().contains(&id) {
            return false;
        }
        self.focus = id;
        true
    }

    /// 折叠或展开窗格；窗格不存在时返回 false。
    ///
    /// 折叠时若焦点在该窗格上，焦点自动移到其余窗格。
    pub fn set_collapsed(&mut self, id: PaneId, collapsed: bool) -> bool {
        if !self.pane_ids().contains(&id) {
            return false;
        }
        if collapsed {
            self.collapsed = Some(id);
            if self.focus == id
                && let Some(next) = self.pane_ids().into_iter().find(|pane| *pane != id)
            {
                self.focus = next;
            }
        } else if self.collapsed == Some(id) {
            self.collapsed = None;
        }
        true
    }

    /// 当前折叠窗格。
    pub fn collapsed(&self) -> Option<PaneId> {
        self.collapsed
    }

    /// 未折叠的窗格，按树顺序。
    pub fn visible_pane_ids(&self) -> Vec<PaneId> {
        self.pane_ids()
            .into_iter()
            .filter(|id| Some(*id) != self.collapsed)
            .collect()
    }

    /// 按树顺序返回全部窗格。
    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        collect_ids(&self.root, &mut ids);
        ids
    }

    /// 焦点移到下一个窗格，循环。
    pub fn focus_next(&mut self) {
        self.cycle_focus(1);
    }

    /// 焦点移到上一个窗格，循环。
    pub fn focus_prev(&mut self) {
        self.cycle_focus(-1);
    }

    fn cycle_focus(&mut self, step: isize) {
        let ids = self.visible_pane_ids();
        if ids.is_empty() {
            return;
        }
        let current = ids.iter().position(|id| *id == self.focus).unwrap_or(0);
        let next = (current as isize + step).rem_euclid(ids.len() as isize) as usize;
        self.focus = ids[next];
    }
}

impl Default for TileLayout {
    fn default() -> Self {
        Self::new()
    }
}

/// 把焦点窗格替换为分割节点。
fn split_node(node: &mut Node, target: PaneId, direction: Direction, ratio: f32, new_id: PaneId) {
    match node {
        Node::Pane(id) if *id == target => {
            *node = Node::Split {
                direction,
                ratio,
                first: Box::new(Node::Pane(*id)),
                second: Box::new(Node::Pane(new_id)),
            };
        }
        Node::Pane(_) => {}
        Node::Split { first, second, .. } => {
            if contains(first, target) {
                split_node(first, target, direction, ratio, new_id);
            } else if contains(second, target) {
                split_node(second, target, direction, ratio, new_id);
            }
        }
    }
}

fn contains(node: &Node, target: PaneId) -> bool {
    match node {
        Node::Pane(id) => *id == target,
        Node::Split { first, second, .. } => contains(first, target) || contains(second, target),
    }
}

fn collect_ids(node: &Node, ids: &mut Vec<PaneId>) {
    match node {
        Node::Pane(id) => ids.push(*id),
        Node::Split { first, second, .. } => {
            collect_ids(first, ids);
            collect_ids(second, ids);
        }
    }
}

/// 计算每个窗格在给定区域内的矩形；折叠窗格压缩为父分割边缘的窄条。
pub fn pane_rects(layout: &TileLayout, area: Rect) -> Vec<(PaneId, Rect)> {
    let mut rects = Vec::new();
    collect_rects(&layout.root, area, layout.collapsed, &mut rects);
    rects
}

fn collect_rects(
    node: &Node,
    area: Rect,
    collapsed: Option<PaneId>,
    rects: &mut Vec<(PaneId, Rect)>,
) {
    match node {
        Node::Pane(id) => rects.push((*id, area)),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_area, second_area) = match collapsed {
                Some(id) if is_pane(first, id) => {
                    split_rect_edge(area, *direction, COLLAPSED_STRIP, false)
                }
                Some(id) if is_pane(second, id) => {
                    split_rect_edge(area, *direction, COLLAPSED_STRIP, true)
                }
                _ => split_rect(area, *direction, *ratio),
            };
            collect_rects(first, first_area, collapsed, rects);
            collect_rects(second, second_area, collapsed, rects);
        }
    }
}

/// 节点是否为指定窗格叶子。
fn is_pane(node: &Node, id: PaneId) -> bool {
    matches!(node, Node::Pane(pane) if *pane == id)
}

/// 按固定宽度分割：`second` 侧取 `len`，另一侧取剩余空间。
fn split_rect_edge(area: Rect, direction: Direction, len: u16, second: bool) -> (Rect, Rect) {
    match direction {
        Direction::Horizontal => {
            let len = len.min(area.width);
            let first_width = if second { area.width - len } else { len };
            (
                Rect {
                    width: first_width,
                    ..area
                },
                Rect {
                    x: area.x + first_width,
                    width: area.width - first_width,
                    ..area
                },
            )
        }
        Direction::Vertical => {
            let len = len.min(area.height);
            let first_height = if second { area.height - len } else { len };
            (
                Rect {
                    height: first_height,
                    ..area
                },
                Rect {
                    y: area.y + first_height,
                    height: area.height - first_height,
                    ..area
                },
            )
        }
    }
}

fn split_rect(area: Rect, direction: Direction, ratio: f32) -> (Rect, Rect) {
    match direction {
        Direction::Horizontal => {
            let first_width = ((f32::from(area.width) * ratio).round() as u16).min(area.width);
            (
                Rect {
                    width: first_width,
                    ..area
                },
                Rect {
                    x: area.x + first_width,
                    width: area.width - first_width,
                    ..area
                },
            )
        }
        Direction::Vertical => {
            let first_height = ((f32::from(area.height) * ratio).round() as u16).min(area.height);
            (
                Rect {
                    height: first_height,
                    ..area
                },
                Rect {
                    y: area.y + first_height,
                    height: area.height - first_height,
                    ..area
                },
            )
        }
    }
}

/// 窗格内容区矩形：去掉边框，可能为零尺寸。
pub fn pane_inner_rect(rect: Rect) -> Rect {
    Rect {
        x: rect.x.saturating_add(1),
        y: rect.y.saturating_add(1),
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    }
}

/// 窗格内容区尺寸：去掉边框，最小 1x1。
pub fn pane_inner_size(rect: Rect) -> (u16, u16) {
    let inner = pane_inner_rect(rect);
    (inner.width.max(1), inner.height.max(1))
}

/// 返回 `from` 指定方向上最近的窗格。
pub fn pane_in_direction(
    rects: &[(PaneId, Rect)],
    from: PaneId,
    direction: NavDirection,
) -> Option<PaneId> {
    let from_rect = rects.iter().find(|(id, _)| *id == from).map(|(_, r)| *r)?;
    let mut best: Option<(PaneId, u16)> = None;
    for (id, rect) in rects {
        if *id == from {
            continue;
        }
        let (candidate, gap) = match direction {
            NavDirection::Left => (
                rect.right() <= from_rect.x
                    && rect.y < from_rect.bottom()
                    && rect.bottom() > from_rect.y,
                from_rect.x.saturating_sub(rect.right()),
            ),
            NavDirection::Right => (
                rect.x >= from_rect.right()
                    && rect.y < from_rect.bottom()
                    && rect.bottom() > from_rect.y,
                rect.x.saturating_sub(from_rect.right()),
            ),
            NavDirection::Up => (
                rect.bottom() <= from_rect.y
                    && rect.x < from_rect.right()
                    && rect.right() > from_rect.x,
                from_rect.y.saturating_sub(rect.bottom()),
            ),
            NavDirection::Down => (
                rect.y >= from_rect.bottom()
                    && rect.x < from_rect.right()
                    && rect.right() > from_rect.x,
                rect.y.saturating_sub(rect.bottom()),
            ),
        };
        if candidate && best.is_none_or(|(_, best_gap)| gap < best_gap) {
            best = Some((*id, gap));
        }
    }
    best.map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let rects = pane_rects(&layout, area(80, 24));
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].0, layout.focus());
        assert_eq!(rects[0].1, area(80, 24));
    }

    #[test]
    fn nested_splits_produce_expected_rects() {
        let (layout, left, right, bottom) = demo_layout();
        let rects = pane_rects(&layout, area(80, 20));
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
        let rects = pane_rects(&layout, area(80, 20));
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

    #[test]
    fn collapsed_right_pane_becomes_edge_strip() {
        let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
        assert!(layout.set_collapsed(right, true));
        let rects = pane_rects(&layout, area(80, 20));
        assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 77, 20)));
        assert_eq!(rect_of(&rects, right), Some(Rect::new(77, 0, 3, 20)));
    }

    #[test]
    fn collapsed_left_pane_becomes_left_strip() {
        let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
        assert!(layout.set_collapsed(left, true));
        let rects = pane_rects(&layout, area(80, 20));
        assert_eq!(rect_of(&rects, left), Some(Rect::new(0, 0, 3, 20)));
        assert_eq!(rect_of(&rects, right), Some(Rect::new(3, 0, 77, 20)));
    }

    #[test]
    fn collapsed_bottom_pane_becomes_bottom_strip() {
        let (mut layout, top, bottom) = two_pane_layout(Direction::Vertical);
        assert!(layout.set_collapsed(bottom, true));
        let rects = pane_rects(&layout, area(80, 20));
        assert_eq!(rect_of(&rects, top), Some(Rect::new(0, 0, 80, 17)));
        assert_eq!(rect_of(&rects, bottom), Some(Rect::new(0, 17, 80, 3)));
    }

    #[test]
    fn expand_restores_full_rect() {
        let (mut layout, left, right) = two_pane_layout(Direction::Horizontal);
        assert!(layout.set_collapsed(right, true));
        assert!(layout.set_collapsed(right, false));
        let rects = pane_rects(&layout, area(80, 20));
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
}
