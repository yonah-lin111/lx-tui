//! 输入框上方的 pending steer 提示区：每条一行，最多 [`MAX_VISIBLE`] 行。
//!
//! 纯几何 + 纯渲染：只读 `Lx`、只写 lx 页网格；pending 属于临时交互态，不进入
//! 历史、不写选区、不参与整页复制。流中按单次 `Esc` 即可按 LIFO 依次回退并回显到输入框。
//!
//! 单行格式：`Steering  <截断文本>  · N attachments`；超过可见条数时末行改为
//! `+N more pending`。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::Lx;
use crate::app::state::lx::chat::steer::PendingSteer;
use crate::ui::{style, text};

use super::Grid;

/// 提示区最多占用的行数。
pub(crate) const MAX_VISIBLE: usize = 3;
/// 行首标签与正文之间的空档（列）。
const LABEL_GAP: u16 = 2;
/// 正文与附件后缀之间的分隔（列宽参与预算）。
const ATTACHMENTS_SEP: &str = "  · ";

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

/// 画一条 steer：标签、单行截断正文与附件后缀（无关闭按钮，统一由单次 `Esc` 回退）。
fn draw_steer(grid: &mut Grid, rect: Rect, y: u16, steer: &PendingSteer) {
    let muted = style::muted();
    let label_width = u16::try_from(text::LX_STEER_LABEL.width()).unwrap_or(u16::MAX);
    grid.put_clipped(rect.x, y, text::LX_STEER_LABEL, muted, rect.width);
    let text_x = rect.x.saturating_add(label_width).saturating_add(LABEL_GAP);
    let text_right = rect.right();
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

#[cfg(test)]
mod tests;
