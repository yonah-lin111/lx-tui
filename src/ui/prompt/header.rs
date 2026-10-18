//! prompt 顶边框标题与页控件：只读状态，无 IO。
//!
//! 编辑视图在**面板顶边框**内嵌 `条目名 · 页名`（左侧）与 `‹ n/m ›`（右侧），
//! 树视图只内嵌固定标题 `Prompts`、不显示页控件；工具栏下方的分割线保持空行。
//! 三段抢宽度时的优先级为 页控件 → 条目名 → 页名（页控件永不截断）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::prompt::{Prompt, PromptMode};
use crate::ui::{style, text, widgets};

use super::{render_divider, render_toolbar};

/// 顶边框页信息：当前页序号（从 1 起）、总页数、页名与重命名输入现场。
pub struct PageHeaderData<'a> {
    pub current: usize,
    pub total: usize,
    pub name: &'a str,
    /// 页重命名输入：文本与光标字符索引；Some 时页名位置渲染为输入。
    pub rename: Option<(&'a str, usize)>,
}

/// 顶边框标题数据：左侧标题（树视图为固定标题、无绑定为 None）、草稿态与页信息。
pub struct BorderTitleData<'a> {
    pub title: Option<&'a str>,
    pub draft: bool,
    pub page: Option<PageHeaderData<'a>>,
}

/// 顶边框页控件几何：左箭头、计数、右箭头。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptPageControls {
    pub prev: Rect,
    pub counter: Rect,
    pub next: Rect,
}

/// 顶边框控件命中种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageControl {
    Prev,
    Next,
    Counter,
    Name,
}

/// 顶边框页控件几何：右对齐贴右边角（`‹ 2/4 ›`），与左右圆角各留 1 列边框线。
///
/// 左边框之后至少要有 2 列留给标题/页名，否则整组隐藏（返回 None）；
/// 页控件是最高优先级，可用时永不截断。
pub fn border_page_controls(area: Rect, counter: &str) -> Option<PromptPageControls> {
    if area.width == 0 {
        return None;
    }
    let counter_width = u16::try_from(UnicodeWidthStr::width(counter)).unwrap_or(u16::MAX);
    // 右边角前一列留 `─`，再往左才是 `›`。
    let next = Rect::new(area.right().saturating_sub(3), area.y, 1, 1);
    let counter_rect = Rect::new(
        next.x.saturating_sub(1).saturating_sub(counter_width),
        area.y,
        counter_width,
        1,
    );
    let prev = Rect::new(counter_rect.x.saturating_sub(2), area.y, 1, 1);
    let title_start = area.x.saturating_add(2);
    let title_width = prev.x.saturating_sub(1).saturating_sub(title_start);
    (title_width >= 2 && prev.x >= title_start).then_some(PromptPageControls {
        prev,
        counter: counter_rect,
        next,
    })
}

/// 顶边框左侧标题区（页控件之前）；无控件时到右边角前一列。
fn border_title_rect(area: Rect, controls: Option<&PromptPageControls>) -> Rect {
    let end = match controls {
        Some(controls) => controls.prev.x.saturating_sub(1),
        None => area.right().saturating_sub(1),
    };
    let x = area.x.saturating_add(2);
    Rect {
        x,
        y: area.y,
        width: end.saturating_sub(x),
        height: 1,
    }
}

/// 顶边框标题排布结果：条目名段、页名段与其起点列；绘制与命中共用同一套截断。
struct BorderText {
    title: String,
    name: Option<String>,
    /// 页名段起点列；无页名时等于标题区起点。
    name_x: u16,
}

/// 按可用宽度排布顶边框标题。优先级：条目名 → 页名（页控件由调用方单独保证）。
///
/// 1. 整段放得下 → 原样；
/// 2. 否则截断页名（连 `· ` 前缀一起按显示宽度截断），条目名保持完整；
/// 3. 页名连 2 列都放不下（或没有页名）→ 整段丢掉，只截断条目名。
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

/// 页重命名输入区（`· ` 之后到标题区右缘）：起点列与宽度。
fn rename_input_span(area: Rect, title: &str, width_budget: u16) -> Option<(u16, u16)> {
    let title_part = if title.is_empty() {
        String::new()
    } else {
        format!(" {title} ")
    };
    let max_prefix = usize::from(area.width).saturating_sub(4);
    let prefix = text::truncate_to_width(&title_part, max_prefix);
    let x = area.x.saturating_add(prefix.width() as u16);
    let remaining = area.right().saturating_sub(x);
    (remaining >= 3).then(|| {
        let input_x = x.saturating_add(2);
        let width = width_budget.min(area.right().saturating_sub(input_x));
        (input_x, width)
    })
}

/// 顶边框标题样式：正式条目沿用边框标题色，临时提示词用未聚焦灰
/// （显式前景 + dim，不被聚焦边框的强调色染色）。
fn border_title_style(draft: bool) -> Style {
    if draft {
        style::muted_on_border()
    } else {
        style::border_title()
    }
}

