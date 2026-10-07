//! BSP 平铺布局模型与几何计算：不涉及渲染，可在无终端环境下测试。

use std::sync::atomic::{AtomicU32, Ordering};

use ratatui::layout::{Direction, Rect};

/// 折叠窄条宽度：3 个内容列 + 1 列贴主区的分隔线。
pub const COLLAPSED_STRIP: u16 = 4;

/// 窗格唯一标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneId(u32);

static NEXT_PANE_ID: AtomicU32 = AtomicU32::new(1);

impl PaneId {
    /// 分配全局唯一的窗格标识；全局 prompt 面板也使用窗格标识参与选择。
    pub(crate) fn alloc() -> Self {
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

    /// 把 `left` 与 `right` 相邻窗格之间的水平分割线拖到 `boundary_x`。
    ///
    /// `area` 为窗格区矩形；两侧窗格各保持至少 `min_width` 列（含边框），
    /// 可用宽度不足 `2 * min_width` 时不做调整并返回 false。
    pub fn resize_boundary(
        &mut self,
        left: PaneId,
        right: PaneId,
        area: Rect,
        boundary_x: u16,
        min_width: u16,
    ) -> bool {
        resize_boundary_node(
            &mut self.root,
            left,
            right,
            area,
            boundary_x,
            min_width,
            self.collapsed,
        )
    }

    /// 删除窗格叶节点并把兄弟子树提升为父节点；返回是否删除。
    ///
    /// 仅剩一个窗格或窗格不存在时返回 false（调用方保证至少保留一个）；
    /// 被删窗格处于折叠态时清除折叠；被删窗格持有焦点时，
    /// 焦点落到提升兄弟子树的树序第一个窗格。
    pub fn remove_pane(&mut self, id: PaneId) -> bool {
        let ids = self.pane_ids();
        if ids.len() <= 1 || !ids.contains(&id) {
            return false;
        }
        if self.collapsed == Some(id) {
            self.collapsed = None;
        }
        let root = std::mem::replace(&mut self.root, Node::Pane(id));
        let (root, promoted) = remove_node(root, id);
        self.root = root;
        if self.focus == id
            && let Some(next) = promoted
        {
            self.focus = next;
        }
        true
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

/// 删除目标叶节点并提升兄弟子树；返回新树与提升子树的树序第一个窗格。
fn remove_node(node: Node, target: PaneId) -> (Node, Option<PaneId>) {
    let Node::Split {
        direction,
        ratio,
        first,
        second,
    } = node
    else {
        return (node, None);
    };
    if is_pane(&first, target) {
        let promoted = first_id(&second);
        return (*second, Some(promoted));
    }
    if is_pane(&second, target) {
        let promoted = first_id(&first);
        return (*first, Some(promoted));
    }
    let (first, promoted) = remove_node(*first, target);
    if promoted.is_some() {
        return (
            Node::Split {
                direction,
                ratio,
                first: Box::new(first),
                second,
            },
            promoted,
        );
    }
    let (second, promoted) = remove_node(*second, target);
    (
        Node::Split {
            direction,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        },
        promoted,
    )
}

/// 子树树序第一个窗格标识。
fn first_id(node: &Node) -> PaneId {
    match node {
        Node::Pane(id) => *id,
        Node::Split { first, .. } => first_id(first),
    }
}

/// 计算每个窗格在给定区域内的矩形；折叠窗格压缩为父分割边缘的窄条。
///
/// 水平分割的两侧宽度不小于 `min_width`（含边框）；可用宽度不足时退回按比例。
pub fn pane_rects(layout: &TileLayout, area: Rect, min_width: u16) -> Vec<(PaneId, Rect)> {
    let mut rects = Vec::new();
    collect_rects(&layout.root, area, layout.collapsed, min_width, &mut rects);
    rects
}

fn collect_rects(
    node: &Node,
    area: Rect,
    collapsed: Option<PaneId>,
    min_width: u16,
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
            let (first_area, second_area) = child_areas(
                *direction, *ratio, area, collapsed, first, second, min_width,
            );
            collect_rects(first, first_area, collapsed, min_width, rects);
            collect_rects(second, second_area, collapsed, min_width, rects);
        }
    }
}

/// 分割节点的两个子区域；折叠子节点退化为固定窄条。
fn child_areas(
    direction: Direction,
    ratio: f32,
    area: Rect,
    collapsed: Option<PaneId>,
    first: &Node,
    second: &Node,
    min_width: u16,
) -> (Rect, Rect) {
    match collapsed {
        Some(id) if is_pane(first, id) => split_rect_edge(area, direction, COLLAPSED_STRIP, false),
        Some(id) if is_pane(second, id) => split_rect_edge(area, direction, COLLAPSED_STRIP, true),
        _ => split_rect(area, direction, ratio, min_width),
    }
}

/// 在树中定位包含 `left`/`right` 的水平分割节点并更新 ratio。
fn resize_boundary_node(
    node: &mut Node,
    left: PaneId,
    right: PaneId,
    area: Rect,
    boundary_x: u16,
    min_width: u16,
    collapsed: Option<PaneId>,
) -> bool {
    let Node::Split {
        direction,
        ratio,
        first,
        second,
    } = node
    else {
        return false;
    };
    if contains(first, left) && contains(second, right) {
        if *direction != Direction::Horizontal || area.width < min_width.saturating_mul(2) {
            return false;
        }
        let first_width = boundary_x
            .saturating_sub(area.x)
            .clamp(min_width, area.width - min_width);
        *ratio = f32::from(first_width) / f32::from(area.width);
        return true;
    }
    if contains(first, left) && contains(first, right) {
        let (first_area, _) = child_areas(
            *direction, *ratio, area, collapsed, first, second, min_width,
        );
        return resize_boundary_node(
            first, left, right, first_area, boundary_x, min_width, collapsed,
        );
    }
    if contains(second, left) && contains(second, right) {
        let (_, second_area) = child_areas(
            *direction, *ratio, area, collapsed, first, second, min_width,
        );
        return resize_boundary_node(
            second,
            left,
            right,
            second_area,
            boundary_x,
            min_width,
            collapsed,
        );
    }
    false
}

/// 命中相邻两列区域边框：返回边界两侧的 (左区域, 右区域)。
///
/// 只命中边框列本身（左区域 `right()-1` 或右区域 `x`），不侵入内容区；
/// 零尺寸区域不参与。窗格与全局 prompt 右栏共用同一命中规则。
pub fn resize_boundary_at(
    rects: &[(PaneId, Rect)],
    column: u16,
    row: u16,
) -> Option<(PaneId, PaneId)> {
    let hovered =
        |rect: &Rect| rect.width > 0 && rect.height > 0 && rect.contains((column, row).into());
    let row_hit =
        |rect: &Rect| rect.width > 0 && rect.height > 0 && row >= rect.y && row < rect.bottom();

    // 命中列若是右区域的左边框，边界即该列；若是左区域的右边框，边界在下一列。
    let boundary = rects
        .iter()
        .find_map(|(_, rect)| (hovered(rect) && rect.x == column).then_some(column))
        .or_else(|| {
            rects.iter().find_map(|(_, rect)| {
                (hovered(rect) && rect.right().saturating_sub(1) == column)
                    .then_some(column.saturating_add(1))
            })
        })?;

    let left = rects
        .iter()
        .find_map(|(id, rect)| (row_hit(rect) && rect.right() == boundary).then_some(*id))?;
    let right = rects
        .iter()
        .find_map(|(id, rect)| (row_hit(rect) && rect.x == boundary).then_some(*id))?;
    Some((left, right))
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

/// 按比例分割；水平分割时首侧宽度钳制在 `[min_width, width - min_width]`。
///
/// 可用宽度不足 `2 * min_width` 时退回纯比例分割。
fn split_rect(area: Rect, direction: Direction, ratio: f32, min_width: u16) -> (Rect, Rect) {
    match direction {
        Direction::Horizontal => {
            let mut first_width = ((f32::from(area.width) * ratio).round() as u16).min(area.width);
            if area.width >= min_width.saturating_mul(2) {
                first_width = first_width.clamp(min_width, area.width - min_width);
            }
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

/// prompt 文本区矩形：内容区最右一列固定预留给滚动条槽（内容区宽度 >= 2 时）。
pub fn prompt_text_rect(panel: Rect) -> Rect {
    let inner = pane_inner_rect(panel);
    if inner.width >= 2 {
        Rect {
            width: inner.width - 1,
            ..inner
        }
    } else {
        inner
    }
}

/// prompt 滚动条槽矩形：内容区最右一列；内容区过窄时为 None。
pub fn prompt_scrollbar_rect(panel: Rect) -> Option<Rect> {
    let inner = pane_inner_rect(panel);
    (inner.width >= 2 && inner.height > 0).then_some(Rect::new(
        inner.right() - 1,
        inner.y,
        1,
        inner.height,
    ))
}

/// prompt 文本区尺寸：预留滚动条槽，最小 1x1。
pub fn prompt_inner_size(rect: Rect) -> (u16, u16) {
    let text = prompt_text_rect(rect);
    (text.width.max(1), text.height.max(1))
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
mod tests;
