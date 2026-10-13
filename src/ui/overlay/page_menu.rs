//! 页码菜单：行文案、几何、命中与渲染。
//!
//! 顶部边框右端内嵌 `[add]` / `[del]` 两个按钮（带 tooltip），其下每页占两行：
//! 首行 `❯` 标记与页名、次行彩色模板块计数；页数超出屏幕时窗口跟随光标滚动。

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::{TemplateCounts, TemplateStatus};
use crate::app::overlay::{PageMenu, PageMenuRow};
use crate::app::prompt::{PromptEditing, PromptPageId};
use crate::app::state::AppState;
use crate::ui::{style, text};

/// 页码菜单几何：整体区域、边框按钮与可见页行（每项两行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageMenuLayout {
    pub area: Rect,
    /// 顶部边框右端的新建按钮。
    pub new_rect: Rect,
    /// 顶部边框右端的删除按钮。
    pub delete_rect: Rect,
    /// 可见页行矩形（顺序对应窗口内页，每项高 2 行）。
    pub page_rects: Vec<Rect>,
    /// 窗口内第一页的页序号（0 起；行快照下标为其加一）。
    pub start: usize,
    /// 页总数（不含边框按钮）。
    pub pages: usize,
}

/// 一页的菜单文案：名称与彩色计数段。
struct PageLabel {
    name: String,
    counts: Vec<(String, TemplateStatus)>,
}

impl PageLabel {
    /// 名称显示宽度（含 `❯ ` 标记位）。
    fn name_width(&self) -> usize {
        self.name.width() + 2
    }

    /// 计数段显示宽度（含 1 列间隔）；无计数为 0。
    fn counts_width(&self) -> usize {
        self.counts
            .iter()
            .map(|(label, _)| label.width())
            .sum::<usize>()
            + self.counts.len().saturating_sub(1)
    }
}

