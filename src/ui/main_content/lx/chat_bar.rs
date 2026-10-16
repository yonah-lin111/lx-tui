//! 输入框上方聊天栏：流中 loading 转轮 + 右端「回到底部」按钮。
//!
//! 纯渲染 + 纯几何：布局、绘制与命中共享同一矩形口径；只读 `Lx`、只写 lx 页网格。
//! 该行贴在输入框上沿、与输入框同宽；无流中且已跟随底部时整行不占位。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::Lx;
use crate::ui::{style, text};

use super::Grid;

/// 聊天栏高度（一行）。
pub(crate) const HEIGHT: u16 = 1;

/// 是否显示聊天栏：流中（loading 转轮）或上滚未跟随底部（回到底部按钮）。
pub(super) fn visible(lx: &Lx) -> bool {
    if lx.chat.messages.is_empty() {
        return false;
    }
    lx.chat.status.streaming || !lx.chat.follow_bottom
}

/// 由输入框矩形推导聊天栏矩形（贴其上沿）；上方空间不足时 None。
pub(super) fn rect(area: Rect, input: Rect) -> Option<Rect> {
    (input.y >= area.y.saturating_add(HEIGHT))
        .then(|| Rect::new(input.x, input.y - HEIGHT, input.width, HEIGHT))
}

/// 回到底部按钮矩形（未跟随底部且空间足够时）；供命中与测试。
pub(crate) fn jump_bottom_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    if lx.chat.follow_bottom || lx.chat.messages.is_empty() {
        return None;
    }
    let input = super::input_rect(area, lx)?;
    let bar = rect(area, input)?;
    jump_rect(bar)
}

/// 栏内「回到底部」按钮矩形：右端留 1 列空档。
fn jump_rect(bar: Rect) -> Option<Rect> {
    let width = u16::try_from(text::LX_JUMP_BOTTOM.width()).unwrap_or(u16::MAX);
    (bar.width > width + 1)
        .then(|| Rect::new(bar.right().saturating_sub(width + 1), bar.y, width, 1))
}

/// 绘制聊天栏：左侧 loading 转轮 + 文案，右端回到底部按钮。
///
/// 流中显示 `⠋ Working`（旋转帧由 `loading_frame` 决定，仅一帧渲染）；
/// 未跟随底部时右端显示 `↓`（悬停高亮）；两种状态可同时出现。
pub(super) fn draw(grid: &mut Grid, bar: Option<Rect>, lx: &Lx) {
    let Some(bar) = bar else {
        return;
    };
    if bar.width == 0 {
        return;
    }
    let mut right = bar.right();
    // 回到底部按钮（仅未跟随底部时）。
    if !lx.chat.follow_bottom
        && let Some(button) = jump_rect(bar)
    {
        grid.put_clipped(
            button.x,
            button.y,
            text::LX_JUMP_BOTTOM,
            style::chat_jump_bottom(lx.chat.jump_bottom_hover),
            button.width,
        );
        right = button.x.saturating_sub(1);
    }
    if !lx.chat.status.streaming {
        return;
    }
    // loading：转轮 + 文案；右侧留 1 列空档避免贴边。
    let frame = text::LX_LOADING_FRAMES[lx.chat.loading_frame % text::LX_LOADING_FRAMES.len()];
    let mut x = bar.x.saturating_add(1);
    let limit = right.saturating_sub(1);
    if x < limit {
        grid.put_char(x, bar.y, frame, style::chat_loading_spinner());
        x = x.saturating_add(2);
    }
    if x < limit {
        grid.put_clipped(
            x,
            bar.y,
            text::LX_LOADING_LABEL,
            style::chat_loading_label(),
            limit - x,
        );
    }
}

/// 命中「回到底部」按钮的窗格；无按钮或未命中返回 None。
pub fn jump_bottom_hit(
    rects: &[(crate::layout::PaneId, Rect)],
    state: &crate::app::state::AppState,
    column: u16,
    row: u16,
) -> Option<crate::layout::PaneId> {
    rects.iter().find_map(|(id, rect)| {
        let target = state
            .pane_anywhere(*id)
            .filter(|pane| pane.view == crate::app::state::PaneView::Lx)?;
        let inner = crate::layout::pane_inner_rect(*rect);
        jump_bottom_rect(inner, &target.lx)
            .filter(|area| area.contains((column, row).into()))
            .map(|_| *id)
    })
}
