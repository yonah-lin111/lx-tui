//! 聊天状态：模型名 · 思考等级 · 上下文占用，绘制在**窗格底边框**上。
//!
//! 输入框保留自己的上下 `─` 分割线；状态数据**覆盖**在 main content 的底边框（`╰──╯`）上：
//! 只改写文字落到的单元格，其余底边线保留为 `─`（边框始终可见）。
//! 左侧 ` 模型名 · 上下文占用 `（模型名用入口标签同款粉→蓝渐变，上下文占用用次要信息色；
//! 形如 `258.7k(26%)`——**不带 `context:` 前缀**，紧跟模型名之后即语义明确）。
//! 思考等级（variant 原始 key，如 `high`）紧跟模型名之后，**加粗黄字**（对齐
//! opencode 的 `theme.text.feedback.warning.base` + BOLD），分隔符仍用次要信息色；
//! 模型无 variants 时整段不显示。
//! 供应商名不再显示（对用户无信息量，位置让给上下文占用）。
//! 用量取最近一次同模型请求的分子、百分比 = 用量 / 模型上限（四舍五入），夹到 100%；
//! **未知用量、未知上限或零消耗整段隐藏**。
//! 宽度不足时依次丢弃上下文占用、丢弃思考等级、再硬切模型名（不使用省略号），
//! 始终保留模型名。
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
/// 左侧模型 · 上下文可用矩形，以及右端的后台任务计数指标块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatusLayout {
    /// 状态绘制行（窗格底边框）；`height == 0` 表示不绘制。
    pub area: Rect,
    /// 思考等级文字矩形（零宽表示已隐藏）；紧跟模型名之后。
    pub variant: Rect,
    /// 上下文占用文字矩形（零宽表示已隐藏）；紧跟思考等级之后。
    pub context: Rect,
    /// 左侧模型 · 上下文可用矩形。
    pub label: Rect,
    /// 模型名实际绘制宽度（由布局的降级链得出，渲染必须用同一值，不得重算）。
    pub model_width: u16,
    /// 右端后台任务计数指标矩形（零宽表示不显示）。
    pub block: Rect,
}

/// 空间不足时的空布局。
fn hidden(left: u16, y: u16) -> StatusLayout {
    StatusLayout {
        area: Rect::new(left, y, 0, 0),
        variant: Rect::new(left, y, 0, 1),
        context: Rect::new(left, y, 0, 1),
        label: Rect::new(left, y, 0, 1),
        model_width: 0,
        block: Rect::new(left, y, 0, 1),
    }
}

