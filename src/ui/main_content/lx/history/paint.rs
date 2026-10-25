//! 块绘制数据（二级缓存）：把每个内容块「排好的行」存下来跨帧复用。
//!
//! 每个变体只存**内容与静态样式**；步骤标记的呼吸字形、thinking 悬停色、用户消息 rewind
//! 选中底色、`loading_frame` 等动态内容一律在 [`BlockPaint::draw`] 时现算。
//!
//! `rows()` 与绘制共用同一份数据：快照行数、滚动条几何、点击面几何都从这里取。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

use crate::app::state::{AssistantStepView, ChatMessageView, Lx, ToolCallView};
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
}

impl BlockSource {
    pub(crate) fn message_index(self) -> Option<usize> {
        match self {
            Self::Step { message_index, .. }
            | Self::Group { message_index, .. }
            | Self::Shell { message_index }
            | Self::Notice { message_index } => Some(message_index),
            Self::Rewind { .. } | Self::Compact { .. } => None,
        }
    }
}

/// 一个块的绘制数据。
#[derive(Debug)]
pub(crate) enum BlockPaint {
    /// 用户正文：软换行后的纯文本行（不含 `> ` 前缀与底色）。
    UserText { lines: Vec<String> },
    /// 助手正文：markdown 渲染后的行。
    Markdown { lines: Vec<MdLine> },
    /// thinking：头部之外只有正文行（折叠时为空；头部文案在绘制时算）。
    Thinking { body: Vec<MdLine> },
    /// 完全静态的单色行块（统计行）。
    Static { lines: Vec<String>, style: Style },
    /// 尚未接入内容缓存的块：只缓存行数，绘制仍走原路径。
    Legacy { rows: usize },
}

impl BlockPaint {
    /// 该块占用的内容行数（与绘制推进行数严格相等）。
    pub(crate) fn rows(&self) -> usize {
        match self {
            Self::UserText { lines } => lines.len(),
            Self::Markdown { lines } => lines.len(),
            Self::Thinking { body } => 1 + body.len(),
            Self::Static { lines, .. } => lines.len(),
            Self::Legacy { rows } => *rows,
        }
    }

