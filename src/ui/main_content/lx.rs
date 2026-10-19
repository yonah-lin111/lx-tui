//! lx 欢迎页：顶部品牌区（像素图标 + 文案）、标签页式入口与底部多行 markdown 输入框。
//!
//! 整页以单元格网格构建：同一份网格既用于绘制，也用于整页文本复制，
//! 因此渲染与取词不会漂移。渲染只读状态、只写 Buffer。

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::BlockPanel;
use crate::app::paste::PasteAction;
use crate::app::selection::Selection;
use crate::app::state::{Lx, LxEntry};
use crate::ui::widgets::command_panel::{self, CommandItem, CommandPanelView};
use crate::ui::widgets::scrollbar;
use crate::ui::{style, text};

mod input;
mod mention;

pub(crate) use mention::{item_at as mention_item_at, layout as mention_layout};

pub(crate) mod brand;
pub(crate) mod chat_bar;
pub(crate) mod grid;
pub(crate) mod history;
pub(crate) mod notice;
pub(crate) mod slash;
pub(crate) mod status;

pub(crate) use slash::{item_at as slash_item_at, layout as slash_layout};

use grid::{CONTINUATION, Grid};

#[cfg(test)]
pub(crate) use brand::{ICON_LX, LOGO_GAP, icon_pixel};
pub(crate) use brand::{LOGO_ROWS, LOGO_WIDTH, draw_art};

/// 输入框水平外边距、最小宽度与最小高度（1 内容行 + 上下分割线）；
/// 边距、高度比例、前缀与滚动条槽与状态层折行宽度共用同一组 `layout` 常量。
const MARGIN: u16 = crate::layout::LX_INPUT_MARGIN;
const MIN_INPUT_WIDTH: u16 = 8;
const INPUT_MIN_BOX: u16 = crate::layout::LX_INPUT_MIN_BOX;
/// 输入框最大高度占内容区高度的百分比。
const INPUT_MAX_PERCENT: u16 = crate::layout::LX_INPUT_HEIGHT_PERCENT;
/// 输入框 `> ` 前缀列数与右侧滚动条预留列数。
const INPUT_PREFIX: u16 = crate::layout::LX_INPUT_PREFIX;
const INPUT_SCROLLBAR: u16 = crate::layout::LX_INPUT_SCROLLBAR;
/// 入口区：单行高度、横排标签间距（含分隔符）、竖排行距与区块间距。
const ENTRY_HEIGHT: u16 = 1;
const ENTRY_GAP: u16 = 3;
const ENTRY_VGAP: u16 = 1;
const SECTION_GAP: u16 = 1;
/// 输入框上下分割线字符。
const RULE: char = '─';

/// 渲染 lx 页；返回聚焦时输入光标的相对坐标（行、列，相对内容区左上角）。
///
/// `selection` 为当前 lx 选区（内容行、列，相对内容区左上角），用于反显高亮。
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    lx: &Lx,
    focused: bool,
    selection: Option<&Selection>,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let (grid, cursor, sticky) = build(area, lx, focused);
    for dy in 0..area.height {
        for dx in 0..area.width {
            let glyph = grid.cell(area.x + dx, area.y + dy);
            if let Some(cell) = buf.cell_mut((area.x + dx, area.y + dy)) {
                // 宽字符的后续列标记为 diff 跳过，禁止渲染出半个字符。
                if glyph.ch == CONTINUATION {
                    cell.set_diff_option(CellDiffOption::Skip);
                    continue;
                }
                cell.set_char(glyph.ch);
                let mut cell_style = glyph.style;
                if selection.is_some_and(|sel| sel.contains(i32::from(dy), dx)) {
                    cell_style = cell_style.add_modifier(Modifier::REVERSED);
                }
                cell.set_style(cell_style);
                cell.set_diff_option(CellDiffOption::None);
            }
        }
    }
    render_input_scrollbar(area, buf, lx);
    // 置顶用户消息行：表现层，blit 之后直写 Buffer（不进网格 → 不参与复制与选区）。
    if let Some(row) = sticky
        && let Some(message) = lx.chat.messages.get(row.message)
    {
        history::render_sticky_prompt(buf, row, message);
    }
    // 状态（模型 · 供应商 · 上下文占用）覆盖绘制在窗格底边框上。
    status::render(buf, area, lx);
    // 提及面板浮于基础内容之上、贴输入框上沿；与粘贴面板互斥。
    mention::render(area, buf, lx);
    render_paste_panel(area, buf, lx);
    render_slash_panel(area, buf, lx);
    cursor
}

