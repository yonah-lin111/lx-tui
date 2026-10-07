//! 全局 toast 覆盖层：渲染与命中几何共用；锚点容器内顶边框下方居中，放不下时降级屏幕顶部居中。

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::config::Config;
use crate::layout::PaneId;
use crate::ui::layout::ViewLayout;
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
pub fn render(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    view: &ViewLayout,
    pane_rects: &[(PaneId, Rect)],
    config: &Config,
) {
    let Some(area) = rect(state, view, pane_rects, screen, config) else {
        return;
    };
    let Some(toast) = state.toast.as_ref() else {
        return;
    };
    frame.render_widget(Clear, area);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style(toast.kind))
        .style(style::overlay_panel());
    if let Some(title) = toast.title.as_deref() {
        block = block.title(Span::styled(
            format!(
                " {} ",
                text::ellipsize(title, usize::from(area.width.saturating_sub(2)))
            ),
            style::muted(),
        ));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = usize::from(inner.width);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text::ellipsize(&toast.message, width),
            style::text(),
        )))
        .alignment(Alignment::Center),
        inner,
    );
}

/// 边框语义色：普通提示用强调色，失败用红色。
fn border_style(kind: ToastKind) -> Style {
    match kind {
        ToastKind::Info => style::accent(),
        ToastKind::Error => style::error(),
    }
}

/// 盒尺寸：正文最长行与边框标题各自加边框与内边距取宽；高度固定三行。
fn box_size(toast: &Toast) -> (u16, u16) {
    let text = toast
        .title
        .as_deref()
        .into_iter()
        .chain(std::iter::once(toast.message.as_str()))
        .map(|line| line.chars().count())
        .max()
        .unwrap_or_default();
    let width = u16::try_from(text.saturating_add(4)).unwrap_or(u16::MAX);
    (width, 3)
}

#[cfg(test)]
mod tests;
