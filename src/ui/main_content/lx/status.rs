//! 聊天状态：模型 · 供应商 + 上下文占用 + 停止控件，绘制在**窗格底边框**上。
//!
//! 输入框保留自己的上下 `─` 分割线；状态数据**覆盖**在 main content 的底边框（`╰──╯`）上：
//! 只有文字/控件落到的单元格被改写，其余底边线保留为 `─`（边框始终可见）。
//! 左侧 ` 模型名 · 供应商名 `（模型名金色高亮、供应商用 prompt 条目标题同款普通文字色），
//! 右端 `context:NN%`（**未知用量或零消耗整段隐藏**；用量很小、四舍五入到 0% 时仍显示
//! `context:0%`——有真实用量就是有上下文），流中再在其右侧显示停止控件 `[■]`。
//! 宽度不足时依次丢弃停止控件 → 上下文占用 → 供应商，始终保留模型名；模型名按列硬切
//! （不使用省略号）。
//! 两端角字符保留，各控件矩形由同一布局函数派生，不随字符串长度改变按钮矩形。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::Lx;
use crate::app::state::lx::chat::ChatStatus;
use crate::ui::{style, text};

/// 停止控件文案宽度（`[■]` 3 列）。
const STOP_WIDTH: u16 = 3;
/// 左侧模型名至少保留的列数（右侧块取舍基准）。
const MIN_MODEL: u16 = 6;
/// 各段之间的空档（列）。
const GAP: u16 = 1;
/// 底边框两端角内侧各留的空白列。
const EDGE_PAD: u16 = 1;

/// 底边框状态布局：绘制行（零高表示空间不足不绘制）、停止控件、上下文占用、
/// 左侧模型 · 供应商可用矩形，以及右端「块」（供清底）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatusLayout {
    /// 状态绘制行（窗格底边框）；`height == 0` 表示不绘制。
    pub area: Rect,
    /// 停止控件矩形（流中且空间足够时）。
    pub stop: Option<Rect>,
    /// 上下文占用文字矩形（零宽表示已隐藏）。
    pub context: Rect,
    /// 左侧模型 · 供应商可用矩形。
    pub label: Rect,
    /// 右端块整体矩形（含各段空档与右侧留白）；零宽表示无右端元素。
    pub block: Rect,
}

/// 空间不足时的空布局。
fn hidden(left: u16, y: u16) -> StatusLayout {
    StatusLayout {
        area: Rect::new(left, y, 0, 0),
        stop: None,
        context: Rect::new(left, y, 0, 1),
        label: Rect::new(left, y, 0, 1),
        block: Rect::new(left, y, 0, 1),
    }
}

