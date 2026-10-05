//! 渲染层：只读取状态，禁止修改状态或执行 IO。

pub mod layout;
pub mod style;
pub mod terminal;
pub mod text;
pub mod toast;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, Paragraph, Wrap};

use crate::app::state::{AppState, Pane, PaneKind};
use crate::config::Config;
use crate::layout::{COLLAPSED_STRIP, PaneId};

/// 展开态折叠按钮：标签宽度与距面板右缘的留白。
const PANEL_BUTTON_WIDTH: u16 = 3;
const PANEL_BUTTON_MARGIN: u16 = 1;

/// 渲染整个界面。
pub fn render(frame: &mut Frame<'_>, state: &AppState, config: &Config) {
    let area = frame.area();
    if area.width < config.min_width || area.height < config.min_height {
        render_min_size_notice(frame, area, config);
        return;
    }

    let view = layout::compute(
        area,
        config,
        state.sidebar_collapsed,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let pane_rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
    );

    if view.sidebar.width > 0 {
        if state.sidebar_collapsed {
            render_collapsed_strip(frame, view.sidebar, true);
        } else {
            render_sidebar(frame, view.sidebar, state);
        }
    }
    render_tab_bar(frame, view.tab_bar, state);
    render_panes(frame, &pane_rects, state);
    render_prompt(frame, view.prompt, state);
    render_collapse_buttons(frame, &view);
    render_resize_hint(frame, &view, state);
    toast::render(frame, area, state, &view, &pane_rects, config);
}

/// 折叠面板的按钮目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseTarget {
    Sidebar,
    Prompt,
}

/// 折叠按钮：目标、命中矩形与当前折叠态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollapseButton {
    pub target: CollapseTarget,
    pub area: Rect,
    pub collapsed: bool,
}

/// 左栏与右栏顶边内的折叠按钮；渲染与鼠标命中共用同一几何。
pub fn collapse_buttons(view: &layout::ViewLayout) -> Vec<CollapseButton> {
    let mut buttons = Vec::new();
    if let Some(button) = panel_button(CollapseTarget::Sidebar, view.sidebar) {
        buttons.push(button);
    }
    if let Some(button) = panel_button(CollapseTarget::Prompt, view.prompt) {
        buttons.push(button);
    }
    buttons
}

/// 命中测试：返回坐标所在折叠按钮的目标。
pub fn collapse_button_at(
    view: &layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<CollapseTarget> {
    collapse_buttons(view)
        .into_iter()
        .find(|button| button.area.contains((column, row).into()))
        .map(|button| button.target)
}

/// 单个面板的折叠按钮：折叠态取整条窄条，展开态在顶边右端。
fn panel_button(target: CollapseTarget, panel: Rect) -> Option<CollapseButton> {
    if panel.width == 0 || panel.height == 0 {
        return None;
    }
    if panel.width == COLLAPSED_STRIP {
        Some(CollapseButton {
            target,
            area: Rect::new(panel.x, panel.y, COLLAPSED_STRIP, 1),
            collapsed: true,
        })
    } else if panel.width > COLLAPSED_STRIP {
        Some(CollapseButton {
            target,
            area: Rect::new(
                panel.right() - PANEL_BUTTON_WIDTH - PANEL_BUTTON_MARGIN,
                panel.y,
                PANEL_BUTTON_WIDTH,
                1,
            ),
            collapsed: false,
        })
    } else {
        None
    }
}

/// 在面板顶边绘制折叠按钮，必须晚于面板内容渲染。
///
/// 折叠态标签恰好占满 3 个内容列（水平居中），展开态贴面板右端。
fn render_collapse_buttons(frame: &mut Frame<'_>, view: &layout::ViewLayout) {
    for button in collapse_buttons(view) {
        let label = match (button.target, button.collapsed) {
            (CollapseTarget::Sidebar, false) => text::SIDEBAR_COLLAPSE_LABEL,
            (CollapseTarget::Sidebar, true) => text::SIDEBAR_EXPAND_LABEL,
            (CollapseTarget::Prompt, false) => text::PROMPT_COLLAPSE_LABEL,
            (CollapseTarget::Prompt, true) => text::PROMPT_EXPAND_LABEL,
        };
        let start = if button.collapsed {
            collapsed_content_x(button.area, button.target == CollapseTarget::Sidebar)
        } else {
            button.area.x
        };
        for (offset, symbol) in label.chars().enumerate() {
            let x = start + offset as u16;
            if let Some(cell) = frame.buffer_mut().cell_mut((x, button.area.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(style::accent());
            }
        }
    }
}

/// 折叠窄条：分隔线贴主区一侧，内容区为空，仅由折叠按钮绘制居中图标。
fn render_collapsed_strip(frame: &mut Frame<'_>, area: Rect, separator_right: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let x = if separator_right {
        area.right() - 1
    } else {
        area.x
    };
    let buf = frame.buffer_mut();
    for y in area.y..area.bottom() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(text::STRIP_LINE);
            cell.set_style(style::muted());
        }
    }
}

