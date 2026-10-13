//! 页码菜单：行文案、几何、命中与渲染。
//!
//! 行数超出屏幕时窗口跟随高亮滚动；当前页整行高亮（Prompt 列表选中语义）。

use crate::app::overlay::{PageMenu, PageMenuRow};
use crate::app::prompt::{PromptEditing, PromptPageId};
use crate::app::state::AppState;
use crate::ui::widgets;
use crate::ui::{style, text};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;
/// 页码菜单行文案（名称 + 有值的彩色计数）；渲染与几何共用。
fn page_menu_labels(state: &AppState, menu: &PageMenu) -> Vec<String> {
    menu.rows
        .iter()
        .map(|row| match row {
            PageMenuRow::NewPage => text::MENU_PAGE_NEW.to_string(),
            PageMenuRow::DeletePage { .. } => text::MENU_PAGE_DELETE.to_string(),
            PageMenuRow::Page { page, counts, .. } => {
                let mut label = page_menu_page_name(state, menu, *page).unwrap_or_default();
                for (status, count) in [
                    (crate::app::markdown::TemplateStatus::Todo, counts.todo),
                    (
                        crate::app::markdown::TemplateStatus::InProgress,
                        counts.in_progress,
                    ),
                    (crate::app::markdown::TemplateStatus::Done, counts.done),
                ] {
                    if count > 0 {
                        label.push(' ');
                        label.push_str(&text::template_count_label(status, count));
                    }
                }
                label
            }
        })
        .collect()
}

/// 页菜单中一页的名称。
fn page_menu_page_name(state: &AppState, menu: &PageMenu, page: PromptPageId) -> Option<String> {
    let panel = &state.workspaces.get(menu.workspace)?.prompt_panel;
    let pages = match menu.target {
        PromptEditing::Item(id) => &panel.tree.item(id)?.pages,
        PromptEditing::Temp(id) => &panel.temp(id)?.pages,
    };
    pages
        .iter()
        .find(|candidate| candidate.id == page)
        .map(|page| page.name.clone())
}

/// 页码菜单可视行数（扣除上下边框）。
pub fn page_menu_max_rows(screen: Rect) -> usize {
    usize::from(screen.height.saturating_sub(2))
}

/// 页码菜单窗口起点：高亮行保持可见，窗口内滚动而非溢出。
pub fn page_menu_visible_start(menu: &PageMenu, max_rows: usize) -> usize {
    if max_rows == 0 {
        return 0;
    }
    menu.selected
        .saturating_add(1)
        .saturating_sub(max_rows)
        .min(menu.rows.len().saturating_sub(max_rows))
}

/// 页码菜单几何；渲染与鼠标命中共用（与右键菜单同款钳制与项矩形）。
/// 行数超出屏幕高度时只布局高亮所在窗口。
pub fn page_menu_layout(
    state: &AppState,
    screen: Rect,
    menu: &PageMenu,
) -> widgets::menu::MenuLayout {
    let labels = page_menu_labels(state, menu);
    let max_rows = page_menu_max_rows(screen);
    let start = page_menu_visible_start(menu, max_rows);
    let end = (start + max_rows).min(labels.len());
    let refs: Vec<&str> = labels[start..end].iter().map(String::as_str).collect();
    widgets::menu::layout(screen, menu.anchor, text::MENU_TITLE_PAGES, &refs)
}

/// 命中页码菜单行快照索引（含窗口偏移）；未命中返回 None。
pub fn page_menu_item_at(
    state: &AppState,
    screen: Rect,
    menu: &PageMenu,
    column: u16,
    row: u16,
) -> Option<usize> {
    let layout = page_menu_layout(state, screen, menu);
    let max_rows = page_menu_max_rows(screen);
    let start = page_menu_visible_start(menu, max_rows);
    widgets::menu::item_at(&layout, column, row).map(|index| start + index)
}

