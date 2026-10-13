//! prompt 标题行渲染与页控件命中：只读状态，无 IO。
//!
//! `title · 页名 ‹ n/m ›` 内嵌于分割线行；页控件右对齐，放不下整组时隐藏。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::prompt::{Prompt, PromptMode};
use crate::layout;
use crate::ui::{style, text};

use super::{render_divider, render_toolbar};

/// 标题行页控件数据：当前页序号（从 1 起）、总页数、页名与重命名输入现场。
pub struct PageHeaderData<'a> {
    pub current: usize,
    pub total: usize,
    pub name: &'a str,
    /// 页重命名输入：文本与光标字符索引；Some 时页名位置渲染为输入。
    pub rename: Option<(&'a str, usize)>,
}

/// 标题行页控件：左箭头、计数、右箭头。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptPageControls {
    pub prev: Rect,
    pub counter: Rect,
    pub next: Rect,
}

/// 标题行页控件命中种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageControl {
    Prev,
    Next,
    Counter,
    Name,
}

/// 页控件几何：右对齐 `‹ 2/4 ›`（箭头各 1 列、两侧空档各 1 列），
/// 整组放不下时隐藏（返回 None）；左侧至少保留 2 列标题/页名空间。
pub fn page_controls(header: Rect, counter: &str) -> Option<PromptPageControls> {
    let counter_width = u16::try_from(UnicodeWidthStr::width(counter)).unwrap_or(u16::MAX);
    let total = counter_width.saturating_add(4);
    let content = header.width.saturating_sub(2);
    if content < total.saturating_add(2) {
        return None;
    }
    let end = header.right().saturating_sub(1);
    let y = header.y.saturating_add(layout::PROMPT_TOOLBAR_HEIGHT);
    let next = Rect::new(end.saturating_sub(1), y, 1, 1);
    let counter = Rect::new(
        next.x.saturating_sub(1).saturating_sub(counter_width),
        y,
        counter_width,
        1,
    );
    let prev = Rect::new(counter.x.saturating_sub(2), y, 1, 1);
    Some(PromptPageControls {
        prev,
        counter,
        next,
    })
}

/// 标题行左侧（页控件之前）的可用矩形；无控件时为整行内容区。
fn header_title_rect(header: Rect, controls: Option<&PromptPageControls>) -> Rect {
    let end = controls
        .map(|controls| controls.prev.x.saturating_sub(1))
        .unwrap_or_else(|| header.right().saturating_sub(1));
    Rect {
        x: header.x.saturating_add(1),
        y: header.y.saturating_add(layout::PROMPT_TOOLBAR_HEIGHT),
        width: end.saturating_sub(header.x.saturating_add(1)),
        height: 1,
    }
}

/// 绘制 prompt 顶部工具栏与固定分割线；内容区过矮时不绘制；
/// 编辑视图在分割线行内嵌编辑目标 title 与当前页控件（`title · 页名 ‹ n/m ›`）。
#[allow(clippy::too_many_arguments)]
pub fn render_header(
    panel: Rect,
    buf: &mut Buffer,
    prompt: &Prompt,
    focused: bool,
    mode: PromptMode,
    title: Option<&str>,
    draft: bool,
    pinned: bool,
    page: Option<PageHeaderData<'_>>,
) {
    let Some(header) = layout::prompt_header_rect(panel) else {
        return;
    };
    render_divider(panel, header, buf, focused);
    if mode == PromptMode::Editor {
        let controls = page
            .as_ref()
            .and_then(|page| page_controls(header, &text::page_counter(page.current, page.total)));
        paint_header_title(header, buf, title, draft, page.as_ref(), controls.as_ref());
        if let (Some(page), Some(controls)) = (page.as_ref(), controls.as_ref()) {
            paint_page_controls(buf, header, controls, page);
        }
    }
    render_toolbar(panel, buf, prompt, mode, draft, pinned);
}

