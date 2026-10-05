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
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style(toast.kind));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = usize::from(inner.width);
    let mut lines: Vec<Line<'_>> = Vec::with_capacity(2);
    if let Some(title) = toast.title.as_deref() {
        lines.push(Line::from(Span::styled(
            text::ellipsize(title, width),
            style::strong(),
        )));
    }
    lines.push(Line::from(Span::styled(
        text::ellipsize(&toast.message, width),
        style::text(),
    )));
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
}

/// 边框语义色：普通提示用强调色，失败用红色。
fn border_style(kind: ToastKind) -> Style {
    match kind {
        ToastKind::Info => style::accent(),
        ToastKind::Error => style::error(),
    }
}

/// 盒尺寸：最长行宽 + 左右边框与内边距；有标题时多一行。
fn box_size(toast: &Toast) -> (u16, u16) {
    let cols = toast
        .title
        .as_deref()
        .into_iter()
        .chain(std::iter::once(toast.message.as_str()))
        .map(|line| line.chars().count())
        .max()
        .unwrap_or_default();
    let width = u16::try_from(cols.saturating_add(4)).unwrap_or(u16::MAX);
    let height = if toast.title.is_some() { 4 } else { 3 };
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::actions::Action;
    use crate::app::update;
    use std::time::Instant;

    fn view_for(state: &AppState) -> ViewLayout {
        crate::ui::layout::compute(
            Rect::new(0, 0, 100, 24),
            &Config::default(),
            state.sidebar_collapsed,
            state.prompt_collapsed,
            state.prompt_width,
        )
    }

    fn pane_rects(state: &AppState, view: &ViewLayout) -> Vec<(PaneId, Rect)> {
        crate::layout::pane_rects(
            &state.active_tab().layout,
            view.panes,
            Config::default().min_pane_width,
        )
    }

    fn show(state: &mut AppState, anchor: Option<PaneId>) {
        update::show_toast(
            state,
            Toast::new(ToastKind::Info, text::TOAST_COPIED, anchor, Instant::now()),
        );
    }

    #[test]
    fn anchored_toast_centers_below_container_top_border() {
        let mut state = AppState::demo();
        let prompt = state.prompt.id();
        show(&mut state, Some(prompt));
        let view = view_for(&state);
        let rects = pane_rects(&state, &view);
        let rect = rect(
            &state,
            &view,
            &rects,
            Rect::new(0, 0, 100, 24),
            &Config::default(),
        )
        .expect("toast area is resolvable");
        assert_eq!(rect.y, view.prompt.y + 1);
        assert_eq!(rect.x, view.prompt.x + (view.prompt.width - rect.width) / 2);
        assert_eq!(rect.height, 3);
        assert_eq!(
            usize::from(rect.width),
            text::TOAST_COPIED.chars().count() + 4
        );
    }

    #[test]
    fn pane_anchored_toast_targets_that_pane() {
        let mut state = AppState::demo();
        let pane = state.active_tab().layout.focus();
        show(&mut state, Some(pane));
        let view = view_for(&state);
        let rects = pane_rects(&state, &view);
        let Some((_, container)) = rects.iter().find(|(id, _)| *id == pane) else {
            panic!("active pane has a rect");
        };
        let rect = rect(
            &state,
            &view,
            &rects,
            Rect::new(0, 0, 100, 24),
            &Config::default(),
        )
        .expect("toast area is resolvable");
        assert_eq!(rect.y, container.y + 1);
        assert_eq!(rect.x, container.x + (container.width - rect.width) / 2);
    }

    #[test]
    fn narrow_container_falls_back_to_screen_top_center() {
        let mut state = AppState::demo();
        let prompt = state.prompt.id();
        update::apply(Action::TogglePrompt, &mut state);
        show(&mut state, Some(prompt));
        let view = view_for(&state);
        let rects = pane_rects(&state, &view);
        let rect = rect(
            &state,
            &view,
            &rects,
            Rect::new(0, 0, 100, 24),
            &Config::default(),
        )
        .expect("toast area is resolvable");
        assert_eq!(rect.y, 1);
        assert_eq!(rect.x, (100 - rect.width) / 2);
    }

    #[test]
    fn missing_anchor_falls_back_to_screen_top_center() {
        let mut state = AppState::demo();
        show(&mut state, None);
        let view = view_for(&state);
        let rects = pane_rects(&state, &view);
        let rect = rect(
            &state,
            &view,
            &rects,
            Rect::new(0, 0, 100, 24),
            &Config::default(),
        )
        .expect("toast area is resolvable");
        assert_eq!(rect.y, 1);
        assert_eq!(rect.x, (100 - rect.width) / 2);
    }

    #[test]
    fn below_minimum_screen_has_no_toast() {
        let mut state = AppState::demo();
        show(&mut state, None);
        let view = view_for(&state);
        let rects = pane_rects(&state, &view);
        assert!(
            rect(
                &state,
                &view,
                &rects,
                Rect::new(0, 0, 30, 8),
                &Config::default()
            )
            .is_none()
        );
    }
}
