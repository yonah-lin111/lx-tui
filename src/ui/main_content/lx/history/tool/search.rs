//! `search` 工具卡片：标题 + 摘要节点 + 可复制、可展开的命中行。
//!
//! ```text
//! ■ Search · 3 matches
//!   ├─ "pattern" in src
//!   └─ src/foo.rs:12: matching line
//!      src/bar.rs:7: another line
//!      … 8 more lines
//! ```
//!
//! - 摘要字段（pattern/scope、命中数、结果级截断）与命中行都走同一套树体；
//! - 命中行就是普通网格文本，历史区的拖拽选区/Ctrl+C 与命令输出同源；
//! - 合成的行尾 `…[truncated]` 不进入可见行：超长行只由 `line_truncated`
//!   元数据追加一个 `…`，不伪造成正文；
//! - 结果级 `truncated` 是**执行事实**，单列节点，不混入预览省略行数。

use ratatui::layout::Rect;
#[cfg(test)]
use ratatui::style::Style;

use crate::app::state::{SearchDetailView, ToolCallView};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::file_summary;
use super::tree::{self, Node, Preview, Section};
use super::{Segment, TEXT_SAMPLE_CHARS, header_with_verb, put_segments, single_line};

/// 折叠态命中行最多显示的物理行数。
const MATCH_PREVIEW_ROWS: usize = 2;

/// 搜索卡片的全部物理行（标题行除外）。
struct SearchLayout {
    layout: tree::Layout,
}

fn hit_line(hit: &crate::app::state::SearchMatchView) -> Vec<Segment> {
    let path = single_line(&hit.path, TEXT_SAMPLE_CHARS);
    let raw = single_line(&hit.text, TEXT_SAMPLE_CHARS);
    let body = raw
        .strip_suffix("…[truncated]")
        .map(str::trim_end)
        .unwrap_or(raw.as_str())
        .to_string();
    let mut spans = vec![
        Segment::new(format!("{path}:{}: ", hit.line), style::agent_step_detail()),
        Segment::new(body, style::text()),
    ];
    if hit.line_truncated {
        spans.push(Segment::new(" …", style::agent_step_detail()));
    }
    spans
}

fn layout(
    call: &ToolCallView,
    view: &SearchDetailView,
    width: u16,
    expanded: bool,
) -> SearchLayout {
    let summary = file_summary::nodes(call);
    let mut sections = Vec::new();
    if !summary.is_empty() {
        sections.push(Section::fixed(summary));
    }
    let hits: Vec<Vec<Segment>> = view.matches.iter().map(hit_line).collect();
    if !hits.is_empty() {
        sections.push(Section::new(
            vec![Node::block(hits)],
            (!expanded).then(|| Preview::head(MATCH_PREVIEW_ROWS)),
        ));
    }
    SearchLayout {
        layout: tree::layout(&sections, width),
    }
}

/// 卡片行数（含标题行）。
pub(super) fn rows(
    call: &ToolCallView,
    view: &SearchDetailView,
    width: u16,
    expanded: bool,
) -> usize {
    if width == 0 {
        return 0;
    }
    1 + layout(call, view, width, expanded).layout.rows.len()
}

/// 展开后是否真的多出内容（决定是否登记点击面）。
pub(super) fn has_more(call: &ToolCallView, view: &SearchDetailView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    layout(call, view, width, false).layout.hidden > 0
}

/// 绘制搜索卡片；推进行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &SearchDetailView,
    loading_frame: usize,
    expanded: bool,
    animating: bool,
) {
    if rect.width == 0 {
        return;
    }
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let status = if matches!(
            call.status,
            crate::app::state::ToolStatus::Failed | crate::app::state::ToolStatus::Interrupted
        ) {
            None
        } else {
            let summary = call.summary.as_deref();
            let matches = summary
                .and_then(|summary| summary.matches)
                .or_else(|| (!view.matches.is_empty()).then_some(view.matches.len()));
            let mut parts = Vec::new();
            if let Some(matches) = matches {
                parts.push(text::agent::matches_label(matches));
            }
            if view.truncated || summary.is_some_and(|summary| summary.truncated) {
                parts.push(text::agent::SEARCH_TRUNCATED.to_string());
            }
            (!parts.is_empty()).then(|| parts.join(text::LX_CHAT_TOOL_STATUS_SEP))
        };
        let segments = header_with_verb(
            call,
            rect.width,
            loading_frame,
            animating,
            text::agent::VERB_TOOL_SEARCH,
            status,
        );
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    tree::draw(
        grid,
        rect,
        scroll,
        row_index,
        &layout(call, view, rect.width, expanded).layout.rows,
    );
}

