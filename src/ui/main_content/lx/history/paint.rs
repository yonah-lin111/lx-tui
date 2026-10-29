//! 块绘制数据（二级缓存）：把每个内容块「排好的行」存下来跨帧复用。
//!
//! 每个变体只存**内容与静态样式**；步骤标记的呼吸字形、thinking 悬停色、用户消息 rewind
//! 选中底色、`loading_frame` 等动态内容一律在 [`BlockPaint::draw`] 时现算。
//!
//! `rows()` 与绘制共用同一份数据：快照行数、滚动条几何、点击面几何都从这里取。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

use crate::app::state::{AssistantStepView, CardSection, ChatMessageView, ToolCallView};

use super::source::HistorySource;
use crate::ui::markdown_render::MdLine;
use crate::ui::style;

use super::super::Grid;
use super::tool::{self, Folds};
use super::user_shell;
use super::{STEP_GUTTER, StepKind, StepMark, USER_INDENT, USER_PREFIX, visible_row};

/// 块在 `Lx` 里的定位（绘制动态样式与回退路径时用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockSource {
    /// 消息里的单个步骤（正文 / thinking / 单个工具）。
    Step {
        message_index: usize,
        step_index: usize,
    },
    /// 合并的文件工具组（连续同种调用）。
    Group {
        message_index: usize,
        first_step: usize,
        count: usize,
    },
    /// 本地命令卡片（整条消息一个块）。
    Shell { message_index: usize },
    /// 消息底部通知。
    Notice { message_index: usize },
    /// 时间线里的撤销提示（合并后的最后一条 + 条数）。
    Rewind { last: usize, count: usize },
    /// 时间线里的 compact 检查点。
    Compact { index: usize, expanded: bool },
    /// 消息内的汇报块（第 `index` 份）。
    Report { message_index: usize, index: usize },
    /// 与 `Lx` 无关的静态块（统计行）：样式已随绘制数据存好，绘制不需要回查状态。
    Plain,
}

/// 块内一个可点击区域（行偏移相对块首行）。
///
/// 它是 `(块内容, 宽度, 折叠态)` 的纯函数结果，因此和绘制数据存在一起、随同一个
/// `BlockKey` 失效：快照重建每帧都要登记点击面，重算一遍 `card_regions` 在 1400 张
/// 工具卡片的会话里要 4.7ms/帧（实测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CardSpan {
    /// 该区域属于块内第几个调用：单工具块恒 0，合并组是组内下标。
    pub call: usize,
    pub section: CardSection,
    /// 该区域当前的展开状态（折叠态也进块指纹，因此能随同样的条件失效）。
    pub expanded: bool,
    pub offset: usize,
    pub rows: usize,
    pub clickable: bool,
}

/// 用户消息下方单条引用树分支。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserRefLine {
    pub is_last: bool,
    pub kind: crate::db::chat::MessageAttachmentKind,
    pub label: String,
}

/// 一个块的绘制数据。
#[derive(Debug)]
pub(crate) enum BlockPaint {
    /// 用户正文：软换行后的纯文本行、折叠提示与引用树。
    UserText {
        text_lines: Vec<String>,
        fold_hint: Option<String>,
        ref_lines: Vec<UserRefLine>,
    },
    /// 助手正文：markdown 渲染后的行。
    Markdown { lines: Vec<MdLine> },
    /// thinking：头部之外只有正文行（折叠时为空；头部文案在绘制时算）。
    Thinking { body: Vec<MdLine> },
    /// 完全静态的单色行块（统计行）。
    Static { lines: Vec<String>, style: Style },
    /// 尚未接入内容缓存的块：缓存行数与点击面几何，绘制仍走原路径。
    Legacy { rows: usize, spans: Vec<CardSpan> },
}

impl BlockPaint {
    /// 该块占用的内容行数（与绘制推进行数严格相等）。
    pub(crate) fn rows(&self) -> usize {
        match self {
            Self::UserText {
                text_lines,
                fold_hint,
                ref_lines,
            } => text_lines.len() + (fold_hint.is_some() as usize) + ref_lines.len(),
            Self::Markdown { lines } => lines.len(),
            Self::Thinking { body } => 1 + body.len(),
            Self::Static { lines, .. } => lines.len(),
            Self::Legacy { rows, .. } => *rows,
        }
    }

