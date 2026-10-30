//! MCP 工具卡片：`■ server · tool  参数摘要  耗时` + 参数/结果/图片树体。
//!
//! 折叠态只显示参数摘要与每个块的前 3 行；展开态铺开参数与结果（各最多 20 行）。
//! 图片只显示一行元数据（路径可复制），不读取字节。

use ratatui::layout::Rect;

use crate::app::state::{McpDetailView, ToolCallView, ToolStatus};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::tree::{self, Node, Preview, Section};
use super::{Folds, Segment, header_with_verb, output_style, put_segments, single_line};

const BLOCK_PREVIEW_ROWS: usize = 3;

pub(super) struct McpLayout {
    layout: tree::Layout,
    /// 参数块（Body）的物理行数（含标题行时为 offset 0 的区域）。
    pub(super) arguments_rows: usize,
    /// 结果块（Output）的物理行数。
    pub(super) result_rows: usize,
    pub(super) arguments_expandable: bool,
    pub(super) result_expandable: bool,
}

fn arguments_section(view: &McpDetailView, expanded: bool) -> Option<Section> {
    if view.arguments.is_empty() {
        return None;
    }
    let mut lines: Vec<Vec<Segment>> = view
        .arguments
        .iter()
        .map(|line| {
            vec![Segment::new(
                single_line(line, 2000),
                style::chat_tool_param(),
            )]
        })
        .collect();
    if view.arguments_more > 0 {
        lines.push(vec![Segment::new(
            text::mcp::more_lines(view.arguments_more),
            style::fold_hint(),
        )]);
    }
    Some(Section::new(
        vec![Node::block(lines)],
        (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
    ))
}

fn result_section(call: &ToolCallView, view: &McpDetailView, expanded: bool) -> Option<Section> {
    let error_fallback = if view.result.is_empty()
        && matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted)
    {
        call.error.as_deref().filter(|error| !error.is_empty())
    } else {
        None
    };
    if view.result.is_empty()
        && error_fallback.is_none()
        && view.images.is_empty()
        && view.images_omitted == 0
    {
        return None;
    }
    let body_style = output_style(expanded);
    let mut lines: Vec<Vec<Segment>> = if let Some(error) = error_fallback {
        error
            .lines()
            .take(20)
            .map(|line| vec![Segment::new(single_line(line, 8000), body_style)])
            .collect()
    } else {
        view.result
            .iter()
            .map(|line| vec![Segment::new(single_line(line, 8000), body_style)])
            .collect()
    };
    let more = if let Some(error) = error_fallback {
        error.lines().count().saturating_sub(20)
    } else {
        view.result_more
    };
    if more > 0 {
        lines.push(vec![Segment::new(
            text::mcp::more_lines(more),
            style::fold_hint(),
        )]);
    }
    for image in &view.images {
        lines.push(vec![Segment::new(
            text::mcp::image_line(&image.mime, image.width, image.height, &image.path),
            body_style,
        )]);
    }
    if view.images_omitted > 0 {
        lines.push(vec![Segment::new(
            text::mcp::images_label(view.images_omitted),
            style::fold_hint(),
        )]);
    }
    Some(Section::new(
        vec![Node::block(lines)],
        (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
    ))
}

pub(super) fn layout(
    call: &ToolCallView,
    view: &McpDetailView,
    width: u16,
    folds: Folds,
) -> McpLayout {
    let folded_arguments = arguments_section(view, false);
    let folded_result = result_section(call, view, false);
    let arguments_expandable = folded_arguments
        .as_ref()
        .is_some_and(|section| tree::layout(std::slice::from_ref(section), width).hidden > 0);
    let result_expandable = folded_result
        .as_ref()
        .is_some_and(|section| tree::layout(std::slice::from_ref(section), width).hidden > 0);
    let arguments = arguments_section(view, folds.body);
    let result = result_section(call, view, folds.output);
    let arguments_rows = arguments.as_ref().map_or(0, |section| {
        tree::layout(std::slice::from_ref(section), width)
            .rows
            .len()
    });
    let result_rows = result.as_ref().map_or(0, |section| {
        tree::layout(std::slice::from_ref(section), width)
            .rows
            .len()
    });
    let sections: Vec<Section> = arguments.into_iter().chain(result).collect();
    McpLayout {
        layout: tree::layout(&sections, width),
        arguments_rows,
        result_rows,
        arguments_expandable,
        result_expandable,
    }
}

/// 卡片行数（含标题行）。
pub(super) fn rows(call: &ToolCallView, view: &McpDetailView, width: u16, folds: Folds) -> usize {
    if width == 0 {
        return 0;
    }
    1 + layout(call, view, width, folds).layout.rows.len()
}

/// 展开后是否真的多出内容。
pub(super) fn has_more(call: &ToolCallView, view: &McpDetailView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    let layout = layout(call, view, width, Folds::default());
    layout.arguments_expandable || layout.result_expandable
}

/// 绘制 MCP 卡片；推进行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &McpDetailView,
    loading_frame: usize,
    folds: Folds,
    animating: bool,
) {
    if rect.width == 0 {
        return;
    }
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let status = if matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
            None
        } else {
            let mut parts = Vec::new();
            if let Some(duration) = view.duration_ms {
                parts.push(text::agent::duration_label(duration));
            }
            if call
                .summary
                .as_deref()
                .is_some_and(|summary| summary.truncated)
            {
                parts.push(text::mcp::RESULT_TRUNCATED.to_string());
            }
            if !view.images.is_empty() {
                parts.push(text::mcp::images_label(view.images.len()));
            }
            (!parts.is_empty()).then(|| parts.join(text::LX_CHAT_TOOL_STATUS_SEP))
        };
        let verb = format!("{} · {}", view.server, view.tool);
        let segments = header_with_verb(call, rect.width, loading_frame, animating, &verb, status);
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    tree::draw(
        grid,
        rect,
        scroll,
        row_index,
        &layout(call, view, rect.width, folds).layout.rows,
    );
}

#[cfg(test)]
mod tests;