/// 渲染页码菜单：整行底色、当前页高亮、逐页彩色计数；删除禁用项置灰。
pub(super) fn render_page_menu(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    menu: &PageMenu,
) {
    let labels = page_menu_labels(state, menu);
    let max_rows = page_menu_max_rows(screen);
    let start = page_menu_visible_start(menu, max_rows);
    let end = (start + max_rows).min(labels.len());
    let refs: Vec<&str> = labels[start..end].iter().map(String::as_str).collect();
    let layout = widgets::menu::layout(screen, menu.anchor, text::MENU_TITLE_PAGES, &refs);
    frame.render_widget(ratatui::widgets::Clear, layout.area);
    let block = ratatui::widgets::Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title(Span::styled(
            format!(" {} ", text::MENU_TITLE_PAGES),
            style::border_title(),
        ));
    frame.render_widget(block, layout.area);
    for (visible, row) in menu.rows.iter().enumerate().skip(start).take(end - start) {
        let Some(rect) = layout.item_rects.get(visible - start) else {
            continue;
        };
        let selected = visible == menu.selected;
        let current = matches!(row, PageMenuRow::Page { current: true, .. });
        let base = if selected {
            style::overlay_selection()
        } else if current {
            style::selection()
        } else {
            style::overlay_panel()
        };
        fill_menu_row(frame, *rect, base);
        match row {
            PageMenuRow::NewPage => {
                paint_menu_text(frame, *rect, text::MENU_PAGE_NEW, base);
            }
            PageMenuRow::DeletePage { enabled } => {
                let row_style = if selected || *enabled {
                    base
                } else {
                    style::overlay_panel().add_modifier(ratatui::style::Modifier::DIM)
                };
                paint_menu_text(frame, *rect, text::MENU_PAGE_DELETE, row_style);
            }
            PageMenuRow::Page { page, counts, .. } => {
                render_page_menu_row(
                    frame,
                    *rect,
                    state,
                    menu,
                    *page,
                    counts,
                    base,
                    selected || current,
                );
            }
        }
    }
}

/// 页行绘制：名称左对齐、计数右对齐；计数按状态取色，高亮行统一用高亮前景。
#[allow(clippy::too_many_arguments)]
fn render_page_menu_row(
    frame: &mut Frame<'_>,
    rect: Rect,
    state: &AppState,
    menu: &PageMenu,
    page: PromptPageId,
    counts: &crate::app::markdown::TemplateCounts,
    base: ratatui::style::Style,
    highlighted: bool,
) {
    let name = page_menu_page_name(state, menu, page).unwrap_or_default();
    let mut segments: Vec<(String, ratatui::style::Style)> = Vec::new();
    for (status, count) in [
        (crate::app::markdown::TemplateStatus::Todo, counts.todo),
        (
            crate::app::markdown::TemplateStatus::InProgress,
            counts.in_progress,
        ),
        (crate::app::markdown::TemplateStatus::Done, counts.done),
    ] {
        if count == 0 {
            continue;
        }
        let segment_style = if highlighted {
            base
        } else {
            style::template_status(status)
        };
        segments.push((text::template_count_label(status, count), segment_style));
    }
    // 计数占位优先于长名称：右侧放完整计数，名称占剩余宽度并按显示列截断。
    let right_width: usize = segments
        .iter()
        .map(|(label, _)| label.width())
        .sum::<usize>()
        + segments.len().saturating_sub(1);
    let gap = usize::from(right_width > 0);
    let name_width = usize::from(rect.width).saturating_sub(right_width + gap);
    let name_text = text::truncate_to_width(&name, name_width);
    paint_menu_text(frame, rect, &name_text, base);
    let mut x = rect.right().saturating_sub(right_width as u16);
    for (index, (label, segment_style)) in segments.iter().enumerate() {
        if index > 0 {
            x = x.saturating_add(1);
        }
        paint_menu_text(frame, Rect { x, ..rect }, label, *segment_style);
        x = x.saturating_add(label.width() as u16);
    }
}

/// 整行底色：选中/当前页行铺满，保证高亮跨满行宽。
fn fill_menu_row(frame: &mut Frame<'_>, rect: Rect, row_style: ratatui::style::Style) {
    for x in rect.x..rect.right() {
        if let Some(cell) = frame.buffer_mut().cell_mut((x, rect.y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(row_style);
        }
    }
}

/// 行内文本绘制：超出右界整体丢弃。
fn paint_menu_text(
    frame: &mut Frame<'_>,
    rect: Rect,
    content: &str,
    cell_style: ratatui::style::Style,
) {
    let mut x = rect.x;
    for ch in content.chars() {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if x.saturating_add(width) > rect.right() {
            break;
        }
        if let Some(cell) = frame.buffer_mut().cell_mut((x, rect.y)) {
            cell.reset();
            cell.set_char(ch);
            cell.set_style(cell_style);
        }
        x = x.saturating_add(width);
    }
}
