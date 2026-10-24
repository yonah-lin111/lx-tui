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

use crate::app::toast::{Toast, ToastKind};
use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// toast 盒与窗格内容区左右边缘之间的留白（列）。
const MARGIN: u16 = 1;
/// 盒内正文左侧额外留白（列）。
const PADDING: u16 = 1;
/// 盒宽相对正文的固定开销：左右边框 2 列 + 左右内边距 2 列。
const FRAME: u16 = 4;
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
/// [`MAX_BODY_ROWS`] + 边框）。
pub(crate) fn rect(area: Rect, toast: &Toast) -> Option<Rect> {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return None;
    }
    let max_width = area.width.saturating_sub(MARGIN.saturating_mul(2));
    if max_width < MIN_WIDTH {
        return None;
    }
    let wanted = toast
        .title
        .as_deref()
        .into_iter()
        .chain(std::iter::once(toast.message.as_str()))
        .map(|line| u16::try_from(line.width()).unwrap_or(u16::MAX))
        .max()
        .unwrap_or_default()
        .saturating_add(FRAME);
    let width = wanted.clamp(MIN_WIDTH, max_width);
    // 正文宽度必须与 `draw` 完全一致：盒内去掉左右边框与内边距。
    let body_width = usize::from(width.saturating_sub(FRAME)).max(1);
    let rows = text::wrap(&toast.message, body_width).len();
    let rows = u16::try_from(rows).unwrap_or(u16::MAX).min(MAX_BODY_ROWS);
    let height = rows.saturating_add(2).min(area.height);
    if height < MIN_HEIGHT {
        return None;
    }
    // 贴顶边下方一行；空间不足时向上夹到内容区底边之内，绝不压窗格边框。
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
/// 先清底再写边框，整块不透明：lx 页的历史正文、代码块底色不会被透出来。
pub(crate) fn draw(buf: &mut Buffer, area: Rect, toast: &Toast) -> Option<Rect> {
    let box_area = rect(area, toast)?;
    clear::render_buffer(buf, box_area);
    // 清底后整块铺浮层底色（与全局 toast 同一个 `overlay_panel`）。
    let base = style::overlay_panel();
    for y in box_area.top()..box_area.bottom() {
        for x in box_area.left()..box_area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_style(base);
            }
        }
    }
    draw_border(buf, box_area, toast.kind);
    if let Some(title) = toast.title.as_deref() {
        draw_title(buf, box_area, title, toast.kind);
    }
    // 右上角关闭按钮：与全局 toast 同款 `[✕]`，点击命中走 `close_button_at`。
    if let Some(button) = crate::ui::widgets::modal::close_button_rect(box_area) {
        let style = style::accent();
        for (index, ch) in text::MODAL_CLOSE.chars().enumerate() {
            let x = button
                .x
                .saturating_add(u16::try_from(index).unwrap_or(u16::MAX));
            put(buf, x, button.y, ch, style);
        }
    }
    let body_area = Rect::new(
        box_area.x + 1,
        box_area.y + 1,
        box_area.width.saturating_sub(2),
        box_area.height.saturating_sub(2),
    );
    if body_area.width == 0 || body_area.height == 0 {
        return Some(box_area);
    }
    let text_x = body_area.x.saturating_add(PADDING);
    let text_width = usize::from(body_area.width.saturating_sub(PADDING.saturating_mul(2))).max(1);
    let lines = text::wrap(&toast.message, text_width);
    let visible = usize::from(body_area.height);
    // 超出正文行数上限：最后一行加省略号，明确告知还有内容，而不是静默切掉。
    let overflow = lines.len() > visible;
    for (offset, line) in lines.iter().take(visible).enumerate() {
        let y = body_area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        let line = if overflow && offset + 1 == visible {
            // 折行后的单行通常正好占满宽度，`ellipsize` 不会加省略号；
            // 这里强制留一列给 `…`。
            let mut truncated: String = line.chars().take(text_width.saturating_sub(1)).collect();
            truncated.push('…');
            truncated
        } else {
            line.clone()
        };
        for (index, ch) in line.chars().enumerate() {
            let x = text_x.saturating_add(u16::try_from(index).unwrap_or(u16::MAX));
            if x >= body_area.right() {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(ch);
                cell.set_style(style::text());
            }
        }
    }
    Some(box_area)
}

/// 圆角边框（只画四边与四角；不依赖 `Frame`，直接写 Buffer）。
fn draw_border(buf: &mut Buffer, area: Rect, kind: ToastKind) {
    let border = border_style(kind);
    let right = area.right().saturating_sub(1);
    let bottom = area.bottom().saturating_sub(1);
    for x in area.x..=right {
        put(buf, x, area.y, '─', border);
        put(buf, x, bottom, '─', border);
    }
    for y in area.y..=bottom {
        put(buf, area.x, y, '│', border);
        put(buf, right, y, '│', border);
    }
    put(buf, area.x, area.y, '╭', border);
    put(buf, right, area.y, '╮', border);
    put(buf, area.x, bottom, '╰', border);
    put(buf, right, bottom, '╯', border);
}

/// 顶边标题：` <title> `；宽度不足时省略，不侵占右侧关闭按钮。
fn draw_title(buf: &mut Buffer, area: Rect, title: &str, kind: ToastKind) {
    let reserved = crate::ui::widgets::modal::close_button_rect(area)
        .map(|_| {
            u16::try_from(text::MODAL_CLOSE.width())
                .unwrap_or(0)
                .saturating_add(2)
        })
        .unwrap_or_default();
    let available = area.width.saturating_sub(2).saturating_sub(reserved);
    if available < 4 {
        return;
    }
    let label = format!(
        " {} ",
        text::ellipsize(title, usize::from(available.saturating_sub(2)))
    );
    let style = title_style(kind);
    for (index, ch) in label.chars().enumerate() {
        let x = area
            .x
            .saturating_add(1)
            .saturating_add(u16::try_from(index).unwrap_or(u16::MAX));
        if x >= area.right().saturating_sub(reserved) {
            break;
        }
        put(buf, x, area.y, ch, style);
    }
}

/// 命中右上角关闭按钮（`[✕]` 与全局 toast 同款、同位置）。
pub(crate) fn close_button_at(area: Rect, column: u16, row: u16) -> bool {
    crate::ui::widgets::modal::close_button_at(area, column, row)
}

/// 写一个单元格（越界忽略）。
fn put(buf: &mut Buffer, x: u16, y: u16, ch: char, style: ratatui::style::Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_char(ch);
        cell.set_style(style);
    }
}

/// 边框语义色：与全局 toast 完全一致（默认强调色 / 警告黄 / 危险红）。
fn border_style(kind: ToastKind) -> ratatui::style::Style {
    crate::ui::toast::border_style(kind)
}

/// 标题语义色：与边框同色规则一致。
fn title_style(kind: ToastKind) -> ratatui::style::Style {
    crate::ui::toast::title_style(kind)
}

#[cfg(test)]
mod tests;
