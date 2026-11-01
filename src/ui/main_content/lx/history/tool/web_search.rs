//! `web_search` 工具卡片：`■ Web search · 2 queries · exa · 840ms` + 查询/结果树体。
//!
//! `├─` 块是本次检索的查询词（首条附非默认参数），`└─` 块是压暗 Markdown 结果正文。
//! 两块各自折叠、各自登记点击面：点查询块只切换查询，点结果块只切换结果。
//! 结果正文与思考块同款（`muted_markdown_lines`），折叠态只显示前 3 行。

use ratatui::layout::Rect;

use crate::app::state::lx::chat::tool_view::MAX_RESULT_LINE_CHARS;
use crate::app::state::{ToolCallView, ToolStatus, WebSearchDetailView};
use crate::chat::tools::web_search::MAX_QUERY_CHARS;
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::tree::{self, Node, Preview, Section};
use super::{Folds, Segment, header_with_verb, muted_markdown_lines, put_segments, single_line};

const BLOCK_PREVIEW_ROWS: usize = 3;
/// 失败/中断原因最多保留的行数。
const MAX_ERROR_LINES: usize = 20;

pub(super) struct WebSearchLayout {
    layout: tree::Layout,
    /// 查询块（Body）的物理行数（含标题行时为 offset 0 的区域）。
    pub(super) queries_rows: usize,
    /// 结果块（Output）的物理行数。
    pub(super) result_rows: usize,
    pub(super) queries_expandable: bool,
    pub(super) result_expandable: bool,
}

/// 首条查询行上的非默认参数（`deep` / `5 results`）；都是默认值时返回 None。
fn non_default_params(view: &WebSearchDetailView) -> Option<String> {
    let mut parts = Vec::new();
    if !view.search_type_is_default() {
        parts.push(view.search_type.clone());
    }
    if !view.num_results_is_default() {
        parts.push(text::agent::results_count_label(view.num_results));
    }
    (!parts.is_empty()).then(|| parts.join(text::LX_CHAT_TOOL_STATUS_SEP))
}

