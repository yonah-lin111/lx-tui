//! 聊天状态：模型 · 供应商 + 上下文占用，绘制在**窗格底边框**上。
//!
//! 输入框保留自己的上下 `─` 分割线；状态数据**覆盖**在 main content 的底边框（`╰──╯`）上：
//! 只改写文字落到的单元格，其余底边线保留为 `─`（边框始终可见）。
//! 左侧 ` 模型名 · 供应商名 `（模型名金色高亮、供应商用本轮统计行同款次要信息色），
//! 右端 `context:当前用量(百分比)`（token 短格式一位小数，如 `context:258.7k(26%)`；
//! 用量取最近一次同模型请求的分子、百分比 = 用量 / 模型上限（四舍五入），夹到 100%；
//! 用**供应商同款次要信息色**（与左侧供应商名一致）；**未知用量、未知上限或零消耗整段隐藏**）。
//! 宽度不足时依次丢弃上下文占用 → 供应商，始终保留模型名；模型名按列硬切（不使用省略号）。
//! 停止生成不再有按钮：流中按 `Esc`（单击待命提示、双击停止，对齐 opencode）。
//! 两端角字符保留，各控件矩形由同一布局函数派生。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::Lx;
use crate::app::state::lx::chat::ChatStatus;
use crate::ui::{style, text};

/// 左侧模型名至少保留的列数（右侧块取舍基准）。
const MIN_MODEL: u16 = 6;
/// 各段之间的空档（列）。
const GAP: u16 = 1;
/// 底边框两端角内侧各留的空白列。
const EDGE_PAD: u16 = 1;

/// 底边框状态布局：绘制行（零高表示空间不足不绘制）、上下文占用、
/// 左侧模型 · 供应商可用矩形，以及右端「块」（供清底）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatusLayout {
    /// 状态绘制行（窗格底边框）；`height == 0` 表示不绘制。
    pub area: Rect,
    /// 上下文占用文字矩形（零宽表示已隐藏）。
    pub context: Rect,
    /// 左侧模型 · 供应商可用矩形。
    pub label: Rect,
    /// 右端块整体矩形（含右侧留白）；零宽表示无右端元素。
    pub block: Rect,
}

/// 空间不足时的空布局。
fn hidden(left: u16, y: u16) -> StatusLayout {
    StatusLayout {
        area: Rect::new(left, y, 0, 0),
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
    // 模型名 = 上下文占用 > 供应商：上下文放不下就整段隐藏。
    let keep_ctx = ctx_w > 0 && avail >= MIN_MODEL.saturating_add(ctx_w);
    let mut right = end;
    let mut context_rect = Rect::new(end, y, 0, 1);
    if keep_ctx {
        right = right.saturating_sub(ctx_w);
        context_rect = Rect::new(right, y, ctx_w, 1);
    }
    // 右端块（含右侧 1 列留白，供清底为空格）；左侧文本区右端再留 1 列空档。
    let block = if keep_ctx {
        Rect::new(
            right,
            y,
            end.saturating_sub(right).saturating_add(EDGE_PAD),
            1,
        )
    } else {
        Rect::new(end, y, 0, 1)
    };
    let label_right = if keep_ctx {
        right.saturating_sub(GAP)
    } else {
        end
    };
    StatusLayout {
        area: Rect::new(inner.x, y, inner.width, 1),
        context: context_rect,
        label: Rect::new(left, y, label_right.saturating_sub(left), 1),
        block,
    }
}

/// 在窗格底边框行上覆盖绘制状态（两端角与未被覆盖的底边线保留为 `─`）。
pub(crate) fn render(buf: &mut Buffer, inner: Rect, lx: &Lx) {
    let status = &lx.chat.status;
    let lay = layout(inner, status);
    if lay.area.height == 0 {
        return;
    }
    let y = lay.area.y;
    // 左端角内侧留 1 列空格（`╰ model…`），右端块内留白清为空格（`…72% ╯`）；
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
            style::chat_provider(),
            lay.context.right(),
        );
    }
}

/// 上下文占用文字（`context:当前用量(百分比)`，用量用一位小数的 token 短格式）；
/// 未知（无上限或无用量）与**零消耗**返回 None（整段隐藏）。
fn context_text(status: &ChatStatus) -> Option<String> {
    let used = status.input_tokens.filter(|tokens| *tokens > 0)?;
    let limit = status.context_limit.filter(|limit| *limit > 0)?;
    // 四舍五入（`258.7k / 1M` → 26%，与展示口径一致）。
    let percent = used.saturating_mul(100).saturating_add(limit / 2) / limit;
    Some(format!(
        "{}{}({}%)",
        text::LX_CHAT_CONTEXT_PREFIX,
        text::token_count_precise(used),
        percent.min(100)
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
