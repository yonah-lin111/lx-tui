//! 补充块附件模态渲染：列表、详情、未保存确认与删除确认共用同一外壳。
//!
//! 只读状态：几何计算与命中测试同源，渲染不在绘制中改状态或做 IO。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use crate::app::supplements::{SupplementDraft, Supplements};
use crate::ui::widgets;
use crate::ui::{style, text};

/// 模态期望外框尺寸：不随附件数量和正文长度变化，按屏幕钳制。
pub const SUPPLEMENTS_WIDTH: u16 = 96;
pub const SUPPLEMENTS_HEIGHT: u16 = 28;
/// 可编辑的最小外框尺寸；小于它只提示放大终端，不丢草稿。
pub const SUPPLEMENTS_MIN_WIDTH: u16 = 32;
pub const SUPPLEMENTS_MIN_HEIGHT: u16 = 10;

/// 详情操作按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplementButton {
    Save,
    Clean,
    Back,
}

/// 列表/详情/确认态的公共几何。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupplementLayout {
    pub shell: widgets::modal::ModalShell,
    /// 列表或正文区（不含标题行与按钮行）。
    pub body: Rect,
    /// 按钮行。
    pub buttons: Rect,
}

/// 模态外壳几何；屏幕过小仍返回钳制后的容器（仅通用容器放不下时为 None）。
pub fn supplements_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, SUPPLEMENTS_WIDTH, SUPPLEMENTS_HEIGHT)
}

/// 模态布局：正文区取外壳内容区去掉首行标题与末行按钮。
pub fn supplements_layout(screen: Rect) -> Option<SupplementLayout> {
    let shell = supplements_shell(screen)?;
    let inner = shell.inner;
    if inner.height < 2 {
        return None;
    }
    let body = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let buttons = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
    Some(SupplementLayout {
        shell,
        body,
        buttons,
    })
}

/// 屏幕是否足以真正编辑附件详情。
pub fn supplements_editable(shell: &widgets::modal::ModalShell) -> bool {
    shell.area.width >= SUPPLEMENTS_MIN_WIDTH && shell.area.height >= SUPPLEMENTS_MIN_HEIGHT
}

/// 列表可见行数：正文区高度与行数（附件 + 新增动作行）的较小者。
pub fn list_rows(layout: &SupplementLayout, session: &Supplements) -> usize {
    usize::from(layout.body.height)
        .max(1)
        .min(session.row_count().max(1))
}

