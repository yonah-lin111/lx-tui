//! 引用文件夹内容搜索弹窗：几何、命中与渲染。
//!
//! 顶部范围路径行（右端复制原始路径）、单行搜索框与双行候选列表（首行图标 + 名称、
//! 次行 `└─ ` 父目录地址）；进入子目录后顶边右端显示当前文件夹；底边为快捷键提示。

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::overlay::ReferencedFolderSearch;
use crate::app::referenced::{ReferencedEntry, menu_window_start};
use crate::ui::widgets;
use crate::ui::{style, text};

use super::{fill_row, paint_text};

/// 内容搜索弹窗宽度（列）。
const SEARCH_WIDTH: u16 = 72;
/// 内容搜索弹窗高度（行）；固定高度，不随候选数量变化。
const SEARCH_HEIGHT: u16 = 18;
/// 每条候选占用的行数（名称行 + 地址行）。
const ROW_HEIGHT: u16 = 2;

/// 内容搜索弹窗几何：顶部范围行、搜索框、双行列表与滚动条。
pub struct ReferencedSearchLayout {
    pub area: Rect,
    pub inner: Rect,
    /// 顶部范围路径行。
    pub title: Rect,
    /// 顶部右端原始路径复制按钮。
    pub copy_path: Rect,
    /// 搜索框行。
    pub query: Rect,
    /// 列表区（不含滚动条列）。
    pub list: Rect,
    /// 列表滚动条；不溢出时为 None。
    pub scrollbar: Option<widgets::scrollbar::ScrollbarLayout>,
    /// 可见候选及其两行矩形。
    pub rows: Vec<ReferencedSearchRow>,
    /// 窗口内首个候选索引。
    pub start: usize,
}

/// 一条可见候选：索引与两行矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferencedSearchRow {
    pub index: usize,
    pub row: Rect,
}

impl ReferencedSearchLayout {
    /// 列表可见候选数。
    pub fn visible(&self) -> usize {
        self.rows.len()
    }
}