/// 页重命名输入的硬件光标位置；输入区放不下时返回 None。
pub fn header_rename_cursor(
    header: Rect,
    title: &str,
    draft: bool,
    page: &PageHeaderData<'_>,
) -> Option<(u16, u16)> {
    let (input, cursor) = page.rename?;
    let controls = page_controls(header, &text::page_counter(page.current, page.total));
    let area = header_title_rect(header, controls.as_ref());
    if area.width == 0 {
        return None;
    }
    let badge = if draft {
        format!("{} ", text::PROMPT_TEMP_BADGE)
    } else {
        String::new()
    };
    let visible = format!(" {title} {badge}");
    let max_prefix = usize::from(area.width).saturating_sub(4);
    let prefix = text::truncate_to_width(&visible, max_prefix);
    let x = area.x.saturating_add(prefix.width() as u16 + 2);
    let available = usize::from(area.right().saturating_sub(x));
    if available < 3 {
        return None;
    }
    let view = crate::ui::widgets::input::view(input, cursor, available);
    Some((
        x.saturating_add(view.cursor_col)
            .min(area.right().saturating_sub(1)),
        area.y,
    ))
}

/// 命中标题行页控件：箭头与计数优先，其次页名区（重命名输入期间不命中页名）
/// 与标题区；未命中返回 None。
pub fn header_page_control_at(
    header: Rect,
    title: &str,
    draft: bool,
    page: &PageHeaderData<'_>,
    column: u16,
    row: u16,
) -> Option<PageControl> {
    if row != header.y.saturating_add(layout::PROMPT_TOOLBAR_HEIGHT) {
        return None;
    }
    let controls = page_controls(header, &text::page_counter(page.current, page.total));
    if let Some(controls) = controls.as_ref() {
        if controls.prev.contains((column, row).into()) {
            return Some(PageControl::Prev);
        }
        if controls.counter.contains((column, row).into()) {
            return Some(PageControl::Counter);
        }
        if controls.next.contains((column, row).into()) {
            return Some(PageControl::Next);
        }
    }
    let area = header_title_rect(header, controls.as_ref());
    if !area.contains((column, row).into()) {
        return None;
    }
    let (name_start, name_width) = header_name_span(area, title, draft, Some(page.name))?;
    (column >= name_start && column < name_start.saturating_add(name_width))
        .then_some(PageControl::Name)
}

/// 页名段的起点列与宽度：整段放得下或仅截断标题时可见；都放不下时为 None。
fn header_name_span(
    area: Rect,
    title: &str,
    draft: bool,
    name: Option<&str>,
) -> Option<(u16, u16)> {
    let name = name.filter(|name| !name.is_empty())?;
    let title_part = format!(" {title} ");
    let badge = if draft {
        format!("{} ", text::PROMPT_TEMP_BADGE)
    } else {
        String::new()
    };
    let name_part = format!("· {name} ");
    let full_width = (title_part.width() + badge.width() + name_part.width()) as u16;
    if full_width <= area.width {
        let start = area.x + (title_part.width() + badge.width()) as u16;
        return Some((start, name_part.width() as u16));
    }
    let name_width = name_part.width() as u16;
    if name_width.saturating_add(2) <= area.width {
        let start = area.right().saturating_sub(name_width);
        return Some((start, name_width));
    }
    None
}

/// 绘制标题 + 页名段：优先保住完整页名，其次截断标题；过窄时整体截断。
fn paint_header_title(
    header: Rect,
    buf: &mut Buffer,
    title: Option<&str>,
    draft: bool,
    page: Option<&PageHeaderData<'_>>,
    controls: Option<&PromptPageControls>,
) {
    let area = header_title_rect(header, controls);
    if area.width == 0 {
        return;
    }
    let title = title.unwrap_or("");
    let badge = if draft {
        format!("{} ", text::PROMPT_TEMP_BADGE)
    } else {
        String::new()
    };
    let title_part = format!(" {title} ");
    if let Some((input, cursor)) = page.and_then(|page| page.rename) {
        paint_header_rename(buf, area, &title_part, &badge, input, cursor);
        return;
    }
    let name_part = page
        .map(|page| page.name)
        .filter(|name| !name.is_empty())
        .map(|name| format!("· {name} "))
        .unwrap_or_default();
    let full = format!("{title_part}{badge}{name_part}");
    let available = usize::from(area.width);
    if full.width() <= available {
        paint_title_segments(buf, area.x, area.y, &title_part, &badge, &full);
        let name_start = area.x + (title_part.width() + badge.width()) as u16;
        buf.set_stringn(
            name_start,
            area.y,
            &name_part,
            name_part.width(),
            style::muted(),
        );
        return;
    }
    let name_width = name_part.width();
    if name_width > 0 && name_width.saturating_add(2) <= available {
        let prefix_available = available - name_width;
        // 截断优先作用于标题；`[tmp]` 徽标完整保留（放不下时整体省略）。
        let title_room = prefix_available.saturating_sub(badge.width());
        let (prefix, badge_shown) = if badge.is_empty() || title_room < 2 {
            (text::truncate_to_width(&title_part, prefix_available), "")
        } else {
            (
                format!(
                    "{}{}",
                    text::truncate_to_width(&title_part, title_room),
                    badge
                ),
                badge.as_str(),
            )
        };
        paint_title_segments(buf, area.x, area.y, &prefix, badge_shown, &prefix);
        let name_start = area.x.saturating_add((available - name_width) as u16);
        buf.set_stringn(name_start, area.y, &name_part, name_width, style::muted());
        return;
    }
    // 页名也放不下：整体截断，不保留页名命中区。
    let label = text::truncate_to_width(&full, available);
    paint_title_segments(buf, area.x, area.y, &label, "", &label);
}