/// 计算底边框状态布局；`inner` 为窗格内容区（边框行 = `inner.bottom()`，
/// 可写区间为两端角内侧，角字符保留）。
///
/// 左段是 `模型 · 上下文占用`；右段是后台任务计数指标（`1 process`），
/// 为 0 时整段不渲染（对齐 opencode 的 `N shell`）。两段互不重叠：左侧可用宽度
/// 先扣掉指标宽度，再决定上下文占用是否还放得下。
pub(super) fn layout(inner: Rect, status: &ChatStatus, tasks: usize) -> StatusLayout {
    let y = inner.bottom();
    let left = inner.x.saturating_add(EDGE_PAD);
    // 可写列区间 `[left, end)`（右端避让角字符）。
    let end = inner.right().saturating_sub(EDGE_PAD);
    if inner.width < 8 || end <= left {
        return hidden(left, y);
    }
    let task_w = task_text(tasks)
        .as_ref()
        .map_or(0, |value| u16::try_from(value.width()).unwrap_or(u16::MAX));
    // 指标贴右端；留一格空档与左侧文字分开。
    //
    // 阈值必须按**真实可写宽度**比较：`inner.width` 还含左右各 `EDGE_PAD` 的内缩，
    // 直接拿它比会宽出 3 列，于是「状态栏认为放得下」而实际模型名被压成 0 列。
    let keep_tasks = task_w > 0
        && end.saturating_sub(left) > task_w.saturating_add(GAP).saturating_add(MIN_MODEL);
    let avail = end.saturating_sub(left).saturating_sub(if keep_tasks {
        task_w.saturating_add(GAP)
    } else {
        0
    });
    // 左段自左向右依次是：模型名 · 思考等级 · 上下文占用。
    //
    // 取舍用**单一降级链**算：模型名先按「什么都不显示」裁剪，再依次尝试
    // 「+ 上下文」「+ 思考等级」「+ 两者」。第一个放得下的组合即结果——
    // 这样任何字段都不会因为「别的字段可能需要空间」而被拖累，
    // 也不会出现字段被判放不下、模型名却仍被它占位压窄的错位。
    let ctx_w = field_width(context_text(status));
    let variant_w = field_width(variant_text(status));
    let model_only = model_width(status, avail);
    let mut chosen = (false, false, model_only);
    if ctx_w > 0 && model_only.saturating_add(sep_w()).saturating_add(ctx_w) <= avail {
        chosen = (false, true, model_only);
    }
    if variant_w > 0 && model_only.saturating_add(sep_w()).saturating_add(variant_w) <= avail {
        chosen = (true, false, model_only);
    }
    if ctx_w > 0 && variant_w > 0 {
        // 两者同显：`模型 · variant · ctx`。
        let need = model_only
            .saturating_add(sep_w())
            .saturating_add(variant_w)
            .saturating_add(sep_w())
            .saturating_add(ctx_w);
        if need <= avail {
            chosen = (true, true, model_only);
        }
    }
    let (keep_variant, keep_ctx, drawn_model) = chosen;
    // 字段起点按实际绘制宽度递推（与渲染同一口径）。
    let variant_x = if keep_variant {
        left.saturating_add(drawn_model).saturating_add(sep_w())
    } else {
        left
    };
    let context_x = if keep_ctx {
        if keep_variant {
            variant_x.saturating_add(variant_w).saturating_add(sep_w())
        } else {
            left.saturating_add(drawn_model).saturating_add(sep_w())
        }
    } else {
        left
    };
    StatusLayout {
        area: Rect::new(inner.x, y, inner.width, 1),
        variant: if keep_variant {
            Rect::new(variant_x, y, variant_w, 1)
        } else {
            Rect::new(left, y, 0, 1)
        },
        context: if keep_ctx {
            Rect::new(context_x, y, ctx_w, 1)
        } else {
            Rect::new(left, y, 0, 1)
        },
        label: Rect::new(left, y, avail, 1),
        model_width: drawn_model,
        block: if keep_tasks {
            Rect::new(end.saturating_sub(task_w), y, task_w, 1)
        } else {
            Rect::new(end, y, 0, 1)
        },
    }
}

/// 分隔符（` · `）的显示宽度。
fn sep_w() -> u16 {
    u16::try_from(text::LX_CHAT_MODEL_SEP.width()).unwrap_or(0)
}

/// 一段可选文字的显示宽度（None → 0）。
fn field_width(value: Option<String>) -> u16 {
    value
        .as_ref()
        .map_or(0, |value| u16::try_from(value.width()).unwrap_or(u16::MAX))
}

/// 模型名实际绘制宽度（按左侧可用宽度 `avail` 裁剪；不预扣任何可选字段）。
///
/// 可选字段（思考等级/上下文）是否显示由 [`layout`] 的降级链决定，
/// 模型名本身**只按总可用宽度裁剪一次**，避免字段之间互相压缩。
fn model_width(status: &ChatStatus, avail: u16) -> u16 {
    let model = status
        .model_name
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_NO_MODEL_SHORT.to_string());
    let model = cut_width(&model, usize::from(avail));
    u16::try_from(model.width()).unwrap_or(u16::MAX)
}

