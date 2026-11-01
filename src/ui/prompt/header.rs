//! prompt 标题行与页控件：只读状态，无 IO。
//!
//! 编辑视图在**工具栏下方的分割线**上内嵌 `条目名 · 页名`（左侧）、
//! 状态标签 `[todo]`/`[run]`/`[done]`（页控件左侧，按状态着色）与 `‹ n/m ›`（右侧），
//! 页列表视图内嵌 `条目名 · pages`（左侧）与无箭头的 `n/m`（右侧），
//! 树视图的标题行为纯 `─`、不显示状态与页控件。
//! 抢宽度时的优先级为 页控件 → 状态标签 → 条目名 → 页名（页控件永不截断）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::TemplateStatus;
use crate::app::prompt::{Prompt, PromptMode};
use crate::ui::{style, text};

use super::{render_divider, render_toolbar};

/// 标题行页信息：当前页序号（从 1 起）、总页数、副标题（页名或 `pages`）与是否显示左右箭头。
pub struct PageHeaderData<'a> {
    pub current: usize,
    pub total: usize,
    pub name: &'a str,
    pub arrows: bool,
}

/// 标题行数据：左侧标题（树视图为 None）、草稿态与页信息。
pub struct TitleRowData<'a> {
    pub title: Option<&'a str>,
    pub draft: bool,
    pub page: Option<PageHeaderData<'a>>,
}

/// 标题行页控件几何：左箭头、计数、右箭头（`arrows == false` 时左右箭头宽度为 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptPageControls {
    pub prev: Rect,
    pub counter: Rect,
    pub next: Rect,
}

/// 标题行控件命中种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageControl {
    Prev,
    Next,
    Counter,
}

/// 标题行页控件几何（带左右箭头 `‹ 2/4 ›`）。
pub fn page_controls(area: Rect, counter: &str) -> Option<PromptPageControls> {
    page_controls_with(area, counter, true)
}

/// 标题行页控件几何：`arrows` 为真时排布 `‹ n/m ›`，为假时仅排布计数 `n/m`（`prev`/`next` 宽 0）。
///
/// 左边框之后至少要有 2 列留给标题/页名，否则整组隐藏（返回 None）；
/// 页控件是最高优先级，可用时永不截断。
pub fn page_controls_with(area: Rect, counter: &str, arrows: bool) -> Option<PromptPageControls> {
    if area.width == 0 {
        return None;
    }
    let counter_width = u16::try_from(UnicodeWidthStr::width(counter)).unwrap_or(u16::MAX);
    let title_start = area.x.saturating_add(2);
    if arrows {
        // 右边角前一列留 `─`，再往左才是 `›`。
        let next = Rect::new(area.right().saturating_sub(3), area.y, 1, 1);
        let counter_rect = Rect::new(
            next.x.saturating_sub(1).saturating_sub(counter_width),
            area.y,
            counter_width,
            1,
        );
        let prev = Rect::new(counter_rect.x.saturating_sub(2), area.y, 1, 1);
        let title_width = prev.x.saturating_sub(1).saturating_sub(title_start);
        (title_width >= 2 && prev.x >= title_start).then_some(PromptPageControls {
            prev,
            counter: counter_rect,
            next,
        })
    } else {
        let right_edge = area.right().saturating_sub(2);
        let counter_x = right_edge.saturating_sub(counter_width);
        let counter_rect = Rect::new(counter_x, area.y, counter_width, 1);
        let prev = Rect::new(counter_x, area.y, 0, 1);
        let next = Rect::new(right_edge, area.y, 0, 1);
        let title_width = prev.x.saturating_sub(1).saturating_sub(title_start);
        (title_width >= 2 && prev.x >= title_start).then_some(PromptPageControls {
            prev,
            counter: counter_rect,
            next,
        })
    }
}

