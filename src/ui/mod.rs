//! 渲染层：只读取状态，禁止修改状态或执行 IO。

pub mod layout;
pub mod style;
pub mod terminal;
pub mod text;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, Paragraph, Wrap};

use crate::app::state::{AppState, Pane, PaneKind};
use crate::config::Config;
use crate::layout::{COLLAPSED_STRIP, PaneId};

/// 渲染整个界面。
pub fn render(frame: &mut Frame<'_>, state: &AppState, config: &Config) {
    let area = frame.area();
    if area.width < config.min_width || area.height < config.min_height {
        render_min_size_notice(frame, area, config);
        return;
    }

    let view = layout::compute(area, config, state.sidebar_collapsed);

    if view.sidebar.width > 0 {
        if state.sidebar_collapsed {
            render_collapsed_strip(frame.buffer_mut(), view.sidebar);
        } else {
            render_sidebar(frame, view.sidebar, state);
        }
    }
    render_tab_bar(frame, view.tab_bar, state);
    render_panes(frame, view.panes, state);
    render_status(frame, view.status, state);
    render_collapse_buttons(frame, state, config);
}

/// 折叠面板的按钮目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseTarget {
    Sidebar,
    Prompt,
}

/// 顶边内的折叠按钮：命中矩形、目标与当前折叠态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollapseButton {
    pub target: CollapseTarget,
    pub area: Rect,
    pub collapsed: bool,
}

/// 各面板顶边内的折叠按钮；渲染与鼠标命中共用同一几何。
pub fn collapse_buttons(state: &AppState, config: &Config, area: Rect) -> Vec<CollapseButton> {
    let view = layout::compute(area, config, state.sidebar_collapsed);
    let mut buttons = Vec::new();
    if view.sidebar.width == COLLAPSED_STRIP {
        buttons.push(CollapseButton {
            target: CollapseTarget::Sidebar,
            area: Rect::new(view.sidebar.x, view.sidebar.y, COLLAPSED_STRIP, 1),
            collapsed: true,
        });
    } else if view.sidebar.width > COLLAPSED_STRIP {
        buttons.push(CollapseButton {
            target: CollapseTarget::Sidebar,
            area: Rect::new(
                view.sidebar.right() - COLLAPSED_STRIP - 1,
                view.sidebar.y,
                COLLAPSED_STRIP,
                1,
            ),
            collapsed: false,
        });
    }

    let tab = state.active_tab();
    for (id, rect) in crate::layout::pane_rects(&tab.layout, view.panes) {
        let Some(pane) = tab.pane(id) else {
            continue;
        };
        if pane.kind != PaneKind::Prompt || rect.width == 0 || rect.height == 0 {
            continue;
        }
        let collapsed = tab.layout.collapsed() == Some(id);
        let button = if collapsed {
            Rect::new(rect.x, rect.y, COLLAPSED_STRIP, 1)
        } else {
            Rect::new(
                rect.right().saturating_sub(COLLAPSED_STRIP + 1),
                rect.y,
                COLLAPSED_STRIP,
                1,
            )
        };
        buttons.push(CollapseButton {
            target: CollapseTarget::Prompt,
            area: button,
            collapsed,
        });
    }
    buttons
}

