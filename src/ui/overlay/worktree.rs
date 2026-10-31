//! worktree 打开对话框：几何、命中与渲染。

use crate::app::overlay::{WorktreeOpen, WorktreeOpenEntry, WorktreeStatus};
use crate::ui::widgets;
use crate::ui::{style, text};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::dialog::{BUTTON_GAP, render_buttons, render_input};

/// worktree 对话框宽度与高度范围（行）：条目两行一条 + 搜索/分隔/按钮与顶底栏框。
const WORKTREE_DIALOG_WIDTH: u16 = 56;
const WORKTREE_DIALOG_MIN_HEIGHT: u16 = 12;
const WORKTREE_DIALOG_MAX_HEIGHT: u16 = 22;
const WORKTREE_DIALOG_BUTTONS: [&str; 2] = [text::BUTTON_OPEN, text::BUTTON_CANCEL];

/// worktree 对话框按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeDialogButton {
    Open,
    Cancel,
}

/// worktree 对话框几何；条目两行一条，高度随条目数夹取。
pub fn worktree_dialog_shell(
    screen: Rect,
    entry_count: usize,
) -> Option<widgets::modal::ModalShell> {
    let rows = entry_count.max(1).saturating_mul(2) as u16;
    let height = rows
        .saturating_add(9)
        .clamp(WORKTREE_DIALOG_MIN_HEIGHT, WORKTREE_DIALOG_MAX_HEIGHT);
    widgets::modal::layout(screen, WORKTREE_DIALOG_WIDTH, height)
}

/// 对话框列表可视条目数；每条两行（扣除搜索行、分割线、底栏分割线与按钮行共 4 行）。
pub fn worktree_dialog_max_rows(shell: &widgets::modal::ModalShell) -> usize {
    usize::from(shell.inner.height.saturating_sub(4)) / 2
}

/// 列表窗口起始条目位置：让选中项落在窗口内。
pub fn worktree_dialog_visible_start(dialog: &WorktreeOpen, max_rows: usize) -> usize {
    if max_rows == 0 {
        return 0;
    }
    let filtered = dialog.filtered_indices();
    let position = dialog
        .selected_entry_index()
        .and_then(|index| filtered.iter().position(|candidate| *candidate == index))
        .unwrap_or(0);
    position.saturating_add(1).saturating_sub(max_rows)
}

/// 命中对话框条目；返回条目索引（非过滤位置）。
pub fn worktree_dialog_entry_at(
    shell: &widgets::modal::ModalShell,
    dialog: &WorktreeOpen,
    column: u16,
    row: u16,
) -> Option<usize> {
    let list_y = shell.inner.y.saturating_add(2);
    if column < shell.inner.x || column >= shell.inner.right() || row < list_y {
        return None;
    }
    let max_rows = worktree_dialog_max_rows(shell);
    if max_rows == 0 {
        return None;
    }
    let offset = usize::from(row - list_y) / 2;
    if offset >= max_rows {
        return None;
    }
    let start = worktree_dialog_visible_start(dialog, max_rows);
    dialog.filtered_indices().get(start + offset).copied()
}

/// 命中对话框按钮。
pub fn worktree_dialog_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<WorktreeDialogButton> {
    let rects = widgets::modal::button_row(
        shell.inner,
        &WORKTREE_DIALOG_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(WorktreeDialogButton::Open),
        Some(1) => Some(WorktreeDialogButton::Cancel),
        _ => None,
    }
}

