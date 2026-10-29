//! MCP 工具卡片：`■ server · tool  参数摘要  耗时` + 参数/结果/图片树体。
//!
//! 折叠态只显示参数摘要与每个块的前 3 行；展开态铺开参数与结果（各最多 20 行）。
//! 图片只显示一行元数据（路径可复制），不读取字节。

use ratatui::layout::Rect;

use crate::app::state::{McpDetailView, ToolCallView, ToolStatus};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::tree::{self, Node, Preview, Section};
use super::{Segment, header_with_verb, put_segments, single_line};

const BLOCK_PREVIEW_ROWS: usize = 3;

struct McpLayout {
    layout: tree::Layout,
}

fn sections(view: &McpDetailView, expanded: bool) -> Vec<Section> {
    let mut sections = Vec::new();
    if let Some(summary) = view.argument_summary() {
        sections.push(Section::fixed(vec![Node::text(
            summary,
            style::chat_tool_param(),
        )]));
    }
    if !view.arguments.is_empty() {
        let lines: Vec<Vec<Segment>> = view
            .arguments
            .iter()
            .map(|line| {
                vec![Segment::new(
                    single_line(line, 2000),
                    style::agent_step_detail(),
                )]
            })
            .collect();
        let mut nodes = vec![Node::block(lines)];
        if view.arguments_more > 0 {
            nodes.push(Node::text(
                text::mcp::more_lines(view.arguments_more),
                style::agent_step_detail(),
            ));
        }
        sections.push(Section::new(
            nodes,
            (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
        ));
    }
    if !view.result.is_empty() {
        let lines: Vec<Vec<Segment>> = view
            .result
            .iter()
            .map(|line| vec![Segment::new(single_line(line, 8000), style::text())])
            .collect();
        let mut nodes = vec![Node::block(lines)];
        if view.result_more > 0 {
            nodes.push(Node::text(
                text::mcp::more_lines(view.result_more),
                style::agent_step_detail(),
            ));
        }
        sections.push(Section::new(
            nodes,
            (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
        ));
    }
    if !view.images.is_empty() || view.images_omitted > 0 {
        let mut nodes: Vec<Node> = view
            .images
            .iter()
            .map(|image| {
                Node::text(
                    text::mcp::image_line(&image.mime, image.width, image.height, &image.path),
                    style::agent_step_detail(),
                )
            })
            .collect();
        if view.images_omitted > 0 {
            nodes.push(Node::text(
                text::mcp::images_label(view.images_omitted),
                style::agent_step_detail(),
            ));
        }
        sections.push(Section::fixed(nodes));
    }
    sections
}

fn layout(view: &McpDetailView, width: u16, expanded: bool) -> McpLayout {
    McpLayout {
        layout: tree::layout(&sections(view, expanded), width),
    }
}

/// 卡片行数（含标题行）。
pub(super) fn rows(view: &McpDetailView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    1 + layout(view, width, expanded).layout.rows.len()
}

/// 展开后是否真的多出内容。
pub(super) fn has_more(view: &McpDetailView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    layout(view, width, false).layout.hidden > 0
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
    expanded: bool,
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
        &layout(view, rect.width, expanded).layout.rows,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{ToolCallView, ToolStatus, ToolSummary};

    fn call() -> ToolCallView {
        ToolCallView {
            call_id: "call-mcp".into(),
            name: "mcp__github__create_issue".into(),
            position: 0,
            arguments_json: r#"{"title":"Fix login crash on iOS","body":"details"}"#.into(),
            status: ToolStatus::Completed,
            summary: Some(Box::new(ToolSummary {
                path: Some("github · create_issue".into()),
                ..ToolSummary::default()
            })),
            error: None,
            result_json: None,
            detail: None,
        }
    }

    fn view() -> McpDetailView {
        McpDetailView {
            server: "github".into(),
            tool: "create_issue".into(),
            summary: Some("Fix login crash on iOS".into()),
            arguments: vec!["{".into(), r#"  "title": "Fix login crash on iOS""#.into()],
            arguments_more: 0,
            result: vec!["created #42".into()],
            result_more: 0,
            images: vec![crate::app::state::McpImageView {
                path: "/tmp/tool.png".into(),
                mime: "image/png".into(),
                width: 10,
                height: 20,
            }],
            images_omitted: 0,
            duration_ms: Some(1200),
        }
    }

    fn render(view: &McpDetailView, width: u16, expanded: bool) -> String {
        let total = rows(view, width, expanded);
        let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
        let mut grid = Grid::new(rect);
        let mut row = 0;
        draw(
            &mut grid,
            rect,
            0,
            &mut row,
            &call(),
            view,
            0,
            expanded,
            false,
        );
        assert_eq!(row, total, "绘制行数必须与 rows 一致");
        let mut text = String::new();
        for y in 0..total {
            for x in 0..width {
                text.push(grid.cell(x, y as u16).ch);
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn folded_card_shows_server_tool_status_and_summary() {
        let text = render(&view(), 100, false);
        assert!(text.contains("github · create_issue"), "{text}");
        assert!(text.contains("1.2s"), "{text}");
        assert!(text.contains("Fix login crash on iOS"), "{text}");
        assert!(text.contains("[image image/png 10×20]"), "{text}");
    }

    #[test]
    fn long_result_is_expandable() {
        let mut view = view();
        view.result = (0..30).map(|index| format!("line {index}")).collect();
        view.result_more = 10;
        assert!(has_more(&view, 80));
        let expanded = render(&view, 80, true);
        assert!(expanded.contains("line 19"), "{expanded}");
        assert!(expanded.contains("10 more line(s)"), "{expanded}");
    }
}
