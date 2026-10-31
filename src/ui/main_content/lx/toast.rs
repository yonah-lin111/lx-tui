//! lx 页自己的 toast：绘制在**窗格内容区内部**，不再占用全局 toast 覆盖层。
//!
//! 为什么不用全局 toast：全局槽只有一个，且锚点容器放不下时会降级到屏幕顶部居中——
//! 一条 lx 页的反馈会横跨整个窗口、指代不明（用户报的真实观感问题）。lx 的反馈只
//! 属于触发它的那个窗格，所以：
//!
//! - 状态存在 `pane.lx.toast`（每个 lx 窗格各一条，互不覆盖）；
//! - 宽度**夹在窗格内容区内**并允许折行，永不越界到别的窗格或屏幕边缘；
//! - 绘制在历史/输入框之上，但仍然只在内容区之内（不遮窗格边框与底栏）。
//!
//! 语义色、三档边框（default/warning/danger）与全局 toast 共用 `ui::toast` 的同一套
//! 样式函数，保证两处观感一致。命中几何 [`rect`] 与绘制 [`draw`] 共用同一份计算。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::toast::Toast;
use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// toast 盒与窗格内容区左右边缘之间的留白（列）。
const MARGIN: u16 = 1;
/// 盒内正文相对左边框的起始偏移（左边框 1 列 + 左侧留白 1 列 + `● ` 2 列 = 4 列）。
const TEXT_OFFSET_X: u16 = 4;
/// 盒宽相对正文的固定开销：左右边框 2 列 + 左右留白 2 列 + `● ` 前缀 2 列 = 6 列。
const FRAME: u16 = 6;
/// 盒的最小宽度（含边框）：再窄连一个字符都放不下有意义的内容。
const MIN_WIDTH: u16 = 8;
/// 正文最多占用的行数：超出的部分截断并加省略号，避免长错误信息吃掉整屏历史。
pub(crate) const MAX_BODY_ROWS: u16 = 4;
/// 盒高度下限（边框 + 至少一行正文）。
const MIN_HEIGHT: u16 = 3;

/// toast 盒矩形（绝对屏幕坐标，已夹在 `area` 内）；无 toast 或空间不足返回 None。
///
/// 水平居中、贴在内容区顶边下方一行（与全局 toast 的容器内位置一致）；宽度按正文
/// 与标题的显示列收缩，但**不超过内容区宽度**，高度按折行结果增长（上限
/// [`MAX_BODY_ROWS`] + 边框，有标题时额外加 1 行标题行，无 `├───┤` 分割线）。
pub(crate) fn rect(area: Rect, toast: &Toast) -> Option<Rect> {
    let header_extra: u16 = u16::from(toast.title.is_some());
    let min_h = MIN_HEIGHT.saturating_add(header_extra);
    if area.width < MIN_WIDTH || area.height < min_h {
        return None;
    }
    let max_width = area.width.saturating_sub(MARGIN.saturating_mul(2));
    if max_width < MIN_WIDTH {
        return None;
    }
    let title_w = toast
        .title
        .as_deref()
        .map(|line| u16::try_from(line.width()).unwrap_or(u16::MAX))
        .unwrap_or_default();
    let msg_w = u16::try_from(toast.message.width()).unwrap_or(u16::MAX);
    let wanted = msg_w.max(title_w).saturating_add(FRAME);
    let width = wanted.clamp(MIN_WIDTH, max_width);
    let body_width = usize::from(width.saturating_sub(FRAME)).max(1);
    let rows = text::wrap(&toast.message, body_width).len();
    let rows = u16::try_from(rows).unwrap_or(u16::MAX).min(MAX_BODY_ROWS);
    let height = rows
        .saturating_add(2)
        .saturating_add(header_extra)
        .min(area.height);
    if height < min_h {
        return None;
    }
    let y = area
        .y
        .saturating_add(1)
        .min(area.bottom().saturating_sub(height))
        .max(area.y);
    Some(Rect::new(
        area.x + (area.width - width) / 2,
        y,
        width,
        height,
    ))
}

/// 绘制 toast 盒；返回绘制出的矩形（供调用方隐藏被覆盖的硬件光标）。
///
/// 先清底擦除底层字符（重置为终端默认透明背景），
/// 再复用 [`crate::ui::toast::draw_frame`] 画边框、语义色圆点 `●` 与可选标题行，
/// 最后按窗格宽度折行绘制正文（超出上限时末行加省略号）。
pub(crate) fn draw(buf: &mut Buffer, area: Rect, toast: &Toast) -> Option<Rect> {
    let box_area = rect(area, toast)?;
    clear::render_buffer(buf, box_area);
    let has_title = crate::ui::toast::draw_frame(buf, box_area, toast);
    let body_top = if has_title {
        box_area.y.saturating_add(2)
    } else {
        box_area.y.saturating_add(1)
    };
    let body_height = box_area.bottom().saturating_sub(1).saturating_sub(body_top);
    let text_width = usize::from(box_area.width.saturating_sub(FRAME)).max(1);
    if box_area.width <= TEXT_OFFSET_X || body_height == 0 {
        return Some(box_area);
    }
    let text_x = box_area.x.saturating_add(TEXT_OFFSET_X);
    let lines = text::wrap(&toast.message, text_width);
    let visible = usize::from(body_height);
    let overflow = lines.len() > visible;
    let body_style = if has_title {
        style::muted()
    } else {
        style::text()
    };
    for (offset, line) in lines.iter().take(visible).enumerate() {
        let y = body_top + u16::try_from(offset).unwrap_or(u16::MAX);
        let line = if overflow && offset + 1 == visible {
            let mut truncated = text::truncate_to_width(line, text_width.saturating_sub(1));
            if !truncated.ends_with('…') {
                truncated.push('…');
            }
            truncated
        } else {
            line.clone()
        };
        buf.set_stringn(text_x, y, &line, text_width, body_style);
    }
    Some(box_area)
}

#[cfg(test)]
mod tests;
