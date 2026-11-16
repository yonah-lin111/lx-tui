//! 连续同名工具组：分组键、组头、调用体行数与 headerless 绘制分派。
//!
//! 组头只占一行、不登记点击面；调用体仍复用各工具模块的树体布局，只跳过标题行。
//! 列表合并组在非末尾调用体尾部补一行 `  │` 分隔，快照组合并只有一个调用体，不补。

use ratatui::layout::Rect;

use crate::app::state::{ToolCallView, ToolDetailView, ToolStatus};
use crate::ui::{style, text};

use super::super::paint::GroupSlot;
use super::super::{Grid, visible_row};
use super::{Folds, Segment, header_with_tint, mark_of, put_segments};

/// 合并形态：普通列表合并，或 `update_plan` / `list_agents` 只显示最新快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupMode {
    List,
    Snapshot,
}

/// MCP 合并键：所有相邻 MCP 调用（含 resources 桥接工具）归同一个 `MCP` 组。
const MCP_GROUP_KEY: &str = "mcp";

/// 分组键：MCP 统一按 `mcp`；普通工具按精确工具名；`write_stdin` 再按显示动词区分。
pub(crate) fn group_key(call: &ToolCallView) -> String {
    if crate::mcp::is_mcp_or_resource_tool(&call.name) {
        MCP_GROUP_KEY.to_string()
    } else if call.name == "write_stdin" {
        format!("write_stdin:{}", display_verb(call))
    } else {
        call.name.clone()
    }
}

/// 分组形态。`ask` / `wait_agent` 由 `is_groupable` 排除，不会走到这里。
pub(crate) fn group_mode(call: &ToolCallView) -> GroupMode {
    match call.name.as_str() {
        "update_plan" | "list_agents" => GroupMode::Snapshot,
        _ => GroupMode::List,
    }
}

/// `ask` 是阻塞交互；`wait_agent` 每次等待的目标集合独立，合并会丢失“等了谁”的语义。
pub(crate) fn is_groupable(call: &ToolCallView) -> bool {
    !matches!(call.name.as_str(), "ask" | "wait_agent")
}

/// 工具标题动词：各模块的标题动词集中在这里，组头与单卡共用同一来源。
pub(crate) fn display_verb(call: &ToolCallView) -> String {
    // MCP 身份卡按工具名判定：流式早期没有 detail 时也必须显示 `MCP`。
    if crate::mcp::is_mcp_or_resource_tool(&call.name) {
        return text::agent::VERB_MCP.to_string();
    }
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(_)) => text::agent::VERB_EXEC_COMMAND.to_string(),
        Some(ToolDetailView::Interaction(view)) => {
            text::agent::interaction_verb(view.action).to_string()
        }
        Some(ToolDetailView::Plan(_)) => text::agent::VERB_UPDATE_PLAN.to_string(),
        Some(ToolDetailView::Ask(_)) => text::agent::VERB_TOOL_ASK.to_string(),
        Some(ToolDetailView::Search(_)) => text::agent::VERB_TOOL_SEARCH.to_string(),
        Some(ToolDetailView::Subagent(_)) => super::subagent::verb(&call.name).to_string(),
        Some(ToolDetailView::Mcp(_)) => text::agent::VERB_MCP.to_string(),
        Some(ToolDetailView::WebSearch(_)) => text::agent::VERB_TOOL_WEB_SEARCH.to_string(),
        Some(ToolDetailView::File(_)) if super::file_diff::is_card(call) => {
            super::file_diff::verb_of(call).to_string()
        }
        Some(ToolDetailView::File(_)) | None => super::verb_label(call),
    }
}