/// 绘制 `/` 命令面板（几何/条目见 `slash` 子模块）。
fn render_slash_panel(area: Rect, buf: &mut Buffer, lx: &Lx) {
    if lx.slash.is_none() {
        return;
    }
    slash::render(area, buf, lx);
}

/// 绘制输入框滚动条：软换行后的视觉行溢出可视高度时，覆盖在最右预留列。
fn render_input_scrollbar(area: Rect, buf: &mut Buffer, lx: &Lx) {
    if lx.input.is_empty() {
        return;
    }
    if let Some(bar) = input_scrollbar(area, lx) {
        scrollbar::render_buffer(buf, &bar);
    }
}

/// 把历史滚动条渲染到网格（贴右缘一列；轨道留空、只画 thumb）。
///
/// 与公共 `scrollbar` 组件同口径：轨道不画符号，只保留占位列与命中几何。
fn render_history_scrollbar(grid: &mut Grid, bar: &scrollbar::ScrollbarLayout) {
    for y in bar.thumb.y..bar.thumb.bottom() {
        grid.set(bar.thumb.x, y, '▐', style::text());
    }
}

/// lx 输入框文本区几何（绝对屏幕坐标）；供鼠标命中与拖拽选择换算。
pub(crate) struct InputGeom {
    text_top: u16,
    content_x: u16,
    content_w: u16,
    visible: usize,
    scroll: usize,
}

impl InputGeom {
    /// 输入框文本内容区左上角单元格（绝对屏幕坐标）；仅测试使用。
    #[cfg(test)]
    pub(crate) fn text_origin(&self) -> (u16, u16) {
        (self.content_x, self.text_top)
    }

    /// 坐标是否落在输入框文本内容区内。
    pub(crate) fn contains(&self, column: u16, row: u16) -> bool {
        self.content_w > 0
            && row >= self.text_top
            && row
                < self
                    .text_top
                    .saturating_add(u16::try_from(self.visible).unwrap_or(u16::MAX))
            && column >= self.content_x
            && column < self.content_x.saturating_add(self.content_w)
    }

    /// 单元格 →（视觉行索引，内容列）；行、列各自钳到可视区。
    pub(crate) fn cell(&self, column: u16, row: u16) -> (usize, u16) {
        let bottom = self
            .text_top
            .saturating_add(u16::try_from(self.visible).unwrap_or(u16::MAX))
            .saturating_sub(1);
        let local = row.clamp(self.text_top, bottom);
        let visual_row = self.scroll + usize::from(local - self.text_top);
        let right = self
            .content_x
            .saturating_add(self.content_w)
            .saturating_sub(1);
        let col = column.clamp(self.content_x, right) - self.content_x;
        (visual_row, col)
    }
}

/// lx 输入框几何（含边框）；（绝对屏幕坐标）；供提及面板贴附计算。
pub(crate) fn input_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    layout(area, lx).input
}

/// lx 输入框文本区几何；`area` 为窗格内容区。
pub(crate) fn input_geom(area: Rect, lx: &Lx) -> Option<InputGeom> {
    let lay = layout(area, lx);
    let input = lay.input?;
    if lay.input_visible == 0 {
        return None;
    }
    Some(InputGeom {
        text_top: input.y + 1,
        content_x: input.x + INPUT_PREFIX,
        content_w: input
            .width
            .saturating_sub(INPUT_PREFIX)
            .saturating_sub(INPUT_SCROLLBAR),
        visible: lay.input_visible,
        scroll: lay.input_scroll,
    })
}

