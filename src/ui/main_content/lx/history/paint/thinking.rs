//! 思考块（thinking / thought）头部文案与绘制（从 `paint.rs` 拆分）。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::ui::markdown_render::MdLine;
use crate::ui::style;

use super::super::super::Grid;
use super::super::source::HistorySource;
use super::super::{STEP_GUTTER, StepKind, StepMark, visible_row};
use super::{BlockSource, step_of};

/// thinking 块耗时（毫秒）：首个增量到最后一个增量的时间差；时间戳缺失返回 None。
fn thinking_duration(part: &crate::app::state::ChatPartView) -> Option<i64> {
    let started = part.started_at_ms?;
    let ended = part.ended_at_ms?;
    Some(ended.saturating_sub(started).max(0))
}

/// 思考头部文案：生成中 `Thinking`；完成 `Thought · 6.3s`（无耗时不加分隔）。
pub(crate) fn thinking_header_label(
    part: &crate::app::state::ChatPartView,
    streaming: bool,
) -> String {
    if streaming {
        crate::ui::text::LX_CHAT_THINKING_LABEL.to_string()
    } else {
        let mut label = String::from(crate::ui::text::LX_CHAT_THOUGHT_LABEL);
        if let Some(duration) = thinking_duration(part) {
            label.push_str(super::super::THINKING_DURATION_SEP);
            label.push_str(&super::super::format_duration(duration));
        }
        label
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_thinking(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    body: &[MdLine],
) {
    let Some((_, part)) = step_of(history, source) else {
        return;
    };
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return;
    };
    let streaming = history.chat.reasoning_streaming(message_index, step_index);
    let expanded = history.chat.reasoning_expanded(message_index, step_index);
    let mark = if streaming {
        StepMark::Running
    } else {
        StepMark::Done
    };
    if start < viewport_end
        && let Some(y) = visible_row(rect, scroll, start)
    {
        // 生成中：`Thinking`；完成后：`Thought · 6.3s`。
        let header_style = StepKind::Thinking.label(streaming, expanded);
        grid.put_char(
            rect.x,
            y,
            mark.glyph(
                StepKind::Thinking,
                history.chat.loading_frame,
                history.animating,
            ),
            mark.style(StepKind::Thinking, expanded),
        );
        let x = rect.x.saturating_add(STEP_GUTTER);
        let limit = rect.right();
        let label = thinking_header_label(part, streaming);
        grid.put_clipped(x, y, &label, header_style, limit.saturating_sub(x));
    }
    let tree = super::super::thinking_tree(rect.width);
    let body_indent = super::super::thinking_body_indent(rect.width);
    for (offset, line) in body.iter().enumerate() {
        let row = start.saturating_add(1).saturating_add(offset);
        if row < scroll {
            continue;
        }
        if row >= viewport_end {
            break;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        if tree && offset == 0 {
            let connector = crate::ui::text::agent::TREE_LEAF;
            let available = rect
                .width
                .min(u16::try_from(connector.chars().count()).unwrap_or(u16::MAX));
            grid.put_clipped(
                rect.x,
                y,
                connector,
                style::chat_notice_connector(),
                available,
            );
        }
        let base_x = rect
            .x
            .saturating_add(body_indent)
            .saturating_add(line.indent);
        let mut cx = base_x;
        if let Some(ref line_ref) = line.table {
            grid.set_table_row(cx, y, line_ref.clone());
        }
        for link in &line.links {
            let start_x = base_x.saturating_add(link.start_col);
            let end_x = base_x.saturating_add(link.end_col).min(rect.right());
            if start_x < end_x {
                grid.set_link_span(start_x, y, end_x - start_x, &link.url);
            }
        }
        for (value, span_style) in &line.spans {
            let cell_style = span_style.patch(style::muted());
            for ch in value.chars() {
                let cell = ch.width().unwrap_or(1).max(1) as u16;
                if cx.saturating_add(cell) > rect.right() {
                    break;
                }
                grid.put_char(cx, y, ch, cell_style);
                cx = cx.saturating_add(cell);
            }
        }
    }
}