/// 组头：`■ 动词 · N calls`（快照合并为 `N updates`）+ 聚合状态/计数。
pub(crate) fn group_header(
    calls: &[&ToolCallView],
    total_count: usize,
    mode: GroupMode,
    width: u16,
    loading_frame: usize,
    animating: bool,
) -> Vec<Segment> {
    let Some(first) = calls.first() else {
        return Vec::new();
    };
    let worst = worst_status(calls);
    let mark = mark_of(worst, loading_frame);
    let tint = match worst {
        ToolStatus::Failed => Some(style::agent_fail()),
        ToolStatus::Interrupted => Some(style::agent_warn()),
        _ => None,
    };
    let count = match mode {
        GroupMode::List => text::agent::group_calls_label(total_count),
        GroupMode::Snapshot => text::agent::group_updates_label(total_count),
    };
    let mut status = vec![count];
    if let Some(aggregate) = aggregate_status(calls) {
        status.push(aggregate);
    }
    if let Some(metric) = aggregate_metric(calls) {
        status.push(metric);
    }
    let status = Some(status.join(text::LX_CHAT_TOOL_STATUS_SEP));
    header_with_tint(
        tint,
        width,
        loading_frame,
        animating,
        &display_verb(first),
        status,
        mark,
    )
}

/// 绘制组头并推进 1 行。
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_group_header(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    calls: &[&ToolCallView],
    total_count: usize,
    mode: GroupMode,
    loading_frame: usize,
    animating: bool,
) {
    if rect.width == 0 {
        return;
    }
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = group_header(
            calls,
            total_count,
            mode,
            rect.width,
            loading_frame,
            animating,
        );
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
}

/// 调用体行数（总行数减标题行；空详情补 1 行兜底）。
///
/// 非末尾调用的尾部额外占 1 行分隔（`  │`），由 [`draw_body_animated`] 绘制。
pub(crate) fn body_rows(call: &ToolCallView, width: u16, folds: Folds, group: GroupSlot) -> usize {
    let total = super::rows_with_group(call, width, folds, true);
    if total == 0 {
        return 0;
    }
    total
        .saturating_sub(1)
        .max(1)
        .saturating_add(usize::from(!group.is_last()))
}

/// 调用体内的 agent 链接（按组内排版计算偏移）。
pub(crate) fn body_agent_links(call: &ToolCallView, width: u16) -> Vec<(usize, String)> {
    super::agent_links_with_group(call, width, true)
}

/// 调用体的点击面：把整卡区域整体上移一行，并去掉标题行覆盖。
pub(crate) fn body_card_regions(
    call: &ToolCallView,
    width: u16,
    folds: Folds,
    total_rows: usize,
    last: bool,
) -> Vec<super::CardRegion> {
    super::card_regions_with_group(call, width, folds, total_rows, true, last)
        .into_iter()
        .filter_map(|region| {
            let offset = region.card.offset;
            let rows = region.card.rows;
            if offset == 0 {
                if rows <= super::HEADER_ROWS {
                    return None;
                }
                Some(super::CardRegion {
                    card: super::RowSpan {
                        offset: 0,
                        rows: rows.saturating_sub(super::HEADER_ROWS),
                    },
                    cols: region.cols.skip_rows(super::HEADER_ROWS),
                    ..region
                })
            } else {
                Some(super::CardRegion {
                    card: super::RowSpan {
                        offset: offset.saturating_sub(super::HEADER_ROWS),
                        rows,
                    },
                    ..region
                })
            }
        })
        .collect()
}

