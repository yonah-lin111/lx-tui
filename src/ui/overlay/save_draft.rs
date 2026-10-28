//! 草稿保存选择器几何、条目层次渲染与按钮命中。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use crate::app::overlay::{SaveDraft, SaveDraftEntry};
use crate::ui::widgets;
use crate::ui::{style, text};

use super::dialog::{BUTTON_GAP, render_buttons, render_input};

/// 草稿保存选择器宽度。
const SAVE_DRAFT_WIDTH: u16 = 44;

/// 选择器底部按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveDraftButton {
    Save,
    Cancel,
}

/// 选择器按钮标签（保存 / 取消）。
pub(crate) const SAVE_DRAFT_BUTTONS: [&str; 2] = [text::BUTTON_SAVE, text::BUTTON_CANCEL];

/// 草稿保存选择器几何：名称输入 + 分割线 + 条目单行 + 按钮行。
pub fn save_draft_shell(screen: Rect, entry_count: usize) -> Option<widgets::modal::ModalShell> {
    let height = (entry_count as u16).saturating_add(6).clamp(8, 30);
    widgets::modal::layout(screen, SAVE_DRAFT_WIDTH, height)
}

/// 选择器可视条目数（扣除名称、分割线与按钮行）。
pub fn save_draft_max_rows(shell: &widgets::modal::ModalShell) -> usize {
    usize::from(shell.inner.height.saturating_sub(3))
}

/// 列表窗口起始条目位置：让选中项落在窗口内。
pub fn save_draft_visible_start(dialog: &SaveDraft, max_rows: usize) -> usize {
    if max_rows == 0 {
        return 0;
    }
    dialog.selected.saturating_add(1).saturating_sub(max_rows)
}

/// 列表区起始行（名称 + 分割线之后）。
fn save_draft_list_y(shell: &widgets::modal::ModalShell) -> u16 {
    shell.inner.y.saturating_add(2)
}

/// 命中选择器条目；按钮行、区域外或越界返回 None。
pub fn save_draft_entry_at(
    shell: &widgets::modal::ModalShell,
    dialog: &SaveDraft,
    column: u16,
    row: u16,
) -> Option<usize> {
    if column < shell.inner.x || column >= shell.inner.right() {
        return None;
    }
    let list_y = save_draft_list_y(shell);
    if row < list_y {
        return None;
    }
    let max_rows = save_draft_max_rows(shell);
    let offset = usize::from(row - list_y);
    if offset >= max_rows {
        return None;
    }
    let index = save_draft_visible_start(dialog, max_rows) + offset;
    (index < dialog.entries.len()).then_some(index)
}

/// 命中选择器按钮。
pub fn save_draft_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<SaveDraftButton> {
    let rects = widgets::modal::button_row(
        shell.inner,
        &SAVE_DRAFT_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(SaveDraftButton::Save),
        Some(1) => Some(SaveDraftButton::Cancel),
        _ => None,
    }
}

/// 条目标签：根层 `/`、文件夹树形连接符（末项 `└─`）与追加动作行。
fn save_draft_entry_label(entries: &[SaveDraftEntry], index: usize) -> String {
    match &entries[index] {
        SaveDraftEntry::Root => text::SAVE_DRAFT_ROOT.to_string(),
        SaveDraftEntry::Folder { depth, name, .. } => {
            if *depth == 0 {
                name.clone()
            } else {
                let connector = if last_sibling(entries, index) {
                    text::PROMPT_TREE_LAST
                } else {
                    text::PROMPT_TREE_MIDDLE
                };
                format!("{connector} {name}")
            }
        }
        SaveDraftEntry::AddFolder => text::SAVE_DRAFT_NEW_FOLDER.to_string(),
    }
}

/// 同级末项判定：其后再无同深度行，或先遇到更浅的行（与树视图一致）。
fn last_sibling(entries: &[SaveDraftEntry], index: usize) -> bool {
    let depth = match &entries[index] {
        SaveDraftEntry::Folder { depth, .. } => *depth,
        _ => return true,
    };
    for entry in &entries[index + 1..] {
        match entry {
            SaveDraftEntry::Folder { depth: later, .. } => {
                if *later < depth {
                    return true;
                }
                if *later == depth {
                    return false;
                }
            }
            _ => return true,
        }
    }
    true
}

/// 渲染草稿保存选择器：名称输入 + 分割线 + 条目列表 + 按钮；
/// 返回活动输入（选中内联行时为该行名称，否则为顶部名称）的硬件光标位置。
pub(crate) fn render_save_draft(
    frame: &mut Frame<'_>,
    screen: Rect,
    dialog: &SaveDraft,
) -> Option<(u16, u16)> {
    let shell = save_draft_shell(screen, dialog.entries.len())?;
    widgets::modal::render(frame, &shell, text::SAVE_DRAFT_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let width = usize::from(shell.inner.width);
    let name_cursor = render_input(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        dialog.name.text(),
        dialog.name.cursor(),
        dialog.name.selection(),
        None,
    );
    // 分割线。
    frame.render_widget(
        Paragraph::new(text::DIVIDER_MID.repeat(width)).style(style::border(false)),
        Rect::new(
            shell.inner.x,
            shell.inner.y.saturating_add(1),
            shell.inner.width,
            1,
        ),
    );
    // 条目列表。
    let list_y = save_draft_list_y(&shell);
    let max_rows = save_draft_max_rows(&shell);
    let start = save_draft_visible_start(dialog, max_rows);
    for (offset, index) in (start..dialog.entries.len()).take(max_rows).enumerate() {
        let y = list_y.saturating_add(offset as u16);
        let selected = index == dialog.selected;
        let entry = &dialog.entries[index];
        let row_area = Rect::new(shell.inner.x, y, shell.inner.width, 1);
        let label = text::ellipsize(&save_draft_entry_label(&dialog.entries, index), width);
        let span_style = if selected {
            style::selection()
        } else {
            match entry {
                SaveDraftEntry::Folder { .. } => style::text(),
                SaveDraftEntry::Root | SaveDraftEntry::AddFolder => style::accent(),
            }
        };
        let paragraph = if selected {
            Paragraph::new(Span::styled(label, span_style)).style(style::selection())
        } else {
            Paragraph::new(Span::styled(label, span_style))
        };
        frame.render_widget(paragraph, row_area);
    }
    // 按钮行。
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &SAVE_DRAFT_BUTTONS,
            BUTTON_GAP,
            shell.inner.height.saturating_sub(1),
        ),
        &[
            (text::BUTTON_SAVE, style::accent()),
            (text::BUTTON_CANCEL, style::muted()),
        ],
    );
    name_cursor
}
