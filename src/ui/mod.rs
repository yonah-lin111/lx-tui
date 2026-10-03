//! 渲染层：只读取状态，禁止修改状态或执行 IO。

pub mod layout;
pub mod style;
pub mod terminal;
pub mod text;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph, Wrap};

use crate::app::state::{AppState, Mode};
use crate::config::Config;

/// 渲染整个界面。
pub fn render(frame: &mut Frame<'_>, state: &AppState, config: &Config) {
    let area = frame.area();
    if area.width < config.min_width || area.height < config.min_height {
        render_min_size_notice(frame, area, config);
        return;
    }

    let view = layout::compute(area, config, state.sidebar_collapsed);

    if view.sidebar.width > 0 {
        render_sidebar(frame, view.sidebar, state);
    }
    render_tab_bar(frame, view.tab_bar, state);
    render_panes(frame, view.panes, state);
    render_status(frame, view.status, state, area.height < config.short_height);

    if state.mode == Mode::Help {
        render_help_overlay(frame, area);
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

/// 主区域：BSP 平铺窗格，内容为各窗格的终端画面。
fn render_panes(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    for (id, rect) in crate::layout::pane_rects(&tab.layout, area) {
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let Some(pane) = tab.pane(id) else {
            continue;
        };
        let focused = id == focus;
        let title_style = if focused {
            style::accent()
        } else {
            style::muted()
        };
        let title = if pane.exited {
            format!(" {} (exited) ", text::pane_title(id, pane.terminal.title()))
        } else {
            format!(" {} ", text::pane_title(id, pane.terminal.title()))
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style::border(focused))
            .title(Span::styled(title, title_style));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if inner.width > 0 && inner.height > 0 {
            terminal::render(inner, frame.buffer_mut(), &pane.terminal, focused);
        }
    }
}

/// 状态栏：左侧上下文，右侧快捷键提示。
fn render_status(frame: &mut Frame<'_>, area: Rect, state: &AppState, compact: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    let pane_title = tab
        .pane(focus)
        .map(|pane| text::pane_title(focus, pane.terminal.title()))
        .unwrap_or_default();
    let left = format!(
        " {} · {} · {} ",
        text::APP_NAME,
        state.active_workspace().name,
        pane_title
    );
    let left_width = left.chars().count() as u16;
    let hint_width = text::STATUS_HINT.chars().count() as u16;

    if !compact && area.width > left_width + hint_width {
        let hint_area = Rect {
            x: area.x + area.width - hint_width,
            width: hint_width,
            ..area
        };
        frame.render_widget(
            Paragraph::new(Span::styled(text::STATUS_HINT, style::muted())),
            hint_area,
        );
    }
    frame.render_widget(Paragraph::new(Span::styled(left, style::muted())), area);
}

/// 帮助浮层：居中显示，Esc 关闭。
fn render_help_overlay(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(58, 70, area);
    if popup.width == 0 || popup.height == 0 {
        return;
    }
    let lines: Vec<Line<'_>> = text::HELP_KEYS
        .iter()
        .map(|(key, description)| {
            Line::from(vec![
                Span::styled(format!("  {key:<18}"), style::accent()),
                Span::styled(*description, style::text()),
            ])
        })
        .collect();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(true))
        .title(Span::styled(
            format!(" {} ", text::HELP_TITLE),
            style::accent(),
        ));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
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

/// 计算居中的百分比矩形。
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let width = (u32::from(area.width) * u32::from(percent_x) / 100) as u16;
    let height = (u32::from(area.height) * u32::from(percent_y) / 100) as u16;
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
