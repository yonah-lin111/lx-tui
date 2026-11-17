//! New agent 弹窗：搜索框、已安装 CLI Agent 单选列表、几何与命中。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::overlay::NewAgentDialog;
use crate::ui::widgets;
use crate::ui::{style, text};

use super::dialog::{BUTTON_GAP, render_buttons, render_input};

/// 弹窗宽度与列表可视行数上限。
const NEW_AGENT_WIDTH: u16 = 48;
const NEW_AGENT_MAX_ENTRIES: usize = 8;
const NEW_AGENT_BUTTONS: [&str; 2] = [text::BUTTON_CREATE, text::BUTTON_CANCEL];

/// New agent 弹窗按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewAgentButton {
    Create,
    Cancel,
}

/// 弹窗几何：高度随候选数增长（搜索行 + 列表 + 按钮），行数夹在 1..=8。
pub fn new_agent_shell(screen: Rect, row_count: usize) -> Option<widgets::modal::ModalShell> {
    let rows = row_count.clamp(1, NEW_AGENT_MAX_ENTRIES) as u16;
    widgets::modal::layout(screen, NEW_AGENT_WIDTH, rows.saturating_add(6))
}

/// 列表可视行数：`inner` 首行留给搜索框、末行留给按钮。
pub fn new_agent_max_rows(shell: &widgets::modal::ModalShell) -> usize {
    usize::from(shell.inner.height.saturating_sub(2))
}

/// 列表窗口起点：保证当前高亮行可见。
pub fn new_agent_visible_start(dialog: &NewAgentDialog, max_rows: usize) -> usize {
    if max_rows == 0 {
        return 0;
    }
    dialog.selected.saturating_add(1).saturating_sub(max_rows)
}

/// 搜索框矩形；内框无行时不可见。
pub fn new_agent_search_rect(shell: &widgets::modal::ModalShell) -> Option<Rect> {
    (shell.inner.width > 0 && shell.inner.height > 0).then_some(Rect::new(
        shell.inner.x,
        shell.inner.y,
        shell.inner.width,
        1,
    ))
}

/// 列表矩形：搜索框下方到按钮行之前。
fn new_agent_rows_rect(shell: &widgets::modal::ModalShell) -> Option<Rect> {
    let max_rows = new_agent_max_rows(shell);
    (shell.inner.width > 0 && max_rows > 0).then_some(Rect::new(
        shell.inner.x,
        shell.inner.y.saturating_add(1),
        shell.inner.width,
        max_rows as u16,
    ))
}

/// 命中候选行；返回 `dialog.filtered_indices()` 下标。
pub fn new_agent_row_at(
    shell: &widgets::modal::ModalShell,
    dialog: &NewAgentDialog,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = new_agent_rows_rect(shell)?;
    if column < area.x || column >= area.right() || row < area.y || row >= area.bottom() {
        return None;
    }
    let offset = usize::from(row - area.y);
    let start = new_agent_visible_start(dialog, usize::from(area.height));
    let index = start.checked_add(offset)?;
    (index < dialog.filtered_indices().len()).then_some(index)
}

/// 命中弹窗按钮；内框不足 2 行时没有独立按钮行。
pub fn new_agent_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<NewAgentButton> {
    if shell.inner.height < 2 {
        return None;
    }
    let rects = widgets::modal::button_row(
        shell.inner,
        &NEW_AGENT_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(NewAgentButton::Create),
        Some(1) => Some(NewAgentButton::Cancel),
        _ => None,
    }
}

/// 渲染弹窗：顶部搜索框、候选单列、选中行整行高亮、底部 `create` / `cancel` 按钮。
pub(super) fn render_new_agent(
    frame: &mut Frame<'_>,
    screen: Rect,
    dialog: &NewAgentDialog,
) -> Option<(u16, u16)> {
    let shell = new_agent_shell(screen, dialog.agents.len())?;
    widgets::modal::render(frame, &shell, text::NEW_AGENT_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let cursor = new_agent_search_rect(&shell).and_then(|area| {
        render_input(
            frame,
            area,
            dialog.query.text(),
            dialog.query.cursor(),
            dialog.query.selection(),
            Some(text::NEW_AGENT_SEARCH),
        )
    });
    render_rows(frame, &shell, dialog);
    if shell.inner.height >= 2 {
        let create_style = if dialog.selected_agent().is_some() {
            style::button_primary()
        } else {
            style::muted()
        };
        render_buttons(
            frame,
            &widgets::modal::button_row(
                shell.inner,
                &NEW_AGENT_BUTTONS,
                BUTTON_GAP,
                shell.inner.height.saturating_sub(1),
            ),
            &[
                (text::BUTTON_CREATE, create_style),
                (text::BUTTON_CANCEL, style::button_secondary()),
            ],
        );
    }
    cursor
}

/// 候选列表；无安装或无匹配时在首行显示对应空态。
fn render_rows(frame: &mut Frame<'_>, shell: &widgets::modal::ModalShell, dialog: &NewAgentDialog) {
    let Some(area) = new_agent_rows_rect(shell) else {
        return;
    };
    let filtered = dialog.filtered_indices();
    if dialog.agents.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(text::NEW_AGENT_EMPTY, style::muted())),
            area,
        );
        return;
    }
    if filtered.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(text::NEW_AGENT_NO_MATCH, style::muted())),
            area,
        );
        return;
    }
    let max_rows = usize::from(area.height);
    let start = new_agent_visible_start(dialog, max_rows);
    for (visible, agent_index) in filtered.iter().skip(start).take(max_rows).enumerate() {
        let Some(agent) = dialog.agents.get(*agent_index) else {
            continue;
        };
        render_row(
            frame,
            area,
            area.y.saturating_add(visible as u16),
            agent,
            start + visible == dialog.selected,
        );
    }
}

/// 单行候选：左侧稳定 Agent 标签，右侧在命中名与标签不同时压暗显示实际命令。
fn render_row(
    frame: &mut Frame<'_>,
    area: Rect,
    y: u16,
    agent: &crate::detect::InstalledAgent,
    selected: bool,
) {
    let highlight = if selected {
        style::overlay_selection()
    } else {
        Style::default()
    };
    if selected {
        frame
            .buffer_mut()
            .set_style(Rect::new(area.x, y, area.width, 1), highlight);
    }
    let width = usize::from(area.width);
    let marker = if selected {
        format!("{} ", text::AGENT_SELECTED_MARKER)
    } else {
        "  ".to_string()
    };
    let label = text::agent_label(agent.kind);
    let command = (agent.command != label).then_some(agent.command.as_str());
    let command_width = command.map(|command| command.width()).unwrap_or(0);
    let label_width = width
        .saturating_sub(marker.width())
        .saturating_sub(command_width)
        .saturating_sub(usize::from(command.is_some()))
        .max(1);
    let label = text::ellipsize_width(label, label_width);
    let mut spans = vec![
        Span::styled(marker.clone(), style::strong().patch(highlight)),
        Span::styled(
            label.clone(),
            style::text().patch(highlight).patch(style::strong()),
        ),
    ];
    if let Some(command) = command {
        let padding = width.saturating_sub(marker.width() + label.width() + command.width());
        spans.push(Span::styled(
            " ".repeat(padding),
            Style::default().patch(highlight),
        ));
        spans.push(Span::styled(
            command.to_string(),
            style::muted().patch(highlight),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, y, area.width, 1),
    );
}

#[cfg(test)]
mod tests;