/// 折叠条内容区起始列：扣除贴主区的分隔线，标签恰好占满内容区（水平居中）。
fn collapsed_content_x(area: Rect, separator_right: bool) -> u16 {
    if separator_right {
        area.x
    } else {
        area.x.saturating_add(1)
    }
}

/// 右栏分割线提示：悬停或拖拽时把两列边框改为强调色。
fn render_resize_hint(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
    if state.prompt_collapsed
        || !(state.resizing_prompt || state.prompt_hover)
        || view.prompt.x == 0
    {
        return;
    }
    let area = frame.area();
    let buf = frame.buffer_mut();
    for column in [view.prompt.x.saturating_sub(1), view.prompt.x] {
        for row in area.y..area.bottom() {
            if let Some(cell) = buf.cell_mut((column, row))
                && cell.symbol() == text::STRIP_LINE
            {
                cell.set_style(style::accent());
            }
        }
    }
}

/// 侧栏：工作区一级导航。
fn render_sidebar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let items: Vec<ListItem<'_>> = state
        .workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| {
            let style = if index == state.active_workspace {
                style::accent()
            } else {
                style::text()
            };
            ListItem::new(Line::from(Span::styled(workspace.name.as_str(), style)))
        })
        .collect();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(false))
        .title(Span::styled(
            format!(" {} ", text::SIDEBAR_TITLE),
            style::muted(),
        ));
    frame.render_widget(List::new(items).block(block), area);
}