/// lx 输入框滚动条几何；内容未溢出时为 None。
pub(crate) fn input_scrollbar(area: Rect, lx: &Lx) -> Option<scrollbar::ScrollbarLayout> {
    let lay = layout(area, lx);
    let input = lay.input?;
    if lay.input_visible == 0 {
        return None;
    }
    let track = Rect::new(
        input.x,
        input.y + 1,
        input.width,
        u16::try_from(lay.input_visible).unwrap_or(u16::MAX),
    );
    scrollbar::layout(
        track,
        lx.visual_rows().len(),
        lay.input_visible,
        lay.input_scroll,
    )
}

/// 绘制路径粘贴参照面板：锚定输入框上沿、复用命令面板渲染（只读状态）。
fn render_paste_panel(area: Rect, buf: &mut Buffer, lx: &Lx) {
    let Some(panel) = lx.paste_panel() else {
        return;
    };
    let Some(input) = layout(area, lx).input else {
        return;
    };
    let margin: u16 = if area.width >= 24 { MARGIN } else { 0 };
    let content_x = area.x + margin;
    let content_w = area.width.saturating_sub(margin.saturating_mul(2));
    let region_h = input.y.saturating_sub(area.y);
    if region_h < 3 {
        return;
    }
    let count = text::paste_item_count(panel.candidates.len());
    let rows: Vec<(String, String)> = PasteAction::ALL
        .iter()
        .map(|action| match action {
            PasteAction::Reference => (
                text::PASTE_ACTION_REFERENCE.to_string(),
                if panel.reference_enabled() {
                    count.clone()
                } else {
                    text::PASTE_REFERENCE_UNSUPPORTED.to_string()
                },
            ),
            PasteAction::Path => (text::PASTE_ACTION_PATH.to_string(), count.clone()),
        })
        .collect();
    let items: Vec<CommandItem<'_>> = rows
        .iter()
        .map(|(label, preview)| CommandItem::Inline {
            label,
            preview,
            tag: "",
        })
        .collect();
    let view = CommandPanelView {
        items: &items,
        active: panel.active(),
        current: None,
        window_anchor: Some(panel.active()),
        window_start: None,
        anchor_row: region_h,
        max_height: None,
        title: Some(text::PASTE_PANEL_TITLE),
        right_title: None,
        footer: None,
        ..Default::default()
    };
    let panel_area = Rect::new(content_x, area.y, content_w, region_h);
    command_panel::render(panel_area, buf, &view);
}