/// headerless 绘制分派：只画树体，组内首/末连接符由各模块按 `last` 调整。
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_body_animated(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    group: GroupSlot,
    loading_frame: usize,
    folds: Folds,
    animating: bool,
) {
    if rect.width == 0 {
        return;
    }
    let last = group.is_last();
    let separator = usize::from(!last);
    let expected = body_rows(call, rect.width, folds, group);
    let tree_expected = expected.saturating_sub(separator);
    let before = *row_index;
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(_)) | Some(ToolDetailView::Interaction(_)) => {
            super::command::draw_body(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                folds,
                animating,
                last,
            );
        }
        Some(ToolDetailView::Plan(_)) => {
            super::plan::draw_body(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                animating,
                last,
            );
        }
        Some(ToolDetailView::Ask(_)) => {
            super::ask::draw_body(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                animating,
                last,
            );
        }
        Some(ToolDetailView::Subagent(_)) => {
            super::subagent::draw_body(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                folds.body,
                animating,
                last,
            );
        }
        Some(ToolDetailView::Search(view)) => {
            super::search::draw_body(
                grid,
                rect,
                scroll,
                row_index,
                call,
                view,
                loading_frame,
                folds.body,
                animating,
                last,
            );
        }
        Some(ToolDetailView::Mcp(view)) => {
            super::mcp::draw_body(
                grid,
                rect,
                scroll,
                row_index,
                call,
                view,
                loading_frame,
                folds,
                animating,
                last,
            );
        }
        Some(ToolDetailView::WebSearch(view)) => {
            super::web_search::draw_body(
                grid,
                rect,
                scroll,
                row_index,
                call,
                view,
                loading_frame,
                folds,
                animating,
                last,
            );
        }
        Some(ToolDetailView::File(_)) if super::file_diff::is_card(call) => {
            super::file_diff::draw_body(grid, rect, scroll, row_index, call, folds.body, last);
        }
        Some(ToolDetailView::File(_)) | None => {
            super::file_summary::draw_body(grid, rect, scroll, row_index, call, folds.body, last);
        }
    }
    // 空详情（旧记录/无参数）必须补一行兜底，保证组内每个调用可见且行数一致。
    let drawn = (*row_index).saturating_sub(before);
    let connector = if last {
        text::agent::TREE_LEAF
    } else {
        text::agent::TREE_BRANCH
    };
    let connector_style = super::tree::connector_style_of(call);
    for _ in drawn..tree_expected {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(rect.x, y, connector, connector_style, rect.width);
        }
        *row_index += 1;
    }
    if separator > 0 {
        // 极窄宽度下树体整段让位，分隔行仍占位但不画孤立竖线。
        if usize::from(rect.width) > super::TREE_NESTED_COLS
            && let Some(y) = visible_row(rect, scroll, *row_index)
        {
            grid.put_clipped(
                rect.x,
                y,
                text::agent::TREE_BAR,
                connector_style,
                rect.width,
            );
        }
        *row_index += 1;
    }
}

/// 综合工具调用状态与后台进程/子 agent 实时状态，得到用于组头聚合的有效状态。
pub(crate) fn effective_status(call: &ToolCallView) -> ToolStatus {
    if matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
        return call.status;
    }
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(view)) if view.state.is_running() => ToolStatus::Running,
        Some(ToolDetailView::Interaction(view)) if view.state.is_running() => ToolStatus::Running,
        Some(ToolDetailView::Subagent(view)) if call.name == "spawn_agent" => {
            match view.agent_status.as_deref() {
                Some("failed") => ToolStatus::Failed,
                Some("blocked" | "interrupted") => ToolStatus::Interrupted,
                Some("cancelled") => ToolStatus::Cancelled,
                _ => call.status,
            }
        }
        _ => call.status,
    }
}

/// 组内最差状态：失败 > 中断 > 运行中 > 取消 > 完成。
fn worst_status(calls: &[&ToolCallView]) -> ToolStatus {
    calls
        .iter()
        .map(|call| effective_status(call))
        .max_by_key(|status| status_rank(*status))
        .unwrap_or(ToolStatus::Completed)
}

fn status_rank(status: ToolStatus) -> u8 {
    match status {
        ToolStatus::Failed => 5,
        ToolStatus::Interrupted => 4,
        ToolStatus::Running | ToolStatus::InputStreaming | ToolStatus::Pending => 3,
        ToolStatus::Cancelled => 2,
        ToolStatus::Completed => 1,
    }
}

fn aggregate_status(calls: &[&ToolCallView]) -> Option<String> {
    let mut failed = 0usize;
    let mut interrupted = 0usize;
    let mut running = 0usize;
    for call in calls {
        match effective_status(call) {
            ToolStatus::Failed => failed += 1,
            ToolStatus::Interrupted => interrupted += 1,
            ToolStatus::Running | ToolStatus::InputStreaming | ToolStatus::Pending => running += 1,
            _ => {}
        }
    }
    let mut parts = Vec::new();
    if failed > 0 {
        parts.push(text::agent::group_failed_label(failed));
    }
    if interrupted > 0 {
        parts.push(text::agent::group_interrupted_label(interrupted));
    }
    if running > 0 {
        parts.push(text::agent::group_running_label(running));
    }
    (!parts.is_empty()).then(|| parts.join(text::LX_CHAT_TOOL_STATUS_SEP))
}