/// 计算底边框状态布局；`inner` 为窗格内容区（边框行 = `inner.bottom()`，
/// 可写区间为两端角内侧，角字符保留）。
pub(super) fn layout(inner: Rect, status: &ChatStatus) -> StatusLayout {
    let y = inner.bottom();
    let left = inner.x.saturating_add(EDGE_PAD);
    // 可写列区间 `[left, end)`（右端避让角字符）。
    let end = inner.right().saturating_sub(EDGE_PAD);
    if inner.width < 8 || end <= left {
        return hidden(left, y);
    }
    let avail = end.saturating_sub(left);
    let context = context_text(status);
    let ctx_w = context
        .as_ref()
        .map_or(0, |value| u16::try_from(value.width()).unwrap_or(u16::MAX));
    let want_ctx = ctx_w > 0;
    let want_stop = status.streaming;
    let fits = |cost: u16| avail >= MIN_MODEL.saturating_add(cost);
    // 降级顺序：上下文占用与模型名优先保留，先丢停止控件、再丢上下文占用。
    let (keep_ctx, keep_stop) = if want_ctx && want_stop && fits(ctx_w + GAP + STOP_WIDTH) {
        (true, true)
    } else if want_ctx && fits(ctx_w) {
        (true, false)
    } else if want_stop && fits(STOP_WIDTH) {
        (false, true)
    } else {
        (false, false)
    };
    // 自右向左落位：停止控件 → 上下文占用（相邻段之间留 `GAP` 空档）。
    let mut right = end;
    let mut first = true;
    let mut stop = None;
    let mut context_rect = Rect::new(end, y, 0, 1);
    if keep_stop {
        right = right.saturating_sub(STOP_WIDTH);
        stop = Some(Rect::new(right, y, STOP_WIDTH, 1));
        first = false;
    }
    if keep_ctx {
        if !first {
            right = right.saturating_sub(GAP);
        }
        right = right.saturating_sub(ctx_w);
        context_rect = Rect::new(right, y, ctx_w, 1);
        first = false;
    }
    // 右端块（含右侧 1 列留白，供清底为空格）；左侧文本区右端再留 1 列空档。
    let block = if first {
        Rect::new(end, y, 0, 1)
    } else {
        Rect::new(right, y, end.saturating_sub(right).saturating_add(1), 1)
    };
    let label_right = if first {
        end
    } else {
        right.saturating_sub(GAP)
    };
    StatusLayout {
        area: Rect::new(inner.x, y, inner.width, 1),
        stop,
        context: context_rect,
        label: Rect::new(left, y, label_right.saturating_sub(left), 1),
        block,
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

/// 在窗格底边框行上覆盖绘制状态（两端角与未被覆盖的底边线保留为 `─`）。
pub(crate) fn render(buf: &mut Buffer, inner: Rect, lx: &Lx) {
    let status = &lx.chat.status;
    let lay = layout(inner, status);
    if lay.area.height == 0 {
        return;
    }
    let y = lay.area.y;
    // 左端角内侧留 1 列空格（`╰ model…`），右端块内留白清为空格（`… [■] ╯`）；
    // 其余底边线保留 `─`（边框始终可见）。
    for x in inner.x..lay.label.x {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(Style::default());
        }
    }
    for x in lay.block.x..lay.block.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(Style::default());
        }
    }
    // 供应商：放得下完整 `模型 · 供应商` 才显示，否则整体隐藏（不省略号）。
    let notice = status
        .omitted_turns
        .map(|omitted| format!("{} {omitted}", text::LX_CHAT_OMITTED_PREFIX));
    let notice_w = notice
        .as_ref()
        .map_or(0, |value| u16::try_from(value.width()).unwrap_or(u16::MAX));
    // 省略提示贴左侧区右端；模型 · 供应商为其预留空间。
    let label_avail = if notice_w > 0 {
        lay.label.width.saturating_sub(notice_w + GAP)
    } else {
        lay.label.width
    };
    let model = status
        .model_name
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_NO_MODEL_SHORT.to_string());
    let provider = status.provider_name.clone().unwrap_or_default();
    let (model, provider) = fit_model_provider(&model, &provider, label_avail);
    let mut x = lay.label.x;
    let limit = lay.label.right();
    if !model.is_empty() && x < limit {
        put(buf, x, y, &model, style::chat_model(), limit);
        x = x.saturating_add(u16::try_from(model.width()).unwrap_or(u16::MAX));
    }
    if let Some(provider) = provider.filter(|_| x < limit) {
        let value = format!("{}{provider}", text::LX_CHAT_MODEL_SEP);
        put(buf, x, y, &value, style::chat_provider(), limit);
    }
    if let Some(notice) = notice.filter(|_| notice_w < lay.label.width) {
        put(
            buf,
            lay.label.right().saturating_sub(notice_w),
            y,
            &notice,
            style::muted(),
            lay.label.right(),
        );
    }
    if lay.context.width > 0
        && let Some(context) = context_text(status)
    {
        put(
            buf,
            lay.context.x,
            y,
            &context,
            style::chat_context_text(),
            lay.context.right(),
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

/// 上下文占用文字（`context:NN%`）；未知（无上限或无用量）与**零消耗**返回 None（整段隐藏）。
///
/// 注意：用量很小、百分比四舍五入到 0 时**仍然显示** `context:0%`——有真实用量就说明
/// 有上下文（否则百万级上下限下历史会话会一直看起来「上下文为空」）。
fn context_text(status: &ChatStatus) -> Option<String> {
    status.input_tokens.filter(|tokens| *tokens > 0)?;
    let percent = status.context_percent?;
    Some(format!(
        "{}{percent}{}",
        text::LX_CHAT_CONTEXT_PREFIX,
        text::LX_CHAT_PERCENT_SUFFIX
    ))
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
            cell.reset();
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

/// 左侧模型 · 供应商：模型名按可用宽度硬切（不加省略号）；供应商仅在「模型 + 分隔 + 供应商」
/// 完整放得下时显示，否则整体隐藏（不省略号）。
fn fit_model_provider(model: &str, provider: &str, label_width: u16) -> (String, Option<String>) {
    let avail = usize::from(label_width);
    let sep = text::LX_CHAT_MODEL_SEP.width();
    if !provider.is_empty() && model.width() + sep + provider.width() <= avail {
        return (model.to_string(), Some(provider.to_string()));
    }
    (cut_width(model, avail), None)
}

/// 按显示宽度硬切（无省略号；宽字符按两列计）。
fn cut_width(value: &str, max: usize) -> String {
    if value.width() <= max {
        return value.to_string();
    }
    let mut out = String::new();
    let mut width = 0;
    for ch in value.chars() {
        let cell = ch.width().unwrap_or(0);
        if width + cell > max {
            break;
        }
        out.push(ch);
        width += cell;
    }
    out
}

/// 上下文占用文字（测试用；None 表示整段隐藏）。
#[cfg(test)]
pub(super) fn context_text_for_test(status: &ChatStatus) -> Option<String> {
    context_text(status)
}

#[cfg(test)]
mod tests;
