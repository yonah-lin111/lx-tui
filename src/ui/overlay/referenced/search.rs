//! 引用文件夹内容搜索弹窗：几何、命中与渲染。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::overlay::ReferencedFolderSearch;
use crate::app::referenced::menu_window_start;
use crate::ui::widgets;
use crate::ui::{style, text};

use super::{fill_row, paint_text};

/// 内容搜索弹窗标题行外的可视列表最小行数。
const SEARCH_MAX_HEIGHT: u16 = 20;
/// 内容搜索弹窗宽度（列）。
const SEARCH_WIDTH: u16 = 60;

/// 内容搜索弹窗几何：顶部范围行、搜索框、列表与底边提示。
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
    /// 列表可视行数。
    pub visible: usize,
    /// 窗口内首个候选索引。
    pub start: usize,
}

/// 内容搜索弹窗几何；屏幕放不下时返回 None。
pub fn referenced_search_layout(
    screen: Rect,
    search: &ReferencedFolderSearch,
) -> Option<ReferencedSearchLayout> {
    let width = SEARCH_WIDTH.min(screen.width);
    // 高度随候选增长（顶部范围行 + 搜索行 + 列表 + 底边），最小保证数行列表可用。
    let height = (u16::try_from(search.items.len())
        .unwrap_or(u16::MAX)
        .saturating_add(4))
    .clamp(8, SEARCH_MAX_HEIGHT)
    .min(screen.height);
    if width < 8 || height < 5 {
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
    let list_height = inner.height.saturating_sub(3);
    let list = Rect::new(inner.x, list_y, inner.width.saturating_sub(1), list_height);
    let visible = usize::from(list_height);
    // 显式视口优先（滚轮滚动，高亮允许不在窗口内）；否则窗口跟随高亮。
    let start = search
        .viewport
        .map(|offset| offset.min(search.items.len().saturating_sub(visible)))
        .unwrap_or_else(|| menu_window_start(search.selected, visible, search.items.len()));
    // 列表溢出时在最右一列显示滚动条。
    let scrollbar = widgets::scrollbar::layout(
        Rect {
            width: inner.width,
            ..list
        },
        search.items.len(),
        visible,
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
        visible,
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
    if !layout.list.contains((column, row).into()) {
        return None;
    }
    let offset = usize::from(row - layout.list.y);
    if offset >= layout.visible {
        return None;
    }
    let index = layout.start + offset;
    (index < search.items.len()).then_some(index)
}

/// 渲染内容搜索弹窗；返回搜索框硬件光标位置（不聚焦时 None）。
pub fn render_referenced_search(
    frame: &mut Frame<'_>,
    screen: Rect,
    search: &ReferencedFolderSearch,
) -> Option<(u16, u16)> {
    let layout = referenced_search_layout(screen, search)?;
    frame.render_widget(Clear, layout.area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title(Span::styled(
            format!(" {} ", text::REFERENCED_SEARCH_TITLE),
            style::border_title(),
        ));
    frame.render_widget(block, layout.area);
    // 顶部范围路径 + 右端原始路径复制按钮。
    let scope = text::ellipsize_middle(
        &search.root.display().to_string(),
        usize::from(layout.title.width).saturating_sub(usize::from(layout.copy_path.width)),
    );
    paint_text(frame, layout.title, &scope, style::accent());
    paint_text(
        frame,
        layout.copy_path,
        text::TEMPLATE_BUTTON_COPY,
        style::accent(),
    );
    // 搜索框。
    let cursor = crate::ui::widgets::input::render(
        frame,
        layout.query,
        search.query.text(),
        search.query.cursor(),
        Some(text::REFERENCED_SEARCH_PLACEHOLDER),
    );
    // 列表或状态提示。
    let status = if search.loading && search.entries.is_empty() {
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
            Paragraph::new(Span::styled(format!(" {status}"), style::muted())),
            Rect {
                height: 1,
                ..layout.list
            },
        );
    } else {
        render_search_rows(frame, &layout, search);
        if let Some(scrollbar) = layout.scrollbar {
            widgets::scrollbar::render(frame, &scrollbar);
        }
    }
    // 截断提示贴底边右端（底边左端留给快捷键提示，二者不重叠）。
    if search.truncated {
        let y = layout.area.bottom().saturating_sub(1);
        let width = u16::try_from(text::REFERENCED_SEARCH_TRUNCATED.width())
            .unwrap_or(u16::MAX)
            .min(layout.area.width.saturating_sub(4));
        let x = layout.area.right().saturating_sub(2 + width);
        paint_text(
            frame,
            Rect::new(x, y, width, 1),
            text::REFERENCED_SEARCH_TRUNCATED,
            style::muted(),
        );
    }
    cursor
}

/// 渲染可见候选行：图标 + 名称、缩进的父目录、选中底色。
fn render_search_rows(
    frame: &mut Frame<'_>,
    layout: &ReferencedSearchLayout,
    search: &ReferencedFolderSearch,
) {
    for (offset, index) in (layout.start..search.items.len())
        .take(layout.visible)
        .enumerate()
    {
        let Some(entry) = search.items.get(index) else {
            continue;
        };
        let y = layout.list.y + offset as u16;
        let row_area = Rect::new(layout.list.x, y, layout.list.width, 1);
        let base = if index == search.selected {
            style::overlay_selection()
        } else {
            style::overlay_panel()
        };
        fill_row(frame, row_area, base);
        let icon = if entry.is_directory {
            text::REFERENCED_DIR_ICON
        } else {
            text::REFERENCED_FILE_ICON
        };
        let suffix = if entry.is_directory { "/" } else { "" };
        let label = format!("{icon} {}{suffix}", entry.name);
        paint_text(
            frame,
            row_area,
            &text::truncate_to_width(&label, usize::from(row_area.width)),
            base.patch(if entry.is_directory {
                style::accent()
            } else {
                style::text()
            }),
        );
    }
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
        style::muted(),
    );
}