/// 提取选区文本（内容行、列相对内容区左上角）；空选区返回空串。
pub fn copy_text(area: Rect, lx: &Lx, range: ((i32, u16), (i32, u16))) -> String {
    let (grid, _, _) = build(area, lx, false);
    let ((start_row, start_col), (end_row, end_col)) = range;
    let mut lines: Vec<String> = Vec::new();
    for row in start_row.max(0)..=end_row.max(0) {
        if row >= i32::from(area.height) {
            break;
        }
        let y = area.y + row as u16;
        let (col0, col1) = if start_row == end_row {
            (start_col, end_col)
        } else if row == start_row {
            (start_col, area.width.saturating_sub(1))
        } else if row == end_row {
            (0, end_col)
        } else {
            (0, area.width.saturating_sub(1))
        };
        let mut line = String::new();
        for col in col0..=col1.min(area.width.saturating_sub(1)) {
            let ch = grid.cell(area.x + col, y).ch;
            if ch != CONTINUATION {
                line.push(ch);
            }
        }
        lines.push(line.trim_end().to_string());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// 命中测试：坐标落在某个入口标签内则返回该入口。
pub(crate) fn entry_hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<LxEntry> {
    layout(area, lx)
        .entries
        .into_iter()
        .find(|(_, rect)| rect.contains((column, row).into()))
        .map(|(entry, _)| entry)
}

/// 一段文案的落位；水平居中于 `width`。
#[derive(Clone, Copy)]
struct TextRect {
    x: u16,
    y: u16,
    width: u16,
}

/// lx 页布局：所有组件矩形（绝对屏幕坐标）。
struct Layout {
    input: Option<Rect>,
    /// 输入框可见内容行的滚动起点（逻辑行索引）。
    input_scroll: usize,
    /// 输入框可见内容行数。
    input_visible: usize,
    /// 输入框上方聊天栏（loading 转轮 + 右端回到底部按钮）；无则为 None。
    chat_bar: Option<Rect>,
    panel: Option<Rect>,
    entries: Vec<(LxEntry, Rect)>,
    horizontal: bool,
    icon: Option<(u16, u16)>,
    description: Option<TextRect>,
    /// 空间不足时退化为居中字标。
    fallback: bool,
}

impl Layout {
    fn empty(fallback: bool) -> Self {
        Self {
            input: None,
            input_scroll: 0,
            input_visible: 0,
            chat_bar: None,
            panel: None,
            entries: Vec::new(),
            horizontal: true,
            icon: None,
            description: None,
            fallback,
        }
    }
}

/// 计算整页布局。
fn layout(area: Rect, lx: &Lx) -> Layout {
    if area.width == 0 || area.height == 0 {
        return Layout::empty(true);
    }
    let margin: u16 = if area.width >= 24 { MARGIN } else { 0 };
    let content_x = area.x + margin;
    let content_w = area.width.saturating_sub(margin.saturating_mul(2));

    if area.height < INPUT_MIN_BOX + 1 || content_w < MIN_INPUT_WIDTH {
        return Layout::empty(true);
    }

    // 输入框：高度随软换行后的视觉行数增长，最多占内容区高度的一定比例（避免挤掉上方内容）。
    // 输入框下沿贴窗格底部；状态（模型 · 供应商 · 上下文占用）绘制在其底边框上。
    let rows = lx.visual_rows();
    let total = rows.len().max(1);
    let max_box = (area.height.saturating_mul(INPUT_MAX_PERCENT) / 100).max(INPUT_MIN_BOX);
    let cap = usize::from(max_box.saturating_sub(2)).max(1);
    let visible = total.min(cap);
    let box_h = u16::try_from(visible).unwrap_or(u16::MAX).saturating_add(2);
    let input = Rect::new(
        content_x,
        area.bottom().saturating_sub(box_h),
        content_w,
        box_h,
    );
    let max_scroll = total.saturating_sub(visible);
    let scroll = lx.scroll().min(max_scroll);

    let mut out = Layout::empty(false);
    out.input = Some(input);
    out.input_scroll = scroll;
    out.input_visible = visible;

    // 输入框上方一行的聊天栏：流中显示 loading 转轮，或上滚未跟随底部时显示回到底部按钮。
    if chat_bar::visible(lx) {
        out.chat_bar = chat_bar::rect(area, input);
    }

    // md 格式面板：贴输入框上方（聊天栏之上）；放不下则省略。
    let panel_base = out.chat_bar.map_or(input.y, |bar| bar.y);
    if let Some(panel) = lx.panel.as_ref() {
        let rows = panel.items().len();
        let panel_h = u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(2);
        let panel_w = panel_width(panel, content_w);
        if panel_base >= area.y + panel_h + SECTION_GAP {
            out.panel = Some(Rect::new(content_x, panel_base - panel_h, panel_w, panel_h));
        }
    }

    let region_top = area.y;

    // 入口区：标签页式单行，横排放不下则竖排。
    let labels: Vec<(LxEntry, &str)> = LxEntry::ALL
        .iter()
        .map(|entry| (*entry, text::lx_entry_label(*entry)))
        .collect();
    let label_widths: Vec<u16> = labels
        .iter()
        .map(|(_, label)| u16::try_from(label.width()).unwrap_or(u16::MAX))
        .collect();
    let max_label = label_widths.iter().copied().max().unwrap_or(0);
    let horizontal_total = label_widths
        .iter()
        .fold(0u16, |sum, width| sum.saturating_add(*width))
        .saturating_add(ENTRY_GAP.saturating_mul(2));
    let horizontal = horizontal_total <= content_w;
    let entries_h = if horizontal {
        ENTRY_HEIGHT
    } else {
        ENTRY_HEIGHT
            .saturating_mul(3)
            .saturating_add(ENTRY_VGAP.saturating_mul(2))
    };
    let entries_fit = horizontal || max_label <= content_w;

    let tip_h = ENTRY_HEIGHT;
    // 入口区与 Tip 的上限：输入框上沿（有聊天栏时为其上沿）。
    let top_limit = out.chat_bar.map_or(input.y, |bar| bar.y);

    // 入口按钮行对齐窗格垂直中心（与主区/prompt 交换按钮同一行），且不与输入框重叠。
    let center = area.y + area.height / 2;
    let entries_top = center
        .min(top_limit.saturating_sub(entries_h + brand::UNDERLINE_ROWS))
        .max(region_top);
    let entries_ok = entries_fit
        && entries_top
            .saturating_add(entries_h)
            .saturating_add(brand::UNDERLINE_ROWS)
            <= top_limit;

    // Tip 行贴按钮上方。
    let tip_top = if entries_ok {
        entries_top
            .checked_sub(SECTION_GAP + tip_h)
            .filter(|y| *y >= region_top)
    } else {
        None
    };

    // 品牌区：像素图标水平居中、底边距 Tip 2 行；放不下则整数倍缩小，仍放不下则隐藏。
    out.icon = tip_top.and_then(|tip_y| brand::place_icon(tip_y, region_top, content_x, content_w));

    if let Some(y) = tip_top {
        out.description = Some(TextRect {
            x: content_x,
            y,
            width: content_w,
        });
    }
    if entries_ok {
        out.horizontal = horizontal;
        out.entries = place_entries(
            &labels,
            &label_widths,
            horizontal,
            content_x,
            content_w,
            entries_top,
        );
    }
    out
}

/// md 面板宽度：最宽条目（标签 + 预览）+ 边框与内边距，夹取到内容区宽度。
fn panel_width(panel: &BlockPanel, content_w: u16) -> u16 {
    let inner = panel
        .items()
        .iter()
        .map(|id| {
            let (label, preview) = text::block_command_text(*id);
            label.width() + 2 + preview.width()
        })
        .max()
        .unwrap_or(0);
    u16::try_from(inner + 4).unwrap_or(u16::MAX).min(content_w)
}

/// 放置入口标签：横排整体居中、竖排逐行居中。
fn place_entries(
    labels: &[(LxEntry, &str)],
    label_widths: &[u16],
    horizontal: bool,
    x0: u16,
    width: u16,
    top: u16,
) -> Vec<(LxEntry, Rect)> {
    let mut out = Vec::with_capacity(labels.len());
    if horizontal {
        let total = label_widths
            .iter()
            .fold(0u16, |sum, entry| sum.saturating_add(*entry))
            .saturating_add(ENTRY_GAP.saturating_mul(2));
        let mut x = x0 + width.saturating_sub(total) / 2;
        for ((entry, _), label_w) in labels.iter().zip(label_widths) {
            out.push((*entry, Rect::new(x, top, *label_w, ENTRY_HEIGHT)));
            x = x.saturating_add(*label_w).saturating_add(ENTRY_GAP);
        }
    } else {
        let mut y = top;
        for ((entry, _), label_w) in labels.iter().zip(label_widths) {
            let x = x0 + width.saturating_sub(*label_w) / 2;
            out.push((*entry, Rect::new(x, y, *label_w, ENTRY_HEIGHT)));
            y = y.saturating_add(ENTRY_HEIGHT).saturating_add(ENTRY_VGAP);
        }
    }
    out
}

/// 构建整页网格；返回网格、聚焦时的输入光标相对坐标与置顶行绘制目标。
fn build(
    area: Rect,
    lx: &Lx,
    focused: bool,
) -> (Grid, Option<(u16, u16)>, Option<history::StickyRow>) {
    let lay = layout(area, lx);
    if !lx.chat.messages.is_empty() {
        return build_chat(area, lx, focused, &lay);
    }
    let mut grid = Grid::new(area);
    if lay.fallback {
        let y = area.y + area.height.saturating_sub(1) / 2;
        grid.put_centered(area.x, area.width, y, text::LX_TITLE, style::strong());
        return (grid, None, None);
    }

    if let Some((ix, iy)) = lay.icon {
        brand::draw_icon(&mut grid, ix, iy, lx.entry);
    }
    if let Some(rect) = lay.description {
        brand::draw_tip(
            &mut grid,
            Rect::new(rect.x, rect.y, rect.width, 1),
            lx.entry,
        );
    }

    brand::draw_entries(&mut grid, lx.entry, &lay.entries, lay.horizontal);

    chat_bar::draw(&mut grid, lay.chat_bar, lx);
    let mut cursor = None;
    if let Some(input) = lay.input {
        cursor = input::draw(
            &mut grid,
            input,
            lx,
            focused,
            lay.input_scroll,
            lay.input_visible,
        )
        .map(|(row, col)| (row.saturating_sub(area.y), col.saturating_sub(area.x)));
    }
    if let (Some(rect), Some(panel)) = (lay.panel, lx.panel.as_ref()) {
        draw_panel(&mut grid, rect, panel);
    }
    (grid, cursor, None)
}

/// 聊天模式构建：历史区 + 输入框（状态在**窗格底边框**上，由 `render` 覆盖绘制）。
fn build_chat(
    area: Rect,
    lx: &Lx,
    focused: bool,
    lay: &Layout,
) -> (Grid, Option<(u16, u16)>, Option<history::StickyRow>) {
    let mut grid = Grid::new(area);
    let history_bottom = if lay.input.is_some() {
        lay.chat_bar.map_or_else(
            || lay.input.map(|input| input.y).unwrap_or(area.bottom()),
            |bar| bar.y,
        )
    } else {
        area.bottom()
    };
    // 历史区与输入框同宽对齐（列 = 输入框列），高度到输入框（或聊天栏）上沿。
    let history_rect = Rect::new(
        lay.input.map(|input| input.x).unwrap_or(area.x),
        area.y,
        lay.input.map(|input| input.width).unwrap_or(area.width),
        history_bottom.saturating_sub(area.y),
    );
    // 历史视图：滚动偏移 / body（置顶行让位）/ 置顶行结论，统一由纯函数给出。
    let history_view = history::view(history_rect, lx);
    history::render(&mut grid, history_view.body, lx, history_view.scroll);
    // 历史滚动条贴右缘一列（内容溢出时；轨道只覆盖 body）。
    if let Some(bar) = history::scrollbar(
        history_view.body,
        history_view.total_rows,
        usize::from(history_view.body.height),
        history_view.scroll,
    ) {
        render_history_scrollbar(&mut grid, &bar);
    }
    let sticky = history::sticky_rect(&history_view);

    chat_bar::draw(&mut grid, lay.chat_bar, lx);
    let mut cursor = None;
    if let Some(input) = lay.input
        && lay.input_visible > 0
    {
        cursor = input::draw(
            &mut grid,
            input,
            lx,
            focused,
            lay.input_scroll,
            lay.input_visible,
        )
        .map(|(row, col)| (row.saturating_sub(area.y), col.saturating_sub(area.x)));
    }
    // 面板：块命令 / 粘贴 / 提及 / `/` 命令。
    if let (Some(rect), Some(panel)) = (lay.panel, lx.panel.as_ref()) {
        draw_panel(&mut grid, rect, panel);
    }
    (grid, cursor, sticky)
}

/// lx 页聊天命中结果。
///
/// 代码块复制按钮携带正文（自有 `String`），因此不是 `Copy`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LxChatHit {
    /// 点击置顶用户消息行（跳到该 prompt）。
    PromptHeader,
    /// 点击 thinking 折叠块头部（消息索引, part 索引）。
    Thinking(usize, usize),
    /// 点击「回到底部」按钮。
    JumpBottom,
    /// 点击代码块头部的复制按钮；`text` 为该块正文。
    CodeCopy(String),
}

/// 聊天历史区矩形（与输入框同宽对齐；无消息返回 None）；滚轮/命中/滚动条用。
pub fn history_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    if lx.chat.messages.is_empty() {
        return None;
    }
    let lay = layout(area, lx);
    // 历史区上沿到输入框（有聊天栏时为其上沿）。
    let bottom = lay.chat_bar.map_or_else(
        || lay.input.map(|input| input.y).unwrap_or(area.bottom()),
        |bar| bar.y,
    );
    let height = bottom.saturating_sub(area.y);
    if height == 0 {
        return None;
    }
    // 与输入框同宽对齐：x/width 取输入框列（扣掉两端外边距），右侧含滚动条预留列。
    let (x, width) = match lay.input {
        Some(input) => (input.x, input.width),
        None => (area.x, area.width),
    };
    Some(Rect::new(x, area.y, width, height))
}

