//! 聊天状态：模型 · 供应商 + 上下文占用组件 + 停止控件，绘制在**输入框底边框**上。
//!
//! 状态不再单独占一行（输入框下沿贴窗格底部），而是覆盖底边 `─` 绘制：左侧
//! ` 模型名 · 供应商名 `（模型名金色高亮、供应商 muted），右端上下文占用组件
//! （复用 `@`/UI 页圆点分档：`● context NN%`，≥90% 红、≥75% 黄、否则绿，未知 `--`），
//! 流中在右端最外侧显示停止控件 `[■]`。字符串长度不改变各控件矩形。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::Lx;
use crate::app::state::lx::chat::ChatStatus;
use crate::ui::{style, text};

use super::Grid;

/// 停止控件矩形宽度（`[■]` 3 列 + 右侧 1 列空档）。
const STOP_WIDTH: u16 = 4;
/// 上下文占用组件与模型 · 供应商之间的最小间距（列）。
const PILL_GAP: u16 = 2;

/// 状态布局：底边框整行、停止控件矩形与左侧文本 / 右侧组件可用矩形。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatusLayout {
    /// 底边框整行（状态绘制的基准行）。
    pub area: Rect,
    /// 停止控件矩形（流中且空间足够时）。
    pub stop: Option<Rect>,
    /// 右侧上下文组件覆盖区（贴右缘）。
    pub pill: Rect,
    /// 左侧模型 · 供应商可用区（上下文组件左侧）。
    pub label: Rect,
}

/// 计算状态布局；`border` 为输入框底边框行矩形，空间不足时返回零行。
pub(super) fn layout(border: Rect, status: &ChatStatus) -> StatusLayout {
    if border.width < 8 || border.height == 0 {
        return StatusLayout {
            area: Rect::new(border.x, border.y, border.width, 0),
            stop: None,
            pill: border,
            label: border,
        };
    }
    let mut right = border.right();
    let mut stop = None;
    if status.streaming && border.width > STOP_WIDTH + 5 {
        stop = Some(Rect::new(right - STOP_WIDTH, border.y, STOP_WIDTH, 1));
        right = right.saturating_sub(STOP_WIDTH);
    }
    let pill_w = context_pill_width(status).min(right.saturating_sub(border.x));
    let pill = Rect::new(right.saturating_sub(pill_w), border.y, pill_w, 1);
    let mut label_right = pill.x;
    if pill_w > 0 {
        label_right = label_right.saturating_sub(PILL_GAP);
    }
    let label = Rect::new(border.x, border.y, label_right.saturating_sub(border.x), 1);
    StatusLayout {
        area: border,
        stop,
        pill,
        label,
    }
}

/// 在底边框行上绘制状态（覆盖 `─`）；返回是否绘制。
pub(super) fn render(grid: &mut Grid, border: Rect, lx: &Lx) -> bool {
    let status = &lx.chat.status;
    let lay = layout(border, status);
    if lay.area.height == 0 {
        return false;
    }
    // 先整行铺空格覆盖底边 `─`，再落模型 · 供应商与上下文组件（不留 `─` 残影）。
    grid.background_row(border.x, border.y, border.width, style::muted());
    let model = status
        .model_name
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_NO_MODEL_SHORT.to_string());
    let provider = status.provider_name.clone().unwrap_or_default();

    // 左侧模型 · 供应商：模型名金色高亮、供应商 muted；两侧各 1 列空格。
    let (model, provider) = fit_model_provider(&model, &provider, lay.label.width);
    let mut x = lay.label.x.saturating_add(1);
    let right_edge = lay.label.right().saturating_sub(1);
    if !model.is_empty() && x < right_edge {
        grid.put_clipped(x, border.y, &model, style::chat_model(), right_edge - x);
        x = x.saturating_add(u16::try_from(model.width()).unwrap_or(u16::MAX));
    }
    if !provider.is_empty() && x < right_edge {
        let value = format!("{}{provider}", text::LX_CHAT_MODEL_SEP);
        grid.put_clipped(x, border.y, &value, style::muted(), right_edge - x);
    }
    // 省略轮次提示贴左侧区右端（有空间时）。
    if let Some(omitted) = status.omitted_turns {
        let notice = format!("{} {omitted}", text::LX_CHAT_OMITTED_PREFIX);
        let width = u16::try_from(notice.width()).unwrap_or(u16::MAX);
        if width < lay.label.width {
            let nx = lay.label.right().saturating_sub(width);
            grid.put_clipped(nx, border.y, &notice, style::muted(), width);
        }
    }
    render_context(grid, lay.pill, status);
    if let Some(stop) = lay.stop {
        grid.put_clipped(
            stop.x,
            stop.y,
            text::LX_CHAT_STOP_BUTTON,
            style::error(),
            stop.width,
        );
    }
    true
}

/// 绘制上下文占用组件 `● context NN%`（未知百分比时 `● context --`）。
fn render_context(grid: &mut Grid, pill: Rect, status: &ChatStatus) {
    if pill.width == 0 {
        return;
    }
    let percent = match status.context_percent {
        Some(percent) => format!("{percent}%"),
        None => text::LX_CHAT_TOKENS_UNKNOWN.to_string(),
    };
    // 组件右端回一列空格，再圆点（分档色）+ label（亮白）。
    let label = format!("{} {percent}", text::agent::TOKENS_PILL_LABEL_CTX);
    let dot_style = match status.context_percent {
        Some(percent) => style::chat_context_dot(percent),
        None => style::muted(),
    };
    grid.put_char(
        pill.x,
        pill.y,
        text::agent::MARKER_DOT.chars().next().unwrap_or('●'),
        dot_style,
    );
    grid.put_clipped(
        pill.x.saturating_add(2),
        pill.y,
        &label,
        style::chat_context_text(),
        pill.width.saturating_sub(2),
    );
}

/// 上下文组件占用的列宽（`●` + 空格 + label；未知仍占位以保持稳定）。
fn context_pill_width(status: &ChatStatus) -> u16 {
    let percent = match status.context_percent {
        Some(percent) => format!("{percent}%"),
        None => text::LX_CHAT_TOKENS_UNKNOWN.to_string(),
    };
    let label = format!("{} {percent}", text::agent::TOKENS_PILL_LABEL_CTX);
    u16::try_from(2 + label.width()).unwrap_or(u16::MAX)
}

/// 单行时按标签宽度裁剪模型名，优先保留供应商（更短更稳定）；返回 (模型, 供应商)。
fn fit_model_provider(model: &str, provider: &str, label_width: u16) -> (String, String) {
    // 两侧各 1 列空格。
    let avail = usize::from(label_width).saturating_sub(2);
    let provider_w = provider.width();
    let model_w = model.width();
    let sep = text::LX_CHAT_MODEL_SEP.width();
    if provider.is_empty() {
        return (truncate_width(model, avail), String::new());
    }
    if model_w + sep + provider_w <= avail {
        return (model.to_string(), provider.to_string());
    }
    if provider_w + sep < avail {
        let model_room = avail - sep - provider_w;
        return (truncate_width(model, model_room), provider.to_string());
    }
    (
        truncate_width(model, avail.saturating_sub(sep)),
        truncate_width(provider, avail.saturating_sub(sep)),
    )
}

/// 截断到显示宽度（宽字符按两列计）。
fn truncate_width(value: &str, max: usize) -> String {
    crate::ui::text::truncate_to_width(value, max)
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

#[cfg(test)]
mod tests;
