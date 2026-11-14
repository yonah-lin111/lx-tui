//! Agents 标题编辑弹窗：自定义输入、Recent 页选择、几何与命中。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::TemplateStatus;
use crate::app::overlay::AgentTitleDialog;
use crate::app::prompt::RecentPageRow;
use crate::ui::widgets;
use crate::ui::{style, text};

use super::dialog::{BUTTON_GAP, render_buttons, render_input};

/// 弹窗宽度与高度范围（行）：每页占两行，最多展示 5 条。
const AGENT_TITLE_WIDTH: u16 = 56;
const AGENT_TITLE_MIN_HEIGHT: u16 = 9;
const AGENT_TITLE_MAX_HEIGHT: u16 = 17;
const AGENT_TITLE_MAX_ENTRIES: usize = 5;
const AGENT_TITLE_BUTTONS: [&str; 3] = [text::BUTTON_SAVE, text::BUTTON_CLEAR, text::BUTTON_CANCEL];

/// Agents 标题弹窗按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTitleButton {
    Save,
    Clear,
    Cancel,
}

/// 弹窗几何；高度随 Recent 条目数增长并夹取。
pub fn agent_title_shell(screen: Rect, row_count: usize) -> Option<widgets::modal::ModalShell> {
    let entries = row_count.clamp(1, AGENT_TITLE_MAX_ENTRIES) as u16;
    let height = entries
        .saturating_mul(2)
        .saturating_add(7)
        .clamp(AGENT_TITLE_MIN_HEIGHT, AGENT_TITLE_MAX_HEIGHT);
    widgets::modal::layout(screen, AGENT_TITLE_WIDTH, height)
}

/// 列表可视条目数：扣除输入行、分割线、按钮行与上下边框。
pub fn agent_title_max_rows(shell: &widgets::modal::ModalShell) -> usize {
    usize::from(shell.inner.height.saturating_sub(3)) / 2
}

/// 列表窗口起点：保证当前高亮条目可见。
pub fn agent_title_visible_start(dialog: &AgentTitleDialog, max_rows: usize) -> usize {
    if max_rows == 0 {
        return 0;
    }
    dialog
        .selected
        .unwrap_or(0)
        .saturating_add(1)
        .saturating_sub(max_rows)
}

/// 命中 Recent 行；返回弹窗行快照下标。
pub fn agent_title_row_at(
    shell: &widgets::modal::ModalShell,
    dialog: &AgentTitleDialog,
    column: u16,
    row: u16,
) -> Option<usize> {
    let list_y = shell.inner.y.saturating_add(2);
    let buttons_y = shell.inner.bottom().saturating_sub(1);
    if column < shell.inner.x || column >= shell.inner.right() || row < list_y || row >= buttons_y {
        return None;
    }
    let max_rows = agent_title_max_rows(shell);
    let offset = usize::from(row - list_y) / 2;
    if offset >= max_rows {
        return None;
    }
    let start = agent_title_visible_start(dialog, max_rows);
    dialog.rows.get(start + offset).map(|_| start + offset)
}

/// 命中弹窗按钮。
pub fn agent_title_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<AgentTitleButton> {
    let rects = widgets::modal::button_row(
        shell.inner,
        &AGENT_TITLE_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(AgentTitleButton::Save),
        Some(1) => Some(AgentTitleButton::Clear),
        Some(2) => Some(AgentTitleButton::Cancel),
        _ => None,
    }
}

/// 标题弹窗：输入行、`recent` 分割线、双行页列表与底部按钮；返回输入光标。
pub(super) fn render_agent_title(
    frame: &mut Frame<'_>,
    screen: Rect,
    dialog: &AgentTitleDialog,
) -> Option<(u16, u16)> {
    let shell = agent_title_shell(screen, dialog.rows.len())?;
    widgets::modal::render(frame, &shell, text::RENAME_AGENT_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let cursor = render_input(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        dialog.input.text(),
        dialog.input.cursor(),
        dialog.input.selection(),
        Some(text::AGENT_TITLE_PLACEHOLDER),
    );
    render_recent_rule(frame, shell.inner);
    render_recent_rows(frame, shell.inner, dialog);
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &AGENT_TITLE_BUTTONS,
            BUTTON_GAP,
            shell.inner.height.saturating_sub(1),
        ),
        &[
            (text::BUTTON_SAVE, style::button_primary()),
            (text::BUTTON_CLEAR, style::button_secondary()),
            (text::BUTTON_CANCEL, style::button_secondary()),
        ],
    );
    cursor
}