/// 标题行左侧标题区（页控件与状态标签之前）；无控件时到右边角前一列。
fn title_text_rect(
    area: Rect,
    controls: Option<&PromptPageControls>,
    status_x: Option<u16>,
) -> Rect {
    let end = match controls {
        Some(controls) => controls.prev.x.saturating_sub(1),
        None => area.right().saturating_sub(1),
    };
    let end = match status_x {
        Some(x) => x.saturating_sub(1),
        None => end,
    };
    let x = area.x.saturating_add(2);
    Rect {
        x,
        y: area.y,
        width: end.saturating_sub(x),
        height: 1,
    }
}

/// 标题行页控件几何（渲染、命中与状态标签共用）。
fn controls_for(area: Rect, data: &TitleRowData<'_>) -> Option<PromptPageControls> {
    let page = data.page.as_ref()?;
    page_controls_with(
        area,
        &text::page_counter(page.current, page.total),
        page.arrows,
    )
}

/// 本页模板块状态标签矩形（渲染、点击命中与 tooltip 共用）；无空间时 None。
pub fn status_rect(area: Rect, data: &TitleRowData<'_>, status: TemplateStatus) -> Option<Rect> {
    status_rect_in(area, controls_for(area, data).as_ref(), status).map(|(rect, _)| rect)
}

/// 本页模板块状态标签：右对齐贴页控件左侧、与标题区隔 1 列；
/// 标题区至少保留 2 列，空间不足时隐藏。
fn status_rect_in(
    area: Rect,
    controls: Option<&PromptPageControls>,
    status: TemplateStatus,
) -> Option<(Rect, Style)> {
    let label = text::template_status_label(status);
    let width = u16::try_from(UnicodeWidthStr::width(label)).ok()?;
    let right = match controls {
        Some(controls) => controls.prev.x.saturating_sub(1),
        None => area.right().saturating_sub(1),
    };
    let x = right.checked_sub(width)?;
    (x >= area.x.saturating_add(5)).then(|| {
        (
            Rect::new(x, area.y, width, 1),
            style::template_border(status),
        )
    })
}

/// 标题行排布结果：条目名段、页名段与其起点列。
struct BorderText {
    title: String,
    name: Option<String>,
    /// 页名段起点列；无页名时等于标题区起点。
    name_x: u16,
}

/// 按可用宽度排布标题行。优先级：条目名 → 页名（页控件由调用方单独保证）。
fn border_text(area: Rect, title: &str, page: Option<&PageHeaderData<'_>>) -> BorderText {
    let available = usize::from(area.width);
    let title_part = if title.is_empty() {
        String::new()
    } else {
        format!(" {title} ")
    };
    let name_part = page
        .map(|page| page.name)
        .filter(|name| !name.is_empty())
        .map(|name| format!("{} {name} ", text::PROMPT_PAGE_NAME_PREFIX))
        .unwrap_or_default();
    if title_part.width().saturating_add(name_part.width()) <= available {
        let name_x = area.x.saturating_add(title_part.width() as u16);
        return BorderText {
            title: title_part,
            name: (!name_part.is_empty()).then_some(name_part),
            name_x,
        };
    }
    let name_budget = available.saturating_sub(title_part.width());
    if !name_part.is_empty() && name_budget >= 2 && title_part.width() <= available {
        let name = text::truncate_to_width(&name_part, name_budget);
        let name_x = area.x.saturating_add(title_part.width() as u16);
        return BorderText {
            title: title_part,
            name: Some(name),
            name_x,
        };
    }
    BorderText {
        title: text::truncate_to_width(&title_part, available),
        name: None,
        name_x: area.x,
    }
}

/// 标题行标题样式：正式条目沿用边框标题色，临时提示词用未聚焦灰。
fn border_title_style(draft: bool) -> Style {
    if draft {
        style::muted_on_border()
    } else {
        style::border_title()
    }
}

