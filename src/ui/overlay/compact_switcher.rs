//! 紧凑布局全屏切换器：几何、渲染与命中。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::overlay::CompactSwitcher;
use crate::app::state::{AppState, CompactRow, CompactSection, RightTab, tab_label};
use crate::ui::text;
use crate::ui::widgets::{clear, scrollbar};
use crate::ui::{style, widgets};

/// 切换器几何：整屏、关闭按钮、列表视口与滚动条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSwitcherLayout {
    pub area: Rect,
    pub close: Option<Rect>,
    pub viewport: Rect,
    pub scrollbar: Option<scrollbar::ScrollbarLayout>,
}

/// 计算切换器几何；渲染、行命中与滚动条共用。
pub fn layout(screen: Rect, switcher: &CompactSwitcher) -> CompactSwitcherLayout {
    let viewport = Rect::new(
        screen.x,
        screen.y.saturating_add(2),
        screen.width,
        screen.height.saturating_sub(2),
    );
    let close_width = u16::try_from(text::COMPACT_CLOSE_LABEL.width()).unwrap_or(u16::MAX);
    let close = (screen.width >= close_width).then(|| {
        Rect::new(
            screen.right() - close_width,
            screen.y,
            close_width,
            1.min(screen.height),
        )
    });
    let scrollbar = scrollbar::layout(
        viewport,
        switcher.rows.len(),
        usize::from(viewport.height),
        switcher.scroll,
    );
    CompactSwitcherLayout {
        area: screen,
        close,
        viewport,
        scrollbar,
    }
}

/// 点击行：返回 `rows` 下标；标题/分隔线/空白返回 None。
pub fn row_at(screen: Rect, switcher: &CompactSwitcher, column: u16, row: u16) -> Option<usize> {
    let layout = layout(screen, switcher);
    if !layout.viewport.contains((column, row).into()) {
        return None;
    }
    let index = switcher
        .scroll
        .saturating_add(usize::from(row.saturating_sub(layout.viewport.y)));
    (index < switcher.rows.len()).then_some(index)
}

/// 是否命中右上角 `[close]`。
pub fn close_at(screen: Rect, switcher: &CompactSwitcher, column: u16, row: u16) -> bool {
    layout(screen, switcher)
        .close
        .is_some_and(|area| area.contains((column, row).into()))
}

/// 渲染全屏切换器；覆盖整屏。
pub fn render(frame: &mut Frame<'_>, screen: Rect, state: &AppState, switcher: &CompactSwitcher) {
    if screen.width == 0 || screen.height == 0 {
        return;
    }
    let layout = layout(screen, switcher);
    clear::render(frame, screen);
    {
        let buf = frame.buffer_mut();
        buf.set_stringn(
            screen.x,
            screen.y,
            format!(" {}", text::COMPACT_TITLE),
            usize::from(screen.width),
            style::strong(),
        );
        if let Some(close) = layout.close {
            buf.set_stringn(
                close.x,
                close.y,
                text::COMPACT_CLOSE_LABEL,
                usize::from(close.width),
                style::accent(),
            );
        }
        if screen.height >= 2 {
            let rule = "─".repeat(usize::from(screen.width));
            buf.set_stringn(
                screen.x,
                screen.y + 1,
                rule,
                usize::from(screen.width),
                style::muted(),
            );
        }
        let visible = usize::from(layout.viewport.height);
        let row_width = screen
            .width
            .saturating_sub(u16::from(layout.scrollbar.is_some()));
        for (offset, row_index) in (switcher.scroll..switcher.rows.len())
            .take(visible)
            .enumerate()
        {
            let y = layout.viewport.y.saturating_add(offset as u16);
            let area = Rect::new(screen.x, y, row_width, 1);
            draw_row(buf, area, row_index, state, switcher);
        }
    }
    if let Some(bar) = layout.scrollbar {
        widgets::scrollbar::render(frame, &bar);
    }
}