/// 聊天历史滚轮命中区（与输入框同宽；上沿到输入框上沿，含聊天栏一行）。
///
/// 与 [`history_rect`] 的区别：滚轮区含输入框上方聊天栏（loading/回到底部按钮）一行，
/// 保证在聊天栏上滚轮同样滚动历史；渲染/滚动条仍用不含聊天栏的 [`history_rect`]。
pub fn history_wheel_region(area: Rect, lx: &Lx) -> Option<Rect> {
    if lx.chat.messages.is_empty() {
        return None;
    }
    let lay = layout(area, lx);
    let (x, width) = match lay.input {
        Some(input) => (input.x, input.width),
        None => (area.x, area.width),
    };
    let bottom = lay.input.map(|input| input.y).unwrap_or(area.bottom());
    let height = bottom.saturating_sub(area.y);
    (height > 0).then(|| Rect::new(x, area.y, width, height))
}

/// 聊天历史视图（滚动偏移 / body / 置顶行结论）；无历史区时为 None。
///
/// 渲染、滚轮钳制、滚动条与命中**统一**走它，保证"可见行数"只有一个口径。
pub(crate) fn history_view(area: Rect, lx: &Lx) -> Option<history::HistoryView> {
    let history_rect = history_rect(area, lx)?;
    Some(history::view(history_rect, lx))
}