/// 思考等级文字（variant 原始 key）；无 variants 时 None（整段不显示）。
fn variant_text(status: &ChatStatus) -> Option<String> {
    status
        .variant
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 在窗格底边框行上覆盖绘制状态（两端角与未被覆盖的底边线保留为 `─`）。
pub(crate) fn render(buf: &mut Buffer, inner: Rect, lx: &Lx) {
    let status = &lx.chat.status;
    let lay = layout(inner, status, lx.background_tasks);
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
    // 供应商已移除；上下文占用紧跟模型名（`模型名 · 258.7k(26%)`）。
    let notice = status
        .omitted_turns
        .map(|omitted| format!("{} {omitted}", text::LX_CHAT_OMITTED_PREFIX));
    let notice_w = notice
        .as_ref()
        .map_or(0, |value| u16::try_from(value.width()).unwrap_or(u16::MAX));
    let context = context_text(status);
    // 模型名宽度直接取布局的结论（`model_width`），不在这里重算——
    // 重算必然与降级链的判断分叉，导致「布局判放不下、渲染却画出来」的错位。
    // 有省略提示时再让出它的位置，避免提示压住模型名。
    let notice_reserve = if notice_w > 0 && notice_w.saturating_add(GAP) < lay.label.width {
        notice_w.saturating_add(GAP)
    } else {
        0
    };
    let model_budget = lay
        .model_width
        .min(lay.label.width.saturating_sub(notice_reserve));
    let model = status
        .model_name
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_NO_MODEL_SHORT.to_string());
    let model = cut_width(&model, usize::from(model_budget));
    let mut x = lay.label.x;
    let limit = lay.label.right();
    if !model.is_empty() && x < limit {
        put_model(buf, x, y, &model, limit);
        x = x.saturating_add(u16::try_from(model.width()).unwrap_or(u16::MAX));
    }
    // 是否画上下文以 `layout` 的结论为唯一真相：`keep_ctx` 为假时它已被判为放不下，
    // 这里若还按剩余列硬塞，会画出半截 `· 10` 而模型名已被剪没。
    // 思考等级同理：`lay.variant.width` 为 0 表示放不下，整段不画。
    if let Some(variant) = variant_text(status).filter(|_| lay.variant.width > 0 && x < limit) {
        let value = format!("{}{variant}", text::LX_CHAT_MODEL_SEP);
        put(buf, x, y, &value, style::chat_provider(), limit);
        let variant_x = x.saturating_add(sep_w());
        put(buf, variant_x, y, &variant, style::chat_variant(), limit);
        x = variant_x.saturating_add(u16::try_from(variant.width()).unwrap_or(0));
    }
    if let Some(context) = context.filter(|_| lay.context.width > 0 && x < limit) {
        let value = format!("{}{context}", text::LX_CHAT_MODEL_SEP);
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
    // 右端：后台任务计数（`1 process`）。为 0 时整段不画，留给底边线。
    if let Some(tasks) = task_text(lx.background_tasks) {
        put(
            buf,
            lay.block.x,
            y,
            &tasks,
            style::chat_provider(),
            lay.block.right(),
        );
    }
}

/// 后台任务计数指标文字：`1 process` / `3 processes`；为 0 时 None（整段隐藏）。
fn task_text(count: usize) -> Option<String> {
    (count > 0).then(|| text::lx_task_count_label(count))
}

/// 上下文占用文字（`当前用量(百分比)`，用量用一位小数的 token 短格式）；
/// 未知（无上限或无用量）与**零消耗**返回 None（整段隐藏）。
///
/// 不再带 `context:` 前缀：紧跟模型名之后（`模型名 · 258.7k(26%)`），语义已明确。
fn context_text(status: &ChatStatus) -> Option<String> {
    let used = status.input_tokens.filter(|tokens| *tokens > 0)?;
    let limit = status.context_limit.filter(|limit| *limit > 0)?;
    // 四舍五入（`258.7k / 1M` → 26%，与展示口径一致）。
    let percent = used.saturating_mul(100).saturating_add(limit / 2) / limit;
    Some(format!(
        "{}({}%)",
        text::token_count_precise(used),
        percent.min(100)
    ))
}

/// 逐字符覆盖写入模型名：粉→蓝渐变与入口标签同款（按**可见**标签长度取样，
/// 硬切后的模型名同样跑完整渐变）。
fn put_model(buf: &mut Buffer, mut x: u16, y: u16, model: &str, limit: u16) {
    let length = model.chars().count();
    for (position, ch) in model.chars().enumerate() {
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
            cell.set_style(style::model_label_style(position, length));
        }
        x = x.saturating_add(width);
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
