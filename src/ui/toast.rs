//! 全局 toast 覆盖层：渲染与命中几何共用；锚点容器内顶边框下方居中，放不下时降级屏幕顶部居中。
//!
//! 布局对齐 herdr `render_toast_notification` / `render_copy_feedback`：
//! - 无标题（单行）：3 行高，`│ ● <message> │`（圆点取语义色、正文加粗）；
//! - 有标题（双行）：4 行高，首行 `│ ● <title> │`（圆点语义色、标题加粗），次行 `│   <message> │`（缩进 2 列与标题对齐、次要色）；
//! - 无 `├───┤` 分割线与 `[✕]` 关闭按钮，点击 toast 本身或外部即关闭。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Widget};
use unicode_width::UnicodeWidthStr;

use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::config::Config;
use crate::layout::PaneId;
use crate::ui::layout::ViewLayout;
use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// 盒宽相对文本内容的固定开销：左右边框 2 列 + 左右各 1 列留白 + 行首 `● ` 2 列 = 6 列。
pub(crate) const FRAME_WIDTH: usize = 6;

/// toast 覆盖层矩形；渲染与鼠标命中共用，保证所见即所点。
///
/// 锚点容器放得下时贴在容器顶边框下方并水平居中；未传锚点、容器缺失或放不下时降级到屏幕顶部居中。
pub fn rect(
    state: &AppState,
    view: &ViewLayout,
    pane_rects: &[(PaneId, Rect)],
    screen: Rect,
    config: &Config,
) -> Option<Rect> {
    let toast = state.toast.as_ref()?;
    if screen.width < config.min_width || screen.height < config.min_height {
        return None;
    }
    let (width, height) = box_size(toast);
    let width = width.min(screen.width);
    let container = match toast.anchor {
        Some(id) if id == state.prompt.id() => Some(view.prompt),
        Some(id) => pane_rects
            .iter()
            .find(|(pane, _)| *pane == id)
            .map(|(_, rect)| *rect),
        None => None,
    };
    if let Some(container) = container
        && width <= container.width.saturating_sub(2)
        && height <= container.height.saturating_sub(1)
    {
        return Some(Rect::new(
            container.x + (container.width - width) / 2,
            container.y + 1,
            width,
            height,
        ));
    }
    Some(Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + 1,
        width,
        height,
    ))
}

/// 绘制 toast 外框、首行语义色圆点 `●` 与可选标题行；返回是否存在独立标题行。
///
/// 全局 toast 与 lx 窗格内 toast 共用这一份绘制，保证两者与 herdr toast 完全同款。
pub(crate) fn draw_frame(buf: &mut Buffer, area: Rect, toast: &Toast) -> bool {
    let border = border_style(toast.kind);
    Block::bordered().border_style(border).render(area, buf);
    if area.width >= 5 && area.height >= 3 {
        buf.set_stringn(
            area.x.saturating_add(2),
            area.y.saturating_add(1),
            text::AGENT_STATUS_DOT,
            1,
            border,
        );
    }
    let has_title = toast.title.is_some() && area.height >= 4;
    if let Some(title) = toast.title.as_deref()
        && has_title
    {
        let max = usize::from(area.width).saturating_sub(FRAME_WIDTH);
        if max > 0 {
            let fitted = text::truncate_to_width(title, max);
            buf.set_stringn(
                area.x.saturating_add(4),
                area.y.saturating_add(1),
                &fitted,
                max,
                title_style(toast.kind),
            );
        }
    }
    has_title
}

/// 绘制 toast；必须晚于全部下层内容。
/// 返回绘制出的盒矩形，供调用方隐藏被其覆盖的硬件光标。
pub fn render(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    view: &ViewLayout,
    pane_rects: &[(PaneId, Rect)],
    config: &Config,
) -> Option<Rect> {
    let area = rect(state, view, pane_rects, screen, config)?;
    let toast = state.toast.as_ref()?;
    clear::render(frame, area);
    let buf = frame.buffer_mut();
    let has_title = draw_frame(buf, area, toast);
    let body_y = if has_title { area.y + 2 } else { area.y + 1 };
    let max_width = usize::from(area.width).saturating_sub(FRAME_WIDTH);
    if max_width == 0 || body_y >= area.bottom().saturating_sub(1) {
        return Some(area);
    }
    let body_style = if has_title {
        style::muted()
    } else {
        style::strong()
    };
    let fitted = text::truncate_to_width(&toast.message, max_width);
    buf.set_stringn(
        area.x.saturating_add(4),
        body_y,
        &fitted,
        max_width,
        body_style,
    );
    Some(area)
}

/// 边框与首行状态圆点语义色（Cyan / Yellow / Red）。
pub(crate) fn border_style(kind: ToastKind) -> Style {
    style::toast_border(kind)
}

/// 标题样式：与边框同色系或强调显示。
pub(crate) fn title_style(kind: ToastKind) -> Style {
    style::toast_title(kind)
}

/// 盒尺寸：按标题与正文显示宽度加 `FRAME_WIDTH`（6 列）取宽；无标题 3 行，有标题 4 行。
fn box_size(toast: &Toast) -> (u16, u16) {
    let msg_w = toast.message.width();
    let title_w = toast.title.as_deref().map(str::width).unwrap_or_default();
    let text_w = msg_w.max(title_w);
    let width = u16::try_from(text_w.saturating_add(FRAME_WIDTH)).unwrap_or(u16::MAX);
    let height = if toast.title.is_some() { 4 } else { 3 };
    (width, height)
}

#[cfg(test)]
mod tests;