    /// 块内的可点击区域（非工具块为空）。几何与行数同源、同一次构建。
    pub(crate) fn card_spans(&self) -> &[CardSpan] {
        match self {
            Self::Legacy { spans, .. } => spans,
            Self::UserText { .. }
            | Self::Markdown { .. }
            | Self::Thinking { .. }
            | Self::Static { .. } => &[],
        }
    }

    /// 近似字节数（只用于 LRU 记账，不追求精确）。
    pub(crate) fn bytes(&self) -> usize {
        match self {
            Self::UserText {
                text_lines,
                fold_hint,
                ref_lines,
            } => {
                text_lines.iter().map(|line| line.len() + 24).sum::<usize>()
                    + fold_hint.as_ref().map_or(0, |h| h.len() + 24)
                    + ref_lines.iter().map(|r| r.label.len() + 24).sum::<usize>()
            }
            Self::Static { lines, .. } => lines.iter().map(|line| line.len() + 24).sum::<usize>(),
            Self::Markdown { lines } => md_bytes(lines),
            Self::Thinking { body } => md_bytes(body),
            // 旧路径没有缓存正文，不占二级缓存预算（行数已在一级快照里）；
            // 点击面几何是小结构，按条数记个近似值。
            Self::Legacy { spans, .. } => spans.len() * std::mem::size_of::<CardSpan>(),
        }
    }

    /// 绘制该块；`start` 是块首行的绝对内容行号。
    ///
    /// 块内按视口裁剪：只遍历视口内的行，绝对行号照常推进。
    pub(crate) fn draw(
        &self,
        grid: &mut Grid,
        rect: Rect,
        scroll: usize,
        start: usize,
        history: HistorySource<'_>,
        source: BlockSource,
    ) {
        let viewport_end = scroll.saturating_add(usize::from(rect.height));
        match self {
            Self::UserText {
                text_lines,
                fold_hint,
                ref_lines,
            } => draw_user_text(
                grid,
                rect,
                history,
                source,
                scroll,
                start,
                viewport_end,
                text_lines,
                fold_hint.as_deref(),
                ref_lines,
            ),
            Self::Markdown { lines } => draw_markdown(
                grid,
                rect,
                history,
                source,
                scroll,
                start,
                viewport_end,
                lines,
            ),
            Self::Thinking { body } => draw_thinking(
                grid,
                rect,
                history,
                source,
                scroll,
                start,
                viewport_end,
                body,
            ),
            Self::Static { lines, style } => {
                for (offset, line) in lines.iter().enumerate().skip(scroll.saturating_sub(start)) {
                    let row = start.saturating_add(offset);
                    if row >= viewport_end {
                        break;
                    }
                    if let Some(y) = visible_row(rect, scroll, row) {
                        grid.put_clipped(rect.x, y, line, *style, rect.width);
                    }
                }
            }
            Self::Legacy { .. } => draw_legacy(grid, rect, history, source, scroll, start),
        }
    }
}

/// markdown 行的近似字节数。
fn md_bytes(lines: &[MdLine]) -> usize {
    lines
        .iter()
        .map(|line| {
            std::mem::size_of::<MdLine>()
                + line
                    .spans
                    .iter()
                    .map(|(text, _)| text.len() + 24)
                    .sum::<usize>()
        })
        .sum()
}

/// thinking 块耗时（毫秒）：首个增量到最后一个增量的时间差；时间戳缺失返回 None。
fn thinking_duration(part: &crate::app::state::ChatPartView) -> Option<i64> {
    let started = part.started_at_ms?;
    let ended = part.ended_at_ms?;
    Some(ended.saturating_sub(started).max(0))
}

/// 消息与步骤 part（正文 / thinking）。
fn step_of(
    history: HistorySource<'_>,
    source: BlockSource,
) -> Option<(&ChatMessageView, &crate::app::state::ChatPartView)> {
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return None;
    };
    let message = history.messages.get(message_index)?;
    match message.steps.get(step_index)? {
        AssistantStepView::Part { part, .. } => Some((message, part)),
        AssistantStepView::Tool { .. } => None,
    }
}

