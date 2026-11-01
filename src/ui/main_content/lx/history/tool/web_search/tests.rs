use super::*;
use crate::app::state::{ToolDetailView, ToolSummary};

fn call() -> ToolCallView {
    ToolCallView {
        call_id: "call-web-search".into(),
        name: "web_search".into(),
        position: 0,
        arguments_json: r#"{"queries":["ratatui 0.29","tokio 1.40"]}"#.into(),
        status: ToolStatus::Completed,
        summary: Some(Box::new(ToolSummary::default())),
        error: None,
        result_json: None,
        detail: None,
    }
}

fn view() -> WebSearchDetailView {
    WebSearchDetailView {
        queries: vec!["ratatui 0.29".into(), "tokio 1.40".into()],
        search_type: "auto".into(),
        num_results: 8,
        provider: Some("exa".into()),
        result: vec!["1. Ratatui 0.29".into(), "https://ratatui.rs".into()],
        result_more: 0,
        duration_ms: Some(840),
        truncated: false,
    }
}

fn render_with(
    call: &ToolCallView,
    view: &WebSearchDetailView,
    width: u16,
    folds: Folds,
) -> String {
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

#[test]
fn completed_card_shows_queries_provider_and_duration() {
    let text = render_with(&call(), &view(), 100, Folds::default());
    assert!(text.contains("Web search"), "{text}");
    assert!(text.contains("2 queries"), "{text}");
    assert!(text.contains("exa"), "{text}");
    assert!(text.contains("840ms"), "{text}");
    assert!(text.contains("\"ratatui 0.29\""), "{text}");
    assert!(text.contains("\"tokio 1.40\""), "{text}");
    assert!(text.contains("https://ratatui.rs"), "{text}");
}

#[test]
fn single_query_completed_card_omits_the_query_count() {
    let mut view = view();
    view.queries.truncate(1);
    let text = render_with(&call(), &view, 100, Folds::default());
    assert!(!text.contains("1 query"), "{text}");
    assert!(text.contains("exa · 840ms"), "{text}");
}

#[test]
fn running_card_reports_query_count_and_searching() {
    let mut call = call();
    call.status = ToolStatus::Running;
    let mut view = view();
    view.provider = None;
    view.duration_ms = None;
    let text = render_with(&call, &view, 100, Folds::default());
    assert!(text.contains("2 queries · Searching…"), "{text}");
    let mut call = call.clone();
    call.status = ToolStatus::Running;
    let mut single = view.clone();
    single.queries.truncate(1);
    let text = render_with(&call, &single, 100, Folds::default());
    assert!(text.contains("· Searching…"), "{text}");
    assert!(!text.contains("1 query"), "{text}");
}

#[test]
fn first_query_carries_only_non_default_parameters() {
    let mut view = view();
    view.search_type = "deep".into();
    view.num_results = 5;
    let folded = layout(&call(), &view, 100, Folds::default());
    let query_row = folded
        .layout
        .rows
        .iter()
        .find(|row| {
            row.segments
                .iter()
                .any(|segment| segment.text.contains("ratatui 0.29"))
        })
        .expect("query row");
    let params_row = query_row
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<String>();
    assert!(params_row.contains("deep · 5 results"), "{params_row}");
    view.search_type = "auto".into();
    view.num_results = 8;
    let folded = layout(&call(), &view, 100, Folds::default());
    let query_row = folded
        .layout
        .rows
        .iter()
        .find(|row| {
            row.segments
                .iter()
                .any(|segment| segment.text.contains("ratatui 0.29"))
        })
        .expect("query row");
    assert!(
        !query_row
            .segments
            .iter()
            .any(|segment| segment.text.contains("results"))
    );
}

#[test]
fn long_result_is_expandable_and_uses_muted_markdown() {
    let mut call = call();
    let mut view = view();
    view.result = (0..10)
        .map(|index| format!("line {index} with `code_{index}`"))
        .collect();
    view.result_more = 7;
    call.detail = Some(Box::new(ToolDetailView::WebSearch(view.clone())));
    assert!(has_more(&call, &view, 100));
    let folded = layout(&call, &view, 100, Folds::default());
    let result_row = folded
        .layout
        .rows
        .iter()
        .find(|row| row.segments.iter().any(|seg| seg.text.contains("line 0")))
        .expect("folded result");
    assert_eq!(result_row.segments[0].style, style::muted());
    let expanded = layout(
        &call,
        &view,
        100,
        Folds {
            body: false,
            output: true,
        },
    );
    let result_row = expanded
        .layout
        .rows
        .iter()
        .find(|row| row.segments.iter().any(|seg| seg.text.contains("line 8")))
        .expect("expanded result");
    assert_eq!(result_row.segments[0].style, style::muted());
    assert_eq!(
        result_row
            .segments
            .iter()
            .find(|seg| seg.text == "code_8")
            .expect("expanded inline code")
            .style,
        style::md_read_inline_code().patch(style::muted())
    );
}

#[test]
fn queries_and_result_register_independent_click_regions() {
    let mut call = call();
    let mut view = view();
    view.queries = (0..6).map(|index| format!("query {index}")).collect();
    view.result = (0..10).map(|index| format!("line {index}")).collect();
    view.result_more = 7;
    call.detail = Some(Box::new(ToolDetailView::WebSearch(view.clone())));
    let folds = Folds::default();
    let total = rows(&call, &view, 100, folds);
    let regions = super::super::card_regions(&call, 100, folds, total);
    assert_eq!(regions.len(), 2, "查询块与结果块各一个点击面");
    assert_eq!(regions[0].section, crate::app::state::CardSection::Body);
    assert_eq!(regions[1].section, crate::app::state::CardSection::Output);
    assert!(regions.iter().all(|region| region.clickable));
    for region in &regions {
        assert!(region.card.offset + region.card.rows <= total);
    }
    // 展开结果块不改变查询块的行数（折叠互不影响）。
    let folded_queries = layout(&call, &view, 100, Folds::default()).queries_rows;
    let expanded_result = layout(
        &call,
        &view,
        100,
        Folds {
            body: false,
            output: true,
        },
    );
    assert_eq!(folded_queries, expanded_result.queries_rows);
}

#[test]
fn expanding_the_query_block_keeps_bar_and_leaf_uses_padding() {
    let mut view = view();
    view.queries = (0..6).map(|index| format!("query {index}")).collect();
    view.result = (0..6).map(|index| format!("line {index}")).collect();
    let expanded = layout(
        &call(),
        &view,
        100,
        Folds {
            body: true,
            output: true,
        },
    );
    let query_row = expanded
        .layout
        .rows
        .iter()
        .find(|row| {
            row.segments
                .iter()
                .any(|segment| segment.text.contains("query 1"))
        })
        .expect("query continuation");
    assert_eq!(query_row.prefix, text::agent::TREE_BAR);
    let result_row = expanded
        .layout
        .rows
        .iter()
        .find(|row| {
            row.segments
                .iter()
                .any(|segment| segment.text.contains("line 1"))
        })
        .expect("result continuation");
    assert_eq!(result_row.prefix, text::agent::TREE_PAD);
}

#[test]
fn result_only_card_registers_a_single_region_from_the_header() {
    // 失败且查询词为空：只有结果块，点击面必须从标题行起算。
    let mut call = call();
    call.status = ToolStatus::Failed;
    call.error = Some("web search failed across all providers".into());
    let mut view = view();
    view.queries.clear();
    view.provider = None;
    view.duration_ms = None;
    view.result = (0..10).map(|index| format!("line {index}")).collect();
    view.result_more = 7;
    call.detail = Some(Box::new(ToolDetailView::WebSearch(view.clone())));
    let folds = Folds::default();
    let card = layout(&call, &view, 100, folds);
    assert_eq!(card.queries_rows, 0);
    assert!(card.result_rows > 0);
    let total = rows(&call, &view, 100, folds);
    let regions = super::super::card_regions(&call, 100, folds, total);
    assert_eq!(regions.len(), 1, "没有查询块时只登记结果点击面");
    assert_eq!(regions[0].section, crate::app::state::CardSection::Output);
    assert_eq!(regions[0].card.offset, 0, "结果块紧接标题行");
    assert_eq!(regions[0].card.rows, total);
    assert!(regions[0].card.offset + regions[0].card.rows <= total);
    assert!(regions[0].clickable);
}

#[test]
fn narrow_width_keeps_draw_rows_in_sync_with_rows() {
    let mut call = call();
    call.status = ToolStatus::Failed;
    call.error = Some("web search failed across all providers: upstream timeout".into());
    let mut view = view();
    view.queries = vec!["a long query that wraps on a narrow card".into()];
    view.result = (0..6)
        .map(|index| format!("line {index} with `code_{index}` and a very long tail to wrap"))
        .collect();
    view.result_more = 4;
    view.provider = None;
    view.duration_ms = None;
    let folded = render_with(&call, &view, 36, Folds::default());
    assert!(folded.contains("line 0"), "{folded}");
    let expanded = render_with(
        &call,
        &view,
        36,
        Folds {
            body: true,
            output: true,
        },
    );
    assert!(expanded.contains("line 5"), "{expanded}");
}

#[test]
fn failed_card_surfaces_the_error_in_the_result_block() {
    let mut call = call();
    call.status = ToolStatus::Failed;
    call.error = Some("web search failed across all providers".into());
    let mut view = view();
    view.result.clear();
    view.result_more = 0;
    view.provider = None;
    view.duration_ms = None;
    let text = render_with(&call, &view, 100, Folds::default());
    assert!(
        text.contains("web search failed across all providers"),
        "{text}"
    );
    assert!(!text.contains("exa"), "{text}");
}