fn aggregate_metric(calls: &[&ToolCallView]) -> Option<String> {
    if calls
        .iter()
        .all(|call| matches!(call.name.as_str(), "edit" | "write"))
    {
        return edit_write_metric(calls);
    }
    if calls.iter().all(|call| call.name == "search") {
        return search_metric(calls);
    }
    if calls.iter().all(|call| call.name == "update_plan") {
        return calls
            .iter()
            .rev()
            .find_map(|call| super::plan::status_of(call));
    }
    None
}

fn edit_write_metric(calls: &[&ToolCallView]) -> Option<String> {
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut has_known_counts = false;
    let mut unknown = false;
    for call in calls {
        let Some(summary) = call.summary.as_deref() else {
            unknown = true;
            continue;
        };
        if !summary.counts_known {
            unknown = true;
        }
        if summary.added.is_some() || summary.removed.is_some() {
            has_known_counts = true;
        }
        added = added.saturating_add(summary.added.unwrap_or(0));
        removed = removed.saturating_add(summary.removed.unwrap_or(0));
    }
    if !has_known_counts {
        return unknown.then(|| text::agent::FILE_DIFF_COUNTS_UNKNOWN.to_string());
    }
    let mut metric = format!(
        "{} {}",
        text::agent::added_label(added),
        text::agent::removed_label(removed)
    );
    if unknown {
        metric.push(' ');
        metric.push_str(text::agent::FILE_DIFF_COUNTS_UNKNOWN);
    }
    Some(metric)
}

fn search_metric(calls: &[&ToolCallView]) -> Option<String> {
    let mut total = 0usize;
    for call in calls {
        let summary = call.summary.as_deref();
        let detail_search = match call.detail.as_deref() {
            Some(ToolDetailView::Search(view)) => Some(view),
            _ => None,
        };
        let matches = summary.and_then(|s| s.matches).or_else(|| {
            detail_search.and_then(|v| (!v.matches.is_empty()).then_some(v.matches.len()))
        })?;
        total = total.saturating_add(matches);
    }
    Some(text::agent::matches_label(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_named(name: &str) -> ToolCallView {
        ToolCallView {
            call_id: "c".into(),
            name: name.into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: None,
        }
    }

    #[test]
    fn mcp_and_resource_bridge_tools_share_one_group_key() {
        let mcp = call_named("mcp__github__create_issue");
        let resource = call_named("list_resources");
        assert_eq!(group_key(&mcp), group_key(&resource));
        assert_eq!(group_key(&mcp), MCP_GROUP_KEY);
        assert_eq!(display_verb(&mcp), text::agent::VERB_MCP);
        assert_eq!(
            display_verb(&resource),
            text::agent::VERB_MCP,
            "流式早期没有 detail 时也必须显示 MCP"
        );
    }

    #[test]
    fn wait_agent_is_never_groupable() {
        assert!(!is_groupable(&call_named("wait_agent")));
        assert!(!is_groupable(&call_named("ask")));
        assert!(is_groupable(&call_named("spawn_agent")));
        assert!(is_groupable(&call_named("read")));
    }

    #[test]
    fn worst_status_prefers_failure_then_interruption_then_running() {
        let statuses = [
            ToolStatus::Completed,
            ToolStatus::Running,
            ToolStatus::Interrupted,
            ToolStatus::Failed,
        ];
        let calls: Vec<ToolCallView> = statuses
            .into_iter()
            .map(|status| ToolCallView {
                call_id: "c".into(),
                name: "read".into(),
                position: 0,
                arguments_json: "{}".into(),
                status,
                summary: None,
                error: None,
                result_json: None,
                detail: None,
            })
            .collect();
        let refs: Vec<&ToolCallView> = calls.iter().collect();
        assert_eq!(worst_status(&refs), ToolStatus::Failed);
    }
}