fn queries_section(view: &WebSearchDetailView, expanded: bool) -> Option<Section> {
    if view.queries.is_empty() {
        return None;
    }
    let params = non_default_params(view);
    let lines: Vec<Vec<Segment>> = view
        .queries
        .iter()
        .enumerate()
        .map(|(index, query)| {
            let mut segments = vec![Segment::new(
                format!("\"{}\"", single_line(query, MAX_QUERY_CHARS)),
                style::chat_tool_param(),
            )];
            if index == 0
                && let Some(params) = params.as_deref()
            {
                segments.push(Segment::new(
                    format!("{}{params}", text::LX_CHAT_TOOL_STATUS_SEP),
                    style::agent_step_detail(),
                ));
            }
            segments
        })
        .collect();
    Some(Section::new(
        vec![Node::block(lines)],
        (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
    ))
}

/// 结果单行净化：控制字符转空格（`markdown_render` 不接受裸控制字符），超长截断补 `…`。
fn sanitize_result_line(line: &str) -> String {
    let mut text = String::with_capacity(line.len().min(MAX_RESULT_LINE_CHARS + 4));
    for (count, ch) in line.chars().enumerate() {
        if count == MAX_RESULT_LINE_CHARS {
            text.push('…');
            break;
        }
        text.push(if ch.is_control() { ' ' } else { ch });
    }
    text.trim_end().to_string()
}

fn result_section(
    call: &ToolCallView,
    view: &WebSearchDetailView,
    width: u16,
    expanded: bool,
) -> Option<Section> {
    let error_fallback = if view.result.is_empty()
        && matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted)
    {
        call.error.as_deref().filter(|error| !error.is_empty())
    } else {
        None
    };
    if view.result.is_empty() && error_fallback.is_none() {
        return None;
    }
    let body_width = u16::try_from(tree::content_width(width)).unwrap_or(u16::MAX);
    let (raw_source, more) = if let Some(error) = error_fallback {
        (
            error
                .lines()
                .take(MAX_ERROR_LINES)
                .map(sanitize_result_line)
                .collect::<Vec<_>>()
                .join("\n"),
            error.lines().count().saturating_sub(MAX_ERROR_LINES),
        )
    } else {
        (
            view.result
                .iter()
                .map(|line| sanitize_result_line(line))
                .collect::<Vec<_>>()
                .join("\n"),
            view.result_more,
        )
    };
    let mut lines = muted_markdown_lines(&raw_source, body_width);
    if more > 0 {
        lines.push(vec![Segment::new(
            text::mcp::more_lines(more),
            style::fold_hint(),
        )]);
    }
    if lines.is_empty() {
        return None;
    }
    Some(Section::new(
        vec![Node::block(lines)],
        (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
    ))
}

pub(super) fn layout(
    call: &ToolCallView,
    view: &WebSearchDetailView,
    width: u16,
    folds: Folds,
) -> WebSearchLayout {
    let folded_queries = queries_section(view, false);
    let folded_result = result_section(call, view, width, false);
    let queries_expandable = folded_queries
        .as_ref()
        .is_some_and(|section| tree::layout(std::slice::from_ref(section), width).hidden > 0);
    let result_expandable = folded_result
        .as_ref()
        .is_some_and(|section| tree::layout(std::slice::from_ref(section), width).hidden > 0);
    let queries = queries_section(view, folds.body);
    let result = result_section(call, view, width, folds.output);
    let queries_rows = queries.as_ref().map_or(0, |section| {
        tree::layout(std::slice::from_ref(section), width)
            .rows
            .len()
    });
    let result_rows = result.as_ref().map_or(0, |section| {
        tree::layout(std::slice::from_ref(section), width)
            .rows
            .len()
    });
    let sections: Vec<Section> = queries.into_iter().chain(result).collect();
    WebSearchLayout {
        layout: tree::layout(&sections, width),
        queries_rows,
        result_rows,
        queries_expandable,
        result_expandable,
    }
}

/// 卡片行数（含标题行）。
pub(super) fn rows(
    call: &ToolCallView,
    view: &WebSearchDetailView,
    width: u16,
    folds: Folds,
) -> usize {
    if width == 0 {
        return 0;
    }
    1 + layout(call, view, width, folds).layout.rows.len()
}

/// 展开后是否真的多出内容（查询块或结果块任一即可）。
pub(super) fn has_more(call: &ToolCallView, view: &WebSearchDetailView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    let layout = layout(call, view, width, Folds::default());
    layout.queries_expandable || layout.result_expandable
}

/// 标题行状态：运行中说在搜什么，完成态报引擎/耗时/截断，失败与中断交给标题染色。
fn title_status(call: &ToolCallView, view: &WebSearchDetailView) -> Option<String> {
    match call.status {
        ToolStatus::Failed | ToolStatus::Interrupted => None,
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running => {
            Some(if view.queries.len() > 1 {
                format!(
                    "{}{}{}",
                    text::agent::queries_label(view.queries.len()),
                    text::LX_CHAT_TOOL_STATUS_SEP,
                    text::agent::WEB_SEARCHING
                )
            } else {
                text::agent::WEB_SEARCHING.to_string()
            })
        }
        ToolStatus::Cancelled => Some(text::tool_status_label(call.status).to_string()),
        ToolStatus::Completed => {
            let mut parts = Vec::new();
            if view.queries.len() > 1 {
                parts.push(text::agent::queries_label(view.queries.len()));
            }
            if let Some(provider) = view.provider.as_deref() {
                parts.push(provider.to_string());
            }
            if let Some(duration_ms) = view.duration_ms {
                parts.push(text::agent::duration_label(duration_ms));
            }
            if view.truncated {
                parts.push(text::agent::SEARCH_TRUNCATED.to_string());
            }
            (!parts.is_empty()).then(|| parts.join(text::LX_CHAT_TOOL_STATUS_SEP))
        }
    }
}

/// 绘制联网搜索卡片；推进行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &WebSearchDetailView,
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
            text::agent::VERB_TOOL_WEB_SEARCH,
            title_status(call, view),
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

#[cfg(test)]
mod tests;