/// 聊天历史滚动条几何（贴右缘一列；无消息或内容未溢出时为 None）。
pub fn history_scrollbar(area: Rect, lx: &Lx) -> Option<scrollbar::ScrollbarLayout> {
    let view = history_view(area, lx)?;
    history::scrollbar(
        view.body,
        view.total_rows,
        usize::from(view.body.height),
        view.scroll,
    )
}

/// 聊天历史的（渲染行数, 视口行数）；无消息时为 None。滚轮滚动按此钳制。
///
/// 视口行数取 **body**（置顶行让位后），与渲染同口径——否则滚不到最后一行。
pub fn history_rows(area: Rect, lx: &Lx) -> Option<(usize, usize)> {
    let view = history_view(area, lx)?;
    Some((view.total_rows, usize::from(view.body.height)))
}

/// 置顶用户消息行的"跳转目标"：该消息起始行（钳到 body 的滚动上限）；无置顶行返回 None。
pub fn sticky_prompt_jump(area: Rect, lx: &Lx) -> Option<usize> {
    let view = history_view(area, lx)?;
    let message = view.sticky?;
    let start = view.messages.get(message)?.start;
    // 跳过去后置顶行会让位，按整块 body 钳制。
    let max_top = view
        .total_rows
        .saturating_sub(usize::from(view.area.height));
    Some(start.min(max_top))
}