/// 合并组的调用列表（单调用时长度为 1）。
fn calls_of(history: HistorySource<'_>, source: BlockSource) -> Vec<&ToolCallView> {
    let (message_index, first_step, count) = match source {
        BlockSource::Step {
            message_index,
            step_index,
        } => (message_index, step_index, 1),
        BlockSource::Group {
            message_index,
            first_step,
            count,
        } => (message_index, first_step, count),
        _ => return Vec::new(),
    };
    let Some(message) = history.messages.get(message_index) else {
        return Vec::new();
    };
    (0..count)
        .filter_map(|offset| match message.steps.get(first_step + offset)? {
            AssistantStepView::Tool { call, .. } => Some(call),
            AssistantStepView::Part { .. } => None,
        })
        .collect()
}

/// 工具的两块折叠状态（绘制时按当前状态重算，不进缓存键）。
fn folds_at(history: HistorySource<'_>, message_index: usize, call: &ToolCallView) -> Folds {
    let Some(message) = history.messages.get(message_index) else {
        return Folds::default();
    };
    super::folds_of(history, &message.id, call)
}

#[allow(clippy::too_many_arguments)]
fn draw_user_text(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    text_lines: &[String],
    fold_hint: Option<&str>,
    ref_lines: &[UserRefLine],
) {
    let BlockSource::Step { message_index, .. } = source else {
        return;
    };
    let Some(message) = history.messages.get(message_index) else {
        return;
    };
    let selected = history.selected_user_message == Some(message.id.as_str());
    let bg = if selected {
        style::chat_user_target_bg()
    } else {
        style::chat_user_bg()
    };
    let mut current_row = start;

    for (offset, line) in text_lines.iter().enumerate() {
        let row = current_row;
        current_row += 1;
        if row < scroll || row >= viewport_end {
            continue;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        grid.background_row(rect.x, y, rect.width, bg);
        if offset == 0 {
            grid.put(rect.x, y, USER_PREFIX, style::accent().patch(bg));
        }
        grid.put_clipped(
            rect.x.saturating_add(USER_INDENT),
            y,
            line,
            bg,
            rect.width.saturating_sub(USER_INDENT),
        );
    }

    if let Some(hint) = fold_hint {
        let row = current_row;
        current_row += 1;
        if row >= scroll
            && row < viewport_end
            && let Some(y) = visible_row(rect, scroll, row)
        {
            grid.background_row(rect.x, y, rect.width, bg);
            grid.put_clipped(
                rect.x.saturating_add(USER_INDENT),
                y,
                hint,
                style::muted().patch(bg),
                rect.width.saturating_sub(USER_INDENT),
            );
        }
    }

    for ref_line in ref_lines {
        let row = current_row;
        current_row += 1;
        if row < scroll || row >= viewport_end {
            continue;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        let connector = if ref_line.is_last {
            "  └─ "
        } else {
            "  ├─ "
        };
        grid.put(rect.x, y, connector, Style::default());
        let label_style = match ref_line.kind {
            crate::db::chat::MessageAttachmentKind::File => style::user_ref_file(),
            crate::db::chat::MessageAttachmentKind::Folder => style::user_ref_folder(),
            crate::db::chat::MessageAttachmentKind::Image => style::user_ref_image(),
        };
        grid.put_clipped(
            rect.x.saturating_add(USER_INDENT + 3),
            y,
            &ref_line.label,
            label_style,
            rect.width.saturating_sub(USER_INDENT + 3),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_markdown(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    lines: &[MdLine],
) {
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return;
    };
    let Some(message) = history.messages.get(message_index) else {
        return;
    };
    // 正文块也是步骤：消息仍在流式且它是最后一步 → 标记呼吸，否则静态。
    let mark = if message.status == "streaming" && step_index + 1 == message.steps.len() {
        StepMark::Running
    } else {
        StepMark::Done
    };
    for (offset, line) in lines.iter().enumerate().skip(scroll.saturating_sub(start)) {
        let row = start.saturating_add(offset);
        if row >= viewport_end {
            break;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        if offset == 0 {
            grid.put_char(
                rect.x,
                y,
                mark.glyph(
                    StepKind::Body,
                    history.chat.loading_frame,
                    history.chat.status.streaming,
                ),
                mark.style(StepKind::Body, false),
            );
        }
        let mut cx = rect
            .x
            .saturating_add(STEP_GUTTER)
            .saturating_add(line.indent);
        for (value, span_style) in &line.spans {
            for ch in value.chars() {
                let cell = ch.width().unwrap_or(1).max(1) as u16;
                if cx.saturating_add(cell) > rect.right() {
                    break;
                }
                grid.put_char(cx, y, ch, *span_style);
                cx = cx.saturating_add(cell);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_thinking(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    body: &[MdLine],
) {
    let Some((_, part)) = step_of(history, source) else {
        return;
    };
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return;
    };
    let streaming = history.chat.reasoning_streaming(message_index, step_index);
    let expanded = history.chat.reasoning_expanded(message_index, step_index);
    let mark = if streaming {
        StepMark::Running
    } else {
        StepMark::Done
    };
    if start < viewport_end
        && let Some(y) = visible_row(rect, scroll, start)
    {
        // 生成中：`Thinking`；完成后：`Thought · 6.3s`。
        let header_style = StepKind::Thinking.label(streaming, expanded);
        grid.put_char(
            rect.x,
            y,
            mark.glyph(
                StepKind::Thinking,
                history.chat.loading_frame,
                history.chat.status.streaming,
            ),
            mark.style(StepKind::Thinking, expanded),
        );
        let x = rect.x.saturating_add(STEP_GUTTER);
        let limit = rect.right();
        if streaming {
            grid.put_clipped(
                x,
                y,
                crate::ui::text::LX_CHAT_THINKING_LABEL,
                header_style,
                limit.saturating_sub(x),
            );
        } else {
            let mut label = String::from(crate::ui::text::LX_CHAT_THOUGHT_LABEL);
            if let Some(duration) = thinking_duration(part) {
                label.push_str(super::THINKING_DURATION_SEP);
                label.push_str(&super::format_duration(duration));
            }
            grid.put_clipped(x, y, &label, header_style, limit.saturating_sub(x));
        }
    }
    for (offset, line) in body.iter().enumerate() {
        let row = start.saturating_add(1).saturating_add(offset);
        if row < scroll {
            continue;
        }
        if row >= viewport_end {
            break;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        let mut cx = rect
            .x
            .saturating_add(STEP_GUTTER)
            .saturating_add(line.indent);
        for (value, span_style) in &line.spans {
            let cell_style = span_style.patch(style::muted());
            for ch in value.chars() {
                let cell = ch.width().unwrap_or(1).max(1) as u16;
                if cx.saturating_add(cell) > rect.right() {
                    break;
                }
                grid.put_char(cx, y, ch, cell_style);
                cx = cx.saturating_add(cell);
            }
        }
    }
}

/// 尚未接入内容缓存的块：保留原绘制路径（行数由快照给出）。
fn draw_legacy(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
) {
    let mut row = start;
    match source {
        BlockSource::Group { message_index, .. } => {
            let calls = calls_of(history, source);
            if calls.is_empty() {
                return;
            }
            let expanded: Vec<bool> = calls
                .iter()
                .map(|call| folds_at(history, message_index, call).body)
                .collect();
            tool::group_draw_animated(
                grid,
                rect,
                scroll,
                &mut row,
                &calls,
                history.chat.loading_frame,
                &expanded,
                history.chat.status.streaming,
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
                history.chat.status.streaming,
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
                history.chat.status.streaming || shell.is_running(),
                super::shell_folds(history, &message.id),
            );
        }
        BlockSource::Notice { message_index } => {
            let Some(message) = history.messages.get(message_index) else {
                return;
            };
            let Some(notice) = super::notice(message) else {
                return;
            };
            super::notice::draw(grid, &notice, rect, scroll, &mut row);
        }
        BlockSource::Rewind { last, count } => {
            let Some(notice) = history.chat.notices.get(last) else {
                return;
            };
            super::rewind::draw(grid, notice, count, rect, scroll, &mut row);
        }
        BlockSource::Compact { index, expanded } => {
            let Some(notice) = history.compact.checkpoints.get(index) else {
                return;
            };
            super::compact::draw(grid, notice, 1, expanded, rect, scroll, &mut row);
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
            super::report_block::draw(
                grid,
                rect,
                report,
                scroll,
                &mut row,
                history.chat.loading_frame,
                history.chat.status.streaming,
                expanded,
                more_steps,
            );
        }
        // 静态块的绘制数据自带样式，没有需要回查的动态状态。
        BlockSource::Plain => {}
    }
}
