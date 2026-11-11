//! MCP 工具卡片：`■ server · tool  参数摘要  耗时` + 参数/结果/图片树体。
//!
//! 折叠态只显示参数摘要与每个块的前 3 行；展开态铺开参数与结果（各最多 20 行）。
//! 图片只显示一行元数据（路径可复制），不读取字节。

use ratatui::layout::Rect;

use crate::app::state::{McpDetailView, ToolCallView, ToolStatus};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::tree::{self, Node, Preview, Section};
use super::{
    Folds, Segment, header_with_verb, muted_markdown_lines, output_style, put_segments, single_line,
};

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

fn arguments_section(view: &McpDetailView, expanded: bool, in_group: bool) -> Option<Section> {
    if view.arguments.is_empty() {
        return in_group.then(|| {
            Section::fixed(vec![Node::text(
                text::agent::MCP_NO_ARGUMENTS,
                style::agent_step_detail(),
            )])
        });
    }
    let lines: Vec<Vec<Segment>> = view
        .arguments
        .iter()
        .map(|line| {
            vec![Segment::new(
                single_line(line, 2000),
                style::chat_tool_param(),
            )]
        })
        .collect();
    Some(
        Section::new(
            vec![Node::block(lines)],
            (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
        )
        .with_omitted(view.arguments_more),
    )
}

fn sanitize_result_line(line: &str, max_chars: usize) -> String {
    let (cleaned, had_ellipsis) = if let Some(prefix) = line
        .strip_suffix("…[truncated]")
        .or_else(|| line.strip_suffix("[...truncated]"))
    {
        (prefix, !prefix.is_empty())
    } else if let Some(prefix) = line.strip_suffix(text::agent::INLINE_TRUNCATED) {
        (prefix, prefix.ends_with('…'))
    } else {
        (line, false)
    };
    let mut text = String::with_capacity(cleaned.len().min(max_chars + 4));
    let mut clipped = false;
    for (count, ch) in cleaned.chars().enumerate() {
        if count == max_chars {
            text.push('…');
            clipped = true;
            break;
        }
        text.push(if ch.is_control() { ' ' } else { ch });
    }
    let trimmed = text.trim_end();
    if had_ellipsis && !clipped && !trimmed.is_empty() && !trimmed.ends_with('…') {
        format!("{trimmed}…")
    } else {
        trimmed.to_string()
    }
}

fn result_section(
    call: &ToolCallView,
    view: &McpDetailView,
    width: u16,
    expanded: bool,
    in_group: bool,
) -> Option<Section> {
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
    let cols = if in_group {
        tree::nested_content_width(width)
    } else {
        tree::content_width(width)
    };
    let body_width = u16::try_from(cols).unwrap_or(u16::MAX);
    let (mut lines, more) = if let Some(error) = error_fallback {
        let error_lines: Vec<Vec<Segment>> = error
            .lines()
            .take(20)
            .map(|line| sanitize_result_line(line, 8000))
            .filter(|line| !line.is_empty())
            .map(|line| vec![Segment::new(line, style::text())])
            .collect();
        (error_lines, error.lines().count().saturating_sub(20))
    } else {
        let raw_source = view
            .result
            .iter()
            .map(|line| sanitize_result_line(line, 8000))
            .collect::<Vec<_>>()
            .join("\n");
        (
            muted_markdown_lines(&raw_source, body_width),
            view.result_more,
        )
    };
    let truncated = error_fallback.is_none()
        && call
            .summary
            .as_deref()
            .is_some_and(|summary| summary.truncated);
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
    if lines.is_empty() {
        return None;
    }
    Some(
        Section::new(
            vec![Node::block(lines)],
            (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
        )
        .with_omitted(more)
        .with_truncated(truncated),
    )
}

pub(super) fn layout(
    call: &ToolCallView,
    view: &McpDetailView,
    width: u16,
    folds: Folds,
) -> McpLayout {
    layout_with_group(call, view, width, folds, false)
}

pub(super) fn layout_with_group(
    call: &ToolCallView,
    view: &McpDetailView,
    width: u16,
    folds: Folds,
    in_group: bool,
) -> McpLayout {
    let folded_arguments = arguments_section(view, false, in_group);
    let folded_result = result_section(call, view, width, false, in_group);
    let arguments_expandable = folded_arguments.as_ref().is_some_and(|section| {
        let l = if in_group && folded_result.is_some() {
            tree::layout_in_group(std::slice::from_ref(section), width, 1)
        } else {
            tree::layout(std::slice::from_ref(section), width)
        };
        l.hidden > 0
    });
    let result_expandable = folded_result.as_ref().is_some_and(|section| {
        let l = if in_group {
            tree::layout_in_group(std::slice::from_ref(section), width, 0)
        } else {
            tree::layout(std::slice::from_ref(section), width)
        };
        l.hidden > 0
    });
    let arguments = arguments_section(view, folds.body, in_group);
    let result = result_section(call, view, width, folds.output, in_group);
    let arguments_rows = arguments.as_ref().map_or(0, |section| {
        if in_group && result.is_some() {
            tree::layout_in_group(std::slice::from_ref(section), width, 1)
                .rows
                .len()
        } else {
            tree::layout(std::slice::from_ref(section), width)
                .rows
                .len()
        }
    });
    let result_rows = result.as_ref().map_or(0, |section| {
        if in_group {
            tree::layout_in_group(std::slice::from_ref(section), width, 0)
                .rows
                .len()
        } else {
            tree::layout(std::slice::from_ref(section), width)
                .rows
                .len()
        }
    });
    let sections: Vec<Section> = arguments.into_iter().chain(result).collect();
    McpLayout {
        layout: if in_group {
            tree::layout_in_group(&sections, width, 1)
        } else {
            tree::layout(&sections, width)
        },
        arguments_rows,
        result_rows,
        arguments_expandable,
        result_expandable,
    }
}

/// 卡片行数（含标题行）。
#[cfg(test)]
pub(super) fn rows(call: &ToolCallView, view: &McpDetailView, width: u16, folds: Folds) -> usize {
    rows_with_group(call, view, width, folds, false)
}

pub(super) fn rows_with_group(
    call: &ToolCallView,
    view: &McpDetailView,
    width: u16,
    folds: Folds,
    in_group: bool,
) -> usize {
    if width == 0 {
        return 0;
    }
    1 + layout_with_group(call, view, width, folds, in_group)
        .layout
        .rows
        .len()
}

/// 展开后是否真的多出内容。
#[cfg(test)]
pub(super) fn has_more(call: &ToolCallView, view: &McpDetailView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    let layout = layout(call, view, width, Folds::default());
    layout.arguments_expandable || layout.result_expandable
}

/// 标题状态：耗时与图片数；失败/中断交给标题染色。
fn status_of(call: &ToolCallView, view: &McpDetailView) -> Option<String> {
    if matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(duration) = view.duration_ms {
        parts.push(text::agent::duration_label(duration));
    }
    if !view.images.is_empty() {
        parts.push(text::mcp::images_label(view.images.len()));
    }
    (!parts.is_empty()).then(|| parts.join(text::LX_CHAT_TOOL_STATUS_SEP))
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
        let verb = format!("{} · {}", view.server, view.tool);
        let segments = header_with_verb(
            call,
            rect.width,
            loading_frame,
            animating,
            &verb,
            status_of(call, view),
        );
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

/// 合并组内调用体：跳过标题，状态追加到首个参数节点尾部。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_body(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &McpDetailView,
    _loading_frame: usize,
    folds: Folds,
    _animating: bool,
    last: bool,
) {
    if rect.width == 0 {
        return;
    }
    let mut layout = layout_with_group(call, view, rect.width, folds, true).layout;
    tree::decorate_group_call(&mut layout.rows, call, status_of(call, view), rect.width);
    tree::apply_group_connector(&mut layout.rows, last);
    tree::draw(grid, rect, scroll, row_index, &layout.rows);
}

#[cfg(test)]
mod tests;