/// 聊天模式命中测试：回到底部按钮 > 置顶行 > thinking 头部。
pub fn chat_hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<LxChatHit> {
    if lx.chat.messages.is_empty() {
        return None;
    }
    // 输入框上方聊天栏右端的「回到底部」控件。
    if let Some(rect) = chat_bar::jump_bottom_rect(area, lx)
        && rect.contains((column, row).into())
    {
        return Some(LxChatHit::JumpBottom);
    }
    let view = history_view(area, lx)?;
    // 置顶行（表现层，整区第 0 行整宽）：点击跳回该 prompt。
    if let Some(sticky) = history::sticky_rect(&view)
        && sticky.rect.contains((column, row).into())
    {
        return Some(LxChatHit::PromptHeader);
    }
    // thinking 头部命中（按 body + 滚动窗口换算屏幕坐标）。
    for (message_index, part_index, header) in history::layout(view.body, lx).thinking {
        let offset = header.y.saturating_sub(view.body.y);
        if (offset as usize) < view.scroll {
            continue;
        }
        let screen_y = view
            .body
            .y
            .saturating_add(u16::try_from(offset as usize - view.scroll).unwrap_or(u16::MAX));
        let rect = Rect::new(view.body.x, screen_y, view.body.width, 1);
        if rect.contains((column, row).into()) {
            return Some(LxChatHit::Thinking(message_index, part_index));
        }
    }
    // 代码块头部的复制按钮（同样按 body + 滚动窗口换算屏幕坐标）。
    for button in &history::layout(view.body, lx).code_copy {
        let offset = button.rect.y.saturating_sub(view.body.y);
        if (offset as usize) < view.scroll {
            continue;
        }
        let screen_y = view
            .body
            .y
            .saturating_add(u16::try_from(offset as usize - view.scroll).unwrap_or(u16::MAX));
        let rect = Rect::new(button.rect.x, screen_y, button.rect.width, 1);
        if rect.contains((column, row).into()) {
            return Some(LxChatHit::CodeCopy(button.body.clone()));
        }
    }
    None
}

