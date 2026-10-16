//! 聊天状态行：模型 · 供应商、上下文占用组件与停止控件（位于输入框下方）。
//!
//! 状态区高度由同一布局函数计算后传给渲染与鼠标命中，字符串长度不改变按钮矩形。
//! 模型名与供应商名过长时截断；上下文占用组件复用 `@`/UI 页同款圆点分档。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::Lx;
use crate::app::state::lx::chat::ChatStatus;
use crate::ui::{style, text};

use super::Grid;

/// 停止按钮占用的列数。
const BUTTON_WIDTH: u16 = 5;
/// 上下文占用组件与模型名之间的最小间距（列）。
const PILL_GAP: u16 = 2;

/// 状态区布局：文本区与停止按钮矩形。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLayout {
    pub area: Rect,
    pub stop: Option<Rect>,
    pub label: Rect,
}

/// 状态区期望行数：窄窗格（< 36 列）2 行，否则 1 行。
pub(super) fn rows(area: Rect, _status: &ChatStatus) -> u16 {
    if area.width < 36 {
        2.min(area.height)
    } else {
        1.min(area.height)
    }
}

/// 计算状态区布局；空间不足时返回零行。
pub(super) fn layout(area: Rect, status: &ChatStatus, rows: u16) -> StatusLayout {
    let rows = rows.min(area.height);
    if area.width < 8 || rows == 0 {
        return StatusLayout {
            area: Rect::new(area.x, area.y, area.width, 0),
            stop: None,
            label: area,
        };
    }
    let status_row = Rect::new(area.x, area.y, area.width, rows);
    let mut right = status_row.right();
    let mut stop = None;
    if status.streaming && right.saturating_sub(status_row.x) > BUTTON_WIDTH + 4 {
        stop = Some(Rect::new(
            right - BUTTON_WIDTH,
            status_row.y,
            BUTTON_WIDTH,
            1,
        ));
        right -= BUTTON_WIDTH;
    }
    StatusLayout {
        area: status_row,
        stop,
        label: Rect::new(
            status_row.x,
            status_row.y,
            right.saturating_sub(status_row.x),
            rows,
        ),
    }
}

/// 渲染状态区；返回实际占用行数。
pub(super) fn render(grid: &mut Grid, area: Rect, lx: &Lx) -> u16 {
    let status = &lx.chat.status;
    let rows = rows(area, status);
    let lay = layout(area, status, rows);
    if lay.area.height == 0 {
        return 0;
    }
    let model = status
        .model_name
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_NO_MODEL_SHORT.to_string());
    let provider = status.provider_name.clone().unwrap_or_default();

    if rows >= 2 {
        // 窄窗格：模型 · 供应商占首行（截断），占用组件第二行。
        grid.put_clipped(
            lay.label.x,
            lay.label.y,
            &model_provider_line(&model, &provider),
            style::muted(),
            lay.label.width,
        );
        render_context(grid, lay.label.x, lay.label.y + 1, status, lay.label.width);
    } else {
        let (model, provider) = fit_model_provider(&model, &provider, lay.label, status);
        let mut x = lay.label.x;
        if !model.is_empty() {
            grid.put_clipped(x, lay.label.y, &model, style::muted(), lay.label.width);
            x = x.saturating_add(u16::try_from(display_width(&model)).unwrap_or(u16::MAX));
        }
        if !provider.is_empty() {
            grid.put_clipped(
                x,
                lay.label.y,
                &format!("{}{provider}", text::LX_CHAT_MODEL_SEP),
                style::muted(),
                lay.label.right().saturating_sub(x),
            );
        }
        // 上下文组件贴右缘（避免与停止按钮重叠）。
        render_context(grid, lay.label.x, lay.label.y, status, lay.label.width);
    }

    if let Some(stop) = lay.stop {
        put_centered(grid, stop, text::LX_CHAT_STOP_BUTTON, style::error());
    }
    // 省略提示贴行右端（有空间时）。
    if let Some(omitted) = status.omitted_turns {
        let notice = format!("{} {omitted}", text::LX_CHAT_OMITTED_PREFIX);
        let width = u16::try_from(notice.width()).unwrap_or(u16::MAX);
        if width < lay.label.width {
            let x = lay.label.x + lay.label.width.saturating_sub(width);
            grid.put_clipped(x, lay.label.y, &notice, style::muted(), lay.label.width);
        }
    }
    lay.area.height
}

