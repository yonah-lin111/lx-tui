//! 紧凑布局顶部状态条：工作区/tab 状态、全局 Agent 摘要与切换器入口。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AppState, tab_label};
use crate::detect::AgentState;
use crate::ui::layout::ViewLayout;
use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// 顶部状态条几何。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactHeaderLayout {
    pub area: Rect,
    pub switch: Option<Rect>,
    pub back: Option<Rect>,
    pub status: Rect,
    pub title: Rect,
}

/// 计算状态条几何；渲染与鼠标命中共用。
pub fn layout(view: &ViewLayout, state: &AppState) -> CompactHeaderLayout {
    let area = view.compact_header;
    if area.width == 0 || area.height == 0 {
        return CompactHeaderLayout {
            area,
            switch: None,
            back: None,
            status: area,
            title: area,
        };
    }
    if state.compact_panel().is_some() {
        return panel_layout(area);
    }
    chat_layout(area)
}

/// Chat：右侧 `[switch]` 占满状态条高度，其余为状态区。
fn chat_layout(area: Rect) -> CompactHeaderLayout {
    let switch_width = u16::try_from(text::COMPACT_SWITCH_LABEL.width()).unwrap_or(u16::MAX);
    let switch = (area.width >= switch_width).then(|| {
        Rect::new(
            area.right() - switch_width,
            area.y,
            switch_width,
            area.height,
        )
    });
    let status = Rect::new(
        area.x,
        area.y,
        switch
            .map(|switch| switch.x.saturating_sub(area.x).saturating_sub(1))
            .unwrap_or(area.width),
        area.height,
    );
    CompactHeaderLayout {
        area,
        switch,
        back: None,
        status,
        title: Rect::new(area.x, area.y, 0, 0),
    }
}

/// 全屏面板：左侧 `[back]`，右侧标题与上下文。
fn panel_layout(area: Rect) -> CompactHeaderLayout {
    let back_width = u16::try_from(text::COMPACT_BACK_LABEL.width()).unwrap_or(u16::MAX);
    let back =
        (area.width >= back_width).then(|| Rect::new(area.x, area.y, back_width, area.height));
    let title_x = back
        .map(|back| back.right().saturating_add(1))
        .unwrap_or(area.x);
    let title = Rect::new(
        title_x,
        area.y,
        area.right().saturating_sub(title_x),
        area.height,
    );
    CompactHeaderLayout {
        area,
        switch: None,
        back,
        status: area,
        title,
    }
}

/// 命中切换器入口。
pub fn switch_at(view: &ViewLayout, state: &AppState, column: u16, row: u16) -> bool {
    layout(view, state)
        .switch
        .is_some_and(|area| area.contains((column, row).into()))
}

/// 命中返回聊天入口。
pub fn back_at(view: &ViewLayout, state: &AppState, column: u16, row: u16) -> bool {
    layout(view, state)
        .back
        .is_some_and(|area| area.contains((column, row).into()))
}

/// 渲染顶部状态条；先清底，避免上一帧残留。
pub fn render(frame: &mut Frame<'_>, view: &ViewLayout, state: &AppState) {
    let layout = layout(view, state);
    if layout.area.width == 0 || layout.area.height == 0 {
        return;
    }
    clear::render(frame, layout.area);
    let buf = frame.buffer_mut();
    if state.compact_panel().is_some() {
        render_panel(buf, &layout, state);
    } else {
        render_chat(buf, &layout, state);
    }
}

