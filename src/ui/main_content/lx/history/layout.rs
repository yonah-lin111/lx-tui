//! 历史布局：按当前展开状态计算消息/时间线条目的屏幕几何。
//!
//! 从 `history.rs` 拆出（单文件行数上限）。`layout` 与 `snapshot` 的行数口径必须
//! 完全一致；这里只做「内容行 → 屏幕矩形」的映射，不重新排版正文。

use ratatui::layout::Rect;

use crate::app::state::{AssistantStepView, CardSection, Lx};

use super::timeline::{TimelineEntry, timeline};
use super::{
    CardHit, CardKind, HistoryLayout, MessageRows, body, compact, content_rect, folds_of,
    needs_spacer, notice, rewind, rows_to_u16, shell_folds, should_register_card, stats_line,
    text_rows, tool, tool_rows, user_shell, wrap,
};

/// 计算历史布局（按当前展开状态）。`area` 为整个历史区（含滚动条列）。
pub(crate) fn layout(area: Rect, lx: &Lx) -> HistoryLayout {
    let content = content_rect(area);
    let width = content.width;
    let mut rows = 0usize;
    let mut thinking = Vec::new();
    let mut cards: Vec<CardHit> = Vec::new();
    if width == 0 {
        return HistoryLayout {
            area,
            thinking,
            cards: Vec::new(),
            messages: Vec::new(),
            total_rows: 0,
        };
    }
    let mut messages_rows: Vec<MessageRows> = Vec::new();
    let timeline = timeline(lx);
    let mut entry_has_previous = false;
    for entry in &timeline {
        let message_index = match entry {
            TimelineEntry::Message(index) => *index,
            TimelineEntry::Notices { last, count } => {
                // 撤销提示是独立历史条目：与消息之间同样留一空行。
                if entry_has_previous {
                    rows += 1;
                }
                entry_has_previous = true;
                rows += rewind::rows(&lx.chat.notices[*last], *count, width);
                continue;
            }
            TimelineEntry::Compact(index) => {
                // compact 条目同样独立成条：与消息之间留一空行，行数与渲染同口径。
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
                cards.push(CardHit {
                    kind: CardKind::Compact {
                        checkpoint_id: notice.id.clone(),
                        expanded,
                    },
                    card: Rect::new(
                        area.x,
                        area.y.saturating_add(rows_to_u16(start)),
                        width,
                        rows_to_u16(notice_rows),
                    ),
                    fold: Rect::new(
                        area.x,
                        area.y.saturating_add(rows_to_u16(start)),
                        width,
                        rows_to_u16(notice_rows),
                    ),
                    clickable: true,
                });
                continue;
            }
        };
        let message = &lx.chat.messages[message_index];
        // 条目间空行归**后**一条消息（codex 的 leading spacer 口径）：顶行落在轮边界
        // 空行上时算用户消息自己的行 → 用户消息自带上下文，不置顶。
        if entry_has_previous {
            rows += 1;
        }
        entry_has_previous = true;
        let start = rows;
        // 本地命令是独立卡片：不进入 assistant 步骤时间线，也不参与置顶用户行。
        if let Some(shell) = &message.shell {
            let folds = shell_folds(lx, &message.id);
            let shell_rows = user_shell::rows(
                shell,
                width,
                lx.chat.loading_frame,
                lx.chat.status.streaming || shell.is_running(),
                folds,
            );
            rows += shell_rows;
            let block_y =
                |offset: usize| -> u16 { area.y.saturating_add(rows_to_u16(start + offset)) };
            for region in user_shell::regions(shell, width, folds) {
                let expanded = folds.of(region.section);
                cards.push(CardHit {
                    kind: CardKind::Shell {
                        message_id: message.id.clone(),
                        section: region.section,
                        expanded,
                    },
                    card: Rect::new(
                        area.x,
                        block_y(region.offset),
                        width,
                        rows_to_u16(region.rows),
                    ),
                    // 折叠内容几何（不再驱动悬停提亮）。
                    fold: Rect::new(
                        area.x,
                        block_y(region.offset),
                        width,
                        rows_to_u16(region.rows),
                    ),
                    clickable: region.clickable,
                });
            }
            messages_rows.push(MessageRows {
                start,
                end: rows,
                is_user: false,
            });
            continue;
        }
        // 消息内块之间一律空一行（见 [`needs_spacer`]）。
        let mut has_previous = false;
        let mut step_index = 0usize;
        while let Some(step) = message.steps.get(step_index) {
            // 同种文件工具的连续调用共用一行合并标题，但**每个调用各自独立**：
            // 自己的文件块、自己的展开状态与自己的展开入口（块间空一行分隔）。
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
                            rows += 1;
                        }
                        has_previous = true;
                        let block_y = |offset: usize| -> u16 {
                            area.y
                                .saturating_add(u16::try_from(rows + offset).unwrap_or(u16::MAX))
                        };
                        let blocks = tool::group_blocks(&group, width, &expanded);
                        for (index, block) in blocks.iter().enumerate() {
                            let call = group[index];
                            // 每块各自一整块点击面：点它只切换这一块（展开互不影响）。
                            if should_register_card(group[index], width) {
                                cards.push(CardHit {
                                    kind: CardKind::Tool {
                                        message_id: message.id.clone(),
                                        call_id: call.call_id.clone(),
                                        section: CardSection::Body,
                                        expanded: expanded[index],
                                    },
                                    card: Rect::new(
                                        area.x,
                                        block_y(block.start),
                                        width,
                                        rows_to_u16(block.len),
                                    ),
                                    clickable: true,
                                    // 逐块折叠内容几何（不再驱动悬停提亮）；
                                    // 展开后没有折叠内容。
                                    fold: if expanded[index] {
                                        Rect::new(area.x, block_y(block.start), width, 0)
                                    } else {
                                        let region = tool::fold_region(call, width);
                                        let start = block.start + region.offset;
                                        let rows = region
                                            .rows
                                            .min(block.len.saturating_sub(region.offset));
                                        Rect::new(area.x, block_y(start), width, rows_to_u16(rows))
                                    },
                                });
                            }
                        }
                        rows += group_rows;
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
                rows += 1;
            }
            has_previous = true;
            let block_y = |offset: usize| -> u16 {
                area.y
                    .saturating_add(u16::try_from(rows + offset).unwrap_or(u16::MAX))
            };
            if let AssistantStepView::Tool { call, .. } = step {
                let folds = folds_of(lx, &message.id, call);
                // 命令卡片登记两条：参数区与输出区各自点击，点其中一块只切那一块。
                // 文件工具是一条整卡点击面。
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
                    cards.push(CardHit {
                        kind: CardKind::Tool {
                            message_id: message.id.clone(),
                            call_id: call.call_id.clone(),
                            section: region.section,
                            expanded,
                        },
                        card: Rect::new(
                            area.x,
                            block_y(region.card.offset),
                            width,
                            rows_to_u16(card_rows),
                        ),
                        fold: Rect::new(
                            area.x,
                            block_y(region.fold.offset),
                            width,
                            rows_to_u16(fold_rows),
                        ),
                        clickable: region.clickable,
                    });
                }
            }
            if is_reasoning {
                let expanded = lx.chat.reasoning_expanded(message_index, step_index);
                let header = Rect::new(area.x, block_y(0), width, 1);
                thinking.push((message_index, step_index, header));
                // 思考块整块可点（折叠态只有头部一行，也必须点得开）。
                cards.push(CardHit {
                    kind: CardKind::Thinking {
                        message_index,
                        step_index,
                        expanded,
                    },
                    card: Rect::new(area.x, block_y(0), width, rows_to_u16(step_rows)),
                    // 思考块恢复原来的 hover：悬停整块（含展开后的正文）都提亮。
                    fold: Rect::new(area.x, block_y(0), width, rows_to_u16(step_rows)),
                    clickable: true,
                });
            }
            rows += step_rows;
            step_index += 1;
        }
        // 消息底部通知（失败/中断）。
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
        messages_rows.push(MessageRows {
            start,
            end: rows,
            is_user: message.role == "user" && message.shell.is_none(),
        });
    }
    // 末尾留一行呼吸位（滚动到底部时不贴死）；归最后一条消息（提示结尾时不并入消息区间）。
    if !timeline.is_empty() {
        rows += 1;
        if matches!(timeline.last(), Some(TimelineEntry::Message(_)))
            && let Some(last) = messages_rows.last_mut()
        {
            last.end = rows;
        }
    }
    HistoryLayout {
        area,
        thinking,
        cards,
        messages: messages_rows,
        total_rows: rows,
    }
}
