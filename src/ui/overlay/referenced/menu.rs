//! 引用文件夹目录菜单：几何、命中与渲染。

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear};
use unicode_width::UnicodeWidthStr;

use crate::app::overlay::{ReferencedFolderMenu, ReferencedFolderMenuRow};
use crate::app::referenced::menu_window_start;
use crate::ui::{style, text};

use super::{fill_row, paint_text};

/// 目录菜单几何：整体区域、顶边 `[add]` 与可见行（每行一个主行 + 右侧两个动作）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedMenuLayout {
    pub area: Rect,
    /// 顶边右端的新建按钮。
    pub add_rect: Rect,
    /// 可见行（顺序对应窗口内行）。
    pub rows: Vec<ReferencedMenuRowLayout>,
    /// 窗口内首行的行快照下标。
    pub start: usize,
    /// 行总数（不含 `[add]`）。
    pub total: usize,
}

/// 目录菜单一行：主行命中区、复制与删除命中区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferencedMenuRowLayout {
    /// 主行整行（含左端名称）；点击打开搜索弹窗。
    pub main: Rect,
    /// 右端复制按钮命中区。
    pub copy: Rect,
    /// 右端删除按钮命中区。
    pub delete: Rect,
}

/// 目录菜单一项的展示宽度片段：名称与其右侧动作区。
fn menu_row_text(row: &ReferencedFolderMenuRow) -> (String, bool) {
    match row {
        ReferencedFolderMenuRow::Empty => (text::REFERENCED_MENU_EMPTY.to_string(), true),
        ReferencedFolderMenuRow::Unavailable => (text::REFERENCED_DB_UNAVAILABLE.to_string(), true),
        ReferencedFolderMenuRow::Add => (String::new(), false),
        ReferencedFolderMenuRow::Folder { path, .. } => (display_name(path), false),
    }
}

/// 目录末段名；无末段时回退完整路径。
fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

/// 目录菜单可视行数（扣除上下边框）。
pub fn referenced_menu_visible_rows(screen: Rect) -> usize {
    usize::from(screen.height.saturating_sub(2))
}

/// 目录菜单行数（`Empty` 计一行，`Add` 不计入列表）。
fn menu_row_count(menu: &ReferencedFolderMenu) -> usize {
    menu.rows
        .iter()
        .filter(|row| !matches!(row, ReferencedFolderMenuRow::Add))
        .count()
}

/// 目录菜单几何；渲染与鼠标命中共用。
pub fn referenced_menu_layout(screen: Rect, menu: &ReferencedFolderMenu) -> ReferencedMenuLayout {
    let total = menu_row_count(menu);
    let visible = referenced_menu_visible_rows(screen).min(total.max(1));
    let selected_in_list = menu.selected.min(total.saturating_sub(1));
    let start = menu_window_start(selected_in_list, visible, total.max(1));
    let shown = visible
        .min(total.saturating_sub(start))
        .max(if total == 0 { 1 } else { 0 });
    let names_max = menu
        .rows
        .iter()
        .map(|row| menu_row_text(row).0.width())
        .max()
        .unwrap_or_default();
    // 行内容 = 左端名称 + 至少 1 列间隔 + 右端 [❐]+空档+[✕]。
    let action_width = text::TEMPLATE_BUTTON_COPY.width() + 1 + text::TEMPLATE_BUTTON_DEL.width();
    let top_width =
        text::MENU_TITLE_REFERENCED_FOLDERS.width() + 2 + text::REFERENCED_MENU_ADD.width() + 3;
    let row_width = names_max + 2 + action_width;
    let desired = u16::try_from(row_width.max(top_width)).unwrap_or(u16::MAX);
    let interior = desired.min(screen.width.saturating_sub(2));
    let width = interior + 2;
    let height = u16::try_from(2 + shown)
        .unwrap_or(u16::MAX)
        .min(screen.height);
    let x = menu
        .anchor
        .0
        .clamp(screen.x, screen.x + screen.width.saturating_sub(width));
    let y = menu
        .anchor
        .1
        .clamp(screen.y, screen.y + screen.height.saturating_sub(height));
    let area = Rect::new(x, y, width, height);
    let add_width = u16::try_from(text::REFERENCED_MENU_ADD.width()).unwrap_or(u16::MAX);
    let add_rect = Rect::new(
        area.right().saturating_sub(1 + add_width),
        area.y,
        add_width,
        1,
    );
    let delete_width = u16::try_from(text::TEMPLATE_BUTTON_DEL.width()).unwrap_or(u16::MAX);
    let copy_width = u16::try_from(text::TEMPLATE_BUTTON_COPY.width()).unwrap_or(u16::MAX);
    let rows = (0..shown)
        .map(|index| {
            let y = area.y + 1 + index as u16;
            let row = Rect::new(area.x + 1, y, interior, 1);
            let delete = Rect::new(row.right().saturating_sub(delete_width), y, delete_width, 1);
            let copy = Rect::new(delete.x.saturating_sub(1 + copy_width), y, copy_width, 1);
            ReferencedMenuRowLayout {
                main: row,
                copy,
                delete,
            }
        })
        .collect();
    ReferencedMenuLayout {
        area,
        add_rect,
        rows,
        start,
        total,
    }
}

