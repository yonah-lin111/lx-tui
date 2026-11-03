//! 历史行几何的共用小工具：宽度换算、可见行判定与软换行。
//!
//! 从 `history.rs` 拆出（单文件行数上限）。布局与绘制必须共用同一份口径，
//! 所以这些函数只在这里定义一次，两边都从这里取。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::paste::{ExpandedPastes, Projection, SegmentKind};

use super::{SCROLLBAR_COLUMNS, tool};

/// 用户消息行内片段样式种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserSpanKind {
    Plain,
    CollapsedChip,
    ExpandedChip,
}

/// 用户消息单行内的连续同样式文本段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserTextSpan {
    pub text: String,
    pub kind: UserSpanKind,
}

/// 用户消息软换行后的单条视觉行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserTextLine {
    pub spans: Vec<UserTextSpan>,
}

impl UserTextLine {
    /// 近似字节数（供二级缓存预算统计）。
    pub(crate) fn byte_len(&self) -> usize {
        self.spans.iter().map(|s| s.text.len() + 16).sum::<usize>() + 16
    }
}

/// 用户消息内一个折叠粘贴芯片在视觉行序列中的行区间（相对消息首行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UserPasteSpan {
    pub paste_id: u64,
    pub occurrence: usize,
    pub expanded: bool,
    pub offset: usize,
    pub rows: usize,
}

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

/// 将用户消息文本按折叠/展开投影与可用列宽软换行为视觉行，并计算每个粘贴芯片的行区间。
pub(crate) fn wrap_user_projection(
    display_text: &str,
    expanded: &ExpandedPastes,
    width: u16,
) -> (Vec<UserTextLine>, Vec<UserPasteSpan>) {
    if display_text.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let proj = Projection::new(display_text, expanded);
    if !proj.has_tokens() {
        let lines = wrap(display_text, width)
            .into_iter()
            .map(|text| UserTextLine {
                spans: if text.is_empty() {
                    Vec::new()
                } else {
                    vec![UserTextSpan {
                        text,
                        kind: UserSpanKind::Plain,
                    }]
                },
            })
            .collect();
        return (lines, Vec::new());
    }

    let avail = usize::from(width.max(1));
    let proj_text = proj.text();
    let mut row_ranges: Vec<(usize, usize)> = Vec::new();
    let mut line_start = 0usize;

    for segment in proj_text.split('\n') {
        let line_end = line_start + segment.len();
        if segment.is_empty() {
            row_ranges.push((line_start, line_end));
        } else {
            let mut row_start = line_start;
            let mut col = 0usize;
            for (offset, ch) in segment.char_indices() {
                let absolute = line_start + offset;
                let cell = ch.width().unwrap_or(1).max(1);
                if col > 0 && col + cell > avail {
                    row_ranges.push((row_start, absolute));
                    row_start = absolute;
                    col = 0;
                }
                col += cell;
            }
            row_ranges.push((row_start, line_end));
        }
        line_start = line_end + 1;
    }

    let mut lines = Vec::with_capacity(row_ranges.len());
    for &(row_start, row_end) in &row_ranges {
        let mut spans: Vec<UserTextSpan> = Vec::new();
        for (offset, ch) in proj_text[row_start..row_end].char_indices() {
            let disp_pos = row_start + offset;
            let kind = match proj.segment_at(disp_pos).map(|s| s.kind) {
                Some(SegmentKind::Chip {
                    expanded: false, ..
                }) => UserSpanKind::CollapsedChip,
                Some(SegmentKind::Chip { expanded: true, .. }) => UserSpanKind::ExpandedChip,
                _ => UserSpanKind::Plain,
            };
            if let Some(last) = spans.last_mut().filter(|s| s.kind == kind) {
                last.text.push(ch);
            } else {
                let mut text = String::new();
                text.push(ch);
                spans.push(UserTextSpan { text, kind });
            }
        }
        lines.push(UserTextLine { spans });
    }

    let mut paste_spans = Vec::new();
    for seg in proj.segments() {
        let SegmentKind::Chip {
            token,
            expanded: chip_exp,
            occurrence,
        } = seg.kind
        else {
            continue;
        };
        let overlaps = |(rs, re): &(usize, usize)| {
            if *rs == *re {
                *rs >= seg.display.start && *rs < seg.display.end
            } else {
                seg.display.start < *re && seg.display.end > *rs
            }
        };
        if let (Some(first), Some(last)) = (
            row_ranges.iter().position(overlaps),
            row_ranges.iter().rposition(overlaps),
        ) {
            paste_spans.push(UserPasteSpan {
                paste_id: proj.tokens()[token].id,
                occurrence,
                expanded: chip_exp,
                offset: first,
                rows: last - first + 1,
            });
        }
    }

    (lines, paste_spans)
}
