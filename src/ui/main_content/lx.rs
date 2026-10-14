//! lx 欢迎页：顶部品牌区（像素图标 + 文案）、标签页式入口与底部多行 markdown 输入框。
//!
//! 整页以单元格网格构建：同一份网格既用于绘制，也用于整页文本复制，
//! 因此渲染与取词不会漂移。渲染只读状态、只写 Buffer。

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::markdown::BlockPanel;
use crate::app::selection::Selection;
use crate::app::state::{Lx, LxEntry};
use crate::ui::markdown::Token;
use crate::ui::{style, text};

/// 宽字符占位单元格的标记；不参与绘制与取词。
const CONTINUATION: char = '\0';

/// 图标像素画尺寸（列 × 像素行 = 单元格行 × 2）。
pub(crate) const LOGO_WIDTH: u16 = 20;
pub(crate) const LOGO_ROWS: u16 = 7;

/// 宽屏水平边距、输入框最小宽度与输入框最小高度（1 内容行 + 上下分割线）。
const MARGIN: u16 = 2;
const MIN_INPUT_WIDTH: u16 = 8;
const INPUT_MIN_BOX: u16 = 3;
/// 入口区：单行高度、横排标签间距（含分隔符）、竖排行距与区块间距。
const ENTRY_HEIGHT: u16 = 1;
const ENTRY_GAP: u16 = 3;
const ENTRY_VGAP: u16 = 1;
const SECTION_GAP: u16 = 1;
/// 输入框上下分割线字符。
const RULE: char = '─';

/// 基准 LX 像素字标（20 列 × 14 像素行）；'.' 透明，其余字符取 `style::logo_pixel` 调色板。
///
/// 三个入口共用同一形状，仅按入口做配色微调，保证 `LX` 形象一致。
const ICON_LX: &[&str] = &[
    ".kkk...kkkkk....kkkk",
    "kppbk..kccck...kpppk",
    "kppbk..kbbbk...kppnk",
    "kppbk..kbbbk..kppnnk",
    "kppbk..kkbbbbkppnnk.",
    "kppbk...kbbbkppnnk..",
    "kppbk....kbkppnnk...",
    "kppbk....kbkppnk....",
    "kppbk....kppnnknpk..",
    "kppbk...kppnnknnpk..",
    "kppbk..kppnnknnnnpk.",
    "kppbkppkpppnkk.knnnk",
    "kbbbbbkpnnkk...knnnk",
    "kkkkkkkkkkk.....kkkk",
];

/// 按入口对基准字标做配色微调：形状不变、只换调色板槽位。
fn icon_pixel(entry: LxEntry, ch: char) -> char {
    match entry {
        LxEntry::Native => ch,
        // 粉/深粉 与 天蓝/冰蓝 互换。
        LxEntry::Hybrid => match ch {
            'p' => 'n',
            'n' => 'p',
            'b' => 'c',
            'c' => 'b',
            other => other,
        },
        // 粉系 与 蓝系 互换。
        LxEntry::Automation => match ch {
            'p' => 'c',
            'c' => 'p',
            'b' => 'n',
            'n' => 'b',
            other => other,
        },
    }
}

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
    let (grid, cursor) = build(area, lx, focused);
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
    cursor
}