/// 目录菜单动作命中：行快照下标 + 动作种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferencedMenuAction {
    /// 顶边 `[add]`。
    Add,
    /// 某行的主区域：打开搜索弹窗。
    Open(usize),
    /// 某行的复制按钮。
    Copy(usize),
    /// 某行的删除按钮。
    Delete(usize),
}

/// 命中目录菜单；未命中返回 None。
pub fn referenced_menu_hit_at(
    screen: Rect,
    menu: &ReferencedFolderMenu,
    column: u16,
    row: u16,
) -> Option<ReferencedMenuAction> {
    let layout = referenced_menu_layout(screen, menu);
    if layout.add_rect.contains((column, row).into()) {
        return Some(ReferencedMenuAction::Add);
    }
    for (offset, line) in layout.rows.iter().enumerate() {
        let at = layout.start + offset;
        if line.delete.contains((column, row).into()) {
            return Some(ReferencedMenuAction::Delete(at));
        }
        if line.copy.contains((column, row).into()) {
            return Some(ReferencedMenuAction::Copy(at));
        }
        if line.main.contains((column, row).into()) {
            return Some(ReferencedMenuAction::Open(at));
        }
    }
    None
}

/// 渲染目录菜单：顶边标题与 `[add]`、逐行名称与右端动作按钮。
pub fn render_referenced_menu(frame: &mut Frame<'_>, screen: Rect, menu: &ReferencedFolderMenu) {
    let layout = referenced_menu_layout(screen, menu);
    frame.render_widget(Clear, layout.area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title_alignment(Alignment::Left)
        .title(Span::styled(
            format!(" {} ", text::MENU_TITLE_REFERENCED_FOLDERS),
            style::border_title(),
        ));
    frame.render_widget(block, layout.area);
    render_chip(frame, layout.add_rect, text::REFERENCED_MENU_ADD, false);
    for (offset, line) in layout.rows.iter().enumerate() {
        let index = layout.start + offset;
        let Some(row) = menu.rows.get(index) else {
            continue;
        };
        let (label, muted) = menu_row_text(row);
        let selected = menu.selected == index && !muted;
        let base = if selected {
            style::overlay_selection()
        } else {
            style::overlay_panel()
        };
        fill_row(frame, line.main, base);
        let name_style = if muted {
            base.patch(style::muted())
        } else {
            base
        };
        let name_area = Rect {
            width: line.main.width.saturating_sub(
                2 + u16::try_from(
                    text::TEMPLATE_BUTTON_COPY.width() + text::TEMPLATE_BUTTON_DEL.width(),
                )
                .unwrap_or(0),
            ),
            ..line.main
        };
        paint_text(
            frame,
            name_area,
            &text::truncate_to_width(&label, usize::from(name_area.width)),
            name_style,
        );
        if !muted {
            render_chip_row(frame, line.copy, text::TEMPLATE_BUTTON_COPY, base);
            render_chip_row(frame, line.delete, text::TEMPLATE_BUTTON_DEL, base);
        }
    }
}

/// 边框按钮：文字覆盖在顶部边框上、贴右端；不可用时置灰。
fn render_chip(frame: &mut Frame<'_>, rect: Rect, label: &str, disabled: bool) {
    let text_style = style::overlay_panel().patch(if disabled {
        style::muted()
    } else {
        style::accent()
    });
    paint_text(frame, rect, label, text_style);
}

/// 行内动作按钮：文字底沿用当前行底色。
fn render_chip_row(frame: &mut Frame<'_>, rect: Rect, label: &str, base: Style) {
    paint_text(frame, rect, label, base.patch(style::accent()));
}