/// 页菜单中一页的名称。
fn page_name(state: &AppState, menu: &PageMenu, page: PromptPageId) -> Option<String> {
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

/// 逐页文案（顺序与 `rows` 中的页行一致）。
fn page_labels(state: &AppState, menu: &PageMenu) -> Vec<PageLabel> {
    menu.rows
        .iter()
        .filter_map(|row| match row {
            PageMenuRow::Page { page, counts, .. } => Some(PageLabel {
                name: page_name(state, menu, *page).unwrap_or_default(),
                counts: count_segments(counts),
            }),
            _ => None,
        })
        .collect()
}

/// 计数段：零数量省略；按 todo / run / done 顺序。
fn count_segments(counts: &TemplateCounts) -> Vec<(String, TemplateStatus)> {
    [
        (TemplateStatus::Todo, counts.todo),
        (TemplateStatus::InProgress, counts.in_progress),
        (TemplateStatus::Done, counts.done),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(status, count)| (text::template_count_label(status, count), status))
    .collect()
}

/// 页码菜单可视页数（扣除上下边框，每页两行）。
pub fn page_menu_visible_pages(screen: Rect) -> usize {
    usize::from(screen.height.saturating_sub(2) / 2)
}

/// 页窗口起点：光标停在页行时保持该页可见（靠底部对齐）；按钮行光标贴两端。
fn page_window_start(menu: &PageMenu, visible: usize, pages: usize) -> usize {
    if visible == 0 || pages <= visible {
        return 0;
    }
    let last = menu.rows.len().saturating_sub(1);
    if menu.selected == 0 {
        return 0;
    }
    if menu.selected >= last {
        return pages - visible;
    }
    let ordinal = menu.selected.saturating_sub(1);
    (ordinal + 1).saturating_sub(visible).min(pages - visible)
}

/// 页码菜单几何；渲染与鼠标命中共用（与右键菜单同款钳制）。
pub fn page_menu_layout(state: &AppState, screen: Rect, menu: &PageMenu) -> PageMenuLayout {
    let labels = page_labels(state, menu);
    let pages = labels.len();
    let visible = page_menu_visible_pages(screen).min(pages);
    let start = page_window_start(menu, visible, pages);
    let end = (start + visible).min(pages);
    let page_width = labels[start..end]
        .iter()
        .map(|label| label.name_width().max(label.counts_width()))
        .max()
        .unwrap_or_default();
    // 顶部边框：左对齐标题 + 右端两个按钮，最小宽度保证不重叠。
    let chips = text::MENU_PAGE_NEW.width() + 1 + text::MENU_PAGE_DELETE.width();
    let top_width = text::MENU_TITLE_PAGES.width() + 2 + chips + 3;
    let desired = u16::try_from(page_width.max(top_width)).unwrap_or(u16::MAX);
    let interior = desired.min(screen.width.saturating_sub(2));
    let width = interior + 2;
    let shown = end - start;
    let height = u16::try_from(2 + shown * 2)
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
    let delete_width = u16::try_from(text::MENU_PAGE_DELETE.width()).unwrap_or(u16::MAX);
    let delete_rect = Rect::new(
        area.right().saturating_sub(1 + delete_width),
        area.y,
        delete_width,
        1,
    );
    let new_width = u16::try_from(text::MENU_PAGE_NEW.width()).unwrap_or(u16::MAX);
    let new_rect = Rect::new(
        delete_rect.x.saturating_sub(1 + new_width),
        area.y,
        new_width,
        1,
    );
    let page_rects = (0..shown)
        .map(|index| Rect::new(area.x + 1, area.y + 1 + (index as u16) * 2, interior, 2))
        .collect();
    PageMenuLayout {
        area,
        new_rect,
        delete_rect,
        page_rects,
        start,
        pages,
    }
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
    if layout.new_rect.contains((column, row).into()) {
        return Some(0);
    }
    if layout.delete_rect.contains((column, row).into()) {
        return Some(menu.rows.len().saturating_sub(1));
    }
    layout
        .page_rects
        .iter()
        .position(|rect| rect.contains((column, row).into()))
        .map(|index| layout.start + index + 1)
}

/// 渲染页码菜单：顶部边框按钮、两行页项、当前页 `❯` 标记与彩色计数。
#[allow(clippy::too_many_arguments)]
pub(super) fn render_page_menu(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    menu: &PageMenu,
) {
    let labels = page_labels(state, menu);
    let layout = page_menu_layout(state, screen, menu);
    frame.render_widget(ratatui::widgets::Clear, layout.area);
    let block = ratatui::widgets::Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title_alignment(Alignment::Left)
        .title(Span::styled(
            format!(" {} ", text::MENU_TITLE_PAGES),
            style::border_title(),
        ));
    frame.render_widget(block, layout.area);
    let last = menu.rows.len().saturating_sub(1);
    let delete_enabled = matches!(
        menu.rows.get(last),
        Some(PageMenuRow::DeletePage { enabled: true })
    );
    render_chip(
        frame,
        layout.new_rect,
        text::MENU_PAGE_NEW,
        state.menu_hover && menu.selected == 0,
        false,
    );
    render_chip(
        frame,
        layout.delete_rect,
        text::MENU_PAGE_DELETE,
        state.menu_hover && menu.selected == last,
        !delete_enabled,
    );
    for (index, rect) in layout.page_rects.iter().enumerate() {
        let row_index = layout.start + index + 1;
        let Some(PageMenuRow::Page { current, .. }) = menu.rows.get(row_index) else {
            continue;
        };
        let Some(label) = labels.get(layout.start + index) else {
            continue;
        };
        let selected = state.menu_hover && menu.selected == row_index;
        let base = if selected {
            style::overlay_selection()
        } else {
            style::overlay_panel()
        };
        fill_row(frame, *rect, base);
        if *current {
            paint_text(
                frame,
                *rect,
                &format!("{} ", text::AGENT_SELECTED_MARKER),
                base,
            );
        }
        paint_text(
            frame,
            Rect {
                x: rect.x + 2,
                ..*rect
            },
            &label.name,
            base,
        );
        let mut x = rect.x + 2;
        for (index, (segment, status)) in label.counts.iter().enumerate() {
            if index > 0 {
                x += 1;
            }
            let segment_style = style::template_status(*status).patch(base);
            paint_text(
                frame,
                Rect {
                    x,
                    y: rect.y + 1,
                    ..*rect
                },
                segment,
                segment_style,
            );
            x += segment.width() as u16;
        }
    }
}

/// 边框按钮：文字覆盖在顶部边框上、贴右端；选中反显、禁用置灰。
fn render_chip(frame: &mut Frame<'_>, rect: Rect, label: &str, selected: bool, disabled: bool) {
    let base = if selected {
        style::overlay_selection()
    } else {
        style::accent()
    };
    let text_style = if disabled {
        base.add_modifier(Modifier::DIM)
    } else {
        base
    };
    paint_text(frame, rect, label, text_style);
}

/// 整行底色：选中/悬停行铺满，保证反馈跨满行宽。
fn fill_row(frame: &mut Frame<'_>, rect: Rect, row_style: Style) {
    for y in rect.y..rect.y.saturating_add(rect.height) {
        for x in rect.x..rect.right() {
            if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                cell.reset();
                cell.set_char(' ');
                cell.set_style(row_style);
            }
        }
    }
}

/// 行内文本绘制：超出右界整体丢弃。
fn paint_text(frame: &mut Frame<'_>, rect: Rect, content: &str, cell_style: Style) {
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
