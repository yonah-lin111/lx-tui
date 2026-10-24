//! 历史绘制：按内容行偏移把消息/时间线条目写进 lx 页网格。
//!
//! 从 `history.rs` 拆出（单文件行数上限）。行数口径与 [`super::layout`] 完全一致；
//! 两者共用同一份步骤行数函数，测试逐行比对以防几何漂移。

use ratatui::layout::Rect;

use crate::app::state::{AssistantStepView, Lx};
use crate::ui::style;
use crate::ui::text;

use super::timeline::{TimelineEntry, timeline};
use super::{
    Grid, STEP_GUTTER, StepKind, StepMark, THINKING_DURATION_SEP, USER_INDENT, USER_PREFIX, body,
    compact, content_rect, folds_of, format_duration, needs_spacer, notice, rewind, shell_folds,
    stats_line, text_rows, tool, tool_rows, user_shell, visible_row, wrap,
};

/// 渲染历史内容；`scroll` 为顶部行偏移（0 表示从最早开始）。`rect` 为内容区
/// （body，不含置顶行；含右侧滚动条列）。
///
/// 返回本次推进的内容行数：与 [`super::layout`] 的 `total_rows` **必须相等**
/// （两者共用同一份步骤行数口径；测试逐一比对，防几何漂移）。
pub(crate) fn render(grid: &mut Grid, rect: Rect, lx: &Lx, scroll: usize) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    let content = content_rect(rect);
    let mut row_index = 0usize;
    let selected_user = lx
        .rewind
        .selected
        .as_ref()
        .map(|target| target.user_message_id.as_str());
    for entry in &timeline(lx) {
        let message_index = match entry {
            TimelineEntry::Message(index) => *index,
            TimelineEntry::Notices { last, count } => {
                rewind::draw(
                    grid,
                    &lx.chat.notices[*last],
                    *count,
                    content,
                    scroll,
                    &mut row_index,
                );
                row_index += 1;
                continue;
            }
            TimelineEntry::Compact(index) => {
                let notice = &lx.compact.checkpoints[*index];
                let expanded = compact::is_expanded(lx, &notice.id);
                compact::draw(grid, notice, 1, expanded, content, scroll, &mut row_index);
                row_index += 1;
                continue;
            }
        };
        let message = &lx.chat.messages[message_index];
        let is_user = message.role == "user";
        // 本地命令卡片：与 layout 共用 `user_shell::rows` 的行数口径。
        if let Some(shell) = &message.shell {
            user_shell::draw(
                grid,
                content,
                scroll,
                &mut row_index,
                shell,
                lx.chat.loading_frame,
                lx.chat.status.streaming || shell.is_running(),
                shell_folds(lx, &message.id),
            );
            row_index += 1; // 消息间空行。
            continue;
        }
        // 消息内块之间一律空一行（与 `layout` 同口径）。
        let mut has_previous = false;
        let mut first_user_line = true;
        let mut step_index = 0usize;
        while let Some(step) = message.steps.get(step_index) {
            // 同种文件工具的连续调用共用一行合并标题（与 `layout` 同一份分组口径）。
            if let AssistantStepView::Tool { .. } = step {
                let group = tool::merge_group(&message.steps, step_index);
                if group.len() > 1 {
                    let expanded: Vec<bool> = group
                        .iter()
                        .map(|call| lx.chat.detail_is_expanded(&message.id, &call.call_id))
                        .collect();
                    let group_rows = tool::group_rows(&group, content.width, &expanded);
                    if group_rows > 0 {
                        if has_previous && needs_spacer() {
                            row_index += 1;
                        }
                        has_previous = true;
                        tool::group_draw_animated(
                            grid,
                            content,
                            scroll,
                            &mut row_index,
                            &group,
                            lx.chat.loading_frame,
                            &expanded,
                            lx.chat.status.streaming,
                        );
                    }
                    step_index += group.len();
                    continue;
                }
            }
            let (block_rows, part_kind) = match step {
                AssistantStepView::Part { part, .. } => {
                    let expanded = lx.chat.reasoning_expanded(message_index, step_index);
                    let rows = match part.kind {
                        crate::chat::events::PartKind::Text => {
                            text_rows(message, &part.content, content.width)
                        }
                        crate::chat::events::PartKind::Reasoning => {
                            1 + if expanded {
                                body::thinking_rows(&part.content, content.width)
                            } else {
                                0
                            }
                        }
                    };
                    (rows, Some((part, expanded)))
                }
                AssistantStepView::Tool { call, .. } => (
                    tool_rows(call, content.width, folds_of(lx, &message.id, call)),
                    None,
                ),
            };
            if block_rows == 0 {
                step_index += 1;
                continue; // 空正文块不占行，也不引入块间空行（与 `layout` 同口径）。
            }
            if has_previous && needs_spacer() {
                row_index += 1;
            }
            has_previous = true;
            let Some((part, expanded)) = part_kind else {
                if let AssistantStepView::Tool { call, .. } = step {
                    tool::draw_animated(
                        grid,
                        content,
                        scroll,
                        &mut row_index,
                        call,
                        lx.chat.loading_frame,
                        folds_of(lx, &message.id, call),
                        lx.chat.status.streaming,
                    );
                }
                step_index += 1;
                continue;
            };
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    if is_user {
                        let selected = selected_user == Some(message.id.as_str());
                        for line in wrap(&part.content, content.width.saturating_sub(USER_INDENT)) {
                            if let Some(y) = visible_row(content, scroll, row_index) {
                                let bg = if selected {
                                    style::chat_user_target_bg()
                                } else {
                                    style::chat_user_bg()
                                };
                                // 整块底色覆盖整个内容列（含 `> ` 前缀列）。
                                grid.background_row(content.x, y, content.width, bg);
                                if first_user_line {
                                    // 图标用输入框同款强调色，并叠加同底色。
                                    grid.put(content.x, y, USER_PREFIX, style::accent().patch(bg));
                                    first_user_line = false;
                                }
                                grid.put_clipped(
                                    content.x.saturating_add(USER_INDENT),
                                    y,
                                    &line,
                                    bg,
                                    content.width.saturating_sub(USER_INDENT),
                                );
                            }
                            row_index += 1;
                        }
                    } else {
                        // 正文块也是步骤：消息仍在流式且它是最后一步 → 方块呼吸，
                        // 否则静态（已完成）。
                        let mark = if message.status == "streaming"
                            && step_index + 1 == message.steps.len()
                        {
                            StepMark::Running
                        } else {
                            StepMark::Done
                        };
                        body::draw_body(
                            grid,
                            content,
                            scroll,
                            &mut row_index,
                            &part.content,
                            body::BodyStyle {
                                mark,
                                kind: StepKind::Body,
                                loading_frame: lx.chat.loading_frame,
                                animating: lx.chat.status.streaming,
                            },
                        );
                    }
                }
                crate::chat::events::PartKind::Reasoning => {
                    let streaming = lx.chat.reasoning_streaming(message_index, step_index);
                    let hovered = lx.chat.thinking_is_hovered(message_index, step_index);
                    let mark = if streaming {
                        StepMark::Running
                    } else {
                        StepMark::Done
                    };
                    if let Some(y) = visible_row(content, scroll, row_index) {
                        // 生成中：`Thinking`；完成后：`Thought · 6.3s`。
                        // 折叠态浅粉、展开/悬停亮粉（见 `style::chat_step_*_thinking`）。
                        let header_style = StepKind::Thinking.label(streaming, expanded, hovered);
                        grid.put_char(
                            content.x,
                            y,
                            mark.glyph(lx.chat.loading_frame, lx.chat.status.streaming),
                            mark.style(StepKind::Thinking, expanded, hovered),
                        );
                        let x = content.x.saturating_add(STEP_GUTTER);
                        let limit = content.right();
                        if streaming {
                            grid.put_clipped(
                                x,
                                y,
                                text::LX_CHAT_THINKING_LABEL,
                                header_style,
                                limit.saturating_sub(x),
                            );
                        } else {
                            let mut label = String::from(text::LX_CHAT_THOUGHT_LABEL);
                            if let Some(duration) = body::thinking_duration(part) {
                                label.push_str(THINKING_DURATION_SEP);
                                label.push_str(&format_duration(duration));
                            }
                            grid.put_clipped(x, y, &label, header_style, limit.saturating_sub(x));
                        }
                    }
                    row_index += 1;
                    if expanded {
                        body::draw_thinking(grid, content, scroll, &mut row_index, &part.content);
                    }
                }
            }
            step_index += 1;
        }
        // 消息底部通知（失败/中断）。
        if let Some(notice) = notice(message) {
            if has_previous && needs_spacer() {
                row_index += 1;
            }
            has_previous = true;
            notice::draw(grid, &notice, content, scroll, &mut row_index);
        }
        if let Some(line) = stats_line(message) {
            if has_previous && needs_spacer() {
                row_index += 1;
            }
            for row in wrap(&line, content.width) {
                if let Some(y) = visible_row(content, scroll, row_index) {
                    grid.put_clipped(content.x, y, &row, style::muted(), content.width);
                }
                row_index += 1;
            }
        }
        row_index += 1; // 消息间空行。
    }
    row_index
}