/// 命中测试：返回坐标所在折叠按钮的目标。
pub fn collapse_button_at(
    state: &AppState,
    config: &Config,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<CollapseTarget> {
    collapse_buttons(state, config, area)
        .into_iter()
        .find(|button| button.area.contains((column, row).into()))
        .map(|button| button.target)
}

/// 在面板顶边绘制折叠按钮，必须晚于面板内容渲染。
fn render_collapse_buttons(frame: &mut Frame<'_>, state: &AppState, config: &Config) {
    let buttons = collapse_buttons(state, config, frame.area());
    let buf = frame.buffer_mut();
    for button in buttons {
        let label = if button.collapsed {
            text::EXPAND_LABEL
        } else {
            text::COLLAPSE_LABEL
        };
        for (offset, symbol) in label.chars().enumerate() {
            let x = button.area.x + offset as u16;
            if let Some(cell) = buf.cell_mut((x, button.area.y)) {
                cell.reset();
                cell.set_char(symbol);
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

/// 主区域：BSP 平铺窗格；折叠窗格渲染为边缘窄条。
fn render_panes(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    let collapsed = tab.layout.collapsed();
    for (id, rect) in crate::layout::pane_rects(&tab.layout, area) {
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let Some(pane) = tab.pane(id) else {
            continue;
        };
        if collapsed == Some(id) {
            render_collapsed_strip(frame.buffer_mut(), rect);
            continue;
        }
        let focused = id == focus;
        let title_style = if focused {
            style::accent()
        } else {
            style::muted()
        };
        let mut title = pane_display_title(id, pane);
        if pane.exited {
            title.push_str(" (exited)");
        }
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style::border(focused))
            .title(Span::styled(format!(" {title} "), title_style));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
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
                    state.selection_for(id),
                );
            }
            PaneKind::Placeholder => {}
        }
    }
}

/// 窗格标题：prompt 固定标题，空占位与终端走通用标题规则。
fn pane_display_title(id: PaneId, pane: &Pane) -> String {
    match pane.kind {
        PaneKind::Prompt => text::PROMPT_TITLE.to_string(),
        PaneKind::Terminal | PaneKind::Placeholder => text::pane_title(id, pane.terminal.title()),
    }
}

/// 折叠窄条：中间列竖线；顶部按钮由折叠按钮统一绘制。
fn render_collapsed_strip(buf: &mut Buffer, area: Rect) {
    if area.width == 0 {
        return;
    }
    let x = area.x + area.width / 2;
    for y in area.y..area.bottom() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(text::STRIP_LINE);
            cell.set_style(style::muted());
        }
    }
}

/// 状态栏：左侧上下文。
fn render_status(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    let pane_title = tab
        .pane(focus)
        .map(|pane| pane_display_title(focus, pane))
        .unwrap_or_default();
    let left = format!(
        " {} · {} · {} ",
        text::APP_NAME,
        state.active_workspace().name,
        pane_title
    );
    frame.render_widget(Paragraph::new(Span::styled(left, style::muted())), area);
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
    use crate::app::update;
    use ratatui::Terminal as RatatuiTerminal;
    use ratatui::backend::TestBackend;

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

    #[test]
    fn prompt_pane_renders_content_and_collapse_icon() {
        let state = AppState::demo();
        let lines = render_lines(&state);
        assert!(lines.iter().any(|line| line.contains("Drag to select")));
        assert!(lines.iter().any(|line| line.contains(text::COLLAPSE_LABEL)));
    }

    #[test]
    fn collapsed_prompt_renders_strip_and_expand_icon() {
        let mut state = AppState::demo();
        update::apply(Action::TogglePrompt, &mut state, &[]);
        let lines = render_lines(&state);
        assert!(lines.iter().any(|line| line.contains(text::EXPAND_LABEL)));
        assert!(!lines.iter().any(|line| line.contains("Drag to select")));
    }

    #[test]
    fn collapse_buttons_hit_follow_geometry() {
        let mut state = AppState::demo();
        let config = Config::default();
        let area = Rect::new(0, 0, 100, 24);
        let sidebar = collapse_buttons(&state, &config, area)
            .into_iter()
            .find(|button| button.target == CollapseTarget::Sidebar);
        assert!(sidebar.is_some());
        let Some(sidebar) = sidebar else { return };
        assert!(!sidebar.collapsed);
        assert_eq!(
            collapse_button_at(&state, &config, area, sidebar.area.x, sidebar.area.y),
            Some(CollapseTarget::Sidebar)
        );
        update::apply(Action::ToggleSidebar, &mut state, &[]);
        let collapsed = collapse_buttons(&state, &config, area)
            .into_iter()
            .find(|button| button.target == CollapseTarget::Sidebar);
        assert!(collapsed.is_some_and(|button| button.collapsed));
    }

    #[test]
    fn collapse_button_cells_use_pure_accent_style() {
        let mut state = AppState::demo();
        let config = Config::default();
        let area = Rect::new(0, 0, 100, 24);
        let mut terminal =
            RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
        for collapsed in [false, true] {
            state.sidebar_collapsed = collapsed;
            if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
                panic!("draw failed: {error}");
            }
            let buffer = terminal.backend().buffer().clone();
            for button in collapse_buttons(&state, &config, area) {
                for x in button.area.x..button.area.right() {
                    let cell = &buffer[(x, button.area.y)];
                    assert_eq!(cell.fg, ratatui::style::Color::Cyan);
                    assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
                    assert!(!cell.modifier.contains(ratatui::style::Modifier::DIM));
                }
            }
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
}