/// 绘制标题段：标题 muted、`[tmp]` 徽标默认文字色；`badge` 为空时按普通标题绘制。
fn paint_title_segments(buf: &mut Buffer, x: u16, y: u16, title: &str, badge: &str, full: &str) {
    let title_width = title.width();
    buf.set_stringn(x, y, title, title_width, style::muted());
    if !badge.is_empty() {
        let offset = full
            .find(badge)
            .map(|byte| full[..byte].width())
            .unwrap_or(0);
        buf.set_stringn(
            x.saturating_add(offset as u16),
            y,
            badge,
            badge.width(),
            style::text(),
        );
    }
}

/// 页重命名输入：`title` 与徽标按剩余宽度截断，页名段位置渲染单行输入视口。
fn paint_header_rename(
    buf: &mut Buffer,
    area: Rect,
    title_part: &str,
    badge: &str,
    input: &str,
    cursor: usize,
) {
    let visible = format!("{title_part}{badge}");
    let max_prefix = usize::from(area.width).saturating_sub(4);
    let prefix = text::truncate_to_width(&visible, max_prefix);
    let prefix_columns = prefix.width();
    paint_title_segments(buf, area.x, area.y, &prefix, "", prefix.as_str());
    let x = area.x.saturating_add(prefix_columns as u16);
    let available = usize::from(area.right().saturating_sub(x));
    if available < 3 {
        return;
    }
    let separator = "· ";
    buf.set_stringn(x, area.y, separator, separator.width(), style::muted());
    let input_x = x.saturating_add(separator.width() as u16);
    let input_area = Rect {
        x: input_x,
        y: area.y,
        width: area.right().saturating_sub(input_x),
        height: 1,
    };
    let view = crate::ui::widgets::input::view(input, cursor, usize::from(input_area.width));
    buf.set_stringn(
        input_area.x,
        input_area.y,
        view.visible,
        usize::from(input_area.width),
        style::text(),
    );
}

/// 绘制页控件：`‹` / `›` 在边界处置灰；计数用 muted；控件区先清为空白。
fn paint_page_controls(
    buf: &mut Buffer,
    header: Rect,
    controls: &PromptPageControls,
    page: &PageHeaderData<'_>,
) {
    let y = header.y + layout::PROMPT_TOOLBAR_HEIGHT;
    for x in controls.prev.x..=controls.next.x {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(style::text());
        }
    }
    let prev_style = if page.current > 1 {
        style::accent()
    } else {
        style::muted()
    };
    let next_style = if page.current < page.total {
        style::accent()
    } else {
        style::muted()
    };
    for (rect, symbol, symbol_style) in [
        (controls.prev, text::PROMPT_PAGE_PREV, prev_style),
        (controls.next, text::PROMPT_PAGE_NEXT, next_style),
    ] {
        if let Some(cell) = buf.cell_mut((rect.x, y)) {
            cell.reset();
            cell.set_char(symbol.chars().next().unwrap_or(' '));
            cell.set_style(symbol_style);
        }
    }
    let counter = text::page_counter(page.current, page.total);
    buf.set_stringn(
        controls.counter.x,
        y,
        &counter,
        usize::from(controls.counter.width),
        style::muted(),
    );
}
