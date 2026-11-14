//! 消息步骤时间线：合并组扫描、组头/调用体块拆分与 thinking 命中登记。

use crate::app::state::AssistantStepView;

use super::super::cache::{
    BlockKey, GROUP_HEADER_SLOT_BASE, tool_body_block_signature, tool_group_header_signature,
};
use super::super::paint::{BlockPaint, BlockSource, GroupSlot};
use super::super::source::HistorySource;
use super::super::{CardKind, folds_of, tool};
use super::{CardRegion, SnapBlock, legacy};

impl super::Builder<'_> {
    /// assistant/用户消息的步骤时间线（含合并组、工具卡片与 thinking 头部）。
    pub(super) fn steps(
        &mut self,
        source: HistorySource<'_>,
        message_index: usize,
        message: &crate::app::state::ChatMessageView,
        blocks: &mut Vec<SnapBlock>,
    ) {
        let mut has_previous = false;
        let mut step_index = 0usize;
        while let Some(step) = message.steps.get(step_index) {
            // 连续同名工具：组头一行 + 每个调用一个仅树体的块。
            if let AssistantStepView::Tool { call, .. } = step
                && tool::is_groupable(call)
            {
                let (end, mode) = tool_group_range(message, step_index);
                if end.saturating_sub(step_index) >= 2 {
                    if has_previous {
                        self.spacer();
                    }
                    has_previous = true;
                    let first_step = step_index;
                    let count = end - first_step;
                    let header_start = self.rows;
                    let header_key = BlockKey {
                        message_index,
                        step_index: GROUP_HEADER_SLOT_BASE.saturating_sub(first_step),
                        width: self.width,
                        sig: tool_group_header_signature(source, message, first_step, count),
                    };
                    let header = self.block(
                        header_key,
                        BlockSource::ToolGroupHeader {
                            message_index,
                            first_step,
                            count,
                            snapshot: mode == tool::GroupMode::Snapshot,
                        },
                        header_start,
                        || BlockPaint::Legacy {
                            rows: 1,
                            spans: Vec::new(),
                        },
                    );
                    self.rows += header.rows;
                    blocks.push(header);

                    let display: Vec<usize> = match mode {
                        tool::GroupMode::List => (first_step..end).collect(),
                        tool::GroupMode::Snapshot => {
                            vec![latest_snapshot_step(message, first_step, end)]
                        }
                    };
                    let display_count = display.len();
                    for (group_index, call_index) in display.into_iter().enumerate() {
                        let AssistantStepView::Tool { call, .. } = &message.steps[call_index]
                        else {
                            continue;
                        };
                        let folds = folds_of(source, &message.id, call);
                        let group = GroupSlot {
                            index: group_index,
                            count: display_count,
                        };
                        let key = BlockKey {
                            message_index,
                            step_index: call_index,
                            width: self.width,
                            sig: tool_body_block_signature(
                                source,
                                message,
                                message_index,
                                call_index,
                                &message.steps[call_index],
                                group,
                            ),
                        };
                        let block_start = self.rows;
                        let width = self.width;
                        let block = self.block(
                            key,
                            BlockSource::ToolBody {
                                message_index,
                                step_index: call_index,
                                group,
                            },
                            block_start,
                            || {
                                let rows = tool::body_rows(call, width, folds, group);
                                // 分隔行是排版留白：点击面不能覆盖它，否则点空行会展开上一个调用。
                                let click_rows = rows.saturating_sub(usize::from(!group.is_last()));
                                let total_rows = click_rows.saturating_add(1);
                                let regions =
                                    tool::body_card_regions(call, width, folds, total_rows);
                                legacy(rows, regions, folds)
                            },
                        );
                        for (offset, agent_id) in tool::body_agent_links(call, self.width) {
                            self.snapshot.cards.push(CardRegion {
                                kind: CardKind::AgentLink { agent_id },
                                card_start: block_start.saturating_add(offset.saturating_sub(1)),
                                card_rows: 1,
                                card_cols: None,
                                clickable: true,
                            });
                        }
                        self.tool_cards(
                            message,
                            call,
                            block.paint.card_spans(),
                            block.rows,
                            block_start,
                        );
                        self.rows += block.rows;
                        blocks.push(block);
                    }
                    step_index = end;
                    continue;
                }
            }

            // 先取块（命中二级缓存时不再重排正文），行数**只**从 paint 取。
            let mut block = self.build_step(source, message_index, step_index, message, step);
            if block.rows == 0 {
                step_index += 1;
                continue; // 空正文块不占行，也不引入块间空行。
            }
            if has_previous {
                self.spacer();
            }
            has_previous = true;
            block.start = self.rows;
            let block_start = self.rows;
            if let AssistantStepView::Tool { call, .. } = step {
                // 链接行必须**先于**整卡折叠面登记：命中取第一条重叠区域，整卡面
                // 覆盖链接行会把打开 Agents 的点击永久吞掉。
                for (offset, agent_id) in tool::agent_links(call, self.width) {
                    self.snapshot.cards.push(CardRegion {
                        kind: CardKind::AgentLink { agent_id },
                        card_start: block_start.saturating_add(offset),
                        card_rows: 1,
                        card_cols: None,
                        clickable: true,
                    });
                }
                self.tool_cards(
                    message,
                    call,
                    block.paint.card_spans(),
                    block.rows,
                    block_start,
                );
            }
            if matches!(
                step,
                AssistantStepView::Part { part, .. }
                    if part.kind == crate::chat::events::PartKind::Reasoning
            ) {
                let expanded = source.chat.reasoning_expanded(message_index, step_index);
                self.snapshot
                    .thinking
                    .push((message_index, step_index, (block_start, 1)));
                self.snapshot.cards.push(CardRegion {
                    kind: CardKind::Thinking {
                        message_index,
                        step_index,
                        expanded,
                    },
                    card_start: block_start,
                    card_rows: block.rows,
                    card_cols: None,
                    clickable: true,
                });
            }
            self.rows = block_start.saturating_add(block.rows);
            blocks.push(block);
            step_index += 1;
        }
    }
}
/// 从 `first` 开始扫描连续同名工具；返回组尾（不含）与合并形态。
fn tool_group_range(
    message: &crate::app::state::ChatMessageView,
    first: usize,
) -> (usize, tool::GroupMode) {
    let Some(AssistantStepView::Tool { call, .. }) = message.steps.get(first) else {
        return (first.saturating_add(1), tool::GroupMode::List);
    };
    if !tool::is_groupable(call) {
        return (first.saturating_add(1), tool::GroupMode::List);
    }
    let key = tool::group_key(call);
    let mode = tool::group_mode(call);
    let mut end = first.saturating_add(1);
    while let Some(AssistantStepView::Tool { call, .. }) = message.steps.get(end) {
        if tool::group_key(call) != key || tool::group_mode(call) != mode {
            break;
        }
        end += 1;
    }
    (end, mode)
}

/// 快照合并组（`update_plan` / `list_agents`）优先选取最后一个已具备有效快照详情的步骤；
/// 若组尾调用尚在流式/待执行且无详情，则保持展示前一次已生效快照。
fn latest_snapshot_step(
    message: &crate::app::state::ChatMessageView,
    first: usize,
    end: usize,
) -> usize {
    for index in (first..end).rev() {
        let Some(AssistantStepView::Tool { call, .. }) = message.steps.get(index) else {
            continue;
        };
        let has_snapshot = match call.detail.as_deref() {
            Some(crate::app::state::ToolDetailView::Plan(_)) => true,
            Some(crate::app::state::ToolDetailView::Subagent(view)) => {
                call.status == crate::app::state::ToolStatus::Completed || !view.agents.is_empty()
            }
            _ => false,
        };
        if has_snapshot {
            return index;
        }
    }
    end.saturating_sub(1)
}
