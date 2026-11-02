//! 底部多行输入框：上下 `─` 分割线 + 首行 `> ` 前缀 + markdown 高亮。
//!
//! 高亮只做**编辑器色板**（`style::markdown_*`，与阅读渲染的 GitHub Dark 色板
//! 完全独立，见 `docs/standards/tui-design-requirements.md`）：token 来自
//! `ui::markdown::scan`，样式统一经 `ui::prompt::token_style` 取用，保证输入框与
//! 全局 prompt 容器两处读法一致。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；返回光标绝对坐标供上层换算。

use ratatui::style::Modifier;
use unicode_width::UnicodeWidthChar;

use crate::app::state::Lx;
use crate::ui::markdown::Token;
use crate::ui::{style, text};

use super::grid::{CONTINUATION, Grid};
use super::{INPUT_PREFIX, INPUT_SCROLLBAR, RULE};

/// 输入框顶边框右上角会话标题胶囊的最大正文显示列宽（不含左右各 1 列内边距空格）。
const SESSION_TITLE_MAX_COLS: u16 = 40;

/// 在输入框顶边框（`rect.y`）右上角边框内部绘制当前会话标题胶囊（如 ` 标题 `，最右侧保留 1 列 `─`）。
fn draw_session_title_badge(grid: &mut Grid, rect: ratatui::layout::Rect, lx: &Lx) {
    let Some(raw_title) = lx.chat.session_title.as_deref() else {
        return;
    };
    let clean = text::chat::single_line_label(raw_title);
    if clean.is_empty() {
        return;
    }
    // 左侧至少留 INPUT_PREFIX (2) 列 `─`，右侧留 1 列 `─`，胶囊左右各 1 列空格内边距（共扣 5 列）。
    let avail = rect
        .width
        .saturating_sub(INPUT_PREFIX.saturating_add(3))
        .min(SESSION_TITLE_MAX_COLS);
    if avail < 2 {
        return;
    }
    let truncated = text::truncate_to_width(&clean, usize::from(avail));
    if truncated.is_empty() {
        return;
    }
    let badge = format!(" {truncated} ");
    let badge_w = u16::try_from(display_width(&badge)).unwrap_or(u16::MAX);
    if rect.width <= badge_w.saturating_add(INPUT_PREFIX) {
        return;
    }
    let x = rect.right().saturating_sub(badge_w).saturating_sub(1);
    let badge_style = style::session_title_badge(lx.chat.session_title_palette);
    grid.put(x, rect.y, &badge, badge_style);
}