/// 内容搜索弹窗几何；屏幕放不下时返回 None。
pub fn referenced_search_layout(
    screen: Rect,
    search: &ReferencedFolderSearch,
) -> Option<ReferencedSearchLayout> {
    let width = SEARCH_WIDTH.min(screen.width);
    // 高度固定（顶部范围行 + 搜索行 + 双行候选列表 + 上下边框），不随候选数量伸缩。
    let height = SEARCH_HEIGHT.min(screen.height);
    if width < 8 || height < 6 {
        return None;
    }
    let x = screen.x + (screen.width - width) / 2;
    let y = screen.y + (screen.height - height) / 2;
    let area = Rect::new(x, y, width, height);
    let inner = Block::bordered().inner(area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let title = Rect::new(inner.x, inner.y, inner.width, 1);
    let copy_width = u16::try_from(text::TEMPLATE_BUTTON_COPY.width()).unwrap_or(3);
    let copy_path = Rect::new(
        title.right().saturating_sub(copy_width),
        title.y,
        copy_width,
        1,
    );
    let query = Rect::new(inner.x, inner.y.saturating_add(1), inner.width, 1);
    let list_y = inner.y.saturating_add(2);
    let list_height = inner.height.saturating_sub(2);
    let list = Rect::new(inner.x, list_y, inner.width.saturating_sub(1), list_height);
    let visible = usize::from(list_height / ROW_HEIGHT);
    // 显式视口优先（滚轮滚动，高亮允许不在窗口内）；否则窗口跟随高亮。
    let max_start = search.items.len().saturating_sub(visible);
    let start = search
        .viewport
        .map(|offset| offset.min(max_start))
        .unwrap_or_else(|| menu_window_start(search.selected, visible, search.items.len()));
    let shown = visible.min(search.items.len().saturating_sub(start));
    let rows = (0..shown)
        .map(|offset| ReferencedSearchRow {
            index: start + offset,
            row: Rect::new(
                list.x,
                list.y + (offset as u16).saturating_mul(ROW_HEIGHT),
                list.width,
                ROW_HEIGHT,
            ),
        })
        .collect();
    // 列表溢出时在最右一列显示滚动条。
    let scrollbar = widgets::scrollbar::layout(
        Rect {
            width: inner.width,
            ..list
        },
        search.items.len(),
        visible.max(1),
        start,
    );
    Some(ReferencedSearchLayout {
        area,
        inner,
        title,
        copy_path,
        query,
        list,
        scrollbar,
        rows,
        start,
    })
}

/// 命中内容搜索弹窗右上角原始路径复制按钮。
pub fn referenced_search_copy_at(
    screen: Rect,
    search: &ReferencedFolderSearch,
    column: u16,
    row: u16,
) -> bool {
    referenced_search_layout(screen, search)
        .is_some_and(|layout| layout.copy_path.contains((column, row).into()))
}

/// 命中内容搜索弹窗列表项；返回候选索引。
pub fn referenced_search_item_at(
    screen: Rect,
    search: &ReferencedFolderSearch,
    column: u16,
    row: u16,
) -> Option<usize> {
    let layout = referenced_search_layout(screen, search)?;
    layout
        .rows
        .iter()
        .find(|line| line.row.contains((column, row).into()))
        .map(|line| line.index)
}

/// 渲染内容搜索弹窗；返回搜索框硬件光标位置（不聚焦时 None）。
pub fn render_referenced_search(
    frame: &mut Frame<'_>,
    screen: Rect,
    search: &ReferencedFolderSearch,
) -> Option<(u16, u16)> {
    let layout = referenced_search_layout(screen, search)?;
    widgets::clear::render(frame, layout.area);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title(Span::styled(
            format!(" {} ", text::REFERENCED_SEARCH_TITLE),
            style::border_title(),
        ));
    // 进入子目录后顶边右端显示当前文件夹名。
    if let Some(scope) = search_scope_name(search) {
        let available = usize::from(layout.area.width.saturating_sub(2))
            .saturating_sub(text::REFERENCED_SEARCH_TITLE.width() + 4);
        block = block.title_top(
            Line::from(Span::styled(
                format!(" {} ", text::truncate_to_width(&scope, available)),
                style::border_title(),
            ))
            .alignment(Alignment::Right),
        );
    }
    frame.render_widget(block, layout.area);
    // 顶部范围路径 + 右端原始路径复制按钮；截断提示紧贴复制按钮左侧（始终可见）。
    let truncated = search
        .truncated
        .then_some(text::REFERENCED_SEARCH_TRUNCATED);
    let reserved = layout
        .copy_path
        .width
        .saturating_add(u16::try_from(truncated.map_or(0, |label| label.width() + 1)).unwrap_or(0));
    let scope_path = text::ellipsize_middle(
        &search.root.display().to_string(),
        usize::from(layout.title.width).saturating_sub(usize::from(reserved)),
    );
    paint_text(frame, layout.title, &scope_path, style::accent());
    if let Some(label) = truncated {
        let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
        paint_text(
            frame,
            Rect::new(
                layout.copy_path.x.saturating_sub(1 + width),
                layout.title.y,
                width,
                1,
            ),
            label,
            style::muted(),
        );
    }
    paint_text(
        frame,
        layout.copy_path,
        text::TEMPLATE_BUTTON_COPY,
        style::accent(),
    );
    // 搜索框（模态单行输入统一样式：`>` 前缀 + 视口）。
    let cursor = super::super::dialog::render_input(
        frame,
        layout.query,
        search.query.text(),
        search.query.cursor(),
        search.query.selection(),
        Some(text::REFERENCED_SEARCH_PLACEHOLDER),
    );
    // 列表或状态提示。
    let status = if search.loading && search.items.is_empty() {
        Some(text::REFERENCED_SEARCH_LOADING)
    } else if search.failed {
        Some(text::REFERENCED_SEARCH_FAILED)
    } else if search.items.is_empty() {
        Some(text::REFERENCED_SEARCH_EMPTY)
    } else {
        None
    };
    if let Some(status) = status {
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {status}"), style::muted()))
                .style(style::overlay_panel()),
            Rect {
                height: 1,
                ..layout.list
            },
        );
    } else {
        render_search_rows(frame, &layout, search);
        if let Some(scrollbar) = layout.scrollbar {
            widgets::scrollbar::render_with_base(frame, &scrollbar, style::overlay_panel());
        }
    }
    cursor
}

