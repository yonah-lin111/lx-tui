//! 紧凑布局顶部栏：左栏入口、tab 条占位、右栏入口与第二行状态。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::AppState;
use crate::detect::AgentState;
use crate::ui::layout::ViewLayout;
use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// 命中左栏入口。
pub fn left_at(view: &ViewLayout, column: u16, row: u16) -> bool {
    view.compact_left_button.contains((column, row).into())
}

/// 命中右栏入口。
pub fn right_at(view: &ViewLayout, column: u16, row: u16) -> bool {
    view.compact_right_button.contains((column, row).into())
}

/// 渲染顶部栏；先清底，tab 条随后由 `tab_bar::render` 画进中间区域。
pub fn render(frame: &mut Frame<'_>, view: &ViewLayout, state: &AppState) {
    let area = view.compact_header;
    if area.width == 0 || area.height == 0 {
        return;
    }
    clear::render(frame, area);
    let buf = frame.buffer_mut();
    render_buttons(buf, view, state);
    if view.compact_status.height > 0 {
        render_status(buf, view.compact_status, state);
    }
}

/// 左侧 `[ws]`、右侧 `[panel]`；打开对应全屏栏时使用实心强调色。
fn render_buttons(buf: &mut Buffer, view: &ViewLayout, state: &AppState) {
    let left_active = state.sidebar_focused();
    let right_active = state.prompt_focused() || state.todo_focused();
    write(
        buf,
        view.compact_left_button.x,
        view.compact_left_button.y,
        text::COMPACT_LEFT_LABEL,
        view.compact_left_button.width,
        if left_active {
            style::selected_item()
        } else {
            style::accent()
        },
    );
    write(
        buf,
        view.compact_right_button.x,
        view.compact_right_button.y,
        text::COMPACT_RIGHT_LABEL,
        view.compact_right_button.width,
        if right_active {
            style::selected_item()
        } else {
            style::accent()
        },
    );
    if view.tab_bar.x > view.compact_left_button.right() {
        write(
            buf,
            view.compact_left_button.right(),
            view.compact_left_button.y,
            "│",
            1,
            style::muted(),
        );
    }
    if view.tab_bar.right() < view.compact_right_button.x {
        write(
            buf,
            view.tab_bar.right(),
            view.compact_right_button.y,
            "│",
            1,
            style::muted(),
        );
    }
}

/// 第二行：左侧当前工作区状态点与名称，右侧全局 Agent 聚合摘要。
fn render_status(buf: &mut Buffer, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let workspace = state.active_workspace();
    let status = workspace_status(state);
    let dot = text::agent_status_dot(status.unwrap_or(AgentState::Unknown));
    let dot_style = status.map(style::agent_status).unwrap_or_else(style::muted);

    let (severity, summary_text) = agent_summary(state);
    let summary_dot = text::agent_status_dot(severity.unwrap_or(AgentState::Unknown));
    let summary_width = u16::try_from(
        summary_dot
            .width()
            .saturating_add(1)
            .saturating_add(summary_text.width()),
    )
    .unwrap_or(u16::MAX);
    let show_summary = area.width > summary_width.saturating_add(12);
    let name_width = if show_summary {
        area.width.saturating_sub(summary_width).saturating_sub(3)
    } else {
        area.width.saturating_sub(2)
    };
    let name = text::ellipsize_width(&workspace.name, usize::from(name_width));
    write(buf, area.x, area.y, dot, 1, dot_style);
    write(
        buf,
        area.x.saturating_add(2),
        area.y,
        &name,
        name_width,
        style::strong(),
    );
    if show_summary {
        let x = area.right().saturating_sub(summary_width);
        let dot_style = severity
            .map(style::agent_status)
            .unwrap_or_else(style::muted);
        write(buf, x, area.y, summary_dot, 1, dot_style);
        write(
            buf,
            x.saturating_add(2),
            area.y,
            &summary_text,
            summary_width.saturating_sub(2),
            style::muted(),
        );
    }
}

/// 当前工作区 Agent 聚合状态；无 Agent 返回 None。
fn workspace_status(state: &AppState) -> Option<AgentState> {
    aggregate(state.agent_entries().into_iter().map(|entry| entry.state))
}

/// 全局 Agent 摘要：严重度与文本（不含状态圆点）。
fn agent_summary(state: &AppState) -> (Option<AgentState>, String) {
    let states = state.global_agent_states();
    if states.is_empty() {
        return (None, text::COMPACT_NO_AGENTS.to_string());
    }
    let mut blocked = 0usize;
    let mut working = 0usize;
    let mut idle = 0usize;
    for state in &states {
        match state {
            AgentState::Blocked => blocked += 1,
            AgentState::Working => working += 1,
            AgentState::Idle | AgentState::Unknown => idle += 1,
        }
    }
    let mut parts = Vec::new();
    if blocked > 0 {
        parts.push(format!("{blocked} {}", text::COMPACT_AGENT_BLOCKED));
    }
    if working > 0 {
        parts.push(format!("{working} {}", text::COMPACT_AGENT_WORKING));
    }
    if idle > 0 {
        parts.push(format!("{idle} {}", text::COMPACT_AGENT_IDLE));
    }
    let severity = aggregate(states.into_iter()).unwrap_or(AgentState::Unknown);
    (Some(severity), parts.join(text::IDENTITY_SEPARATOR))
}

/// 状态严重度：Blocked > Working > Idle/Unknown。
fn aggregate(states: impl Iterator<Item = AgentState>) -> Option<AgentState> {
    let mut result = None;
    for state in states {
        result = Some(match (result, state) {
            (Some(AgentState::Blocked), _) | (_, AgentState::Blocked) => AgentState::Blocked,
            (Some(AgentState::Working), _) | (_, AgentState::Working) => AgentState::Working,
            _ => AgentState::Idle,
        });
    }
    result
}

fn write(buf: &mut Buffer, x: u16, y: u16, value: &str, max_width: u16, style: Style) {
    if max_width == 0 || value.is_empty() {
        return;
    }
    buf.set_stringn(x, y, value, usize::from(max_width), style);
}

#[cfg(test)]
mod tests;