/// 绘制标题行标题、状态标签与页控件；须在面板边框绘制之后调用（覆盖 `─`）。
///
/// `status` 为本页 `&&&` 模板块状态：`Some` 时在页控件左侧画 `[todo]`/`[run]`/`[done]`，
/// 颜色按状态（蓝/黄/绿）；空间不足时隐藏标签，优先保留页控件与标题。
pub fn paint_title_row(
    buf: &mut Buffer,
    area: Rect,
    data: &TitleRowData<'_>,
    status: Option<TemplateStatus>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let controls = controls_for(area, data);
    let status_area = status.and_then(|status| status_rect_in(area, controls.as_ref(), status));
    let title_area = title_text_rect(
        area,
        controls.as_ref(),
        status_area.as_ref().map(|(rect, _)| rect.x),
    );
    let title_style = border_title_style(data.draft);
    if title_area.width > 0 {
        paint_border_text(buf, title_area, data, title_style);
    }
    if let (Some(status), Some((rect, status_style))) = (status, status_area) {
        let label = text::template_status_label(status);
        buf.set_stringn(rect.x, rect.y, label, usize::from(rect.width), status_style);
    }
    if let (Some(controls), Some(page)) = (controls.as_ref(), data.page.as_ref()) {
        paint_page_controls(buf, controls, page);
    }
}

/// 绘制 `条目名 · 页名`：页名段固定在标题区右缘之后，条目名按剩余宽度截断。
fn paint_border_text(buf: &mut Buffer, area: Rect, data: &TitleRowData<'_>, title_style: Style) {
    let layout = border_text(area, data.title.unwrap_or(""), data.page.as_ref());
    if !layout.title.is_empty() {
        buf.set_stringn(
            area.x,
            area.y,
            &layout.title,
            layout.title.width(),
            title_style,
        );
    }
    if let Some(name) = layout.name.as_deref() {
        buf.set_stringn(
            layout.name_x,
            area.y,
            name,
            name.width(),
            style::muted_on_border(),
        );
    }
}

/// 命中标题行页控件：仅箭头與计数响应，页名与条目名为纯展示（返回 None）。
pub fn page_control_at(
    area: Rect,
    data: &TitleRowData<'_>,
    column: u16,
    row: u16,
) -> Option<PageControl> {
    if row != area.y {
        return None;
    }
    let page = data.page.as_ref()?;
    let controls = page_controls_with(
        area,
        &text::page_counter(page.current, page.total),
        page.arrows,
    )?;
    for (rect, control) in [
        (controls.prev, PageControl::Prev),
        (controls.counter, PageControl::Counter),
        (controls.next, PageControl::Next),
    ] {
        if rect.contains((column, row).into()) {
            return Some(control);
        }
    }
    None
}

/// 绘制 prompt 顶部工具栏与固定分割线；内容区过矮时不绘制。
///
/// 分割线先画成空行；调用方随后用 `paint_title_row` 把标题/页控件覆盖上去。
pub fn render_header(
    panel: Rect,
    buf: &mut Buffer,
    prompt: &Prompt,
    focused: bool,
    mode: PromptMode,
    draft: bool,
    pinned: bool,
) {
    let Some(header) = crate::layout::prompt_header_rect(panel) else {
        return;
    };
    render_divider(panel, header, buf, focused);
    render_toolbar(panel, buf, prompt, mode, draft, pinned);
}

/// 绘制页控件：`‹` / `›` 在边界处置灰（`arrows == false` 时不绘制）；计数用边框标题色。
fn paint_page_controls(buf: &mut Buffer, controls: &PromptPageControls, page: &PageHeaderData<'_>) {
    let y = controls.counter.y;
    if page.arrows {
        for x in controls.prev.x..=controls.next.x {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_char(' ');
                cell.set_style(style::text_on_border());
            }
        }
        let prev_style = if page.current > 1 {
            style::accent()
        } else {
            style::muted_on_border()
        };
        let next_style = if page.current < page.total {
            style::accent()
        } else {
            style::muted_on_border()
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
    } else {
        let start = controls.counter.x.saturating_sub(1);
        let end = controls.counter.right();
        for x in start..=end {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_char(' ');
                cell.set_style(style::text_on_border());
            }
        }
    }
    let counter = text::page_counter(page.current, page.total);
    buf.set_stringn(
        controls.counter.x,
        y,
        &counter,
        usize::from(controls.counter.width),
        style::border_title(),
    );
}