/// 绘制输入框并返回光标绝对坐标（`(row, column)`）；未聚焦时为 None。
pub(super) fn draw(
    grid: &mut Grid,
    rect: ratatui::layout::Rect,
    lx: &Lx,
    focused: bool,
    scroll: usize,
    visible: usize,
) -> Option<(u16, u16)> {
    let text_top = rect.y + 1;
    let bottom_rule = rect.bottom() - 1;
    for x in rect.x..rect.right() {
        grid.set(x, rect.y, RULE, style::muted());
        grid.set(x, bottom_rule, RULE, style::muted());
    }
    draw_session_title_badge(grid, rect, lx);

    let content_x = rect.x + INPUT_PREFIX;
    let content_w = rect
        .width
        .saturating_sub(INPUT_PREFIX)
        .saturating_sub(INPUT_SCROLLBAR);
    let rows = lx.visual_rows();
    let shell = lx.shell_mode();
    // Shell 正文按原始脚本绘制：不做 Markdown 词法高亮。
    let tokens = if shell {
        Vec::new()
    } else {
        crate::ui::markdown::scan(&lx.input)
    };
    let lines: Vec<&str> = lx.input.split('\n').collect();
    // 规则行右端的模式/排队提示：窄宽度先隐藏，绝不改变输入框高度。
    let queued = lx.chat.queue.len();
    let label = match (shell, queued) {
        (true, 0) => Some(text::LX_INPUT_SHELL_MODE.to_string()),
        (true, n) => Some(format!("{} · {n} queued", text::LX_INPUT_SHELL_MODE)),
        (false, 0) => None,
        (false, n) => Some(format!("{n} queued")),
    };
    if let Some(label) = label {
        let width = u16::try_from(display_width(&label)).unwrap_or(u16::MAX);
        if rect.width > width.saturating_add(INPUT_PREFIX).saturating_add(2) {
            let x = rect.right().saturating_sub(width).saturating_sub(1);
            let label_style = if shell {
                style::shell_prompt()
            } else {
                style::muted()
            };
            grid.put(x, bottom_rule, &label, label_style);
        }
    }
    let (cursor_row, _) = lx.cursor_visual();
    let selection = lx.text_selection_range();

    let mut cursor = None;
    for i in 0..visible {
        let Some(row_index) = scroll.checked_add(i) else {
            break;
        };
        let Some(row) = rows.get(row_index) else {
            break;
        };
        let y = text_top + i as u16;
        let line_index = lx.input[..row.start].matches('\n').count();
        let Some(line) = lines.get(line_index) else {
            break;
        };
        let line_start = lx.input[..row.start]
            .rfind('\n')
            .map_or(0, |offset| offset + 1);
        if row_index == 0 {
            if shell {
                grid.put(
                    rect.x,
                    y,
                    text::LX_INPUT_SHELL_PROMPT,
                    style::shell_prompt(),
                );
            } else {
                grid.put(rect.x, y, text::LX_INPUT_PROMPT, style::accent());
            }
        }
        let line_tokens = tokens
            .get(line_index)
            .map(Vec::as_slice)
            .unwrap_or_default();
        paint_line(
            grid,
            content_x,
            y,
            line,
            line_tokens,
            row.start - line_start,
            row.end - line_start,
            content_w,
            line_start,
            selection,
        );
        if row_index == cursor_row && focused {
            let col = content_x
                + u16::try_from(display_width(&lx.input[row.start..lx.cursor])).unwrap_or(u16::MAX);
            cursor = Some((y, col));
        }
    }

    if lx.input.is_empty() {
        let placeholder = text::truncate_to_width(
            if shell {
                text::LX_INPUT_SHELL_PLACEHOLDER
            } else {
                text::LX_INPUT_PLACEHOLDER
            },
            usize::from(content_w),
        );
        grid.put(content_x, text_top, &placeholder, style::muted());
    } else if !shell
        && scroll == 0
        && rows.len() == 1
        && let Some(hint) = crate::app::state::lx::slash::active_argument_hint(&lx.input)
    {
        // 在命令词（如 `/rename`）与 `[name]` 之间保持至少 2 列间距（1 列尾随空格 + 1 列光标/间隔）。
        let cmd_width = display_width(lx.input.trim());
        let min_offset = cmd_width.saturating_add(2);
        let offset = display_width(&lx.input).max(min_offset);
        let offset_u16 = u16::try_from(offset).unwrap_or(u16::MAX);
        if offset_u16 < content_w {
            let avail = usize::from(content_w - offset_u16);
            let placeholder = text::truncate_to_width(hint, avail);
            grid.put(
                content_x.saturating_add(offset_u16),
                text_top,
                &placeholder,
                style::muted(),
            );
        }
    }
    cursor
}

/// 按 token 高亮绘制一行 markdown 的 `[row_start, row_end)` 字节区间；
/// 落在选中字节范围内的字符额外反显；超宽截断。
#[expect(clippy::too_many_arguments)]
fn paint_line(
    grid: &mut Grid,
    x: u16,
    y: u16,
    line: &str,
    tokens: &[Token],
    row_start: usize,
    row_end: usize,
    max_w: u16,
    line_offset: usize,
    selection: Option<(usize, usize)>,
) {
    let mut cx = x;
    let mut token_index = 0;
    for (offset, ch) in line.char_indices() {
        if offset < row_start {
            continue;
        }
        if offset >= row_end {
            break;
        }
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            continue;
        }
        while tokens
            .get(token_index)
            .is_some_and(|token| token.range.end <= offset)
        {
            token_index += 1;
        }
        let mut cell_style = tokens
            .get(token_index)
            .filter(|token| token.range.contains(&offset))
            .map_or_else(style::text, |token| {
                crate::ui::prompt::token_style(token.kind)
            });
        let absolute = line_offset + offset;
        if selection.is_some_and(|(start, end)| absolute >= start && absolute < end) {
            cell_style = cell_style.add_modifier(Modifier::REVERSED);
        }
        if cx.saturating_add(width as u16) > x.saturating_add(max_w) {
            break;
        }
        grid.set(cx, y, ch, cell_style);
        for dx in 1..width as u16 {
            grid.set(cx.saturating_add(dx), y, CONTINUATION, cell_style);
        }
        cx = cx.saturating_add(width as u16);
    }
}

/// 文本显示宽度（宽字符按两列计）。
fn display_width(content: &str) -> usize {
    content.chars().map(|ch| ch.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests;
