//! 全局 toast 覆盖层：渲染与命中几何共用；锚点容器内顶边框下方居中，放不下时降级屏幕顶部居中。

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::config::Config;
use crate::layout::PaneId;
use crate::ui::layout::ViewLayout;
use crate::ui::widgets::clear;
use crate::ui::{style, text};

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
    let border = border_style(toast.kind);
    let block = Block::bordered().border_style(border);
    frame.render_widget(block, area);
    let has_title = toast.title.is_some() && area.height >= 5;
    if let Some(title) = toast.title.as_deref()
        && has_title
    {
        let title_max = usize::from(area.width.saturating_sub(8));
        frame.render_widget(
            Paragraph::new(Span::styled(
                text::truncate_to_width(title, title_max),
                title_style(toast.kind),
            )),
            Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(4), 1),
        );
    }
    if let Some(rect) = close_button_rect_for(area, has_title) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, border)),
            rect,
        );
    }
    let body_y = if has_title { area.y + 3 } else { area.y + 1 };
    let inner = Rect::new(
        area.x + 1,
        body_y,
        area.width.saturating_sub(2),
        area.bottom().saturating_sub(1).saturating_sub(body_y),
    );
    if inner.width == 0 || inner.height == 0 {
        return Some(area);
    }
    let width = usize::from(inner.width);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text::truncate_to_width(&toast.message, width),
            style::text(),
        )))
        .alignment(Alignment::Center),
        inner,
    );
    Some(area)
}

/// 右上角关闭按钮矩形：有标题栏时落在标题栏行（`area.y + 1`），无标题时落在顶边框（`area.y`）。
fn close_button_rect_for(area: Rect, has_title: bool) -> Option<Rect> {
    let y = if has_title { area.y + 1 } else { area.y };
    (area.width >= 10 && y < area.bottom()).then_some(Rect::new(
        area.right().saturating_sub(4),
        y,
        3,
        1,
    ))
}

/// 命中 toast 右上角关闭按钮。
pub fn close_button_at(area: Rect, column: u16, row: u16) -> bool {
    let has_title = area.height >= 5;
    close_button_rect_for(area, has_title).is_some_and(|rect| rect.contains((column, row).into()))
        || crate::ui::widgets::modal::close_button_at(area, column, row)
}

/// 边框语义色：三档独立 256 色（薰衣草紫 / 琥珀金橙 / 珊瑚玫红），不与项目主色调 Cyan 重叠。
pub(crate) fn border_style(kind: ToastKind) -> Style {
    style::toast_border(kind)
}

/// 标题语义色：与该档边框保持同色。
pub(crate) fn title_style(kind: ToastKind) -> Style {
    style::toast_border(kind)
}

/// 盒尺寸：正文最长行与边框标题各自按显示列宽加边框与内边距取宽；有标题时含顶栏框共 5 行，无标题 3 行。
fn box_size(toast: &Toast) -> (u16, u16) {
    let msg_w = toast.message.width();
    let title_w = toast
        .title
        .as_deref()
        .map(|t| t.width().saturating_add(4))
        .unwrap_or_default();
    let text = msg_w.max(title_w);
    let width = u16::try_from(text.saturating_add(4)).unwrap_or(u16::MAX);
    let height = if toast.title.is_some() { 5 } else { 3 };
    (width, height)
}

#[cfg(test)]
mod tests;