    /// 近似字节数（只用于 LRU 记账，不追求精确）。
    pub(crate) fn bytes(&self) -> usize {
        match self {
            Self::UserText { lines } | Self::Static { lines, .. } => {
                lines.iter().map(|line| line.len() + 24).sum::<usize>()
            }
            Self::Markdown { lines } => md_bytes(lines),
            Self::Thinking { body } => md_bytes(body),
            // 旧路径没有缓存正文，不占二级缓存预算（行数已在一级快照里）。
            Self::Legacy { .. } => 0,
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
        lx: &Lx,
        source: BlockSource,
    ) {
        let viewport_end = scroll.saturating_add(usize::from(rect.height));
        match self {
            Self::UserText { lines } => {
                draw_user_text(grid, rect, lx, source, scroll, start, viewport_end, lines)
            }
            Self::Markdown { lines } => {
                draw_markdown(grid, rect, lx, source, scroll, start, viewport_end, lines)
            }
            Self::Thinking { body } => {
                draw_thinking(grid, rect, lx, source, scroll, start, viewport_end, body)
            }
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
            Self::Legacy { .. } => draw_legacy(grid, rect, lx, source, scroll, start),
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
    lx: &Lx,
    source: BlockSource,
) -> Option<(&ChatMessageView, &crate::app::state::ChatPartView)> {
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return None;
    };
    let message = lx.chat.messages.get(message_index)?;
    match message.steps.get(step_index)? {
        AssistantStepView::Part { part, .. } => Some((message, part)),
        AssistantStepView::Tool { .. } => None,
    }
}

/// 合并组的调用列表（单调用时长度为 1）。
fn calls_of(lx: &Lx, source: BlockSource) -> Vec<&ToolCallView> {
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
    let Some(message) = lx.chat.messages.get(message_index) else {
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
fn folds_at(lx: &Lx, source: BlockSource, call: &ToolCallView) -> Folds {
    let Some(message_index) = source.message_index() else {
        return Folds::default();
    };
    let Some(message) = lx.chat.messages.get(message_index) else {
        return Folds::default();
    };
    super::folds_of(lx, &message.id, call)
}

#[allow(clippy::too_many_arguments)]
fn draw_user_text(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    lines: &[String],
) {
    let BlockSource::Step { message_index, .. } = source else {
        return;
    };
    let Some(message) = lx.chat.messages.get(message_index) else {
        return;
    };
    let selected = lx
        .rewind
        .selected
        .as_ref()
        .map(|target| target.user_message_id.as_str())
        == Some(message.id.as_str());
    let bg = if selected {
        style::chat_user_target_bg()
    } else {
        style::chat_user_bg()
    };
    for (offset, line) in lines.iter().enumerate().skip(scroll.saturating_sub(start)) {
        let row = start.saturating_add(offset);
        if row >= viewport_end {
            break;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        // 整块底色覆盖整个内容列（含 `> ` 前缀列）。
        grid.background_row(rect.x, y, rect.width, bg);
        if offset == 0 {
            // 图标用输入框同款强调色，并叠加同底色。
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
}

#[allow(clippy::too_many_arguments)]
fn draw_markdown(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
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
    let Some(message) = lx.chat.messages.get(message_index) else {
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
                    lx.chat.loading_frame,
                    lx.chat.status.streaming,
                ),
                mark.style(StepKind::Body, false, false),
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
    lx: &Lx,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    body: &[MdLine],
) {
    let Some((_, part)) = step_of(lx, source) else {
        return;
    };
    let BlockSource::Step {
        message_index,
        step_index,
    } = source
    else {
        return;
    };
    let streaming = lx.chat.reasoning_streaming(message_index, step_index);
    let hovered = lx.chat.thinking_is_hovered(message_index, step_index);
    let expanded = lx.chat.reasoning_expanded(message_index, step_index);
    let mark = if streaming {
        StepMark::Running
    } else {
        StepMark::Done
    };
    if start < viewport_end
        && let Some(y) = visible_row(rect, scroll, start)
    {
        // 生成中：`Thinking`；完成后：`Thought · 6.3s`。
        let header_style = StepKind::Thinking.label(streaming, expanded, hovered);
        grid.put_char(
            rect.x,
            y,
            mark.glyph(
                StepKind::Thinking,
                lx.chat.loading_frame,
                lx.chat.status.streaming,
            ),
            mark.style(StepKind::Thinking, expanded, hovered),
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
    lx: &Lx,
    source: BlockSource,
    scroll: usize,
    start: usize,
) {
    let mut row = start;
    match source {
        BlockSource::Group { .. } => {
            let calls = calls_of(lx, source);
            if calls.is_empty() {
                return;
            }
            let expanded: Vec<bool> = calls
                .iter()
                .map(|call| folds_at(lx, source, call).body)
                .collect();
            tool::group_draw_animated(
                grid,
                rect,
                scroll,
                &mut row,
                &calls,
                lx.chat.loading_frame,
                &expanded,
                lx.chat.status.streaming,
            );
        }
        BlockSource::Step { .. } => {
            let calls = calls_of(lx, source);
            let Some(call) = calls.first() else {
                return;
            };
            tool::draw_animated(
                grid,
                rect,
                scroll,
                &mut row,
                call,
                lx.chat.loading_frame,
                folds_at(lx, source, call),
                lx.chat.status.streaming,
            );
        }
        BlockSource::Shell { message_index } => {
            let Some(message) = lx.chat.messages.get(message_index) else {
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
                lx.chat.loading_frame,
                lx.chat.status.streaming || shell.is_running(),
                super::shell_folds(lx, &message.id),
            );
        }
        BlockSource::Notice { message_index } => {
            let Some(message) = lx.chat.messages.get(message_index) else {
                return;
            };
            let Some(notice) = super::notice(message) else {
                return;
            };
            super::notice::draw(grid, &notice, rect, scroll, &mut row);
        }
        BlockSource::Rewind { last, count } => {
            let Some(notice) = lx.chat.notices.get(last) else {
                return;
            };
            super::rewind::draw(grid, notice, count, rect, scroll, &mut row);
        }
        BlockSource::Compact { index, expanded } => {
            let Some(notice) = lx.compact.checkpoints.get(index) else {
                return;
            };
            super::compact::draw(grid, notice, 1, expanded, rect, scroll, &mut row);
        }
    }
}
