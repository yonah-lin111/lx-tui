//! prompt 标题行与页控件：只读状态，无 IO。
//!
//! 工具栏行：左组 `[≡]/[/] [rec] [←] [→]`，右组 `[refDir] ‹ n/m › ●`（页视图为 `n/m`）；
//! 分割线标题行：左侧 `条目名 · 页名`，右侧状态标签 `[todo]`/`[run]`/`[done]`；
//! 树视图的标题行为纯 `─`、不显示状态与页控件。
//! 抢宽度时的优先级为 页控件 → 状态标签 → 条目名 → 页名（页控件永不截断）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::TemplateStatus;
use crate::app::prompt::{Prompt, PromptMode};
use crate::ui::{style, text};

pub use crate::layout::{
    PageControl, PromptPageControls, page_controls, page_controls_with, toolbar_page_controls,
};

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

/// 标题行文字区（距左端边框 2 列，到右端边框前 1 列）。
fn title_text_rect(area: Rect) -> Rect {
    let end = area.right().saturating_sub(1);
    let x = area.x.saturating_add(2);
    Rect {
        x,
        y: area.y,
        width: end.saturating_sub(x),
        height: 1,
    }
}

/// 本页标题行页名矩形（编辑视图且本页存在已闭合 `&&&` 时用于点击切换状态与 tooltip）；
/// 页名被窄屏完全挤掉时返回 None。
pub fn status_rect(area: Rect, data: &TitleRowData<'_>, _status: TemplateStatus) -> Option<Rect> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let title_area = title_text_rect(area);
    if title_area.width == 0 {
        return None;
    }
    let layout = border_text(title_area, data.title.unwrap_or(""), data.page.as_ref());
    let (_, name_x, hit_width) = layout.name?;
    (hit_width > 0).then_some(Rect::new(name_x, title_area.y, hit_width, 1))
}

/// 标题行排布结果：条目名段、`· ` 分隔符段、页名段及其可见文字矩形宽度。
struct BorderText {
    title: String,
    sep: Option<(String, u16)>,
    /// `(带尾部空格的绘制文本, 起点列, 可见页名列宽)`。
    name: Option<(String, u16, u16)>,
}

/// 按可用宽度排布标题行。优先级：条目名 → 页名。
fn border_text(area: Rect, title: &str, page: Option<&PageHeaderData<'_>>) -> BorderText {
    let available = usize::from(area.width);
    let title_part = if title.is_empty() {
        String::new()
    } else {
        format!(" {title} ")
    };
    let sep = format!("{} ", text::PROMPT_PAGE_NAME_PREFIX);
    if let Some(raw_name) = page.map(|page| page.name).filter(|name| !name.is_empty()) {
        let name_padded = format!("{raw_name} ");
        let full_name_width = sep.width().saturating_add(name_padded.width());
        if title_part.width().saturating_add(full_name_width) <= available {
            let sep_x = area.x.saturating_add(title_part.width() as u16);
            let name_x = sep_x.saturating_add(sep.width() as u16);
            let hit_width = u16::try_from(raw_name.width()).unwrap_or(0);
            return BorderText {
                title: title_part,
                sep: (hit_width > 0).then_some((sep, sep_x)),
                name: (hit_width > 0).then_some((name_padded, name_x, hit_width)),
            };
        }
        let name_budget = available.saturating_sub(title_part.width());
        if name_budget > sep.width() && title_part.width() <= available {
            let sep_x = area.x.saturating_add(title_part.width() as u16);
            let name_x = sep_x.saturating_add(sep.width() as u16);
            let page_budget = name_budget.saturating_sub(sep.width());
            let truncated = text::truncate_to_width(&name_padded, page_budget);
            let hit_width = u16::try_from(truncated.trim_end_matches(' ').width()).unwrap_or(0);
            return BorderText {
                title: title_part,
                sep: (hit_width > 0).then_some((sep, sep_x)),
                name: (hit_width > 0).then_some((truncated, name_x, hit_width)),
            };
        }
    }
    BorderText {
        title: text::truncate_to_width(&title_part, available),
        sep: None,
        name: None,
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

/// 绘制标题行 `条目名 · 页名`；须在面板分割线绘制之后调用（覆盖 `─`）。
///
/// `status` 为本页已闭合 `&&&` 模板块状态：`Some` 时按状态为页名（`· ` 之后）着色，
/// `None` 时页名使用 `muted_on_border`。
pub fn paint_title_row(
    buf: &mut Buffer,
    area: Rect,
    data: &TitleRowData<'_>,
    status: Option<TemplateStatus>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let title_area = title_text_rect(area);
    let title_style = border_title_style(data.draft);
    let name_style = match status {
        Some(status) => style::template_border(status),
        None => style::muted_on_border(),
    };
    if title_area.width > 0 {
        paint_border_text(buf, title_area, data, title_style, name_style);
    }
}

/// 绘制 `条目名 · 页名`：`· ` 恒为暗灰，页名按 `name_style` 着色。
fn paint_border_text(
    buf: &mut Buffer,
    area: Rect,
    data: &TitleRowData<'_>,
    title_style: Style,
    name_style: Style,
) {
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
    if let Some((sep, sep_x)) = layout.sep.as_ref() {
        buf.set_stringn(*sep_x, area.y, sep, sep.width(), style::muted_on_border());
    }
    if let Some((name, name_x, _)) = layout.name.as_ref() {
        buf.set_stringn(*name_x, area.y, name, name.width(), name_style);
    }
}

/// 命中工具栏页控件：仅箭头与计数响应，页名与条目名为纯展示（返回 None）。
pub fn page_control_at(
    bar: Rect,
    data: &TitleRowData<'_>,
    column: u16,
    row: u16,
) -> Option<PageControl> {
    if row != bar.y {
        return None;
    }
    let page = data.page.as_ref()?;
    let controls = toolbar_page_controls(
        bar,
        &text::page_counter(page.current, page.total),
        page.arrows,
    )?;
    for (rect, control) in [
        (controls.prev, PageControl::Prev),
        (controls.counter, PageControl::Counter),
        (controls.next, PageControl::Next),
    ] {
        if rect.width > 0 && rect.contains((column, row).into()) {
            return Some(control);
        }
    }
    None
}

/// 绘制 prompt 顶部工具栏与固定分割线；内容区过矮时不绘制。
///
/// 分割线先画成空行；调用方随后用 `paint_title_row` 把标题/状态标签覆盖上去，
/// 工具栏页控件（`‹ n/m ›` / `n/m`）由本函数覆盖绘制。
pub fn render_header(
    panel: Rect,
    buf: &mut Buffer,
    prompt: &Prompt,
    focused: bool,
    mode: PromptMode,
    draft: bool,
    page: Option<&PageHeaderData<'_>>,
) {
    let Some(header) = crate::layout::prompt_header_rect(panel) else {
        return;
    };
    render_divider(panel, header, buf, focused);
    let counter = page.map(|page| text::page_counter(page.current, page.total));
    render_toolbar(panel, buf, prompt, mode, draft, counter.as_deref());
    if let (Some(page), Some(counter), Some(bar)) = (
        page,
        counter.as_deref(),
        crate::layout::prompt_toolbar_rect(panel),
    ) && let Some(controls) = toolbar_page_controls(bar, counter, page.arrows)
    {
        paint_page_controls(buf, &controls, page);
    }
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
    let counter_style = if page.arrows {
        style::text_on_border()
    } else {
        style::accent()
    };
    buf.set_stringn(
        controls.counter.x,
        y,
        &counter,
        usize::from(controls.counter.width),
        counter_style,
    );
}
