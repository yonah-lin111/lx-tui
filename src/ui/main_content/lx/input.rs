//! 底部多行输入框：上下 `─` 分割线 + 首行 `> ` 前缀 + markdown 高亮与折叠粘贴芯片。
//!
//! 高亮只做**编辑器色板**（`style::markdown_*`，与阅读渲染的 GitHub Dark 色板
//! 完全独立，见 `docs/standards/tui-design-requirements.md`）：token 来自
//! `ui::markdown::scan`，样式统一经 `ui::prompt::token_style` 取用，保证输入框与
//! 全局 prompt 容器两处读法一致。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；返回光标绝对坐标供上层换算。

use ratatui::style::Modifier;
use unicode_width::UnicodeWidthChar;

use crate::app::paste::{Projection, SegmentKind, mask_pasted_text};
use crate::app::state::Lx;
use crate::app::state::lx::LxRow;
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
    let proj = lx.projection();
    let shell = lx.shell_mode();
    let masked = mask_pasted_text(&lx.input);
    // Shell 正文按原始脚本绘制：不做 Markdown 词法高亮。
    let tokens = if shell {
        Vec::new()
    } else {
        crate::ui::markdown::scan(&masked)
    };
    let mut raw_line_starts = Vec::new();
    let mut offset = 0;
    for line in lx.input.split('\n') {
        raw_line_starts.push(offset);
        offset += line.len() + 1;
    }
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
    let preview_sel = lx.preview_selection();

    let mut cursor = None;
    for i in 0..visible {
        let Some(row_index) = scroll.checked_add(i) else {
            break;
        };
        let Some(row) = rows.get(row_index) else {
            break;
        };
        let y = text_top + i as u16;
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
        paint_line(
            grid,
            content_x,
            y,
            &proj,
            row,
            &tokens,
            &raw_line_starts,
            content_w,
            selection,
            preview_sel,
        );
        if row_index == cursor_row && focused {
            let disp_cursor = proj.raw_to_display(lx.cursor).clamp(row.start, row.end);
            let col = content_x.saturating_add(
                u16::try_from(display_width(&proj.text()[row.start..disp_cursor]))
                    .unwrap_or(u16::MAX),
            );
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
        let cmd_width = display_width(lx.input.trim());
        let min_offset = cmd_width.saturating_add(2);
        let offset = display_width(proj.text()).max(min_offset);
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

/// 按投影段与 token 高亮绘制一行 `[row.start, row.end)` 字节区间；
/// 落在选中字节范围内的字符额外反显；超宽截断。
#[expect(clippy::too_many_arguments)]
fn paint_line(
    grid: &mut Grid,
    x: u16,
    y: u16,
    proj: &Projection<'_>,
    row: &LxRow,
    tokens: &[Vec<Token>],
    raw_line_starts: &[usize],
    max_w: u16,
    selection: Option<(usize, usize)>,
    preview_selection: Option<(usize, usize)>,
) {
    let right_limit = x.saturating_add(max_w);
    let mut cx = x;
    let disp_text = proj.text();
    for (offset, ch) in disp_text[row.start..row.end].char_indices() {
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            continue;
        }
        let disp_pos = row.start + offset;
        let seg = proj.segment_at(disp_pos);
        let mut cell_style = match seg.map(|s| s.kind) {
            Some(SegmentKind::Chip {
                kind,
                expanded: false,
                ..
            }) => style::pasted_chip(kind),
            Some(SegmentKind::Chip {
                kind,
                expanded: true,
                ..
            }) => style::pasted_bg(kind),
            _ => {
                let raw_pos = proj.display_to_raw(disp_pos);
                let raw_line = raw_line_starts
                    .partition_point(|&start| start <= raw_pos)
                    .saturating_sub(1);
                let line_start = raw_line_starts.get(raw_line).copied().unwrap_or(0);
                let rel_raw = raw_pos.saturating_sub(line_start);
                tokens
                    .get(raw_line)
                    .and_then(|line_tokens| {
                        line_tokens
                            .iter()
                            .find(|token| token.range.contains(&rel_raw))
                    })
                    .map_or_else(style::text, |token| {
                        crate::ui::prompt::token_style(token.kind)
                    })
            }
        };
        if preview_selection.is_some_and(|(s, e)| disp_pos >= s && disp_pos < e) {
            cell_style = cell_style.add_modifier(Modifier::REVERSED);
        } else if let Some((sel_start, sel_end)) = selection {
            let selected = match seg {
                Some(seg)
                    if matches!(
                        seg.kind,
                        SegmentKind::Chip {
                            expanded: false,
                            ..
                        }
                    ) =>
                {
                    seg.raw.start < sel_end && seg.raw.end > sel_start
                }
                _ => {
                    let raw_pos = proj.display_to_raw(disp_pos);
                    raw_pos >= sel_start && raw_pos < sel_end
                }
            };
            if selected {
                cell_style = cell_style.add_modifier(Modifier::REVERSED);
            }
        }
        if cx.saturating_add(width as u16) > right_limit {
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
