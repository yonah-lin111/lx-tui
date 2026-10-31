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

fn render_with(call: &ToolCallView, view: &McpDetailView, width: u16, folds: Folds) -> String {
    let total = rows(call, view, width, folds);
    let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row = 0;
    draw(&mut grid, rect, 0, &mut row, call, view, 0, folds, false);
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

fn render(view: &McpDetailView, width: u16, folds: Folds) -> String {
    render_with(&call(), view, width, folds)
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
fn running_and_failed_mcp_cards_keep_formatted_header_and_arguments() {
    let mut running = ToolCallView::new("call-1", "mcp__codegraph__codegraph_explore", 0);
    assert!(running.accept_arguments(r#"{"query":"McpManager"}"#.to_string()));
    running.status = ToolStatus::Running;
    let Some(crate::app::state::ToolDetailView::Mcp(running_view)) = running.detail.as_deref()
    else {
        panic!("running MCP tool must have McpDetailView");
    };
    let rendered = render_with(&running, running_view, 100, Folds::default());
    assert!(
        rendered.contains("codegraph · codegraph_explore"),
        "{rendered}"
    );
    assert!(rendered.contains("McpManager"), "{rendered}");

    let mut failed = running.clone();
    failed.settle(
        ToolStatus::Failed,
        Some("mcp tool timed out after 30s".into()),
    );
    let Some(crate::app::state::ToolDetailView::Mcp(failed_view)) = failed.detail.as_deref() else {
        panic!("failed MCP tool must preserve McpDetailView");
    };
    let rendered_failed = render_with(&failed, failed_view, 100, Folds::default());
    assert!(
        rendered_failed.contains("codegraph · codegraph_explore"),
        "{rendered_failed}"
    );
    assert!(
        rendered_failed.contains("mcp tool timed out after 30s"),
        "{rendered_failed}"
    );
}

#[test]
fn long_result_is_expandable() {
    let mut view = view();
    view.result = (0..30).map(|index| format!("line {index}")).collect();
    view.result_more = 10;
    assert!(has_more(&call(), &view, 80));
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
    let total = rows(&call, &view, 100, folds);
    let regions = super::super::card_regions(&call, 100, folds, total);
    assert!(regions.iter().any(|region| region.clickable));
    assert_eq!(regions.len(), 2, "args and result have separate regions");
    assert_eq!(regions[0].section, crate::app::state::CardSection::Body);
    assert_eq!(regions[1].section, crate::app::state::CardSection::Output);
    for region in &regions {
        assert!(region.card.offset + region.card.rows <= total);
    }
    let expanded = layout(
        &call,
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
    assert_eq!(result_row.segments[0].style, style::muted());
}

#[test]
fn collapsed_and_expanded_result_use_muted_markdown_styles() {
    let mut view = view();
    view.result = (0..10)
        .map(|index| format!("line {index} with `code_{index}`"))
        .collect();
    view.result_more = 7;
    let folded = layout(&call(), &view, 100, Folds::default());
    let result_row = folded
        .layout
        .rows
        .iter()
        .find(|row| row.segments.iter().any(|seg| seg.text.contains("line 0")))
        .expect("folded result");
    assert_eq!(result_row.segments[0].style, style::muted());
    assert_eq!(
        result_row
            .segments
            .iter()
            .find(|seg| seg.text == "code_0")
            .expect("folded inline code")
            .style,
        style::md_read_inline_code().patch(style::muted())
    );
    let expanded = layout(
        &call(),
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
    assert_eq!(result_row.segments[0].style, style::muted());
    assert_eq!(
        result_row
            .segments
            .iter()
            .find(|seg| seg.text == "code_0")
            .expect("expanded inline code")
            .style,
        style::md_read_inline_code().patch(style::muted())
    );
}

#[test]
fn expanding_branch_keeps_bar_and_expanding_leaf_uses_padding() {
    let mut view = view();
    view.arguments = (0..10).map(|index| format!("arg {index}")).collect();
    view.result = (0..10).map(|index| format!("line {index}")).collect();
    let expanded = layout(
        &call(),
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