/// `recent` 分割线：与右侧 Prompt 面板的抽屉标签保持一致。
fn render_recent_rule(frame: &mut Frame<'_>, inner: Rect) {
    let y = inner.y.saturating_add(1);
    frame.render_widget(
        Paragraph::new(text::DIVIDER_MID.repeat(usize::from(inner.width)))
            .style(style::border(false)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    let label = text::PROMPT_RECENT_SECTION_LABEL;
    let label_x = inner.x.saturating_add(2);
    if label_x.saturating_add(label.width() as u16) < inner.right() {
        frame
            .buffer_mut()
            .set_stringn(label_x, y, label, label.width(), style::muted_on_border());
    }
}

/// 双行 Recent 列表；无记录时居中显示空态。
fn render_recent_rows(frame: &mut Frame<'_>, inner: Rect, dialog: &AgentTitleDialog) {
    let list_y = inner.y.saturating_add(2);
    let list_height = inner.height.saturating_sub(3);
    let list_area = Rect::new(inner.x, list_y, inner.width, list_height);
    if dialog.rows.is_empty() {
        let label = text::PROMPT_RECENT_EMPTY;
        let label_width = u16::try_from(label.width()).unwrap_or(u16::MAX);
        let x = list_area
            .x
            .saturating_add(list_area.width.saturating_sub(label_width) / 2);
        let y = list_area
            .y
            .saturating_add(list_area.height.saturating_sub(1) / 2);
        frame
            .buffer_mut()
            .set_stringn(x, y, label, usize::from(list_area.width), style::muted());
        return;
    }
    let max_rows = usize::from(inner.height.saturating_sub(3)) / 2;
    let start = agent_title_visible_start(dialog, max_rows);
    for (visible, row) in dialog.rows.iter().skip(start).take(max_rows).enumerate() {
        let y = list_y.saturating_add((visible as u16).saturating_mul(2));
        render_recent_row(
            frame,
            list_area,
            y,
            row,
            dialog.selected == Some(start + visible),
        );
    }
}

/// 单条双行记录：第一行页标题 + 时间，第二行提示词名；选中项整行铺底色。
fn render_recent_row(
    frame: &mut Frame<'_>,
    area: Rect,
    y: u16,
    row: &RecentPageRow,
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
            .set_style(Rect::new(area.x, y, area.width, 2), highlight);
    }
    let width = usize::from(area.width);
    let marker = if selected {
        format!("{} ", text::AGENT_SELECTED_MARKER)
    } else {
        "  ".to_string()
    };
    let time_width = row.time_label.width();
    let title_width = width
        .saturating_sub(marker.width())
        .saturating_sub(time_width)
        .saturating_sub(1)
        .max(1);
    let title = text::ellipsize_width(&row.page_title, title_width);
    let padding = width.saturating_sub(marker.width() + title.width() + time_width);
    // 页标题状态色与 Prompt 抽屉的 Recent 一致：todo 默认亮白、in_progress 黄、
    // done 绿、无状态压暗；选中项在此基础上加粗。
    let title_style = match row.status {
        Some(TemplateStatus::Todo) => style::text(),
        Some(status) => style::template_status(status),
        None => style::muted(),
    };
    let title_style = if selected {
        title_style.patch(style::strong())
    } else {
        title_style
    };
    // 状态色优先于选中行前景色，选中底色继续保留。
    let title_style = style::text().patch(highlight).patch(title_style);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(marker.clone(), style::strong().patch(highlight)),
            Span::styled(title, title_style),
            Span::styled(" ".repeat(padding), Style::default().patch(highlight)),
            Span::styled(row.time_label.clone(), style::muted().patch(highlight)),
        ])),
        Rect::new(area.x, y, area.width, 1),
    );
    let name_width = width.saturating_sub(2);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ".to_string(), Style::default().patch(highlight)),
            Span::styled(
                text::ellipsize_width(&row.prompt_name, name_width),
                style::muted().patch(highlight),
            ),
        ])),
        Rect::new(area.x, y.saturating_add(1), area.width, 1),
    );
}

#[cfg(test)]
mod tests;