/// 当前视图的按钮文案与样式。
fn detail_buttons() -> [(&'static str, Style); 3] {
    [
        (text::SUPPLEMENTS_SAVE, style::accent()),
        (text::SUPPLEMENTS_CLEAN, style::muted()),
        (text::SUPPLEMENTS_BACK, style::muted()),
    ]
}

/// 未保存确认行的按钮。
fn unsaved_buttons() -> [(&'static str, Style); 3] {
    [
        (text::SUPPLEMENTS_UNSAVED_SAVE, style::accent()),
        (text::SUPPLEMENTS_UNSAVED_DISCARD, style::error()),
        (text::SUPPLEMENTS_UNSAVED_KEEP, style::muted()),
    ]
}

/// 删除确认行的按钮。
fn confirm_buttons() -> [(&'static str, Style); 2] {
    [
        (text::SUPPLEMENTS_CONFIRM_DELETE, style::error()),
        (text::SUPPLEMENTS_CANCEL, style::muted()),
    ]
}

/// 命中详情按钮行。
pub fn supplement_button_at(
    layout: &SupplementLayout,
    column: u16,
    row: u16,
) -> Option<SupplementButton> {
    let labels: Vec<&str> = detail_buttons().iter().map(|(label, _)| *label).collect();
    let rects = widgets::modal::button_row(layout.buttons, &labels, 1, 0);
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(SupplementButton::Save),
        Some(1) => Some(SupplementButton::Clean),
        Some(2) => Some(SupplementButton::Back),
        _ => None,
    }
}

/// 命中未保存确认按钮：0 = save、1 = discard、2 = keep editing。
pub fn supplement_unsaved_button_at(
    layout: &SupplementLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let labels: Vec<&str> = unsaved_buttons().iter().map(|(label, _)| *label).collect();
    let rects = widgets::modal::button_row(layout.buttons, &labels, 1, 0);
    widgets::modal::button_at(&rects, column, row)
}

/// 命中删除确认按钮：0 = confirm、1 = cancel。
pub fn supplement_confirm_button_at(
    layout: &SupplementLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let labels: Vec<&str> = confirm_buttons().iter().map(|(label, _)| *label).collect();
    let rects = widgets::modal::button_row(layout.buttons, &labels, 1, 0);
    widgets::modal::button_at(&rects, column, row)
}

/// 行矩形（列表与详情共用）；越界返回 None。
fn row_rect(layout: &SupplementLayout, offset: usize) -> Option<Rect> {
    if offset >= usize::from(layout.body.height) {
        return None;
    }
    Some(Rect::new(
        layout.body.x,
        layout.body.y + offset as u16,
        layout.body.width,
        1,
    ))
}

/// 列表行命中：返回行下标（含末尾新增动作行）；空态/空白返回 None。
///
/// 行序与绘制共用 `window_start` / `list_rows`，命中矩形与绘制矩形一致。
pub fn list_row_at(
    layout: &SupplementLayout,
    session: &Supplements,
    column: u16,
    row: u16,
) -> Option<usize> {
    let start = session.window_start(list_rows(layout, session));
    let offset = usize::from(row.checked_sub(layout.body.y)?);
    if offset >= usize::from(layout.body.height)
        || column < layout.body.x
        || column >= layout.body.right()
    {
        return None;
    }
    let index = start + offset;
    (index < session.row_count()).then_some(index)
}

/// 详情正文区（仅正文可视区域；标题行在正文区首行）。
pub fn detail_body_rect(layout: &SupplementLayout) -> Rect {
    if layout.body.height <= 1 {
        return Rect::new(layout.body.x, layout.body.y, layout.body.width, 0);
    }
    Rect::new(
        layout.body.x,
        layout.body.y + 1,
        layout.body.width,
        layout.body.height - 1,
    )
}

/// 详情标题输入行命中。
pub fn title_row_at(layout: &SupplementLayout, column: u16, row: u16) -> bool {
    Rect::new(layout.body.x, layout.body.y, layout.body.width, 1).contains((column, row).into())
}

/// 渲染附件模态；返回硬件光标位置（标题输入或正文光标）。
pub fn render(frame: &mut Frame<'_>, screen: Rect, session: &Supplements) -> Option<(u16, u16)> {
    use crate::app::supplements::SupplementView;
    let layout = supplements_layout(screen)?;
    widgets::modal::render(frame, &layout.shell, text::SUPPLEMENTS_MODAL_TITLE);
    if layout.body.width == 0 || layout.body.height == 0 {
        return None;
    }
    if !supplements_editable(&layout.shell) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::SUPPLEMENTS_RESIZE_HINT, style::muted())),
            layout.body,
        );
        return None;
    }
    match session.view {
        SupplementView::List => {
            render_list(frame, &layout, session);
            None
        }
        SupplementView::Edit => session
            .draft
            .as_ref()
            .and_then(|draft| render_detail(frame, &layout, draft)),
        SupplementView::Unsaved => {
            render_unsaved(frame, &layout, session);
            None
        }
        SupplementView::ConfirmDelete => {
            render_confirm(frame, &layout, session);
            None
        }
    }
}

/// 列表：序号、标题、选中/悬停底色与底部新增动作行。
fn render_list(frame: &mut Frame<'_>, layout: &SupplementLayout, session: &Supplements) {
    if session.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(text::SUPPLEMENTS_EMPTY, style::muted())),
            Rect::new(layout.body.x, layout.body.y, layout.body.width, 1),
        );
    }
    let visible = list_rows(layout, session);
    let start = session.window_start(visible);
    for (offset, entry) in session.entries.iter().enumerate().skip(start).take(visible) {
        let Some(rect) = row_rect(layout, offset - start) else {
            break;
        };
        let selected = offset == session.selected;
        let hovered = session.hover == Some(offset);
        let marker = if selected { "❯" } else { " " };
        let label = format!(
            "{marker} {:>2}  {}",
            offset + 1,
            text::ellipsize(&entry.label, usize::from(rect.width.saturating_sub(5)))
        );
        let row_style = if selected {
            style::selected_item()
        } else if hovered {
            style::overlay_selection()
        } else {
            style::text()
        };
        frame.render_widget(Paragraph::new(Span::styled(label, row_style)), rect);
    }
    // 末尾新增动作行：键盘可达（Enter 打开空草稿），鼠标单击同样命中。
    let add_index = session.entries.len();
    if add_index >= start
        && let Some(rect) = row_rect(layout, add_index - start)
    {
        let selected = session.selected == add_index;
        let hovered = session.hover == Some(add_index);
        let marker = if selected { "❯ " } else { "" };
        let label = format!("{marker}{}", text::SUPPLEMENTS_ADD);
        let row_style = if selected {
            style::selected_item()
        } else if hovered {
            style::overlay_selection()
        } else {
            style::accent()
        };
        frame.render_widget(Paragraph::new(Span::styled(label, row_style)), rect);
    }
}

