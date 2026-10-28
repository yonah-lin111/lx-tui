//! `search` 工具卡片：标题 + 摘要 + 可复制、可展开的命中行列表。
//!
//! ```text
//! ■ Search
//!   └─ "pattern" in src · 3 matches
//!      src/foo.rs:12: matching line
//!      src/bar.rs:5: another line
//!      … 8 more lines
//! ```
//!
//! 命中行就是普通网格文本（缩进 5 列），因此历史区的拖拽选区/Ctrl+C
//! 与命令输出完全同源，不需要单独的复制按钮或剪贴板分支。

use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::state::{SearchDetailView, ToolCallView};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::{Segment, TEXT_SAMPLE_CHARS, header_with_verb, put_segments, single_line, wrap_spans};

/// 折叠态摘要最多显示的行数。
const SUMMARY_PREVIEW_ROWS: usize = 2;
/// 折叠态命中行最多显示的行数。
const MATCH_PREVIEW_ROWS: usize = 2;
/// 命中行缩进（与 `  └─ ` 的前缀等宽）。
const MATCH_INDENT: usize = 5;

/// 搜索卡片的全部物理行（标题行除外），折叠/展开共用同一份布局。
struct SearchLayout {
    rows: Vec<Vec<Segment>>,
    /// 展开后的全部内容行数（不含折叠提示行）。
    content_rows: usize,
    /// 折叠态实际展示的内容行数。
    preview_rows: usize,
}

/// 摘要详情：复用普通文件工具的 `└─` 详情排版。
fn summary_rows(call: &ToolCallView, width: u16) -> Vec<Vec<Segment>> {
    super::file_summary::detail(call, width, true)
}

/// 单条命中行：`path:line: text`，按列宽折行并保持续行缩进。
fn match_rows(view: &SearchDetailView, width: u16) -> Vec<Vec<Segment>> {
    let body_width = usize::from(width).saturating_sub(MATCH_INDENT).max(1);
    let mut rows = Vec::new();
    for hit in &view.matches {
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
        for line in wrap_spans(&spans, body_width) {
            let mut row = vec![Segment::new(" ".repeat(MATCH_INDENT), Style::default())];
            row.extend(line);
            rows.push(row);
        }
    }
    rows
}

/// 结果级截断提示：不内联到摘要行，放在命中列表末尾。
fn truncated_row(view: &SearchDetailView) -> Option<Vec<Segment>> {
    view.truncated.then(|| {
        vec![Segment::new(
            format!("{}results truncated", " ".repeat(MATCH_INDENT)),
            style::agent_warn(),
        )]
    })
}

/// 组装折叠/展开两种形态。
fn layout(
    call: &ToolCallView,
    view: &SearchDetailView,
    width: u16,
    expanded: bool,
) -> SearchLayout {
    let summary = summary_rows(call, width);
    let summary_len = summary.len();
    let matches = match_rows(view, width);
    let match_len = matches.len();
    let mut all = summary;
    all.extend(matches);
    if let Some(note) = truncated_row(view) {
        all.push(note);
    }
    let preview_rows = summary_len.min(SUMMARY_PREVIEW_ROWS) + match_len.min(MATCH_PREVIEW_ROWS);
    let content_rows = all.len();
    let mut rows = if expanded {
        all
    } else {
        all.into_iter().take(preview_rows).collect()
    };
    if !expanded && content_rows > preview_rows {
        let hint = text::agent::file_diff_more_lines(content_rows - preview_rows);
        if hint.len() <= usize::from(width).saturating_sub(MATCH_INDENT) {
            rows.push(vec![Segment::new(
                format!("{}{hint}", " ".repeat(MATCH_INDENT)),
                style::agent_step_detail(),
            )]);
        }
    }
    SearchLayout {
        rows,
        content_rows,
        preview_rows,
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
    1 + layout(call, view, width, expanded).rows.len()
}

/// 展开后是否真的多出内容（决定是否登记点击面）。
pub(super) fn has_more(call: &ToolCallView, view: &SearchDetailView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    let layout = layout(call, view, width, false);
    layout.content_rows > layout.preview_rows
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
        let segments = header_with_verb(
            call,
            rect.width,
            loading_frame,
            animating,
            text::agent::VERB_TOOL_SEARCH,
            None,
        );
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    for segments in layout(call, view, rect.width, expanded).rows {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            put_segments(grid, rect.x, y, rect.width, &segments);
        }
        *row_index += 1;
    }
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
        assert!(
            !has_more(&call, &view, 100)
                || rows(&call, &view, 100, true) > rows(&call, &view, 100, false)
        );
    }

    #[test]
    fn search_match_text_is_grid_text_for_copy() {
        // 命中行已进入普通网格；历史区 copy_text 按网格取词，不依赖额外剪贴板路径。
        let call = call();
        let view = view();
        let text = text_of(&call, &view, 80, true);
        assert!(text.contains("pub fn render"), "{text}");
    }
}