/// 提取选区文本（内容行、列相对内容区左上角）；空选区返回空串。
pub fn copy_text(area: Rect, lx: &Lx, range: ((i32, u16), (i32, u16))) -> String {
    let (grid, _) = build(area, lx, false);
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

/// 一个待绘制单元格。
#[derive(Clone, Copy)]
struct Glyph {
    ch: char,
    style: Style,
}

impl Glyph {
    fn blank() -> Self {
        Self {
            ch: ' ',
            style: Style::default(),
        }
    }
}

/// 可写字符网格（坐标为绝对屏幕坐标）。
struct Grid {
    area: Rect,
    cells: Vec<Glyph>,
}

impl Grid {
    fn new(area: Rect) -> Self {
        Self {
            area,
            cells: vec![Glyph::blank(); usize::from(area.width) * usize::from(area.height)],
        }
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.area.x || y < self.area.y {
            return None;
        }
        let dx = x - self.area.x;
        let dy = y - self.area.y;
        (dx < self.area.width && dy < self.area.height)
            .then(|| usize::from(dy) * usize::from(self.area.width) + usize::from(dx))
    }

    fn cell(&self, x: u16, y: u16) -> Glyph {
        self.index(x, y)
            .map(|index| self.cells[index])
            .unwrap_or_else(Glyph::blank)
    }

    fn set(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if let Some(index) = self.index(x, y) {
            self.cells[index] = Glyph { ch, style };
        }
    }

    /// 逐字符写入；宽字符按显示宽度占位，后续列为占位标记。
    fn put(&mut self, x: u16, y: u16, content: &str, style: Style) {
        let mut cx = x;
        for ch in content.chars() {
            let width = ch.width().unwrap_or(1).max(1);
            self.set(cx, y, ch, style);
            for offset in 1..width {
                self.set(cx.saturating_add(offset as u16), y, CONTINUATION, style);
            }
            cx = cx.saturating_add(width as u16);
        }
    }

    fn put_centered(&mut self, x0: u16, width: u16, y: u16, content: &str, style: Style) {
        let content_width = u16::try_from(content.width()).unwrap_or(u16::MAX);
        if content_width > width {
            return;
        }
        self.put(x0 + (width - content_width) / 2, y, content, style);
    }
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

    // 输入框：高度随内容行数增长，最多占内容区高度的一半。
    let lines = lx.input.split('\n').count().max(1);
    let max_box = (area.height / 2).max(INPUT_MIN_BOX);
    let cap = usize::from(max_box.saturating_sub(2)).max(1);
    let visible = lines.min(cap);
    let box_h = u16::try_from(visible).unwrap_or(u16::MAX).saturating_add(2);
    let input = Rect::new(content_x, area.bottom() - box_h, content_w, box_h);
    let cursor_line = lx.input[..lx.cursor].matches('\n').count();
    let scroll = cursor_line.saturating_sub(visible.saturating_sub(1));

    let mut out = Layout::empty(false);
    out.input = Some(input);
    out.input_scroll = scroll;
    out.input_visible = visible;

    // md 格式面板：贴输入框上方；放不下则省略。
    if let Some(panel) = lx.panel.as_ref() {
        let rows = panel.items().len();
        let panel_h = u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(2);
        let panel_w = panel_width(panel, content_w);
        if input.y >= area.y + panel_h + SECTION_GAP {
            out.panel = Some(Rect::new(content_x, input.y - panel_h, panel_w, panel_h));
        }
    }

    let region_top = area.y;
    let region_bottom = out.panel.map(|rect| rect.y).unwrap_or(input.y);
    let region_h = region_bottom.saturating_sub(region_top);

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

    // 品牌区：仅图标，靠左上。入口区与其上方的 Tip 行作为一个整体。
    let icon_fits = content_w >= LOGO_WIDTH;
    let brand_h = LOGO_ROWS;
    let tip_h = ENTRY_HEIGHT;
    let group_h = tip_h + SECTION_GAP + entries_h;

    let mut brand_top: Option<u16> = None;
    let mut group_top: Option<u16> = None;
    if entries_fit && icon_fits && region_h >= brand_h + SECTION_GAP + group_h {
        brand_top = Some(region_top);
        let top = region_top + brand_h + SECTION_GAP;
        let avail = region_h - brand_h - SECTION_GAP;
        group_top = Some(top + (avail - group_h) / 2);
    } else if entries_fit && region_h >= group_h {
        group_top = Some(region_top + (region_h - group_h) / 2);
    } else if icon_fits && region_h >= brand_h {
        brand_top = Some(region_top);
    }

    if let Some(top) = brand_top {
        out.icon = Some((content_x, top));
    }

    if let Some(top) = group_top {
        out.description = Some(TextRect {
            x: content_x,
            y: top,
            width: content_w,
        });
        out.horizontal = horizontal;
        out.entries = place_entries(
            &labels,
            &label_widths,
            horizontal,
            content_x,
            content_w,
            top + tip_h + SECTION_GAP,
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

/// 构建整页网格；返回网格与聚焦时的输入光标相对坐标。
fn build(area: Rect, lx: &Lx, focused: bool) -> (Grid, Option<(u16, u16)>) {
    let mut grid = Grid::new(area);
    let lay = layout(area, lx);
    if lay.fallback {
        let y = area.y + area.height.saturating_sub(1) / 2;
        grid.put_centered(area.x, area.width, y, text::LX_TITLE, style::strong());
        return (grid, None);
    }

    if let Some((ix, iy)) = lay.icon {
        draw_icon(&mut grid, ix, iy, lx.entry);
    }
    if let Some(rect) = lay.description {
        draw_tip(&mut grid, rect, text::lx_entry_description(lx.entry));
    }

    draw_entries(&mut grid, lx.entry, &lay.entries, lay.horizontal);

    let mut cursor = None;
    if let Some(input) = lay.input {
        cursor = draw_input(
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
    (grid, cursor)
}

/// 绘制 Tip 提示行：`Tip:` 前缀强调色、句子 muted，整体在 `rect` 内居中、超宽截断。
fn draw_tip(grid: &mut Grid, rect: TextRect, sentence: &str) {
    let full = format!("{} {}", text::LX_TIP_PREFIX, sentence);
    let full = text::truncate_to_width(&full, usize::from(rect.width));
    let full_w = u16::try_from(full.width()).unwrap_or(u16::MAX);
    let start = rect.x + rect.width.saturating_sub(full_w) / 2;
    match full.strip_prefix(text::LX_TIP_PREFIX) {
        Some(rest) => {
            grid.put(start, rect.y, text::LX_TIP_PREFIX, style::accent());
            let prefix_w = u16::try_from(text::LX_TIP_PREFIX.width()).unwrap_or(0);
            grid.put(start + prefix_w, rect.y, rest, style::muted());
        }
        None => grid.put(start, rect.y, &full, style::muted()),
    }
}

/// 标签页式入口：选中 accent、未选中 muted；横排用 `·` 分隔。
fn draw_entries(grid: &mut Grid, selected: LxEntry, entries: &[(LxEntry, Rect)], horizontal: bool) {
    for (index, (entry, rect)) in entries.iter().enumerate() {
        grid.put(
            rect.x,
            rect.y,
            text::lx_entry_label(*entry),
            style::lx_entry_label(*entry == selected),
        );
        if horizontal && let Some((_, next)) = entries.get(index + 1) {
            let gap = next.x.saturating_sub(rect.right());
            grid.put(
                rect.right() + gap / 2,
                rect.y,
                text::LX_ENTRY_SEPARATOR,
                style::muted(),
            );
        }
    }
}

/// 底部多行输入框：上下 `─` 分割线 + 首行 `> ` 前缀 + markdown 高亮；返回光标绝对坐标。
fn draw_input(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
    focused: bool,
    scroll: usize,
    visible: usize,
) -> Option<(u16, u16)> {
    let text_top = rect.y + 1;
    let bottom_rule = rect.bottom() - 1;
    for x in rect.x..rect.right() {
        grid.set(x, rect.y, RULE, style::muted());
        grid.set(x, bottom_rule, RULE, style::muted());
    }

    let prefix = u16::try_from(text::LX_INPUT_PROMPT.width()).unwrap_or(1) + 1;
    let content_x = rect.x + prefix;
    let content_w = rect.width.saturating_sub(prefix);
    let tokens = crate::ui::markdown::scan(&lx.input);
    let lines: Vec<&str> = lx.input.split('\n').collect();
    let cursor_line = lx.input[..lx.cursor].matches('\n').count();
    let line_start = lx.input[..lx.cursor]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let cursor_in_line = lx.cursor - line_start;

    let mut cursor = None;
    for i in 0..visible {
        let Some(line_index) = scroll.checked_add(i) else {
            break;
        };
        let Some(line) = lines.get(line_index) else {
            break;
        };
        let y = text_top + i as u16;
        if line_index == 0 {
            grid.put(rect.x, y, text::LX_INPUT_PROMPT, style::accent());
        }
        let line_tokens = tokens
            .get(line_index)
            .map(Vec::as_slice)
            .unwrap_or_default();
        paint_markdown_line(grid, content_x, y, line, line_tokens, content_w);
        if line_index == cursor_line && focused {
            let col = content_x
                + u16::try_from(display_width(&line[..cursor_in_line])).unwrap_or(u16::MAX);
            cursor = Some((y, col));
        }
    }

    if lx.input.is_empty() {
        let placeholder =
            text::truncate_to_width(text::LX_INPUT_PLACEHOLDER, usize::from(content_w));
        grid.put(content_x, text_top, &placeholder, style::muted());
    }
    cursor
}

/// 按 token 高亮绘制一行 markdown；超宽截断。
fn paint_markdown_line(grid: &mut Grid, x: u16, y: u16, line: &str, tokens: &[Token], max_w: u16) {
    let mut cx = x;
    let mut token_index = 0;
    for (offset, ch) in line.char_indices() {
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            continue;
        }
        while tokens
            .get(token_index)
            .is_some_and(|token| token.range.end <= offset)
        {
            token_index += 1;
        }
        let cell_style = tokens
            .get(token_index)
            .filter(|token| token.range.contains(&offset))
            .map_or_else(style::text, |token| {
                crate::ui::prompt::token_style(token.kind)
            });
        if cx.saturating_add(width as u16) > x.saturating_add(max_w) {
            break;
        }
        grid.set(cx, y, ch, cell_style);
        for dx in 1..width as u16 {
            grid.set(cx.saturating_add(dx), y, CONTINUATION, cell_style);
        }
        cx = cx.saturating_add(width as u16);
    }
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

/// 把像素图标画进以 `(x, y)` 为左上角的单元格区域（按入口配色微调）。
fn draw_icon(grid: &mut Grid, x: u16, y: u16, entry: LxEntry) {
    for (row, pair) in ICON_LX.chunks(2).enumerate() {
        let [top, bottom] = pair else {
            break;
        };
        let cell_y = y + row as u16;
        for column in 0..LOGO_WIDTH {
            let top = icon_pixel(entry, top.chars().nth(usize::from(column)).unwrap_or('.'));
            let bottom = icon_pixel(
                entry,
                bottom.chars().nth(usize::from(column)).unwrap_or('.'),
            );
            if let Some((ch, pixel_style)) = pixel_glyph(top, bottom) {
                grid.set(x + column, cell_y, ch, pixel_style);
            }
        }
    }
}

/// 直接把基准 LX 图标像素画进缓冲区（信任页品牌区复用；不走整页网格）。
pub(crate) fn draw_art(buf: &mut Buffer, x: u16, y: u16) {
    for (row, pair) in ICON_LX.chunks(2).enumerate() {
        let [top, bottom] = pair else {
            break;
        };
        let cell_y = y + row as u16;
        for column in 0..LOGO_WIDTH {
            let top = top.chars().nth(usize::from(column)).unwrap_or('.');
            let bottom = bottom.chars().nth(usize::from(column)).unwrap_or('.');
            if let Some((ch, pixel_style)) = pixel_glyph(top, bottom)
                && let Some(cell) = buf.cell_mut((x + column, cell_y))
            {
                cell.set_char(ch);
                cell.set_style(pixel_style);
            }
        }
    }
}

/// 两个纵向像素画进一个单元格：同色实块，异色上半块（前景上/背景下）。
fn pixel_glyph(top: char, bottom: char) -> Option<(char, Style)> {
    match (style::logo_pixel(top), style::logo_pixel(bottom)) {
        (None, None) => None,
        (Some(color), None) => Some(('▀', Style::default().fg(color))),
        (None, Some(color)) => Some(('▄', Style::default().fg(color))),
        (Some(color), Some(_)) if top == bottom => Some(('█', Style::default().fg(color))),
        (Some(top_color), Some(bottom_color)) => {
            Some(('▀', Style::default().fg(top_color).bg(bottom_color)))
        }
    }
}

/// 文本显示宽度（宽字符按两列计）。
fn display_width(content: &str) -> usize {
    content.chars().map(|ch| ch.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests;