/// worktree 对话框：搜索行、分隔线、两行条目与底部按钮；返回搜索光标位置。
pub(super) fn render_worktree_open(
    frame: &mut Frame<'_>,
    screen: Rect,
    dialog: &WorktreeOpen,
) -> Option<(u16, u16)> {
    let shell = worktree_dialog_shell(screen, dialog.entries.len())?;
    widgets::modal::render(frame, &shell, text::WORKTREE_OPEN_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let search_area = Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1);
    let cursor = render_input(
        frame,
        search_area,
        dialog.query.text(),
        dialog.query.cursor(),
        dialog.query.selection(),
        Some(text::WORKTREE_OPEN_FILTER),
    );
    let separator_area = Rect::new(
        shell.inner.x,
        shell.inner.y.saturating_add(1),
        shell.inner.width,
        1,
    );
    frame.render_widget(
        Paragraph::new(text::DIVIDER_MID.repeat(usize::from(shell.inner.width)))
            .style(style::border(false)),
        separator_area,
    );

    let list_y = shell.inner.y.saturating_add(2);
    let list_area = Rect::new(
        shell.inner.x,
        list_y,
        shell.inner.width,
        shell.inner.height.saturating_sub(4),
    );
    let message = if dialog.loading {
        Some(text::WORKTREE_OPEN_LOADING)
    } else if dialog.failed {
        Some(text::WORKTREE_OPEN_FAILED)
    } else if dialog.filtered_indices().is_empty() {
        Some(text::WORKTREE_OPEN_EMPTY)
    } else {
        None
    };
    if let Some(message) = message {
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {message}"), style::muted())),
            Rect {
                height: 1,
                ..list_area
            },
        );
    } else {
        let max_rows = worktree_dialog_max_rows(&shell);
        let start = worktree_dialog_visible_start(dialog, max_rows);
        let selected = dialog.selected_entry_index();
        for (visible, entry_index) in dialog
            .filtered_indices()
            .iter()
            .skip(start)
            .take(max_rows)
            .enumerate()
        {
            let Some(entry) = dialog.entries.get(*entry_index) else {
                continue;
            };
            let y = list_y.saturating_add((visible as u16).saturating_mul(2));
            render_worktree_entry(frame, shell.inner, y, entry, Some(*entry_index) == selected);
        }
    }

    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &WORKTREE_DIALOG_BUTTONS,
            BUTTON_GAP,
            shell.inner.height.saturating_sub(1),
        ),
        &[
            (text::BUTTON_OPEN, style::button_primary()),
            (text::BUTTON_CANCEL, style::button_secondary()),
        ],
    );
    cursor
}

/// 单条 worktree：首行 `› 名字` 加右对齐状态，次行缩进路径；选中项整行铺 surface 底色（对齐 herdr）。
fn render_worktree_entry(
    frame: &mut Frame<'_>,
    inner: Rect,
    y: u16,
    entry: &WorktreeOpenEntry,
    selected: bool,
) {
    let width = usize::from(inner.width);
    let marker = if selected {
        text::WORKTREE_OPEN_MARKER
    } else {
        " "
    };
    let row_base = if selected {
        style::overlay_selection()
    } else {
        style::text()
    };
    let name_style = if selected {
        row_base.add_modifier(ratatui::style::Modifier::BOLD)
    } else {
        style::text()
    };
    let muted_style = row_base.patch(style::muted());
    let status = match entry.status() {
        WorktreeStatus::Open => text::WORKTREE_STATUS_OPEN,
        WorktreeStatus::Detached => text::WORKTREE_STATUS_DETACHED,
        WorktreeStatus::Root => text::WORKTREE_STATUS_ROOT,
        WorktreeStatus::Branch => "",
    };
    let prefix = format!("{marker} ");
    let name_width = width
        .saturating_sub(prefix.chars().count())
        .saturating_sub(status.chars().count())
        .saturating_sub(1);
    let title = format!(
        "{prefix}{}",
        text::ellipsize(&entry.display_name(), name_width)
    );
    let pad = width
        .saturating_sub(title.chars().count())
        .saturating_sub(status.chars().count())
        .max(1);
    let top_rect = Rect::new(inner.x, y, inner.width, 1);
    let bottom_rect = Rect::new(inner.x, y.saturating_add(1), inner.width, 1);
    if selected {
        frame.buffer_mut().set_style(top_rect, row_base);
        frame.buffer_mut().set_style(bottom_rect, row_base);
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(title, name_style),
            Span::styled(" ".repeat(pad), name_style),
            Span::styled(status, muted_style),
        ])),
        top_rect,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::ellipsize(&format!("  {}", entry.path.display()), width),
            muted_style,
        )),
        bottom_rect,
    );
}