fn render_chat(buf: &mut Buffer, layout: &CompactHeaderLayout, state: &AppState) {
    let Some(switch) = layout.switch else {
        return;
    };
    let workspace = state.active_workspace();
    let status = workspace_status(state);
    let dot = text::agent_status_dot(status.unwrap_or(AgentState::Unknown));
    let dot_style = status.map(style::agent_status).unwrap_or_else(style::muted);

    let name_width = layout
        .status
        .width
        .saturating_sub(u16::try_from(dot.width() + 1).unwrap_or(u16::MAX));
    let name = text::ellipsize_width(&workspace.name, usize::from(name_width));
    write(
        buf,
        layout.status.x,
        layout.status.y,
        dot,
        layout.status.width,
        dot_style,
    );
    write(
        buf,
        layout.status.x.saturating_add(2),
        layout.status.y,
        &name,
        name_width,
        style::strong(),
    );

    let tab = tab_status(state);
    let tab_width = u16::try_from(tab.width()).unwrap_or(u16::MAX);
    let left_used = 2u16.saturating_add(u16::try_from(name.width()).unwrap_or(u16::MAX));
    if layout.status.width > left_used.saturating_add(tab_width).saturating_add(1) {
        let x = layout.status.right().saturating_sub(tab_width);
        write(buf, x, layout.status.y, &tab, tab_width, style::muted());
    }

    if layout.area.height >= 2 {
        let (severity, summary) = agent_summary(state);
        let dot = text::agent_status_dot(severity.unwrap_or(AgentState::Unknown));
        let dot_style = severity
            .map(style::agent_status)
            .unwrap_or_else(style::muted);
        let text_width = layout.area.width.saturating_sub(2);
        let summary = text::ellipsize_width(&summary, usize::from(text_width));
        write(
            buf,
            layout.area.x,
            layout.area.y.saturating_add(1),
            dot,
            1,
            dot_style,
        );
        write(
            buf,
            layout.area.x.saturating_add(2),
            layout.area.y.saturating_add(1),
            &summary,
            text_width,
            style::muted(),
        );
    }

    write(
        buf,
        switch.x,
        switch.y,
        text::COMPACT_SWITCH_LABEL,
        switch.width,
        style::accent(),
    );
    if switch.x > layout.area.x {
        write(
            buf,
            switch.x.saturating_sub(1),
            layout.area.y,
            "│",
            1,
            style::muted(),
        );
    }
}

fn render_panel(buf: &mut Buffer, layout: &CompactHeaderLayout, state: &AppState) {
    let Some(back) = layout.back else {
        return;
    };
    write(
        buf,
        back.x,
        back.y,
        text::COMPACT_BACK_LABEL,
        back.width,
        style::accent(),
    );
    let title = match state.compact_panel() {
        Some(crate::app::state::RightTab::Todo) => text::RIGHT_TAB_TODO_LABEL,
        _ => text::RIGHT_TAB_PROMPT_LABEL,
    };
    write(
        buf,
        layout.title.x,
        layout.title.y,
        title,
        layout.title.width,
        style::strong(),
    );
    let context = panel_context(state);
    let context_width = u16::try_from(context.width()).unwrap_or(u16::MAX);
    let title_width = u16::try_from(title.width()).unwrap_or(u16::MAX);
    if layout.title.width > title_width.saturating_add(context_width).saturating_add(1) {
        write(
            buf,
            layout.title.right().saturating_sub(context_width),
            layout.title.y,
            &context,
            context_width,
            style::muted(),
        );
    }
}

/// Chat 右侧 tab 状态：名称 + 多标签时的位置计数。
fn tab_status(state: &AppState) -> String {
    let workspace = state.active_workspace();
    let label = workspace
        .tabs
        .get(workspace.active_tab)
        .map(|tab| tab_label(workspace.active_tab, tab.name.as_deref()))
        .unwrap_or_default();
    if workspace.tabs.len() > 1 {
        format!(
            "{label}{}{}/{}",
            text::COMPACT_TAB_COUNT_PREFIX,
            workspace.active_tab.saturating_add(1),
            workspace.tabs.len()
        )
    } else {
        label
    }
}

/// 全屏面板右上角上下文：工作区 · tab。
fn panel_context(state: &AppState) -> String {
    let workspace = state.active_workspace();
    let label = workspace
        .tabs
        .get(workspace.active_tab)
        .map(|tab| tab_label(workspace.active_tab, tab.name.as_deref()))
        .unwrap_or_default();
    format!("{}{}{}", workspace.name, text::IDENTITY_SEPARATOR, label)
}

/// 当前工作区 Agent 聚合状态；无 Agent 返回 None。
fn workspace_status(state: &AppState) -> Option<AgentState> {
    aggregate(state.agent_entries().into_iter().map(|entry| entry.state))
}

/// 全局 Agent 摘要：严重度与文本（不含状态圆点）。
fn agent_summary(state: &AppState) -> (Option<AgentState>, String) {
    let entries = state.global_agent_entries();
    if entries.is_empty() {
        return (None, text::COMPACT_NO_AGENTS.to_string());
    }
    let mut blocked = 0usize;
    let mut working = 0usize;
    let mut idle = 0usize;
    for entry in &entries {
        match entry.entry.state {
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
    let severity =
        aggregate(entries.iter().map(|entry| entry.entry.state)).unwrap_or(AgentState::Unknown);
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
