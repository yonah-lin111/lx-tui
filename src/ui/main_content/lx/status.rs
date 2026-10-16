//! 聊天状态：模型 · 供应商 + 上下文占用组件 + 停止控件，绘制在**窗格底边框**上。
//!
//! 输入框保留自己的上下 `─` 分割线；原本输入框下方那一行的状态数据改绘制在
//! main content 的底边框（`╰──╯`）上：左侧 ` 模型名 · 供应商名 `（模型名金色高亮、
//! 供应商 muted），右端上下文占用组件（复用 `@`/UI 页圆点分档：`● context NN%`，
//! ≥90% 红、≥75% 黄、否则绿，未知 `--`），流中在最右端显示停止控件 `[■]`。
//! 两端角字符保留，各控件矩形由同一布局函数派生，字符串长度不改变按钮矩形。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::Lx;
use crate::app::state::lx::chat::ChatStatus;
use crate::ui::{style, text};

/// 停止控件文案宽度（`[■]` 3 列）。
const STOP_WIDTH: u16 = 3;
/// 上下文组件与模型 · 供应商之间的最小间距（列）。
const PILL_GAP: u16 = 2;
/// 底边框两端角内侧各留的空白列。
const EDGE_PAD: u16 = 1;

/// 底边框状态布局：绘制行（零高表示空间不足不绘制）、停止控件、上下文组件与左侧文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatusLayout {
    /// 状态绘制行（窗格底边框）；`height == 0` 表示不绘制。
    pub area: Rect,
    /// 停止控件矩形（流中且空间足够时）。
    pub stop: Option<Rect>,
    /// 右侧上下文组件矩形。
    pub pill: Rect,
    /// 左侧模型 · 供应商可用矩形。
    pub label: Rect,
}

/// 计算底边框状态布局；`inner` 为窗格内容区（边框行 = `inner.bottom()`，
/// 可写区间为两端角内侧，角字符保留）。
pub(super) fn layout(inner: Rect, status: &ChatStatus) -> StatusLayout {
    let y = inner.bottom();
    let left = inner.x.saturating_add(EDGE_PAD);
    // 可写列区间 `[left, end)`（右端同样避让角字符）。
    let end = inner.right().saturating_sub(EDGE_PAD);
    if inner.width < 8 || end <= left {
        return StatusLayout {
            area: Rect::new(inner.x, y, 0, 0),
            stop: None,
            pill: Rect::new(left, y, 0, 1),
            label: Rect::new(left, y, 0, 1),
        };
    }
    let mut end = end;
    let mut stop = None;
    if status.streaming && end.saturating_sub(left) > STOP_WIDTH + 8 {
        stop = Some(Rect::new(end.saturating_sub(STOP_WIDTH), y, STOP_WIDTH, 1));
        end = end.saturating_sub(STOP_WIDTH + 1);
    }
    let pill_w = context_pill_width(status).min(end.saturating_sub(left));
    let pill = Rect::new(end.saturating_sub(pill_w), y, pill_w, 1);
    let label_right = if pill_w > 0 {
        pill.x.saturating_sub(PILL_GAP)
    } else {
        end
    };
    StatusLayout {
        area: Rect::new(inner.x, y, inner.width, 1),
        stop,
        pill,
        label: Rect::new(left, y, label_right.saturating_sub(left), 1),
    }
}

/// 停止控件矩形（窗格底边框上，流中且空间足够时）；供命中复用。
pub(crate) fn stop_rect(inner: Rect, lx: &Lx) -> Option<Rect> {
    layout(inner, &lx.chat.status).stop
}

/// 命中窗格底边框上的停止控件；未命中返回 false。
pub(crate) fn status_stop_at(inner: Rect, lx: &Lx, column: u16, row: u16) -> bool {
    stop_rect(inner, lx).is_some_and(|rect| rect.contains((column, row).into()))
}

/// 在窗格底边框行上覆盖绘制状态（保留两端角）。
pub(crate) fn render(buf: &mut Buffer, inner: Rect, lx: &Lx) {
    let status = &lx.chat.status;
    let lay = layout(inner, status);
    if lay.area.height == 0 {
        return;
    }
    let y = lay.area.y;
    // 先铺空格清掉底边内段的 `─`（保留两端角：内段 = `[inner.x, inner.right())`）。
    for x in inner.x..inner.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(Style::default());
        }
    }
    let model = status
        .model_name
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_NO_MODEL_SHORT.to_string());
    let provider = status.provider_name.clone().unwrap_or_default();
    let (model, provider) = fit_model_provider(&model, &provider, lay.label.width);
    let mut x = lay.label.x;
    let limit = lay.label.right();
    if !model.is_empty() && x < limit {
        put(buf, x, y, &model, style::chat_model(), limit);
        x = x.saturating_add(u16::try_from(model.width()).unwrap_or(u16::MAX));
    }
    if !provider.is_empty() && x < limit {
        let value = format!("{}{provider}", text::LX_CHAT_MODEL_SEP);
        put(buf, x, y, &value, style::muted(), limit);
    }
    // 省略轮次提示贴左侧区右端（有空间时）。
    if let Some(omitted) = status.omitted_turns {
        let notice = format!("{} {omitted}", text::LX_CHAT_OMITTED_PREFIX);
        let width = u16::try_from(notice.width()).unwrap_or(u16::MAX);
        if width < lay.label.width {
            put(
                buf,
                lay.label.right().saturating_sub(width),
                y,
                &notice,
                style::muted(),
                lay.label.right(),
            );
        }
    }
    if lay.pill.width > 0 {
        let dot_style = match status.context_percent {
            Some(percent) => style::chat_context_dot(percent),
            None => style::muted(),
        };
        put(
            buf,
            lay.pill.x,
            y,
            text::agent::MARKER_DOT,
            dot_style,
            lay.pill.right(),
        );
        let percent = match status.context_percent {
            Some(percent) => format!("{percent}%"),
            None => text::LX_CHAT_TOKENS_UNKNOWN.to_string(),
        };
        let label = format!("{} {percent}", text::agent::TOKENS_PILL_LABEL_CTX);
        put(
            buf,
            lay.pill.x.saturating_add(2),
            y,
            &label,
            style::chat_context_text(),
            lay.pill.right(),
        );
    }
    if let Some(stop) = lay.stop {
        put(
            buf,
            stop.x,
            y,
            text::LX_CHAT_STOP_BUTTON,
            style::error(),
            stop.right(),
        );
    }
}

/// 逐字符覆盖写入（宽字符占两列、不跨 `limit` 半写）。
fn put(buf: &mut Buffer, mut x: u16, y: u16, content: &str, style: Style, limit: u16) {
    for ch in content.chars() {
        let width = ch.width().unwrap_or(0) as u16;
        if width == 0 {
            continue;
        }
        if x.saturating_add(width) > limit {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(ch);
            cell.set_style(style);
        }
        for offset in 1..width {
            if let Some(cell) = buf.cell_mut((x.saturating_add(offset), y)) {
                cell.reset();
                cell.set_char(' ');
                cell.set_style(style);
            }
        }
        x = x.saturating_add(width);
    }
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

/// 单行时按可用宽度裁剪模型名，优先保留供应商（更短更稳定）；返回 (模型, 供应商)。
fn fit_model_provider(model: &str, provider: &str, label_width: u16) -> (String, String) {
    let avail = usize::from(label_width);
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