/// 绘制一行：高亮/悬停整行铺底，再叠加内容。
fn draw_row(
    buf: &mut Buffer,
    area: Rect,
    row_index: usize,
    state: &AppState,
    switcher: &CompactSwitcher,
) {
    let Some(row) = switcher.rows.get(row_index) else {
        return;
    };
    let highlight = if row_index == switcher.selected {
        style::overlay_selection()
    } else if switcher.hover == Some(row_index) {
        style::selection()
    } else {
        Style::default()
    };
    if highlight != Style::default() {
        buf.set_style(area, highlight);
    }
    let selected = row_index == switcher.selected;

    match row {
        CompactRow::Section(section) => {
            let label = match section {
                CompactSection::Agents => text::COMPACT_SECTION_AGENTS,
                CompactSection::Spaces => text::COMPACT_SECTION_SPACES,
                CompactSection::Tabs => text::COMPACT_SECTION_TABS,
                CompactSection::Panels => text::COMPACT_SECTION_PANELS,
                CompactSection::Actions => text::COMPACT_SECTION_ACTIONS,
            };
            put(
                buf,
                area.x.saturating_add(1),
                area.y,
                label,
                area.width.saturating_sub(1),
                style::muted()
                    .patch(highlight)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            );
        }
        CompactRow::Agent(agent) => {
            let Some(workspace) = state.workspaces.get(agent.workspace) else {
                return;
            };
            let entry = &agent.entry;
            let right = format!(
                "{} {}",
                workspace.name,
                text::agent_location(entry.tab_index, entry.pane_index)
            );
            let prefix = selection_marker(selected);
            let dot = format!("{} ", text::agent_status_dot(entry.state));
            let mut name = text::sidebar_agent_label(entry.kind).to_string();
            if let Some(title) = agent.title.as_deref().filter(|title| !title.is_empty()) {
                name.push_str(text::IDENTITY_SEPARATOR);
                name.push_str(title);
            }
            draw_prefixed_row(
                buf,
                area,
                prefix,
                highlight,
                &dot,
                style::agent_status(entry.state),
                &name,
                style::strong(),
                Some((right.as_str(), right.width())),
            );
        }
        CompactRow::Workspace(index) => {
            let Some(workspace) = state.workspaces.get(*index) else {
                return;
            };
            let active = *index == state.active_workspace;
            let dot = format!(
                "{} ",
                if active {
                    text::CURRENT_ITEM_MARKER
                } else {
                    text::AGENT_STATUS_RING
                }
            );
            let right = workspace
                .git
                .as_ref()
                .and_then(|git| git.short_branch())
                .map(|branch| format!("{}{branch}", text::PROMPT_BRANCH_PREFIX));
            draw_prefixed_row(
                buf,
                area,
                selection_marker(selected),
                highlight,
                &dot,
                if active {
                    style::accent().patch(highlight)
                } else {
                    style::muted().patch(highlight)
                },
                &workspace.name,
                style::strong().patch(highlight),
                right.as_deref().map(|text| (text, text.width())),
            );
        }
        CompactRow::Tab(index) => {
            let Some(workspace) = state.active_workspace().tabs.get(*index) else {
                return;
            };
            let active = *index == state.active_workspace().active_tab;
            let dot = format!(
                "{} ",
                if active {
                    text::CURRENT_ITEM_MARKER
                } else {
                    text::AGENT_STATUS_RING
                }
            );
            let mut label = format!(
                "{} {}",
                text::TAB_ITEM_ICON,
                tab_label(*index, workspace.name.as_deref())
            );
            if workspace.has_pending_ask() {
                label.push(' ');
                label.push_str(text::TAB_ASK_MARKER);
            }
            draw_prefixed_row(
                buf,
                area,
                selection_marker(selected),
                highlight,
                &dot,
                if active {
                    style::accent().patch(highlight)
                } else {
                    style::muted().patch(highlight)
                },
                &label,
                style::strong().patch(highlight),
                None,
            );
        }
        CompactRow::Panel(panel) => {
            let active = state.right_tab == *panel;
            let dot = format!(
                "{} ",
                if active {
                    text::CURRENT_ITEM_MARKER
                } else {
                    text::AGENT_STATUS_RING
                }
            );
            let label = match panel {
                RightTab::Prompt => text::RIGHT_TAB_PROMPT_LABEL,
                RightTab::Todo => text::RIGHT_TAB_TODO_LABEL,
            };
            draw_prefixed_row(
                buf,
                area,
                selection_marker(selected),
                highlight,
                &dot,
                if active {
                    style::accent().patch(highlight)
                } else {
                    style::muted().patch(highlight)
                },
                label,
                style::strong().patch(highlight),
                None,
            );
        }
        CompactRow::NewWorkspace | CompactRow::NewTab => {
            let label = match row {
                CompactRow::NewWorkspace => text::COMPACT_NEW_WORKSPACE,
                CompactRow::NewTab => text::COMPACT_NEW_TAB,
                _ => return,
            };
            draw_prefixed_row(
                buf,
                area,
                selection_marker(selected),
                highlight,
                "",
                Style::default(),
                label,
                style::text().patch(highlight),
                None,
            );
        }
    }
}

/// 选中行行首标记；未选中行留出同宽空白。
fn selection_marker(selected: bool) -> &'static str {
    if selected {
        text::CURRENT_ITEM_MARKER
    } else {
        " "
    }
}

/// 绘制「选中标记 + 状态圆点 + 名称 + 右侧定位」；空间不足先丢右侧定位。
#[allow(clippy::too_many_arguments)]
fn draw_prefixed_row(
    buf: &mut Buffer,
    area: Rect,
    marker: &str,
    highlight: Style,
    dot: &str,
    dot_style: Style,
    label: &str,
    label_style: Style,
    right: Option<(&str, usize)>,
) -> u16 {
    let mut x = area.x;
    let marker_width = marker.width();
    put(
        buf,
        x,
        area.y,
        marker,
        area.width,
        style::accent().patch(highlight),
    );
    x = x.saturating_add(marker_width as u16);

    let dot_width = dot.width();
    if !dot.is_empty() {
        put(
            buf,
            x,
            area.y,
            dot,
            area.right().saturating_sub(x),
            dot_style.patch(highlight),
        );
        x = x.saturating_add(dot_width as u16);
    }

    let remaining = usize::from(area.right().saturating_sub(x));
    let mut reserved = 0;
    if let Some((text, width)) = right
        && remaining > width + 1
    {
        let x_right = area.right().saturating_sub(width as u16);
        put(
            buf,
            x_right,
            area.y,
            text,
            width as u16,
            style::muted().patch(highlight),
        );
        reserved = width + 1;
    }
    let label_width = remaining.saturating_sub(reserved);
    let label = text::ellipsize_width(label, label_width);
    put(
        buf,
        x,
        area.y,
        &label,
        label_width as u16,
        label_style.patch(highlight),
    );
    x.saturating_add(label.width() as u16)
        .saturating_sub(area.x)
}

/// 写入单段文本；不越过 `max_width`。
fn put(buf: &mut Buffer, x: u16, y: u16, value: &str, max_width: u16, style: Style) {
    if max_width == 0 || value.is_empty() {
        return;
    }
    buf.set_stringn(x, y, value, usize::from(max_width), style);
}

#[cfg(test)]
mod tests;
