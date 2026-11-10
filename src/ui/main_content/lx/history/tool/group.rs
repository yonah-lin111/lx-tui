//! 连续同名工具组：分组键、组头、调用体行数与 headerless 绘制分派。
//!
//! 组头只占一行、不登记点击面；调用体仍复用各工具模块的树体布局，只跳过标题行。

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

/// 分组键：普通工具按精确工具名；`write_stdin` 再按显示动词区分。
pub(crate) fn group_key(call: &ToolCallView) -> String {
    if call.name == "write_stdin" {
        format!("write_stdin:{}", display_verb(call))
    } else {
        call.name.clone()
    }
}

/// 分组形态。`ask` 由 `is_groupable` 排除，不会走到这里。
pub(crate) fn group_mode(call: &ToolCallView) -> GroupMode {
    match call.name.as_str() {
        "update_plan" | "list_agents" => GroupMode::Snapshot,
        _ => GroupMode::List,
    }
}

/// `ask` 是阻塞交互，每次问答独立成块。
pub(crate) fn is_groupable(call: &ToolCallView) -> bool {
    call.name != "ask"
}

/// 工具标题动词：各模块的标题动词集中在这里，组头与单卡共用同一来源。
pub(crate) fn display_verb(call: &ToolCallView) -> String {
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(_)) => text::agent::VERB_EXEC_COMMAND.to_string(),
        Some(ToolDetailView::Interaction(view)) => {
            text::agent::interaction_verb(view.action).to_string()
        }
        Some(ToolDetailView::Plan(_)) => text::agent::VERB_UPDATE_PLAN.to_string(),
        Some(ToolDetailView::Ask(_)) => text::agent::VERB_TOOL_ASK.to_string(),
        Some(ToolDetailView::Search(_)) => text::agent::VERB_TOOL_SEARCH.to_string(),
        Some(ToolDetailView::Subagent(_)) => super::subagent::verb(&call.name).to_string(),
        Some(ToolDetailView::Mcp(view)) => format!("{} · {}", view.server, view.tool),
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
pub(crate) fn body_rows(call: &ToolCallView, width: u16, folds: Folds) -> usize {
    let total = super::rows(call, width, folds);
    if total == 0 {
        return 0;
    }
    total.saturating_sub(1).max(1)
}

/// 调用体的点击面：把整卡区域整体上移一行，并去掉标题行覆盖。
pub(crate) fn body_card_regions(
    call: &ToolCallView,
    width: u16,
    folds: Folds,
    total_rows: usize,
) -> Vec<super::CardRegion> {
    super::card_regions(call, width, folds, total_rows)
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
    let expected = body_rows(call, rect.width, folds);
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
    for _ in drawn..expected {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(
                rect.x,
                y,
                connector,
                style::chat_notice_connector(),
                rect.width,
            );
        }
        *row_index += 1;
    }
}

/// 组内最差状态：失败 > 中断 > 运行中 > 取消 > 完成。
fn worst_status(calls: &[&ToolCallView]) -> ToolStatus {
    calls
        .iter()
        .map(|call| call.status)
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
    let failed = calls
        .iter()
        .filter(|call| call.status == ToolStatus::Failed)
        .count();
    let interrupted = calls
        .iter()
        .filter(|call| call.status == ToolStatus::Interrupted)
        .count();
    let running = calls
        .iter()
        .filter(|call| {
            matches!(
                call.status,
                ToolStatus::Running | ToolStatus::InputStreaming | ToolStatus::Pending
            )
        })
        .count();
    let mut parts = Vec::new();
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if interrupted > 0 {
        parts.push(format!("{interrupted} interrupted"));
    }
    if running > 0 {
        parts.push(format!("{running} running"));
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
    None
}

fn edit_write_metric(calls: &[&ToolCallView]) -> Option<String> {
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut unknown = false;
    for call in calls {
        let Some(summary) = call.summary.as_deref() else {
            unknown = true;
            continue;
        };
        if !summary.counts_known {
            unknown = true;
        }
        added = added.saturating_add(summary.added.unwrap_or(0));
        removed = removed.saturating_add(summary.removed.unwrap_or(0));
    }
    let mut metric = format!(
        "{} {}",
        text::agent::added_label(added),
        text::agent::removed_label(removed)
    );
    if unknown {
        metric.push_str(" (?)");
    }
    Some(metric)
}

fn search_metric(calls: &[&ToolCallView]) -> Option<String> {
    let mut total = 0usize;
    for call in calls {
        let matches = call.summary.as_deref().and_then(|summary| summary.matches);
        total = total.saturating_add(matches?);
    }
    Some(text::agent::matches_label(total))
}

#[cfg(test)]
mod tests {
    use super::*;

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
