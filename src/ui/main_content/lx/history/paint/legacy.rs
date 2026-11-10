//! 尚未接入内容缓存的块：保留原绘制路径（行数由快照给出）。

use ratatui::layout::Rect;

use crate::app::state::{AssistantStepView, ToolCallView};

use super::super::source::HistorySource;
use super::super::{Grid, tool, user_shell};
use super::{BlockSource, calls_of, folds_at};

pub(super) fn draw_legacy(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
) {
    let mut row = start;
    match source {
        BlockSource::ToolGroupHeader {
            message_index,
            first_step,
            count,
            snapshot,
        } => {
            let Some(message) = history.messages.get(message_index) else {
                return;
            };
            let calls: Vec<&ToolCallView> = message.steps
                [first_step..first_step.saturating_add(count).min(message.steps.len())]
                .iter()
                .filter_map(|step| match step {
                    AssistantStepView::Tool { call, .. } => Some(call),
                    _ => None,
                })
                .collect();
            let mode = if snapshot {
                super::tool::GroupMode::Snapshot
            } else {
                super::tool::GroupMode::List
            };
            tool::draw_group_header(
                grid,
                rect,
                scroll,
                &mut row,
                &calls,
                count,
                mode,
                history.chat.loading_frame,
                history.animating,
            );
        }
        BlockSource::ToolBody {
            message_index,
            step_index,
            group,
        } => {
            let Some(message) = history.messages.get(message_index) else {
                return;
            };
            let Some(AssistantStepView::Tool { call, .. }) = message.steps.get(step_index) else {
                return;
            };
            tool::draw_body_animated(
                grid,
                rect,
                scroll,
                &mut row,
                call,
                group,
                history.chat.loading_frame,
                folds_at(history, message_index, call),
                history.animating,
            );
        }
        BlockSource::Step { message_index, .. } => {
            let calls = calls_of(history, source);
            let Some(call) = calls.first() else {
                return;
            };
            tool::draw_animated(
                grid,
                rect,
                scroll,
                &mut row,
                call,
                history.chat.loading_frame,
                folds_at(history, message_index, call),
                history.animating,
            );
        }
        BlockSource::Shell { message_index } => {
            let Some(message) = history.messages.get(message_index) else {
                return;
            };
            let Some(shell) = message.shell.as_ref() else {
                return;
            };
            user_shell::draw(
                grid,
                rect,
                scroll,
                &mut row,
                shell,
                history.chat.loading_frame,
                history.animating || shell.is_running(),
                super::super::shell_folds(history, &message.id),
            );
        }
        BlockSource::Notice { message_index } => {
            let Some(message) = history.messages.get(message_index) else {
                return;
            };
            let Some(notice) = super::super::notice(message) else {
                return;
            };
            super::super::notice::draw(grid, &notice, rect, scroll, &mut row);
        }
        // compact / 模型切换 / rewind 已由 `BlockPaint::SystemEvent` 承载：
        // Legacy 路径不再构造这三个来源，保留空分支只为穷尽匹配。
        BlockSource::Rewind { .. }
        | BlockSource::Compact { .. }
        | BlockSource::ModelSwitch { .. }
        | BlockSource::Recap { .. } => {}
        BlockSource::RunningCompact => {
            super::super::compact::draw_running(
                grid,
                history.compact,
                rect,
                scroll,
                &mut row,
                history.chat.loading_frame,
                history.animating,
            );
        }
        BlockSource::Report {
            message_index,
            index,
        } => {
            let Some(message) = history.messages.get(message_index) else {
                return;
            };
            let Some(report) = message.reports.get(index) else {
                return;
            };
            let slot = format!("report:{}", report.agent_id);
            let expanded = history.chat.detail_is_expanded(&message.id, &slot);
            let more_steps = history.chat.report_expand_steps(&message.id, &slot);
            super::super::report_block::draw(
                grid,
                rect,
                report,
                scroll,
                &mut row,
                history.chat.loading_frame,
                history.animating,
                expanded,
                more_steps,
            );
        }
        // 静态块的绘制数据自带样式，没有需要回查的动态状态。
        BlockSource::Plain => {}
    }
}