/// 命中行的样式在网格里读取（测试用）。
#[cfg(test)]
fn row_style(layout: &SearchLayout, needle: &str) -> Option<Style> {
    layout
        .layout
        .rows
        .iter()
        .flat_map(|row| row.segments.iter())
        .find(|segment| segment.text.contains(needle))
        .map(|segment| segment.style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{SearchMatchView, ToolCallView, ToolStatus, ToolSummary};

    fn call() -> ToolCallView {
        ToolCallView {
            call_id: "call-search".into(),
            name: "search".into(),
            position: 0,
            arguments_json: r#"{"pattern":"fn render","path":"src"}"#.into(),
            status: ToolStatus::Completed,
            summary: Some(Box::new(ToolSummary {
                path: Some("src".into()),
                pattern: Some("fn render".into()),
                matches: Some(3),
                ..ToolSummary::default()
            })),
            error: None,
            result_json: None,
            detail: None,
        }
    }

    fn view() -> SearchDetailView {
        SearchDetailView {
            matches: vec![
                SearchMatchView {
                    path: "src/foo.rs".into(),
                    line: 12,
                    text: "pub fn render() {".into(),
                    line_truncated: false,
                },
                SearchMatchView {
                    path: "src/bar.rs".into(),
                    line: 7,
                    text: "fn render_search() {".into(),
                    line_truncated: false,
                },
                SearchMatchView {
                    path: "src/baz.rs".into(),
                    line: 3,
                    text: "render…[truncated]".into(),
                    line_truncated: true,
                },
            ],
            truncated: true,
            files_scanned: 12,
        }
    }

    fn text_of(call: &ToolCallView, view: &SearchDetailView, width: u16, expanded: bool) -> String {
        let total = rows(call, view, width, expanded);
        let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
        let mut grid = Grid::new(rect);
        let mut row = 0;
        draw(&mut grid, rect, 0, &mut row, call, view, 0, expanded, false);
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
    fn folded_search_shows_match_lines_and_more_hint() {
        let call = call();
        let view = view();
        let folded = text_of(&call, &view, 80, false);
        assert!(
            folded.contains("src/foo.rs:12: pub fn render() {"),
            "{folded}"
        );
        assert!(
            folded.contains("src/bar.rs:7: fn render_search() {"),
            "{folded}"
        );
        assert!(folded.contains("more line"), "{folded}");
        assert!(!folded.contains("…[truncated]"), "不内联截断标记：{folded}");
        assert!(
            folded.contains("results truncated"),
            "结果级截断事实：{folded}"
        );
        assert!(has_more(&call, &view, 80));
    }

    #[test]
    fn expanded_search_shows_all_matches_and_truncation_note() {
        let call = call();
        let view = view();
        let expanded = text_of(&call, &view, 100, true);
        assert!(expanded.contains("src/baz.rs:3: render"), "{expanded}");
        assert!(
            !expanded.contains("…[truncated]"),
            "命中行内不显示截断标记：{expanded}"
        );
        assert!(expanded.contains("results truncated"), "{expanded}");
        assert!(
            !expanded.contains("more line"),
            "全部展开后没有提示：{expanded}"
        );
    }

    #[test]
    fn zero_matches_is_a_valid_result() {
        let mut call = call();
        call.summary = Some(Box::new(ToolSummary {
            path: Some("src".into()),
            pattern: Some("nothing".into()),
            matches: Some(0),
            ..ToolSummary::default()
        }));
        let view = SearchDetailView {
            matches: Vec::new(),
            truncated: false,
            files_scanned: 3,
        };
        let text = text_of(&call, &view, 60, false);
        assert!(text.contains("0 matches"), "{text}");
        assert!(!has_more(&call, &view, 60));
    }

    #[test]
    fn search_match_text_is_grid_text_for_copy() {
        let call = call();
        let view = view();
        let text = text_of(&call, &view, 80, true);
        assert!(text.contains("pub fn render"), "{text}");
        assert_eq!(
            row_style(&layout(&call, &view, 80, true), "pub fn render"),
            Some(style::text())
        );
    }

    #[test]
    fn long_match_wraps_under_a_hit_block_and_keeps_count_exact() {
        let mut call = call();
        call.summary = Some(Box::new(ToolSummary {
            path: Some("src".into()),
            pattern: Some("x".into()),
            matches: Some(1),
            ..ToolSummary::default()
        }));
        let view = SearchDetailView {
            matches: vec![SearchMatchView {
                path: "src/a.rs".into(),
                line: 1,
                text: "y".repeat(120),
                line_truncated: false,
            }],
            truncated: false,
            files_scanned: 1,
        };
        let folded = layout(&call, &view, 40, false);
        let text: Vec<String> = folded
            .layout
            .segments()
            .iter()
            .map(|row| row.iter().map(|s| s.text.as_str()).collect())
            .collect();
        assert!(
            text[0].contains("Search") || text[0].contains('"'),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|line| line.contains("src/a.rs:1") && line.starts_with(text::agent::TREE_LEAF)),
            "命中块首行收口：{text:?}"
        );
        assert!(
            text.iter()
                .skip(2)
                .all(|line| line.starts_with(text::agent::TREE_PAD)),
            "命中续行与省略提示左对齐：{text:?}"
        );
        assert!(folded.layout.hidden > 0, "折叠态只展示前 2 个物理行");
        assert!(has_more(&call, &view, 40), "长命中行必须可以展开");
        let expanded = layout(&call, &view, 40, true);
        assert_eq!(expanded.layout.hidden, 0, "展开后没有隐藏行");
        // 失败/中断态标题不拼命中计数与截断状态。
        call.status = ToolStatus::Failed;
        call.error = Some("search failed".into());
        let failed = text_of(&call, &view, 80, false);
        let title = failed.lines().next().unwrap_or("");
        assert!(!title.contains("match"), "失败标题不带命中数：{title}");
    }
}