/// 绘制顶边框标题与页控件；须在面板边框绘制之后调用（覆盖 `─`）。
pub fn paint_border_title(buf: &mut Buffer, area: Rect, data: &BorderTitleData<'_>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let title = data.title.unwrap_or("");
    let controls = data
        .page
        .as_ref()
        .and_then(|page| border_page_controls(area, &text::page_counter(page.current, page.total)));
    let title_area = border_title_rect(area, controls.as_ref());
    let title_style = border_title_style(data.draft);
    if title_area.width > 0 {
        match data.page.as_ref().and_then(|page| page.rename) {
            Some((input, cursor)) => {
                paint_border_rename(buf, title_area, title, title_style, input, cursor);
            }
            None => paint_border_text(buf, title_area, data, title_style),
        }
    }
    if let (Some(controls), Some(page)) = (controls.as_ref(), data.page.as_ref()) {
        paint_page_controls(buf, controls, page);
    }
}

/// 绘制 `条目名 · 页名`：页名段固定在标题区右缘之后，条目名按剩余宽度截断。
fn paint_border_text(buf: &mut Buffer, area: Rect, data: &BorderTitleData<'_>, title_style: Style) {
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

/// 页重命名输入：标题按剩余宽度截断，页名段位置渲染单行输入视口。
///
/// 输入文本只覆盖自身占据的单元格，右侧保留外层 Block 画好的顶边框线 `─`；
/// 不要整段清空，否则聚焦期间右侧边框线会消失、取消聚焦后才由 Block 重绘恢复。
fn paint_border_rename(
    buf: &mut Buffer,
    area: Rect,
    title: &str,
    title_style: Style,
    input: &str,
    cursor: usize,
) {
    let Some((input_x, width)) = rename_input_span(area, title, u16::MAX) else {
        return;
    };
    let title_part = if title.is_empty() {
        String::new()
    } else {
        format!(" {title} ")
    };
    let max_prefix = usize::from(area.width).saturating_sub(4);
    let prefix = text::truncate_to_width(&title_part, max_prefix);
    if !prefix.is_empty() {
        buf.set_stringn(area.x, area.y, &prefix, prefix.width(), title_style);
    }
    let separator = format!("{} ", text::PROMPT_PAGE_NAME_PREFIX);
    buf.set_stringn(
        input_x.saturating_sub(2),
        area.y,
        &separator,
        separator.width(),
        style::muted_on_border(),
    );
    let view = widgets::input::view(input, cursor, usize::from(width));
    buf.set_stringn(
        input_x,
        area.y,
        view.visible,
        usize::from(width),
        style::text_on_border(),
    );
    // 输入文本后保留 1 列空白再接边框线，与失焦态 `· 页名 ` 的收尾一致。
    let text_end = input_x.saturating_add(u16::try_from(view.visible.width()).unwrap_or(u16::MAX));
    if text_end < area.right()
        && let Some(cell) = buf.cell_mut((text_end, area.y))
    {
        cell.reset();
        cell.set_char(' ');
        cell.set_style(style::text_on_border());
    }
}

/// 页重命名输入的硬件光标位置（顶边框行）；输入区放不下时返回 None。
pub fn border_rename_cursor(
    area: Rect,
    title: Option<&str>,
    page: &PageHeaderData<'_>,
) -> Option<(u16, u16)> {
    let (input, cursor) = page.rename?;
    let controls = border_page_controls(area, &text::page_counter(page.current, page.total));
    let title_area = border_title_rect(area, controls.as_ref());
    if title_area.width == 0 {
        return None;
    }
    let (input_x, width) = rename_input_span(title_area, title.unwrap_or(""), u16::MAX)?;
    let view = widgets::input::view(input, cursor, usize::from(width));
    Some((
        input_x
            .saturating_add(view.cursor_col)
            .min(title_area.right().saturating_sub(1)),
        area.y,
    ))
}

/// 命中顶边框页控件：箭头与计数优先，其次页名段（重命名输入期间命中输入区）；
/// 条目名与边框空白返回 None。
pub fn border_page_control_at(
    area: Rect,
    data: &BorderTitleData<'_>,
    column: u16,
    row: u16,
) -> Option<PageControl> {
    if row != area.y {
        return None;
    }
    let page = data.page.as_ref()?;
    let controls = border_page_controls(area, &text::page_counter(page.current, page.total));
    if let Some(controls) = controls.as_ref() {
        for (rect, control) in [
            (controls.prev, PageControl::Prev),
            (controls.counter, PageControl::Counter),
            (controls.next, PageControl::Next),
        ] {
            if rect.contains((column, row).into()) {
                return Some(control);
            }
        }
    }
    let title_area = border_title_rect(area, controls.as_ref());
    let title = data.title.unwrap_or("");
    let span = match page.rename {
        Some(_) => rename_input_span(title_area, title, u16::MAX)?,
        None => {
            let layout = border_text(title_area, title, Some(page));
            let name = layout.name.as_deref()?;
            (
                layout.name_x,
                u16::try_from(name.width()).unwrap_or(u16::MAX),
            )
        }
    };
    (column >= span.0 && column < span.0.saturating_add(span.1)).then_some(PageControl::Name)
}

/// 绘制 prompt 顶部工具栏与固定分割线；内容区过矮时不绘制。
///
/// 分割线保持空行——条目名、页名与页控件都在面板顶边框上（见 `paint_border_title`）。
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

/// 绘制页控件：`‹` / `›` 在边界处置灰；计数用边框标题色；控件区先清为空白。
fn paint_page_controls(buf: &mut Buffer, controls: &PromptPageControls, page: &PageHeaderData<'_>) {
    let y = controls.prev.y;
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
    let counter = text::page_counter(page.current, page.total);
    buf.set_stringn(
        controls.counter.x,
        y,
        &counter,
        usize::from(controls.counter.width),
        style::border_title(),
    );
}