/// md 格式面板：圆角边框 + 标题 + 候选行（标签与预览，高亮项 accent）。
fn draw_panel(grid: &mut Grid, rect: Rect, panel: &BlockPanel) {
    draw_box(grid, rect, style::muted());
    grid.put(
        rect.x + 2,
        rect.y,
        text::LX_BLOCK_PANEL_TITLE,
        style::muted(),
    );
    for (index, id) in panel.items().iter().enumerate() {
        let y = rect.y + 1 + index as u16;
        let (label, preview) = text::block_command_text(*id);
        let active = index == panel.active();
        let label_style = if active {
            style::accent()
        } else {
            style::muted()
        };
        grid.put(rect.x + 2, y, &label, label_style);
        let preview_x = rect.x + 2 + u16::try_from(label.width()).unwrap_or(0) + 2;
        grid.put(preview_x, y, &preview, style::muted());
    }
}

/// 圆角细线边框（md 面板用）。
fn draw_box(grid: &mut Grid, rect: Rect, border: Style) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    grid.set(rect.x, rect.y, '╭', border);
    grid.set(right, rect.y, '╮', border);
    grid.set(rect.x, bottom, '╰', border);
    grid.set(right, bottom, '╯', border);
    for x in rect.x + 1..right {
        grid.set(x, rect.y, '─', border);
        grid.set(x, bottom, '─', border);
    }
    for y in rect.y + 1..bottom {
        grid.set(rect.x, y, '│', border);
        grid.set(right, y, '│', border);
    }
}

/// 测试辅助：仅构建聊天历史网格并 blit（不含输入框与面板）。
#[cfg(test)]
pub(crate) fn render_history_for_test(area: Rect, buf: &mut Buffer, lx: &Lx, scroll: usize) {
    let mut grid = Grid::new(area);
    history::render(&mut grid, area, lx, scroll);
    for dy in 0..area.height {
        for dx in 0..area.width {
            let glyph = grid.cell(area.x + dx, area.y + dy);
            if let Some(cell) = buf.cell_mut((area.x + dx, area.y + dy)) {
                if glyph.ch == CONTINUATION {
                    cell.set_diff_option(CellDiffOption::Skip);
                    continue;
                }
                cell.set_char(glyph.ch);
                cell.set_style(glyph.style);
                cell.set_diff_option(CellDiffOption::None);
            }
        }
    }
}

/// 测试辅助：把网格 blit 到缓冲区。
#[cfg(test)]
pub(super) fn blit_grid(area: Rect, buf: &mut Buffer, grid: &Grid) {
    for dy in 0..area.height {
        for dx in 0..area.width {
            let glyph = grid.cell(area.x + dx, area.y + dy);
            if let Some(cell) = buf.cell_mut((area.x + dx, area.y + dy)) {
                if glyph.ch == CONTINUATION {
                    cell.set_diff_option(CellDiffOption::Skip);
                    continue;
                }
                cell.set_char(glyph.ch);
                cell.set_style(glyph.style);
                cell.set_diff_option(CellDiffOption::None);
            }
        }
    }
}

#[cfg(test)]
mod tests;
