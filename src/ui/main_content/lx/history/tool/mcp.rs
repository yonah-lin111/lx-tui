//! MCP 工具身份卡：`■ MCP · 耗时` + `server · tool`。
//!
//! 卡片只回答“调了哪个 server / tool、花了多久、有没有图片、失败原因是什么”：
//! 参数与结果正文一律不渲染，也不登记点击面。合并组内 server 为一级节点
//! （承载耗时/图片状态），tool 与其错误原因缩进为二级子节点。

use ratatui::layout::Rect;

use crate::app::state::{McpDetailView, ToolCallView, ToolStatus};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::tree::{self, Node, Section};
use super::{Folds, Segment, TEXT_SAMPLE_CHARS, header_with_verb, put_segments, single_line};

/// MCP 卡片的排版结果：只含身份行与错误原因行。
pub(super) struct McpLayout {
    layout: tree::Layout,
}

/// 单个调用的身份行：server 浅黄，tool 正常文字色。
fn identity_node(view: &McpDetailView) -> Node {
    Node::new(vec![
        Segment::new(
            single_line(&view.server, TEXT_SAMPLE_CHARS),
            style::chat_tool_param(),
        ),
        Segment::new(text::LX_CHAT_TOOL_STATUS_SEP, style::agent_step_detail()),
        Segment::new(single_line(&view.tool, TEXT_SAMPLE_CHARS), style::text()),
    ])
}

/// 失败/中断原因行：`failed: ...` / `interrupted: ...`，恒一行尾部省略。
///
/// 中断的稳定错误码本身就是 `interrupted`，与状态词重复时只输出一次。
fn error_node(call: &ToolCallView, width: u16, in_group: bool) -> Option<Node> {
    if !matches!(
        call.status,
        ToolStatus::Failed | ToolStatus::Interrupted | ToolStatus::Cancelled
    ) {
        return None;
    }
    let label = text::tool_status_label(call.status);
    let reason = call
        .error
        .as_deref()
        .map(|reason| single_line(reason, TEXT_SAMPLE_CHARS))
        .unwrap_or_default();
    let reason = reason.trim();
    let message = if reason.is_empty() {
        if call.status == ToolStatus::Cancelled {
            label.to_string()
        } else {
            return None;
        }
    } else if reason.eq_ignore_ascii_case(label) {
        label.to_string()
    } else {
        format!("{label}: {reason}")
    };
    let body = if in_group {
        tree::nested_content_width(width)
    } else {
        tree::content_width(width)
    };
    Some(Node::text(
        crate::ui::main_content::lx::history::truncate_with_ellipsis(&message, body),
        style::text(),
    ))
}

pub(super) fn layout_with_group(
    call: &ToolCallView,
    view: &McpDetailView,
    width: u16,
    _folds: Folds,
    in_group: bool,
) -> McpLayout {
    let mut sections = Vec::with_capacity(2);
    if in_group {
        // 合并态：server 为一级节点（承载耗时/图片状态），tool 与错误原因为二级子节点。
        sections.push(Section::fixed(vec![Node::text(
            single_line(&view.server, TEXT_SAMPLE_CHARS),
            style::chat_tool_param(),
        )]));
        let mut children = vec![Node::text(
            single_line(&view.tool, TEXT_SAMPLE_CHARS),
            style::text(),
        )];
        children.extend(error_node(call, width, true));
        sections.push(Section::fixed(children));
    } else {
        // 单体：`server · tool` 一行，`└─` 收口；错误原因是兄弟节点。
        let mut roots = vec![identity_node(view)];
        roots.extend(error_node(call, width, false));
        sections.push(Section::fixed(roots));
    }
    McpLayout {
        layout: if in_group {
            tree::layout_in_group(&sections, width, 1)
        } else {
            tree::layout(&sections, width)
        },
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

/// 标题状态：耗时与图片数；失败/中断交给标题染色。
fn status_of(call: &ToolCallView, view: &McpDetailView) -> Option<String> {
    if matches!(
        call.status,
        ToolStatus::Failed | ToolStatus::Interrupted | ToolStatus::Cancelled
    ) {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(duration) = view.duration_ms {
        parts.push(text::agent::duration_label(duration));
    }
    let images = view.images.len().saturating_add(view.images_omitted);
    if images > 0 {
        parts.push(text::mcp::images_label(images));
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
        let segments = header_with_verb(
            call,
            rect.width,
            loading_frame,
            animating,
            text::agent::VERB_MCP,
            status_of(call, view),
        );
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    let mut layout = layout_with_group(call, view, rect.width, folds, false).layout;
    if matches!(
        call.status,
        ToolStatus::Failed | ToolStatus::Interrupted | ToolStatus::Cancelled
    ) && let Some(first) = layout.rows.first_mut()
    {
        first.prefix_style = Some(tree::connector_style_of(call));
    }
    tree::draw(grid, rect, scroll, row_index, &layout.rows);
}

/// 合并组内调用体：跳过标题，耗时/图片状态追加到 server 节点尾部。
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
