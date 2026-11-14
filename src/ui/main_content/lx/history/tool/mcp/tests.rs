use super::*;
use crate::app::state::{McpDetailView, McpImageView, ToolCallView, ToolDetailView, ToolStatus};
use crate::ui::text;

fn view() -> McpDetailView {
    McpDetailView {
        server: "github".into(),
        tool: "create_issue".into(),
        summary: Some("Fix login crash on iOS".into()),
        arguments: vec!["{".into(), r#"  "title": "Fix login crash on iOS""#.into()],
        arguments_more: 0,
        result: vec!["created #42".into()],
        result_more: 0,
        images: vec![McpImageView {
            path: "/tmp/tool.png".into(),
            mime: "image/png".into(),
            width: 10,
            height: 20,
        }],
        images_omitted: 0,
        duration_ms: Some(1200),
    }
}

fn call(status: ToolStatus, error: Option<&str>) -> ToolCallView {
    ToolCallView {
        call_id: "call-mcp".into(),
        name: "mcp__github__create_issue".into(),
        position: 0,
        arguments_json: r#"{"title":"Fix login crash on iOS","body":"details"}"#.into(),
        status,
        summary: None,
        error: error.map(str::to_string),
        result_json: None,
        detail: None,
    }
}

fn render_with(call: &ToolCallView, view: &McpDetailView, width: u16, folds: Folds) -> String {
    let total = rows(call, view, width, folds);
    let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row = 0;
    draw(&mut grid, rect, 0, &mut row, call, view, 0, folds, false);
    assert_eq!(row, total, "绘制行数必须与 rows 一致");
    text_of(&grid, width, total)
}

fn render_body(call: &ToolCallView, view: &McpDetailView, width: u16, last: bool) -> String {
    let total = layout_with_group(call, view, width, Folds::default(), true)
        .layout
        .rows
        .len();
    let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row = 0;
    draw_body(
        &mut grid,
        rect,
        0,
        &mut row,
        call,
        view,
        0,
        Folds::default(),
        false,
        last,
    );
    assert_eq!(row, total, "调用体绘制行数必须与排版一致");
    text_of(&grid, width, total)
}

fn text_of(grid: &Grid, width: u16, rows: usize) -> String {
    let mut text = String::new();
    for y in 0..rows {
        for x in 0..width {
            text.push(grid.cell(x, y as u16).ch);
        }
        text.push('\n');
    }
    text
}

#[test]
fn single_card_shows_only_identity_duration_and_image_count() {
    let call = call(ToolStatus::Completed, None);
    let view = view();
    let text = render_with(&call, &view, 100, Folds::default());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(rows(&call, &view, 100, Folds::default()), 2);
    assert!(
        lines[0].starts_with(text::agent::STEP_MARK_FILLED)
            && lines[0].contains(text::agent::VERB_MCP)
            && lines[0].contains("1.2s")
            && lines[0].contains("1 image(s)"),
        "标题只有 MCP + 耗时 + 图片数：{lines:?}"
    );
    assert!(
        lines[1].starts_with(text::agent::TREE_LEAF) && lines[1].contains("github · create_issue"),
        "正文只有 server · tool 身份行：{lines:?}"
    );
    assert!(
        !text.contains("Fix login crash on iOS") && !text.contains("created #42"),
        "MCP 身份卡不得显示参数或结果：{text}"
    );
    assert!(!text.contains("[image"), "不得渲染图片元数据行：{text}");
}

#[test]
fn identity_line_uses_param_normal_and_muted_styles() {
    let layout = layout_with_group(
        &call(ToolStatus::Completed, None),
        &view(),
        100,
        Folds::default(),
        false,
    );
    let row = &layout.layout.rows[0];
    let server = row
        .segments
        .iter()
        .find(|segment| segment.text == "github")
        .expect("server 段");
    assert_eq!(server.style, style::chat_tool_param());
    let tool = row
        .segments
        .iter()
        .find(|segment| segment.text == "create_issue")
        .expect("tool 段");
    assert_eq!(tool.style, style::text());
    let sep = row
        .segments
        .iter()
        .find(|segment| segment.text == text::LX_CHAT_TOOL_STATUS_SEP)
        .expect("分隔段");
    assert_eq!(sep.style, style::agent_step_detail());
}

#[test]
fn failed_card_appends_one_reason_line_without_duration() {
    let call = call(ToolStatus::Failed, Some("mcp tool timed out after 30s"));
    let text = render_with(&call, &view(), 100, Folds::default());
    assert!(text.contains("github · create_issue"), "{text}");
    assert!(
        text.contains("failed: mcp tool timed out after 30s"),
        "{text}"
    );
    assert!(!text.contains("1.2s"), "失败卡片不显示耗时状态：{text}");
}

#[test]
fn interrupted_card_appends_one_reason_line() {
    let call = call(ToolStatus::Interrupted, Some("cancelled by user"));
    let text = render_with(&call, &view(), 100, Folds::default());
    assert!(text.contains("interrupted: cancelled by user"), "{text}");
    assert!(!text.contains("1.2s"), "中断卡片不显示耗时状态：{text}");
}

#[test]
fn interrupted_reason_equal_to_status_word_is_not_duplicated() {
    let call = call(
        ToolStatus::Interrupted,
        Some(crate::db::chat::INTERRUPTED_ERROR_CODE),
    );
    let text = render_with(&call, &view(), 100, Folds::default());
    assert_eq!(
        text.matches(crate::db::chat::INTERRUPTED_ERROR_CODE)
            .count(),
        1,
        "稳定错误码与状态词重复时只显示一次：{text}"
    );
    assert!(!text.contains("interrupted: interrupted"), "{text}");
}

#[test]
fn interrupted_single_card_uses_warn_connector_color() {
    let call = call(ToolStatus::Interrupted, Some("cancelled by user"));
    let view = view();
    let total = rows(&call, &view, 80, Folds::default());
    let rect = Rect::new(0, 0, 80, u16::try_from(total).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row = 0;
    draw(
        &mut grid,
        rect,
        0,
        &mut row,
        &call,
        &view,
        0,
        Folds::default(),
        false,
    );
    assert_eq!(
        Some(grid.cell(2, 1).style.fg),
        Some(style::agent_warn().fg),
        "中断单体身份行的连接符应染黄"
    );
}

#[test]
fn cancelled_card_shows_status_word_without_duration() {
    let call = call(ToolStatus::Cancelled, None);
    let text = render_with(&call, &view(), 100, Folds::default());
    assert!(text.contains("cancelled"), "{text}");
    assert!(!text.contains("1.2s"), "取消卡片不显示耗时状态：{text}");
}

#[test]
fn image_count_includes_omitted_images() {
    let mut view = view();
    view.images_omitted = 2;
    let text = render_with(
        &call(ToolStatus::Completed, None),
        &view,
        100,
        Folds::default(),
    );
    assert!(text.contains("3 image(s)"), "省略图片也计入总数：{text}");
}

#[test]
fn identity_fields_are_single_line_and_bounded() {
    let mut view = view();
    view.server = format!("server\nwith-control{}", "x".repeat(10_000));
    view.tool = format!("tool\twith-control{}", "y".repeat(10_000));
    let call = call(ToolStatus::Completed, None);
    let total = rows(&call, &view, 40, Folds::default());
    assert!(total < 100, "超长身份字段必须有界：{total} rows");
    let text = render_with(&call, &view, 40, Folds::default());
    assert!(text.contains("server with-control"), "{text}");
    assert!(text.contains("tool with-"), "{text}");
    assert!(text.contains("controly"), "{text}");
}

#[test]
fn streaming_new_call_renders_identity_card_not_generic_card() {
    let call = ToolCallView::new("call-streaming", "mcp__github__create_issue", 0);
    let total = super::super::rows(&call, 80, Folds::default());
    let rect = Rect::new(0, 0, 80, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row = 0;
    super::super::draw(&mut grid, rect, 0, &mut row, &call, 0, Folds::default());
    let text = text_of(&grid, 80, total);
    assert!(text.contains(text::agent::VERB_MCP), "{text}");
    assert!(text.contains("github · create_issue"), "{text}");
    assert!(!text.contains("receiving input"), "{text}");
}

#[test]
fn failed_card_without_reason_has_no_reason_line() {
    for error in [None, Some("")] {
        let call = call(ToolStatus::Failed, error);
        let view = view();
        assert_eq!(rows(&call, &view, 100, Folds::default()), 2);
        let text = render_with(&call, &view, 100, Folds::default());
        assert!(!text.contains("failed:"), "{text}");
    }
}

#[test]
fn failed_single_card_uses_semantic_connector_color() {
    let call = call(ToolStatus::Failed, Some("server not connected"));
    let view = view();
    let total = rows(&call, &view, 80, Folds::default());
    let rect = Rect::new(0, 0, 80, u16::try_from(total).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row = 0;
    draw(
        &mut grid,
        rect,
        0,
        &mut row,
        &call,
        &view,
        0,
        Folds::default(),
        false,
    );
    assert_eq!(
        Some(grid.cell(2, 1).style.fg),
        Some(style::agent_fail().fg),
        "失败单体身份行的连接符应染红"
    );
}

#[test]
fn multiline_reason_is_collapsed_to_one_truncated_line() {
    let call = call(
        ToolStatus::Failed,
        Some("first line\nsecond line that is far too long for the card"),
    );
    let text = render_with(&call, &view(), 30, Folds::default());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "标题 + 身份行 + 原因行：{lines:?}");
    assert!(
        lines[2].contains("failed:") && lines[2].contains('…'),
        "多行原因折叠为单行并尾部省略：{lines:?}"
    );
}

#[test]
fn mcp_card_has_no_expandable_content_or_click_region() {
    let mut call = call(ToolStatus::Completed, None);
    let view = view();
    call.detail = Some(Box::new(ToolDetailView::Mcp(view.clone())));
    let folds = Folds::default();
    assert!(!super::super::has_expandable_content(&call, 100));
    let total = super::super::rows(&call, 100, folds);
    let regions = super::super::card_regions(&call, 100, folds, total);
    assert!(regions.is_empty(), "MCP 身份卡不得登记点击面");
}

#[test]
fn long_server_and_tool_wrap_without_losing_content() {
    let mut view = view();
    view.server = "a-very-long-mcp-server-name-that-needs-wrapping".into();
    view.tool = "a_very_long_tool_name_that_also_needs_wrapping".into();
    let text = render_with(
        &call(ToolStatus::Completed, None),
        &view,
        24,
        Folds::default(),
    );
    assert!(text.contains("a-very-long"), "{text}");
    assert!(
        text.contains("a_very_") && text.contains("long_tool_name"),
        "折行不得吞掉长 tool 名的后半段：{text}"
    );
}

#[test]
fn narrow_widths_keep_rows_and_draw_in_step() {
    for width in [1u16, 4, 8, 12, 20] {
        let text = render_with(
            &call(ToolStatus::Completed, None),
            &view(),
            width,
            Folds::default(),
        );
        assert!(!text.is_empty(), "width {width} must render something");
    }
}

#[test]
fn group_body_puts_server_at_root_and_tool_below_it() {
    let call = call(ToolStatus::Completed, None);
    let text = render_body(&call, &view(), 80, false);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with(text::agent::TREE_BRANCH)
            && lines[0].contains("github")
            && lines[0].contains("1.2s")
            && lines[0].contains("1 image(s)"),
        "server 是一级分支并承载耗时与图片数：{lines:?}"
    );
    assert!(
        lines[1].starts_with(text::agent::TREE_NESTED_BRANCH_LEAF)
            && lines[1].contains("create_issue"),
        "tool 是 server 下的二级叶子：{lines:?}"
    );
}

#[test]
fn group_body_puts_error_reason_below_tool() {
    let call = call(ToolStatus::Failed, Some("server not connected"));
    let text = render_body(&call, &view(), 80, true);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with(text::agent::TREE_LEAF) && lines[0].contains("github"),
        "末调用 server 用 └─ 收口：{lines:?}"
    );
    assert!(
        !lines[0].contains("1.2s"),
        "失败调用的 server 行不写耗时：{lines:?}"
    );
    assert!(
        lines[1].starts_with(text::agent::TREE_NESTED_LEAF_BRANCH)
            && lines[1].contains("create_issue"),
        "tool 是 server 下的二级分支：{lines:?}"
    );
    assert!(
        lines[2].starts_with(text::agent::TREE_NESTED_LEAF_LEAF)
            && lines[2].contains("failed: server not connected"),
        "错误原因是 tool 的兄弟二级叶子：{lines:?}"
    );
}
