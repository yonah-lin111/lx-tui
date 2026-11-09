//! 输入框上方的 pending steer 提示区：每条一行，最多 [`MAX_VISIBLE`] 行。
//!
//! 纯几何 + 纯渲染：只读 `Lx`、只写 lx 页网格；pending 属于临时交互态，不进入
//! 历史、不写选区、不参与整页复制。行内 `×` 固定在最右一列，命中只覆盖这一列，
//! 点文本其它位置不拦截（继续走历史/输入命中）。
//!
//! 单行格式：`Steering  <截断文本>  · N attachments  ×`；超过可见条数时末行改为
//! `+N more pending` 且不可点击。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::lx::chat::steer::PendingSteer;
use crate::app::state::{Lx, RequestState};
use crate::ui::{style, text};

use super::Grid;

/// 提示区最多占用的行数。
pub(crate) const MAX_VISIBLE: usize = 3;
/// 行首标签与正文之间的空档（列）。
const LABEL_GAP: u16 = 2;
/// 正文与 `×` 之间的空档（列）。
const CANCEL_GAP: u16 = 1;
/// 正文与附件后缀之间的分隔（列宽参与预算）。
const ATTACHMENTS_SEP: &str = "  · ";
/// `×` 字形。
const CANCEL: char = '×';

/// 当前要展示的 pending 列表；task/ask 打开时整块隐藏。
fn pending(lx: &Lx) -> &[PendingSteer] {
    if lx.task.is_some() || lx.ask.is_some() {
        return &[];
    }
    &lx.chat.pending_steers
}

/// 提示区高度（行）；无 pending 或 task/ask 打开时为 0。
pub(crate) fn height(lx: &Lx) -> u16 {
    u16::try_from(pending(lx).len().min(MAX_VISIBLE)).unwrap_or(0)
}

/// 绘制提示区；`rect` 由 [`super::layout`] 给出，宽度与输入框一致。
pub(crate) fn draw(grid: &mut Grid, rect: Rect, lx: &Lx) {
    let pending = pending(lx);
    if rect.width == 0 || rect.height == 0 || pending.is_empty() {
        return;
    }
    let rows = usize::from(rect.height).min(pending.len()).min(MAX_VISIBLE);
    // 超过可见条数时最后一行改画汇总，保证不无限挤占历史。
    let more = pending.len() > MAX_VISIBLE && rows == MAX_VISIBLE;
    let steer_rows = if more { rows - 1 } else { rows };
    for (row, steer) in pending.iter().take(steer_rows).enumerate() {
        let y = rect
            .y
            .saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
        draw_steer(grid, rect, y, steer);
    }
    if more {
        let y = rect
            .y
            .saturating_add(u16::try_from(steer_rows).unwrap_or(u16::MAX));
        let label = text::lx_steer_more(pending.len() - steer_rows);
        grid.put_clipped(rect.x, y, &label, style::muted(), rect.width);
    }
}

/// 画一条 steer：标签、单行截断正文、附件后缀与最右列 `×`。
fn draw_steer(grid: &mut Grid, rect: Rect, y: u16, steer: &PendingSteer) {
    let muted = style::muted();
    let cancel_x = rect.right().saturating_sub(1);
    let label_width = u16::try_from(text::LX_STEER_LABEL.width()).unwrap_or(u16::MAX);
    // 标签优先保留：先裁掉 `×` 所在列，正文再按剩余宽度决定是否出现。
    grid.put_clipped(
        rect.x,
        y,
        text::LX_STEER_LABEL,
        muted,
        rect.width.saturating_sub(1),
    );
    let text_x = rect.x.saturating_add(label_width).saturating_add(LABEL_GAP);
    let text_right = cancel_x.saturating_sub(CANCEL_GAP);
    if text_x < text_right {
        let total = text_right - text_x;
        let mut suffix = String::new();
        let mut body_budget = total;
        if !steer.attachments.is_empty() {
            let candidate = format!(
                "{ATTACHMENTS_SEP}{}",
                text::lx_steer_attachments(steer.attachments.len())
            );
            let width = u16::try_from(candidate.width()).unwrap_or(u16::MAX);
            if width <= total {
                body_budget = total.saturating_sub(width);
                suffix = candidate;
            }
        }
        let body = if body_budget > 0 {
            folded_prefix(&steer.text, body_budget)
        } else {
            String::new()
        };
        let body_width = u16::try_from(body.width()).unwrap_or(u16::MAX);
        grid.put_clipped(text_x, y, &body, muted, body_budget);
        if !suffix.is_empty() {
            let suffix_x = text_x.saturating_add(body_width);
            grid.put_clipped(
                suffix_x,
                y,
                &suffix,
                muted,
                text_right.saturating_sub(suffix_x),
            );
        }
    }
    grid.put_char(cancel_x, y, CANCEL, muted);
}

/// 单行化并截断到 `budget` 列：换行/制表/控制字符与连续空白折叠为单个空格，
/// 超宽时以 `…` 收尾（宽字符不半写）。
fn folded_prefix(text: &str, budget: u16) -> String {
    let budget = usize::from(budget);
    if budget == 0 {
        return String::new();
    }
    let mut chars: Vec<char> = Vec::new();
    let mut width = 0usize;
    let mut pending_space = false;
    let mut truncated = false;
    for original in text.chars() {
        let ch = if original.is_control() { ' ' } else { original };
        if ch.is_whitespace() {
            pending_space = !chars.is_empty();
            continue;
        }
        let cell = ch.width().unwrap_or(0);
        if cell == 0 {
            continue;
        }
        let lead = usize::from(pending_space);
        if width.saturating_add(lead).saturating_add(cell) > budget {
            truncated = true;
            break;
        }
        if pending_space {
            chars.push(' ');
            width += 1;
            pending_space = false;
        }
        chars.push(ch);
        width += cell;
    }
    if truncated {
        // 预留一列给省略号；被截掉的宽字符整字退出，绝不只画一半。
        while width.saturating_add(1) > budget {
            let Some(last) = chars.pop() else {
                break;
            };
            width = width.saturating_sub(last.width().unwrap_or(0));
        }
        while chars.last().is_some_and(|ch| ch.is_whitespace()) {
            chars.pop();
            width = width.saturating_sub(1);
        }
        chars.push('…');
    }
    chars.into_iter().collect()
}

/// `×` 命中：仅 `Streaming` 且该行确有可撤回的 steer 时返回其原索引。
///
/// 最后一行是 `+N more pending` 汇总（不可点击），因此超过 [`MAX_VISIBLE`] 时
/// 只有前 `MAX_VISIBLE - 1` 行可命中；索引与 `pending_steers` 的 Vec 对齐。
pub(crate) fn recall_hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    if !matches!(lx.chat.request, RequestState::Streaming { .. }) {
        return None;
    }
    let rect = super::layout(area, lx).pending?;
    if !rect.contains((column, row).into()) {
        return None;
    }
    if column != rect.right().saturating_sub(1) {
        return None;
    }
    let total = lx.chat.pending_steers.len();
    let steer_rows = if total > MAX_VISIBLE {
        MAX_VISIBLE - 1
    } else {
        total.min(MAX_VISIBLE)
    };
    let index = usize::from(row.saturating_sub(rect.y));
    (index < steer_rows).then_some(index)
}

#[cfg(test)]
mod tests;