/// 模型 · 供应商展示行（供应商为空时仅模型名）。
fn model_provider_line(model: &str, provider: &str) -> String {
    if provider.is_empty() {
        model.to_string()
    } else {
        format!("{model}{}{provider}", text::LX_CHAT_MODEL_SEP)
    }
}

/// 单行时按标签宽度裁剪模型名，给上下文组件与供应商留位；返回 (模型, 供应商)。
fn fit_model_provider(
    model: &str,
    provider: &str,
    label: Rect,
    status: &ChatStatus,
) -> (String, String) {
    let pill = context_pill_width(status);
    let pill_reserve = if pill > 0 { pill + PILL_GAP } else { 0 };
    let avail = usize::from(label.width).saturating_sub(usize::from(pill_reserve));
    let provider_w = provider.width();
    let model_w = model.width();
    let sep = text::LX_CHAT_MODEL_SEP.width();
    if provider.is_empty() {
        return (truncate_width(model, avail), String::new());
    }
    // 模型名过长时先截断模型名，保留供应商（更短更稳定）。
    if model_w + sep + provider_w <= avail {
        return (model.to_string(), provider.to_string());
    }
    if provider_w + sep < avail {
        let model_room = avail - sep - provider_w;
        return (truncate_width(model, model_room), provider.to_string());
    }
    // 供应商也放不下：只截断供应商。
    (
        truncate_width(model, avail.saturating_sub(sep)),
        truncate_width(provider, avail.saturating_sub(sep)),
    )
}

/// 绘制上下文占用组件 `● context NN%`（未知百分比时显示 `● context --`）。
fn render_context(grid: &mut Grid, x: u16, y: u16, status: &ChatStatus, width: u16) {
    let right = x.saturating_add(width);
    let percent = match status.context_percent {
        Some(percent) => format!("{percent}%"),
        None => text::LX_CHAT_TOKENS_UNKNOWN.to_string(),
    };
    let label = format!("{} {percent}", text::agent::TOKENS_PILL_LABEL_CTX);
    let pill_w = u16::try_from(1 + 1 + display_width(&label)).unwrap_or(u16::MAX);
    if pill_w > width {
        return;
    }
    let px = right.saturating_sub(pill_w);
    let dot_style = match status.context_percent {
        Some(percent) => style::chat_context_dot(percent),
        None => style::muted(),
    };
    grid.put_char(
        px,
        y,
        text::agent::MARKER_DOT.chars().next().unwrap_or('●'),
        dot_style,
    );
    grid.put_clipped(px + 2, y, &label, style::muted(), width);
}

/// 上下文组件占用的列宽（无上限/未知时仍占位以保持稳定）。
fn context_pill_width(status: &ChatStatus) -> u16 {
    let percent = match status.context_percent {
        Some(percent) => format!("{percent}%"),
        None => text::LX_CHAT_TOKENS_UNKNOWN.to_string(),
    };
    let label = format!("{} {percent}", text::agent::TOKENS_PILL_LABEL_CTX);
    u16::try_from(2 + display_width(&label)).unwrap_or(u16::MAX)
}

/// 上下文占用百分比文本（测试用）：未知用 `--`。
#[cfg(test)]
pub(super) fn context_text(status: &ChatStatus) -> String {
    let percent = match status.context_percent {
        Some(percent) => format!("{percent}%"),
        None => text::LX_CHAT_TOKENS_UNKNOWN.to_string(),
    };
    format!("{} {percent}", text::agent::TOKENS_PILL_LABEL_CTX)
}

/// 截断到显示宽度（宽字符按两列计）。
fn truncate_width(value: &str, max: usize) -> String {
    crate::ui::text::truncate_to_width(value, max)
}

/// 文本显示宽度（宽字符按两列计）。
fn display_width(value: &str) -> usize {
    value.width()
}

/// 在矩形内水平居中写入。
fn put_centered(grid: &mut Grid, rect: Rect, content: &str, style: Style) {
    let width = u16::try_from(content.width()).unwrap_or(u16::MAX);
    if width > rect.width {
        grid.put_clipped(rect.x, rect.y, content, style, rect.width);
        return;
    }
    let x = rect.x + (rect.width - width) / 2;
    grid.put_clipped(x, rect.y, content, style, width);
}

#[cfg(test)]
mod tests;
