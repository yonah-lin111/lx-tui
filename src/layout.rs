//! BSP 平铺布局模型与几何计算：不涉及渲染，可在无终端环境下测试。

use std::sync::atomic::{AtomicU32, Ordering};

use ratatui::layout::{Direction, Rect};

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

/// 平铺布局：记录 BSP 树与焦点窗格。
#[derive(Debug, Clone)]
pub struct TileLayout {
    root: Node,
    focus: PaneId,
}

impl TileLayout {
    /// 创建只含一个窗格的布局。
    pub fn new() -> Self {
        let id = PaneId::alloc();
        Self {
            root: Node::Pane(id),
            focus: id,
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

    /// 聚焦指定窗格；窗格不存在时返回 false。
    pub fn focus_pane(&mut self, id: PaneId) -> bool {
        if self.pane_ids().contains(&id) {
            self.focus = id;
            true
        } else {
            false
        }
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
        let ids = self.pane_ids();
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

/// 计算每个窗格在给定区域内的矩形。
pub fn pane_rects(layout: &TileLayout, area: Rect) -> Vec<(PaneId, Rect)> {
    let mut rects = Vec::new();
    collect_rects(&layout.root, area, &mut rects);
    rects
}

fn collect_rects(node: &Node, area: Rect, rects: &mut Vec<(PaneId, Rect)>) {
    match node {
        Node::Pane(id) => rects.push((*id, area)),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_area, second_area) = split_rect(area, *direction, *ratio);
            collect_rects(first, first_area, rects);
            collect_rects(second, second_area, rects);
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

/// 窗格内容区尺寸：去掉边框，最小 1x1。
pub fn pane_inner_size(rect: Rect) -> (u16, u16) {
    (
        rect.width.saturating_sub(2).max(1),
        rect.height.saturating_sub(2).max(1),
    )
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
}
