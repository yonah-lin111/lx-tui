//! 历史几何快照：只依赖「内容宽度 + 列表状态」，产出内容行空间
//! （0 = 最早，不含屏幕坐标与滚动）的行区间。
//!
//! `layout` 把内容行映射成屏幕矩形；`render` 用 `entries` 跳过视口外的条目；
//! 命中入口用 `cards` 换算内容行。三处读同一份快照，行数口径只有一个来源。
//!
//! 快照**不含**高度/`y`/滚动偏移：这些随视口每帧变化，进了键就会每帧重建。

use crate::app::state::{AssistantStepView, CardSection, Lx};

use super::timeline::{TimelineEntry, timeline};
use super::{
    CardKind, MessageRows, body, compact, folds_of, needs_spacer, notice, rewind, shell_folds,
    should_register_card, stats_line, text_rows, tool, tool_rows, user_shell, wrap,
};

/// 时间线条目类别（与 [`TimelineEntry`] 一一对应，但可跨帧独立持有）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapEntryKind {
    Message(usize),
    Notices { last: usize, count: usize },
    Compact(usize),
}

/// 一条时间线条目的内容行区间（`end` 不含）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SnapEntry {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: SnapEntryKind,
}

/// 一个可折叠块的内容行区间（点击面 + 折叠内容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CardRegion {
    pub(crate) kind: CardKind,
    pub(crate) card_start: usize,
    pub(crate) card_rows: usize,
    pub(crate) fold_start: usize,
    pub(crate) fold_rows: usize,
    pub(crate) clickable: bool,
}

/// 历史几何快照（内容行空间）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct HistorySnapshot {
    /// 每条时间线条目的起止行。
    pub(crate) entries: Vec<SnapEntry>,
    /// 每条消息的行区间（置顶行归属判定用；下标即消息下标）。
    pub(crate) messages: Vec<MessageRows>,
    /// （消息索引, 步骤下标, (起始行, 行数)）：thinking 头部。
    pub(crate) thinking: Vec<(usize, usize, (usize, usize))>,
    /// 可折叠块的内容行区间。
    pub(crate) cards: Vec<CardRegion>,
    /// 内容总行数（滚动上限用）。
    pub(crate) total_rows: usize,
}

/// 构建快照。`width` 是历史内容列宽（已扣除滚动条预留列）。
pub(crate) fn build(width: u16, lx: &Lx) -> HistorySnapshot {
    let mut rows = 0usize;
    let mut snapshot = HistorySnapshot::default();
    if width == 0 {
        return snapshot;
    }
    let timeline = timeline(lx);
    let mut entry_has_previous = false;
    for entry in &timeline {
        match entry {
            TimelineEntry::Notices { last, count } => {
                if entry_has_previous {
                    rows += 1;
                }
                entry_has_previous = true;
                let start = rows;
                rows += rewind::rows(&lx.chat.notices[*last], *count, width);
                snapshot.entries.push(SnapEntry {
                    start,
                    end: rows,
                    kind: SnapEntryKind::Notices {
                        last: *last,
                        count: *count,
                    },
                });
            }
            TimelineEntry::Compact(index) => {
                if entry_has_previous {
                    rows += 1;
                }
                entry_has_previous = true;
                let start = rows;
                let notice = &lx.compact.checkpoints[*index];
                let expanded = compact::is_expanded(lx, &notice.id);
                let notice_rows = compact::rows(notice, 1, width, expanded);
                rows += notice_rows;
                // 整块都是点击面：折叠态只有头部两行，点哪里都应能展开。
                snapshot.cards.push(CardRegion {
                    kind: CardKind::Compact {
                        checkpoint_id: notice.id.clone(),
                        expanded,
                    },
                    card_start: start,
                    card_rows: notice_rows,
                    fold_start: start,
                    fold_rows: notice_rows,
                    clickable: true,
                });
                snapshot.entries.push(SnapEntry {
                    start,
                    end: rows,
                    kind: SnapEntryKind::Compact(*index),
                });
            }
            TimelineEntry::Message(message_index) => {
                if entry_has_previous {
                    rows += 1;
                }
                entry_has_previous = true;
                let message = &lx.chat.messages[*message_index];
                let start = rows;
                if let Some(shell) = &message.shell {
                    build_shell(&mut snapshot, lx, message, shell, width, start, &mut rows);
                    snapshot.messages.push(MessageRows {
                        start,
                        end: rows,
                        is_user: false,
                    });
                    snapshot.entries.push(SnapEntry {
                        start,
                        end: rows,
                        kind: SnapEntryKind::Message(*message_index),
                    });
                    continue;
                }
                let mut has_previous =
                    build_steps(&mut snapshot, lx, message, *message_index, width, &mut rows);
                if let Some(notice) = notice(message) {
                    if has_previous && needs_spacer() {
                        rows += 1;
                    }
                    has_previous = true;
                    rows += notice::rows(&notice, width);
                }
                if let Some(line) = stats_line(message) {
                    if has_previous && needs_spacer() {
                        rows += 1;
                    }
                    rows += wrap(&line, width).len();
                }
                snapshot.messages.push(MessageRows {
                    start,
                    end: rows,
                    is_user: message.role == "user" && message.shell.is_none(),
                });
                snapshot.entries.push(SnapEntry {
                    start,
                    end: rows,
                    kind: SnapEntryKind::Message(*message_index),
                });
            }
        }
    }
    // 末尾留一行呼吸位（滚动到底部时不贴死）；归最后一条消息。
    if !timeline.is_empty() {
        rows += 1;
        if matches!(timeline.last(), Some(TimelineEntry::Message(_)))
            && let Some(last) = snapshot.messages.last_mut()
        {
            last.end = rows;
        }
    }
    snapshot.total_rows = rows;
    snapshot
}

