//! assistant 正文块与展开思考正文的绘制。
//!
//! 两块都走共享阅读渲染器 [`crate::ui::markdown_render`]，区别只在缩进与配色：
//! - **正文块**：首行以步骤方块起头，所有行缩进 [`STEP_GUTTER`] 列（2 列）与方块对齐，
//!   保持 markdown 自己的配色；代码块头部的复制按钮支持悬停高亮（加粗 + 下划线）。
//! - **展开的思考正文**：整体压暗（`style::muted`），缩进 [`STEP_GUTTER`] 列（2 列）
//!   与头部方块之后的 `Thinking` 文案对齐。
//!
//! 行数口径必须与 `history::layout` 一致——`rows` 与 `draw` 的换行宽度是同一个公式，
//! 否则滚动条、置顶行与命中矩形都会漂移。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::ui::markdown_render;
use crate::ui::style;

use super::super::Grid;
use super::{STEP_GUTTER, StepKind, StepMark, visible_row};

/// 正文块绘制参数：行首标记的状态、所属类别与呼吸相位。
pub(super) struct BodyStyle {
    pub(super) mark: StepMark,
    pub(super) kind: StepKind,
    pub(super) loading_frame: usize,
    /// 呼吸相位是否还在推进（动画停止时方块恒实心）。
    pub(super) animating: bool,
}

/// 绘制正文块：首行以步骤方块起头，所有行缩进 [`STEP_GUTTER`] 列与方块对齐
/// （渲染宽度已相应扣除，无需再换行）。
pub(super) fn draw_body(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    content: &str,
    style_args: BodyStyle,
) {
    let BodyStyle {
        mark,
        kind,
        loading_frame,
        animating,
    } = style_args;
    let inner = rect.width.saturating_sub(STEP_GUTTER);
    let mut first = true;
    for line in markdown_render::render_wrapped(content, inner) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            if first {
                grid.put_char(
                    rect.x,
                    y,
                    mark.glyph(loading_frame, animating),
                    mark.style(kind),
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
        first = false;
        *row_index += 1;
    }
}

/// thinking 块耗时（毫秒）：首个增量到最后一个增量的时间差；时间戳缺失返回 None。
pub(super) fn thinking_duration(part: &crate::app::state::ChatPartView) -> Option<i64> {
    let started = part.started_at_ms?;
    let ended = part.ended_at_ms?;
    Some(ended.saturating_sub(started).max(0))
}

/// thinking 正文渲染行数（markdown；左缩进 [`STEP_GUTTER`]，与头部文案对齐）。
pub(super) fn thinking_rows(content: &str, width: u16) -> usize {
    markdown_render::render_wrapped(content, width.saturating_sub(STEP_GUTTER)).len()
}

/// 绘制展开的 thinking 正文：与正文同款 markdown 渲染，但整体压暗（`style::muted`），
/// 所有行缩进 [`STEP_GUTTER`] 列，与头部文案对齐。
pub(super) fn draw_thinking(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    content: &str,
) {
    let inner = rect.width.saturating_sub(STEP_GUTTER);
    for line in markdown_render::render_wrapped(content, inner) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
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
        *row_index += 1;
    }
}