/// 可见候选行：首行图标 + 名称（目录带 `/`），次行 `└─ ` 父目录地址；选中行铺底色。
fn render_search_rows(
    frame: &mut Frame<'_>,
    layout: &ReferencedSearchLayout,
    search: &ReferencedFolderSearch,
) {
    for line in &layout.rows {
        let Some(entry) = search.items.get(line.index) else {
            continue;
        };
        let base = if line.index == search.selected {
            style::overlay_selection()
        } else {
            style::overlay_panel()
        };
        fill_row(frame, line.row, base);
        render_entry_row(frame, line.row, entry, base);
    }
}

/// 单条候选的两行绘制；底色沿用 `base`。
fn render_entry_row(frame: &mut Frame<'_>, row: Rect, entry: &ReferencedEntry, base: Style) {
    if row.width < 3 {
        return;
    }
    let (icon, icon_style, label_style) = if entry.is_directory {
        (
            text::REFERENCED_DIR_ICON,
            style::directory(),
            style::directory(),
        )
    } else {
        (text::REFERENCED_FILE_ICON, style::muted(), style::text())
    };
    let suffix = if entry.is_directory { "/" } else { "" };
    let name_row = Rect { height: 1, ..row };
    paint_text(frame, name_row, icon, base.patch(icon_style));
    let name_area = Rect {
        x: row.x.saturating_add(2),
        width: row.width.saturating_sub(2),
        ..name_row
    };
    paint_text(
        frame,
        name_area,
        &text::truncate_to_width(
            &format!("{}{suffix}", entry.name),
            usize::from(name_area.width),
        ),
        base.patch(label_style),
    );
    // 次行：地址（父目录绝对路径，按显示列中间截断）。
    let detail_row = Rect {
        y: row.y.saturating_add(1),
        height: 1,
        ..row
    };
    let address = referenced_entry_address(entry, usize::from(detail_row.width.saturating_sub(2)));
    paint_text(
        frame,
        Rect {
            x: detail_row.x.saturating_add(2),
            width: detail_row.width.saturating_sub(2),
            ..detail_row
        },
        &address,
        base.patch(style::muted()),
    );
}

/// 当前进入的文件夹名（末段）；仍在初始根时 None。
fn search_scope_name(search: &ReferencedFolderSearch) -> Option<String> {
    if search.stack.is_empty() {
        return None;
    }
    search
        .root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// 候选地址行：`└─ ` 前缀 + 父目录绝对路径（中间截断）。
fn referenced_entry_address(entry: &ReferencedEntry, width: usize) -> String {
    let parent = entry
        .path
        .parent()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let prefix = format!("{} ", text::WORKSPACE_TREE_LAST);
    let available = width.saturating_sub(prefix.width());
    format!("{prefix}{}", text::ellipsize_middle(&parent, available))
}

/// 渲染内容搜索弹窗底边快捷键提示（贴底边，覆盖边框上）。
pub fn render_referenced_search_footer(
    frame: &mut Frame<'_>,
    screen: Rect,
    search: &ReferencedFolderSearch,
) {
    let Some(layout) = referenced_search_layout(screen, search) else {
        return;
    };
    let y = layout.area.bottom().saturating_sub(1);
    paint_text(
        frame,
        Rect::new(layout.area.x + 2, y, layout.area.width.saturating_sub(4), 1),
        text::REFERENCED_SEARCH_FOOTER,
        style::muted_on_border(),
    );
}