/// 本地命令卡片（独立条目，不进入步骤时间线）。
fn build_shell(
    snapshot: &mut HistorySnapshot,
    lx: &Lx,
    message: &crate::app::state::ChatMessageView,
    shell: &crate::app::state::UserShellView,
    width: u16,
    start: usize,
    rows: &mut usize,
) {
    let folds = shell_folds(lx, &message.id);
    *rows += user_shell::rows(
        shell,
        width,
        lx.chat.loading_frame,
        lx.chat.status.streaming || shell.is_running(),
        folds,
    );
    for region in user_shell::regions(shell, width, folds) {
        let expanded = folds.of(region.section);
        snapshot.cards.push(CardRegion {
            kind: CardKind::Shell {
                message_id: message.id.clone(),
                section: region.section,
                expanded,
            },
            card_start: start + region.offset,
            card_rows: region.rows,
            fold_start: start + region.offset,
            fold_rows: region.rows,
            clickable: region.clickable,
        });
    }
}

/// assistant/用户消息的步骤时间线（含合并组、工具卡片与 thinking 头部）。
///
/// 返回消息内是否有过块（决定其后的通知/统计行是否留空行）。
fn build_steps(
    snapshot: &mut HistorySnapshot,
    lx: &Lx,
    message: &crate::app::state::ChatMessageView,
    message_index: usize,
    width: u16,
    rows: &mut usize,
) -> bool {
    let mut has_previous = false;
    let mut step_index = 0usize;
    while let Some(step) = message.steps.get(step_index) {
        // 同种文件工具的连续调用共用一行合并标题，但每个调用各自独立成块。
        if let AssistantStepView::Tool { .. } = step {
            let group = tool::merge_group(&message.steps, step_index);
            if group.len() > 1 {
                let expanded: Vec<bool> = group
                    .iter()
                    .map(|call| lx.chat.detail_is_expanded(&message.id, &call.call_id))
                    .collect();
                let group_rows = tool::group_rows(&group, width, &expanded);
                if group_rows > 0 {
                    if has_previous && needs_spacer() {
                        *rows += 1;
                    }
                    has_previous = true;
                    let block_start = *rows;
                    let blocks = tool::group_blocks(&group, width, &expanded);
                    for (index, block) in blocks.iter().enumerate() {
                        let call = group[index];
                        if !should_register_card(group[index], width) {
                            continue;
                        }
                        let card_start = block_start + block.start;
                        let (fold_start, fold_rows) = if expanded[index] {
                            (card_start, 0)
                        } else {
                            let region = tool::fold_region(call, width);
                            (
                                block_start + block.start + region.offset,
                                region.rows.min(block.len.saturating_sub(region.offset)),
                            )
                        };
                        snapshot.cards.push(CardRegion {
                            kind: CardKind::Tool {
                                message_id: message.id.clone(),
                                call_id: call.call_id.clone(),
                                section: CardSection::Body,
                                expanded: expanded[index],
                            },
                            card_start,
                            card_rows: block.len,
                            fold_start,
                            fold_rows,
                            clickable: true,
                        });
                    }
                    *rows += group_rows;
                }
                step_index += group.len();
                continue;
            }
        }
        let (is_reasoning, step_rows) = match step {
            AssistantStepView::Part { part, .. } => match part.kind {
                crate::chat::events::PartKind::Text => {
                    (false, text_rows(message, &part.content, width))
                }
                crate::chat::events::PartKind::Reasoning => (
                    true,
                    1 + if lx.chat.reasoning_expanded(message_index, step_index) {
                        body::thinking_rows(&part.content, width)
                    } else {
                        0
                    },
                ),
            },
            AssistantStepView::Tool { call, .. } => (
                false,
                tool_rows(call, width, folds_of(lx, &message.id, call)),
            ),
        };
        if step_rows == 0 {
            step_index += 1;
            continue; // 空正文块不占行，也不引入块间空行。
        }
        if has_previous && needs_spacer() {
            *rows += 1;
        }
        has_previous = true;
        let block_start = *rows;
        if let AssistantStepView::Tool { call, .. } = step {
            let folds = folds_of(lx, &message.id, call);
            // 命令卡片登记两条：参数区与输出区各自点击；文件工具是一条整卡点击面。
            for region in tool::card_regions(call, width, folds, step_rows) {
                let card_rows = region
                    .card
                    .rows
                    .min(step_rows.saturating_sub(region.card.offset));
                if card_rows == 0 {
                    continue;
                }
                let fold_rows = region
                    .fold
                    .rows
                    .min(step_rows.saturating_sub(region.fold.offset));
                let expanded = match region.section {
                    CardSection::Body => folds.body,
                    CardSection::Output => folds.output,
                };
                snapshot.cards.push(CardRegion {
                    kind: CardKind::Tool {
                        message_id: message.id.clone(),
                        call_id: call.call_id.clone(),
                        section: region.section,
                        expanded,
                    },
                    card_start: block_start + region.card.offset,
                    card_rows,
                    fold_start: block_start + region.fold.offset,
                    fold_rows,
                    clickable: region.clickable,
                });
            }
        }
        if is_reasoning {
            let expanded = lx.chat.reasoning_expanded(message_index, step_index);
            snapshot
                .thinking
                .push((message_index, step_index, (block_start, 1)));
            snapshot.cards.push(CardRegion {
                kind: CardKind::Thinking {
                    message_index,
                    step_index,
                    expanded,
                },
                card_start: block_start,
                card_rows: step_rows,
                fold_start: block_start,
                fold_rows: step_rows,
                clickable: true,
            });
        }
        *rows += step_rows;
        step_index += 1;
    }
    has_previous
}