/// 标签栏：当前工作区的标签切换。
fn render_tab_bar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let workspace = state.active_workspace();
    let mut spans: Vec<Span<'_>> = Vec::with_capacity(workspace.tabs.len() * 2);
    for (index, tab) in workspace.tabs.iter().enumerate() {
        let style = if index == workspace.active_tab {
            style::accent()
        } else {
            style::muted()
        };
        spans.push(Span::styled(format!(" {} ", tab.title), style));
        spans.push(Span::styled("│", style::muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// 主区域：BSP 平铺窗格；矩形由调用方按当前几何计算，与命中测试共用。
fn render_panes(frame: &mut Frame<'_>, pane_rects: &[(PaneId, Rect)], state: &AppState) {
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    for (id, rect) in pane_rects {
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let Some(pane) = tab.pane(*id) else {
            continue;
        };
        let focused = *id == focus;
        let title_style = if focused {
            style::accent()
        } else {
            style::muted()
        };
        let mut title = pane_display_title(*id, pane);
        if pane.exited {
            title.push_str(" (exited)");
        }
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style::border(focused))
            .title(Span::styled(format!(" {title} "), title_style));
        let inner = block.inner(*rect);
        frame.render_widget(block, *rect);
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        match pane.kind {
            PaneKind::Terminal | PaneKind::Prompt => {
                terminal::render(
                    inner,
                    frame.buffer_mut(),
                    &pane.terminal,
                    focused,
                    state.selection_for(*id),
                );
            }
            PaneKind::Placeholder => {}
        }
    }
}

/// 右栏 prompt 面板：全局固定区域，内容可选择复制；折叠时渲染为窄条。
fn render_prompt(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if state.prompt_collapsed {
        render_collapsed_strip(frame, area, false);
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(false))
        .title(Span::styled(
            format!(" {} ", text::PROMPT_TITLE),
            style::muted(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    terminal::render(
        inner,
        frame.buffer_mut(),
        &state.prompt.pane().terminal,
        false,
        state.selection_for(state.prompt.id()),
    );
}

/// 窗格标题：prompt 固定标题，空占位与终端走通用标题规则。
fn pane_display_title(id: PaneId, pane: &Pane) -> String {
    match pane.kind {
        PaneKind::Prompt => text::PROMPT_TITLE.to_string(),
        PaneKind::Terminal | PaneKind::Placeholder => text::pane_title(id, pane.terminal.title()),
    }
}

/// 终端尺寸不足时的提示。
fn render_min_size_notice(frame: &mut Frame<'_>, area: Rect, config: &Config) {
    let message = format!(
        "{} ({}x{})",
        text::MIN_SIZE_HINT,
        config.min_width,
        config.min_height
    );
    let line = Rect {
        y: area.y + area.height / 2,
        height: 1.min(area.height),
        ..area
    };
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        line,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::actions::Action;
    use crate::app::toast::{TOAST_DURATION, Toast, ToastKind};
    use crate::app::update;
    use ratatui::Terminal as RatatuiTerminal;
    use ratatui::backend::TestBackend;
    use std::time::Instant;

    fn render_lines(state: &AppState) -> Vec<String> {
        let config = Config::default();
        let mut terminal =
            RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
        if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
            panic!("draw failed: {error}");
        }
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    fn view_for(state: &AppState) -> layout::ViewLayout {
        layout::compute(
            Rect::new(0, 0, 100, 24),
            &Config::default(),
            state.sidebar_collapsed,
            state.prompt_collapsed,
            state.prompt_width,
        )
    }

    fn button_for(view: &layout::ViewLayout, target: CollapseTarget) -> CollapseButton {
        collapse_buttons(view)
            .into_iter()
            .find(|button| button.target == target)
            .expect("panel button is visible")
    }

    fn rendered_label(lines: &[String], button: &CollapseButton) -> String {
        let row: Vec<char> = lines[button.area.y as usize].chars().collect();
        (button.area.x..button.area.right())
            .map(|x| row[x as usize])
            .collect()
    }

    fn rendered_collapsed_label(
        lines: &[String],
        view: &layout::ViewLayout,
        target: CollapseTarget,
    ) -> String {
        let button = button_for(view, target);
        assert!(button.collapsed);
        let separator_right = target == CollapseTarget::Sidebar;
        let start = collapsed_content_x(button.area, separator_right);
        let end = if separator_right {
            button.area.right() - 1
        } else {
            button.area.right()
        };
        let row: Vec<char> = lines[button.area.y as usize].chars().collect();
        (start..end).map(|x| row[x as usize]).collect()
    }

    #[test]
    fn collapse_buttons_render_directional_arrow_labels() {
        let mut state = AppState::demo();
        let view = view_for(&state);
        let lines = render_lines(&state);
        assert_eq!(
            rendered_label(&lines, &button_for(&view, CollapseTarget::Sidebar)),
            text::SIDEBAR_COLLAPSE_LABEL
        );
        assert_eq!(
            rendered_label(&lines, &button_for(&view, CollapseTarget::Prompt)),
            text::PROMPT_COLLAPSE_LABEL
        );

        update::apply(Action::ToggleSidebar, &mut state);
        let view = view_for(&state);
        let lines = render_lines(&state);
        assert_eq!(
            rendered_collapsed_label(&lines, &view, CollapseTarget::Sidebar),
            text::SIDEBAR_EXPAND_LABEL
        );
        assert_eq!(
            rendered_label(&lines, &button_for(&view, CollapseTarget::Prompt)),
            text::PROMPT_COLLAPSE_LABEL
        );

        update::apply(Action::TogglePrompt, &mut state);
        let view = view_for(&state);
        let lines = render_lines(&state);
        assert_eq!(
            rendered_collapsed_label(&lines, &view, CollapseTarget::Sidebar),
            text::SIDEBAR_EXPAND_LABEL
        );
        assert_eq!(
            rendered_collapsed_label(&lines, &view, CollapseTarget::Prompt),
            text::PROMPT_EXPAND_LABEL
        );
    }

    #[test]
    fn prompt_sidebar_renders_content_and_collapse_button() {
        let state = AppState::demo();
        let lines = render_lines(&state);
        assert!(lines.iter().any(|line| line.contains("Drag to select")));
        assert!(lines.iter().any(|line| line.contains(text::PROMPT_TITLE)));
        assert!(
            lines
                .iter()
                .any(|line| line.contains(text::SIDEBAR_COLLAPSE_LABEL))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains(text::PROMPT_COLLAPSE_LABEL))
        );
    }

    #[test]
    fn collapsed_prompt_renders_strip_and_expand_icon() {
        let mut state = AppState::demo();
        update::apply(Action::TogglePrompt, &mut state);
        let view = view_for(&state);
        let lines = render_lines(&state);
        assert_eq!(
            rendered_collapsed_label(&lines, &view, CollapseTarget::Prompt),
            text::PROMPT_EXPAND_LABEL
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains(text::PROMPT_COLLAPSE_LABEL))
        );
        assert!(!lines.iter().any(|line| line.contains("Drag to select")));
    }

    #[test]
    fn sidebar_collapse_button_renders_icons() {
        let mut state = AppState::demo();
        let lines = render_lines(&state);
        assert!(
            lines
                .iter()
                .any(|line| line.contains(text::SIDEBAR_COLLAPSE_LABEL))
        );
        update::apply(Action::ToggleSidebar, &mut state);
        let view = view_for(&state);
        let lines = render_lines(&state);
        assert_eq!(
            rendered_collapsed_label(&lines, &view, CollapseTarget::Sidebar),
            text::SIDEBAR_EXPAND_LABEL
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains(text::SIDEBAR_COLLAPSE_LABEL))
        );
    }

    #[test]
    fn collapsed_labels_fill_content_area_with_continuous_separator() {
        let mut state = AppState::demo();
        update::apply(Action::ToggleSidebar, &mut state);
        update::apply(Action::TogglePrompt, &mut state);
        let view = view_for(&state);
        let lines = render_lines(&state);

        let sidebar_row: Vec<char> = lines[view.sidebar.y as usize].chars().collect();
        let sidebar_start = collapsed_content_x(view.sidebar, true);
        assert_eq!(sidebar_start, view.sidebar.x);
        let sidebar_label: String = (sidebar_start..view.sidebar.right() - 1)
            .map(|x| sidebar_row[x as usize])
            .collect();
        assert_eq!(sidebar_label, text::SIDEBAR_EXPAND_LABEL);
        assert_eq!(
            sidebar_row[(view.sidebar.right() - 1) as usize].to_string(),
            text::STRIP_LINE
        );

        let prompt_row: Vec<char> = lines[view.prompt.y as usize].chars().collect();
        let prompt_start = collapsed_content_x(view.prompt, false);
        assert_eq!(prompt_start, view.prompt.x + 1);
        let prompt_label: String = (prompt_start..view.prompt.right())
            .map(|x| prompt_row[x as usize])
            .collect();
        assert_eq!(prompt_label, text::PROMPT_EXPAND_LABEL);
        assert_eq!(
            prompt_row[view.prompt.x as usize].to_string(),
            text::STRIP_LINE
        );
    }

    #[test]
    fn collapsed_strips_hide_panel_content() {
        let mut state = AppState::demo();
        update::apply(Action::ToggleSidebar, &mut state);
        update::apply(Action::TogglePrompt, &mut state);
        let view = view_for(&state);
        let lines = render_lines(&state);

        let strip = |y: usize| -> String {
            lines[y]
                .chars()
                .skip(view.sidebar.x as usize)
                .take(view.sidebar.width as usize)
                .collect()
        };
        assert_eq!(strip(1), "   │");
        assert_eq!(strip(10), "   │");

        let prompt = |y: usize| -> String {
            lines[y]
                .chars()
                .skip(view.prompt.x as usize)
                .take(view.prompt.width as usize)
                .collect()
        };
        assert_eq!(prompt(1), "│   ");
        assert_eq!(prompt(10), "│   ");
    }

    #[test]
    fn collapse_buttons_hit_their_targets() {
        let mut state = AppState::demo();
        let view = view_for(&state);
        let sidebar = collapse_buttons(&view)
            .into_iter()
            .find(|button| button.target == CollapseTarget::Sidebar);
        assert!(sidebar.is_some());
        let Some(sidebar) = sidebar else { return };
        assert!(!sidebar.collapsed);
        assert_eq!(
            collapse_button_at(&view, sidebar.area.x, sidebar.area.y),
            Some(CollapseTarget::Sidebar)
        );
        let prompt = collapse_buttons(&view)
            .into_iter()
            .find(|button| button.target == CollapseTarget::Prompt);
        assert!(prompt.is_some());
        let Some(prompt) = prompt else { return };
        assert_eq!(
            collapse_button_at(&view, prompt.area.x, prompt.area.y),
            Some(CollapseTarget::Prompt)
        );

        update::apply(Action::TogglePrompt, &mut state);
        let view = view_for(&state);
        let collapsed = collapse_buttons(&view)
            .into_iter()
            .find(|button| button.target == CollapseTarget::Prompt);
        assert!(collapsed.is_some_and(|button| button.collapsed));
    }

    #[test]
    fn collapse_button_cells_use_pure_accent_style() {
        let state = AppState::demo();
        let view = view_for(&state);
        let mut terminal =
            RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
        if let Err(error) = terminal.draw(|frame| render(frame, &state, &Config::default())) {
            panic!("draw failed: {error}");
        }
        let buffer = terminal.backend().buffer().clone();
        for button in collapse_buttons(&view) {
            for x in button.area.x..button.area.right() {
                let cell = &buffer[(x, button.area.y)];
                assert_eq!(cell.fg, ratatui::style::Color::Cyan);
                assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
                assert!(!cell.modifier.contains(ratatui::style::Modifier::DIM));
            }
        }
    }

    #[test]
    fn resize_hint_highlights_divider_on_hover() {
        let mut state = AppState::demo();
        let config = Config::default();
        let view = view_for(&state);
        let mut terminal =
            RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");

        if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
            panic!("draw failed: {error}");
        }
        let plain = terminal.backend().buffer().clone();
        let left = &plain[(view.prompt.x, 10)];
        assert_ne!(left.fg, ratatui::style::Color::Cyan);

        state.prompt_hover = true;
        if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
            panic!("draw failed: {error}");
        }
        let hovered = terminal.backend().buffer().clone();
        for column in [view.prompt.x - 1, view.prompt.x] {
            let cell = &hovered[(column, 10)];
            assert_eq!(cell.symbol(), text::STRIP_LINE);
            assert_eq!(cell.fg, ratatui::style::Color::Cyan);
        }
    }

    #[test]
    fn placeholder_pane_renders_title_without_content() {
        let state = AppState::demo();
        let focus = state.active_tab().layout.focus();
        let expected = format!("pane {}", focus.raw());
        let lines = render_lines(&state);
        assert!(lines.iter().any(|line| line.contains(&expected)));
        assert!(!lines.iter().any(|line| line.contains("exited")));
    }

    #[test]
    fn status_bar_is_removed() {
        let state = AppState::demo();
        let lines = render_lines(&state);
        assert!(!lines.iter().any(|line| line.contains("lx-tui")));
        let last = lines.last().map(String::as_str).unwrap_or_default();
        assert!(last.contains('╰'), "last row should be pane border");
    }

    #[test]
    fn toast_renders_title_and_message() {
        let mut state = AppState::demo();
        let anchor = state.prompt.id();
        update::show_toast(
            &mut state,
            Toast::new(
                ToastKind::Info,
                text::TOAST_COPIED,
                Some(anchor),
                Instant::now(),
            )
            .with_title("Clipboard"),
        );
        let lines = render_lines(&state);
        assert!(lines.iter().any(|line| line.contains("Clipboard")));
        assert!(lines.iter().any(|line| line.contains(text::TOAST_COPIED)));
    }

    #[test]
    fn expired_toast_is_not_rendered() {
        let mut state = AppState::demo();
        update::show_toast(
            &mut state,
            Toast::new(ToastKind::Info, text::TOAST_COPIED, None, Instant::now()),
        );
        update::tick(&mut state, Instant::now() + TOAST_DURATION);
        let lines = render_lines(&state);
        assert!(!lines.iter().any(|line| line.contains(text::TOAST_COPIED)));
    }

    #[test]
    fn error_toast_uses_red_border() {
        let mut state = AppState::demo();
        let anchor = state.prompt.id();
        update::show_toast(
            &mut state,
            Toast::new(
                ToastKind::Error,
                text::TOAST_COPY_FAILED,
                Some(anchor),
                Instant::now(),
            ),
        );
        let config = Config::default();
        let view = view_for(&state);
        let pane_rects = crate::layout::pane_rects(
            &state.active_tab().layout,
            view.panes,
            config.min_pane_width,
        );
        let area = toast::rect(
            &state,
            &view,
            &pane_rects,
            Rect::new(0, 0, 100, 24),
            &config,
        )
        .expect("toast area is resolvable");
        let mut terminal =
            RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
        if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
            panic!("draw failed: {error}");
        }
        let buffer = terminal.backend().buffer().clone();
        assert_eq!(buffer[(area.x, area.y)].fg, ratatui::style::Color::Red);
    }
}
