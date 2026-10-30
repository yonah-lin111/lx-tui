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

fn result_section(view: &McpDetailView, expanded: bool) -> Option<Section> {
    if view.result.is_empty() && view.images.is_empty() && view.images_omitted == 0 {
        return None;
    }
    let body_style = output_style(expanded);
    let mut lines: Vec<Vec<Segment>> = view
        .result
        .iter()
        .map(|line| vec![Segment::new(single_line(line, 8000), body_style)])
        .collect();
    if view.result_more > 0 {
        lines.push(vec![Segment::new(
            text::mcp::more_lines(view.result_more),
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

pub(super) fn layout(view: &McpDetailView, width: u16, folds: Folds) -> McpLayout {
    let folded_arguments = arguments_section(view, false);
    let folded_result = result_section(view, false);
    let arguments_expandable = folded_arguments
        .as_ref()
        .is_some_and(|section| tree::layout(std::slice::from_ref(section), width).hidden > 0);
    let result_expandable = folded_result
        .as_ref()
        .is_some_and(|section| tree::layout(std::slice::from_ref(section), width).hidden > 0);
    let arguments = arguments_section(view, folds.body);
    let result = result_section(view, folds.output);
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
pub(super) fn rows(view: &McpDetailView, width: u16, folds: Folds) -> usize {
    if width == 0 {
        return 0;
    }
    1 + layout(view, width, folds).layout.rows.len()
}

/// 展开后是否真的多出内容。
pub(super) fn has_more(view: &McpDetailView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    let layout = layout(view, width, Folds::default());
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
        &layout(view, rect.width, folds).layout.rows,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{ToolCallView, ToolStatus, ToolSummary};
    use crate::ui::text;

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

    fn render(view: &McpDetailView, width: u16, folds: Folds) -> String {
        let total = rows(view, width, folds);
        let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
        let mut grid = Grid::new(rect);
        let mut row = 0;
        draw(&mut grid, rect, 0, &mut row, &call(), view, 0, folds, false);
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
        let text = render(&view(), 100, Folds::default());
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
        let expanded = render(
            &view,
            80,
            Folds {
                body: false,
                output: true,
            },
        );
        assert!(expanded.contains("line 19"), "{expanded}");
        assert!(expanded.contains("10 more lines"), "{expanded}");
    }

    #[test]
    fn mcp_card_registers_a_click_region_and_connector_colors() {
        use crate::app::state::ToolDetailView;
        let mut call = call();
        let mut view = view();
        view.result = (0..10).map(|index| format!("line {index}")).collect();
        view.result_more = 7;
        call.detail = Some(Box::new(ToolDetailView::Mcp(view.clone())));
        let folds = Folds::default();
        let total = rows(&view, 100, folds);
        let regions = super::super::card_regions(&call, 100, folds, total);
        assert!(regions.iter().any(|region| region.clickable));
        assert_eq!(regions.len(), 2, "args and result have separate regions");
        assert_eq!(regions[0].section, crate::app::state::CardSection::Body);
        assert_eq!(regions[1].section, crate::app::state::CardSection::Output);
        for region in &regions {
            assert!(region.card.offset + region.card.rows <= total);
        }
        let expanded = layout(
            &view,
            100,
            Folds {
                body: true,
                output: true,
            },
        );
        let arg_row = expanded
            .layout
            .rows
            .iter()
            .find(|row| {
                row.segments
                    .iter()
                    .any(|segment| segment.text.contains("title"))
            })
            .expect("argument row");
        assert_eq!(
            arg_row.segments[0].style,
            style::chat_tool_param(),
            "args use the param color"
        );
        let result_row = expanded
            .layout
            .rows
            .iter()
            .find(|row| {
                row.segments
                    .iter()
                    .any(|segment| segment.text.contains("line 0"))
            })
            .expect("result row");
        assert_eq!(result_row.segments[0].style, style::text());
    }

    #[test]
    fn collapsed_leaf_uses_dim_and_expanded_leaf_uses_default_foreground() {
        let mut view = view();
        view.result = (0..10).map(|index| format!("line {index}")).collect();
        view.result_more = 7;
        let folded = layout(&view, 100, Folds::default());
        let result_row = folded
            .layout
            .rows
            .iter()
            .find(|row| row.segments.iter().any(|seg| seg.text.contains("line 0")))
            .expect("folded result");
        assert_eq!(result_row.segments[0].style, style::muted());
        let expanded = layout(
            &view,
            100,
            Folds {
                body: true,
                output: true,
            },
        );
        let result_row = expanded
            .layout
            .rows
            .iter()
            .find(|row| row.segments.iter().any(|seg| seg.text.contains("line 0")))
            .expect("expanded result");
        assert_eq!(result_row.segments[0].style, style::text());
    }

    #[test]
    fn expanding_branch_keeps_bar_and_expanding_leaf_uses_padding() {
        let mut view = view();
        view.arguments = (0..10).map(|index| format!("arg {index}")).collect();
        view.result = (0..10).map(|index| format!("line {index}")).collect();
        let expanded = layout(
            &view,
            100,
            Folds {
                body: true,
                output: true,
            },
        );
        let arg_row = expanded
            .layout
            .rows
            .iter()
            .find(|row| row.segments.iter().any(|seg| seg.text.contains("arg 1")))
            .expect("argument continuation");
        assert_eq!(arg_row.prefix, text::agent::TREE_BAR);
        let result_row = expanded
            .layout
            .rows
            .iter()
            .find(|row| row.segments.iter().any(|seg| seg.text.contains("line 1")))
            .expect("result continuation");
        assert_eq!(result_row.prefix, text::agent::TREE_PAD);
    }
}
