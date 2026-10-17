//! 输入框上方聊天栏：流中 loading 转轮 + 右端「回到底部」控件。
//!
//! 纯渲染 + 纯几何：布局、绘制与命中共享同一矩形口径；只读 `Lx`、只写 lx 页网格。
//! 该行贴在输入框上沿、与输入框同宽；无流中且已跟随底部时整行不占位。
//! 「回到底部」控件为箭头 + 右侧说明文字（`↓ jump to bottom`），整体右对齐。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::Lx;
use crate::ui::{style, text};

use super::Grid;

/// 聊天栏高度（一行）。
pub(crate) const HEIGHT: u16 = 1;
/// 右侧控件与栏右缘之间的空档（列）。
const RIGHT_PAD: u16 = 1;
/// 箭头与说明文字之间的空档（列）。
const HINT_GAP: u16 = 1;

/// 是否显示聊天栏：流中（loading 转轮）或上滚未跟随底部（回到底部控件）。
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

/// 回到底部控件矩形（未跟随底部且空间足够时）；供命中与测试。
pub(crate) fn jump_bottom_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    if lx.chat.follow_bottom || lx.chat.messages.is_empty() {
        return None;
    }
    let input = super::input_rect(area, lx)?;
    jump_rect(rect(area, input)?)
}

/// 栏内「回到底部」控件矩形（整体右对齐）。
///
/// 优先「箭头 + 空档 + 说明」；宽度只够箭头时退回纯箭头；再窄返回 None。
fn jump_rect(bar: Rect) -> Option<Rect> {
    let arrow = u16::try_from(text::LX_JUMP_BOTTOM.width()).unwrap_or(u16::MAX);
    let hint = u16::try_from(text::LX_JUMP_BOTTOM_HINT.width()).unwrap_or(u16::MAX);
    let width = arrow
        .saturating_add(HINT_GAP)
        .saturating_add(hint)
        .saturating_add(RIGHT_PAD);
    if bar.width >= width {
        let inner = arrow.saturating_add(HINT_GAP).saturating_add(hint);
        return Some(Rect::new(bar.right() - RIGHT_PAD - inner, bar.y, inner, 1));
    }
    (bar.width > arrow + RIGHT_PAD)
        .then(|| Rect::new(bar.right() - RIGHT_PAD - arrow, bar.y, arrow, 1))
}

/// 绘制聊天栏：左侧 loading 转轮 + 文案，右端「回到底部」控件。
///
/// 流中显示 `⠋ Working`（旋转帧由 `loading_frame` 决定，仅一帧渲染）；
/// Esc 待命（`stop_armed_at`）时文案换成 `Press esc again to stop`（按宽度取短）；
/// 未跟随底部时右端显示 `↓ 说明`（悬停高亮）；各状态可同时出现。
pub(super) fn draw(grid: &mut Grid, bar: Option<Rect>, lx: &Lx) {
    let Some(bar) = bar else {
        return;
    };
    if bar.width == 0 {
        return;
    }
    let mut right = bar.right();
    // 回到底部控件（仅未跟随底部时）。
    if !lx.chat.follow_bottom
        && let Some(button) = jump_rect(bar)
    {
        let jump_style = style::chat_jump_bottom(lx.chat.jump_bottom_hover);
        grid.put_clipped(
            button.x,
            button.y,
            text::LX_JUMP_BOTTOM,
            jump_style,
            button.width,
        );
        // 说明文字在箭头右侧（宽度只够箭头时跳过）。
        let arrow = u16::try_from(text::LX_JUMP_BOTTOM.width()).unwrap_or(u16::MAX);
        let hint_x = button.x.saturating_add(arrow).saturating_add(HINT_GAP);
        if hint_x < button.right() {
            grid.put_clipped(
                hint_x,
                button.y,
                text::LX_JUMP_BOTTOM_HINT,
                jump_style,
                button.right() - hint_x,
            );
        }
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
        // Esc 待命：提示「再按一次停止」（取第一条放得下的短语，对齐 opencode）。
        let label = if lx.chat.stop_armed_at.is_some() {
            stop_phrase(limit.saturating_sub(x))
        } else {
            text::LX_LOADING_LABEL
        };
        grid.put_clipped(x, bar.y, label, style::chat_loading_label(), limit - x);
    }
}

/// Esc 待命提示短语：从长到短取第一条放得下的；都放不下时用最短的（由调用方裁剪）。
fn stop_phrase(width: u16) -> &'static str {
    let width = usize::from(width);
    text::LX_CHAT_STOP_PHRASES
        .iter()
        .find(|phrase| phrase.width() <= width)
        .copied()
        .unwrap_or_else(|| text::LX_CHAT_STOP_PHRASES[text::LX_CHAT_STOP_PHRASES.len() - 1])
}

/// 命中「回到底部」控件的窗格；无控件或未命中返回 None。
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
