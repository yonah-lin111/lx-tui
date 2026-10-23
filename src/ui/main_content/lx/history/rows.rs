//! 历史行几何的共用小工具：宽度换算、可见行判定与软换行。
//!
//! 从 `history.rs` 拆出（单文件行数上限）。布局与绘制必须共用同一份口径，
//! 所以这些函数只在这里定义一次，两边都从这里取。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::ui::markdown_render;

use super::{SCROLLBAR_COLUMNS, STEP_GUTTER, USER_INDENT, tool};

/// 行数 → 矩形高度（饱和转换：极端长历史不会回绕成 0）。
pub(crate) fn rows_to_u16(rows: usize) -> u16 {
    u16::try_from(rows).unwrap_or(u16::MAX)
}

/// 历史内容列（扣除右侧滚动条预留列）；宽度不足时退回整宽。
pub(crate) fn content_rect(area: Rect) -> Rect {
    if area.width > SCROLLBAR_COLUMNS {
        Rect {
            width: area.width - SCROLLBAR_COLUMNS,
            ..area
        }
    } else {
        area
    }
}

/// 消息正文渲染行数：assistant 用 markdown 渲染（宽度自适应，扣步骤标记缩进），
/// 用户按纯文本换行（扣 `> ` 前缀）。
pub(crate) fn text_rows(
    message: &crate::app::state::ChatMessageView,
    content: &str,
    width: u16,
) -> usize {
    if message.role == "user" {
        return wrap(content, width.saturating_sub(USER_INDENT)).len();
    }
    markdown_rows(content, width.saturating_sub(STEP_GUTTER))
}

/// assistant markdown 正文渲染成若干行（已按 `width` 自适应：表格收缩、其余软换行）。
fn markdown_rows(content: &str, width: u16) -> usize {
    markdown_render::render_wrapped(content, width).len()
}

/// 工具步骤占用的渲染行数。
///
/// `layout` 与 `render` 共用本入口（内部即 `tool::rows`）：工具行一旦换行，滚动条、
/// 置顶用户行与 thinking 命中矩形都会漂移。`expanded` 影响带差异正文的文件卡片、
/// 进程卡片与计划卡片；没有差异正文的文件工具恒为 2 行。
pub(crate) fn tool_rows(
    call: &crate::app::state::ToolCallView,
    width: u16,
    folds: tool::Folds,
) -> usize {
    tool::rows(call, width, folds)
}

/// 行可见时返回其屏幕 Y。
pub(crate) fn visible_row(rect: Rect, scroll: usize, row: usize) -> Option<u16> {
    if row < scroll {
        return None;
    }
    let offset = u16::try_from(row - scroll).unwrap_or(u16::MAX);
    let screen_y = rect.y.saturating_add(offset);
    (screen_y < rect.bottom()).then_some(screen_y)
}

/// 按内容宽度换行（保留原换行；宽字符按两列计）。
pub(crate) fn wrap(content: &str, width: u16) -> Vec<String> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();
    for line in content.split('\n') {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut col = 0usize;
        for ch in line.chars() {
            let cell = ch.width().unwrap_or(1).max(1);
            if col + cell > width && col > 0 {
                out.push(std::mem::take(&mut current));
                col = 0;
            }
            current.push(ch);
            col += cell;
        }
        out.push(current);
    }
    out
}