/// 详情：标题输入行 + 正文编辑器（固定原文视图，不压缩附件）。
fn render_detail(
    frame: &mut Frame<'_>,
    layout: &SupplementLayout,
    draft: &SupplementDraft,
) -> Option<(u16, u16)> {
    let title_rect = Rect::new(layout.body.x, layout.body.y, layout.body.width, 1);
    let title_style = if draft.title_focused {
        style::accent()
    } else {
        style::muted()
    };
    let title = if draft.title.text().is_empty() {
        text::SUPPLEMENTS_TITLE_PLACEHOLDER
    } else {
        draft.title.text()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::ellipsize(title, usize::from(title_rect.width)),
            title_style,
        )),
        title_rect,
    );
    let body = detail_body_rect(layout);
    if body.width == 0 || body.height == 0 {
        return None;
    }
    crate::ui::prompt::render(body, frame.buffer_mut(), &draft.body, None);
    let buttons = detail_buttons();
    let labels: Vec<&str> = buttons.iter().map(|(label, _)| *label).collect();
    let rects = widgets::modal::button_row(layout.buttons, &labels, 1, 0);
    for (rect, (label, button_style)) in rects.iter().zip(buttons) {
        frame.render_widget(Paragraph::new(Span::styled(label, button_style)), *rect);
    }
    if draft.title_focused {
        let col = draft
            .title
            .cursor()
            .min(usize::from(title_rect.width.saturating_sub(1)));
        return Some((title_rect.x + col as u16, title_rect.y));
    }
    let (row, col) = draft.body.cursor_cell()?;
    if row >= body.height {
        return None;
    }
    Some((body.x + col.min(body.width.saturating_sub(1)), body.y + row))
}

/// 未保存确认：提示行 + 三选一按钮；当前焦点按钮提亮反显。
fn render_unsaved(frame: &mut Frame<'_>, layout: &SupplementLayout, session: &Supplements) {
    let prompt = text::SUPPLEMENTS_UNSAVED_QUESTION;
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::ellipsize(prompt, usize::from(layout.body.width)),
            style::text(),
        )),
        Rect::new(layout.body.x, layout.body.y, layout.body.width, 1),
    );
    let buttons = unsaved_buttons();
    let labels: Vec<&str> = buttons.iter().map(|(label, _)| *label).collect();
    let rects = widgets::modal::button_row(layout.buttons, &labels, 1, 0);
    for (index, (rect, (label, button_style))) in rects.iter().zip(buttons).enumerate() {
        let focused = index == session.button_focus;
        let style = if focused {
            style::selected_item()
        } else {
            button_style
        };
        frame.render_widget(Paragraph::new(Span::styled(label, style)), *rect);
    }
}

/// 删除确认：显示序号与标题，默认焦点在取消。
fn render_confirm(frame: &mut Frame<'_>, layout: &SupplementLayout, session: &Supplements) {
    let label = session
        .selected_entry()
        .map(|entry| entry.label.clone())
        .unwrap_or_default();
    let question = format!(
        "{} {} — {}",
        text::SUPPLEMENTS_DELETE_QUESTION,
        session.selected + 1,
        label
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::ellipsize(&question, usize::from(layout.body.width)),
            style::text(),
        )),
        Rect::new(layout.body.x, layout.body.y, layout.body.width, 1),
    );
    let buttons = confirm_buttons();
    let labels: Vec<&str> = buttons.iter().map(|(label, _)| *label).collect();
    let rects = widgets::modal::button_row(layout.buttons, &labels, 1, 0);
    for (index, (rect, (label, button_style))) in rects.iter().zip(buttons).enumerate() {
        let focused = index == session.button_focus;
        let style = if focused {
            style::selected_item()
        } else {
            button_style
        };
        frame.render_widget(Paragraph::new(Span::styled(label, style)), *rect);
    }
}
