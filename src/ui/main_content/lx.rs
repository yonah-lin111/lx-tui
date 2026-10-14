//! lx 欢迎页：顶部品牌区（像素图标 + 文案）、极简入口按钮与底部单行输入框。
//!
//! 整页以单元格网格构建：同一份网格既用于绘制，也用于整页文本复制，
//! 因此渲染与取词不会漂移。渲染只读状态、只写 Buffer。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;

use crate::app::selection::Selection;
use crate::app::state::{Lx, LxEntry};
use crate::ui::{style, text};

/// 图标像素画尺寸（列 × 像素行 = 单元格行 × 2）。
pub(crate) const LOGO_WIDTH: u16 = 20;
pub(crate) const LOGO_ROWS: u16 = 7;

/// 底部单行输入框高度、宽屏水平边距与最小宽度。
const INPUT_HEIGHT: u16 = 1;
const MARGIN: u16 = 2;
const MIN_INPUT_WIDTH: u16 = 8;
/// 入口按钮高度（单行）、按钮间距、区块间距与图标右侧文案最小宽度。
const ENTRY_HEIGHT: u16 = 1;
const ENTRY_GAP: u16 = 2;
const SECTION_GAP: u16 = 1;
const MIN_TEXT_WIDTH: u16 = 12;

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
        LxEntry::Cognitive => match ch {
            'p' => 'n',
            'n' => 'p',
            'b' => 'c',
            'c' => 'b',
            other => other,
        },
        // 粉系 与 蓝系 互换。
        LxEntry::Assistant => match ch {
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
                cell.set_char(glyph.ch);
                let mut cell_style = glyph.style;
                if selection.is_some_and(|sel| sel.contains(i32::from(dy), dx)) {
                    cell_style = cell_style.add_modifier(Modifier::REVERSED);
                }
                cell.set_style(cell_style);
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
            line.push(grid.cell(area.x + col, y).ch);
        }
        lines.push(line.trim_end().to_string());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// 命中测试：坐标落在某个入口按钮内则返回该入口。
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

    /// 逐字符写入（假定单宽字符；`lx` 页文案均为单宽）。
    fn put(&mut self, x: u16, y: u16, content: &str, style: Style) {
        let mut cx = x;
        for ch in content.chars() {
            self.set(cx, y, ch, style);
            cx = cx.saturating_add(1);
        }
    }

    fn put_centered(&mut self, x0: u16, width: u16, y: u16, content: &str, style: Style) {
        let content_width = u16::try_from(content.chars().count()).unwrap_or(u16::MAX);
        if content_width > width {
            return;
        }
        self.put(x0 + (width - content_width) / 2, y, content, style);
    }
}

/// lx 页布局：所有组件矩形（绝对屏幕坐标）。
struct Layout {
    input: Option<Rect>,
    entries: Vec<(LxEntry, Rect)>,
    /// 入口按钮统一宽度（三按钮等宽）。
    button_width: u16,
    /// 最长入口标签宽度（按钮内左右填充用）。
    max_label: u16,
    icon: Option<(u16, u16)>,
    description: Option<Rect>,
    examples: Option<Rect>,
    /// 空间不足时退化为居中字标。
    fallback: bool,
}

impl Layout {
    fn empty(fallback: bool) -> Self {
        Self {
            input: None,
            entries: Vec::new(),
            button_width: 0,
            max_label: 0,
            icon: None,
            description: None,
            examples: None,
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

    // 底部单行输入框；放不下时退化为居中字标。
    if area.height < 2 || content_w < MIN_INPUT_WIDTH {
        return Layout::empty(true);
    }
    let input = Rect::new(content_x, area.bottom() - 1, content_w, INPUT_HEIGHT);
    let mut out = Layout::empty(false);
    out.input = Some(input);

    let region_top = area.y;
    let region_bottom = input.y;
    let region_h = region_bottom.saturating_sub(region_top);

    // 入口区：三按钮等宽（标签左右空格填充），横排放不下则竖排。
    let labels: Vec<(LxEntry, &str)> = LxEntry::ALL
        .iter()
        .map(|entry| (*entry, text::lx_entry_label(*entry)))
        .collect();
    let max_label = labels
        .iter()
        .map(|(_, label)| u16::try_from(label.chars().count()).unwrap_or(u16::MAX))
        .max()
        .unwrap_or(0);
    let button_w = max_label.saturating_add(2);
    out.button_width = button_w;
    out.max_label = max_label;
    let horizontal_total = button_w
        .saturating_mul(3)
        .saturating_add(ENTRY_GAP.saturating_mul(2));
    let horizontal = horizontal_total <= content_w;
    let entries_h = if horizontal {
        ENTRY_HEIGHT
    } else {
        ENTRY_HEIGHT
            .saturating_mul(3)
            .saturating_add(ENTRY_GAP.saturating_mul(2))
    };
    let entries_fit = horizontal || button_w <= content_w;

    // 品牌区：图标 + 右侧文案。
    let icon_fits = content_w >= LOGO_WIDTH;
    let text_x = content_x + LOGO_WIDTH + 2;
    let text_w = (area.right() - margin).saturating_sub(text_x);
    let text_fits = text_w >= MIN_TEXT_WIDTH;

    let mut brand_top: Option<u16> = None;
    let mut entries_top: Option<u16> = None;
    if entries_fit && icon_fits && region_h >= LOGO_ROWS + SECTION_GAP + entries_h {
        // 品牌区贴顶，入口区在其下方剩余空间内垂直居中。
        brand_top = Some(region_top);
        let top = region_top + LOGO_ROWS + SECTION_GAP;
        let avail = region_h - LOGO_ROWS - SECTION_GAP;
        entries_top = Some(top + (avail - entries_h) / 2);
    } else if entries_fit && region_h >= entries_h {
        entries_top = Some(region_top + (region_h - entries_h) / 2);
    } else if icon_fits && region_h >= LOGO_ROWS {
        brand_top = Some(region_top);
    }

    if let Some(top) = brand_top {
        out.icon = Some((content_x, top));
        if text_fits {
            out.description = Some(Rect::new(text_x, top, text_w, 1));
            if text::lx_entry_examples(lx.entry).is_some() && top + 1 < region_bottom {
                out.examples = Some(Rect::new(text_x, top + 1, text_w, 1));
            }
        }
    }

    if let Some(top) = entries_top {
        out.entries = place_entries(&labels, button_w, horizontal, content_x, content_w, top);
    }
    out
}

/// 按排布方向放置入口按钮：横排整体居中，竖排逐行居中；三按钮等宽。
fn place_entries(
    labels: &[(LxEntry, &str)],
    button_w: u16,
    horizontal: bool,
    x0: u16,
    width: u16,
    top: u16,
) -> Vec<(LxEntry, Rect)> {
    let mut out = Vec::with_capacity(labels.len());
    if horizontal {
        let total = button_w
            .saturating_mul(3)
            .saturating_add(ENTRY_GAP.saturating_mul(2));
        let mut x = x0 + width.saturating_sub(total) / 2;
        for (entry, _) in labels {
            out.push((*entry, Rect::new(x, top, button_w, ENTRY_HEIGHT)));
            x = x.saturating_add(button_w).saturating_add(ENTRY_GAP);
        }
    } else {
        let mut y = top;
        for (entry, _) in labels {
            let x = x0 + width.saturating_sub(button_w) / 2;
            out.push((*entry, Rect::new(x, y, button_w, ENTRY_HEIGHT)));
            y = y.saturating_add(ENTRY_HEIGHT).saturating_add(ENTRY_GAP);
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
        let content = text::truncate_to_width(
            text::lx_entry_description(lx.entry),
            usize::from(rect.width),
        );
        grid.put(rect.x, rect.y, &content, style::muted());
    }
    if let Some(rect) = lay.examples
        && let Some(examples) = text::lx_entry_examples(lx.entry)
    {
        let content = text::truncate_to_width(examples, usize::from(rect.width));
        grid.put(rect.x, rect.y, &content, style::muted());
    }

    for (entry, rect) in &lay.entries {
        draw_button(
            &mut grid,
            *rect,
            text::lx_entry_label(*entry),
            lay.max_label,
            *entry == lx.entry,
        );
    }

    let mut cursor = None;
    if let Some(input) = lay.input {
        cursor = draw_input(&mut grid, input, lx, focused)
            .map(|(row, col)| (row.saturating_sub(area.y), col.saturating_sub(area.x)));
    }
    (grid, cursor)
}

/// 极简入口按钮：`[` + 左右空格填充至等宽 + `]`；选中用强调色。
fn draw_button(grid: &mut Grid, rect: Rect, label: &str, max_label: u16, selected: bool) {
    let label_w = u16::try_from(label.chars().count()).unwrap_or(u16::MAX);
    let pad = max_label.saturating_sub(label_w);
    let left = pad / 2;
    let right = pad - left;
    let content = format!(
        "[{}{}{}]",
        " ".repeat(usize::from(left)),
        label,
        " ".repeat(usize::from(right))
    );
    grid.put(rect.x, rect.y, &content, style::lx_entry_label(selected));
}

/// 极简单行输入框：`> ` 前缀 + 输入内容/占位；返回光标相对坐标。
fn draw_input(grid: &mut Grid, rect: Rect, lx: &Lx, focused: bool) -> Option<(u16, u16)> {
    let row = rect.y;
    grid.put(rect.x, row, text::LX_INPUT_PROMPT, style::accent());
    let prefix = u16::try_from(text::LX_INPUT_PROMPT.chars().count()).unwrap_or(1) + 1;
    let content_x = rect.x + prefix;
    let content_w = usize::from(rect.width.saturating_sub(prefix));
    if content_w == 0 {
        return None;
    }
    let cursor_cols = display_width(&lx.input[..lx.cursor]);
    let skip = cursor_cols.saturating_sub(content_w.saturating_sub(1));
    let visible = slice_by_width(&lx.input, skip, content_w);
    if lx.input.is_empty() {
        let placeholder = text::truncate_to_width(text::LX_INPUT_PLACEHOLDER, content_w);
        grid.put(content_x, row, &placeholder, style::muted());
    } else {
        grid.put(content_x, row, &visible, style::text());
    }
    focused.then(|| {
        let col = content_x + u16::try_from(cursor_cols - skip).unwrap_or(u16::MAX);
        (row, col)
    })
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

/// 从显示列 `skip` 起取不超过 `max` 列的子串；宽字符不越界。
fn slice_by_width(content: &str, skip: usize, max: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    let mut width = 0usize;
    for ch in content.chars() {
        let char_width = ch.width().unwrap_or(0);
        if col < skip {
            col += char_width;
            continue;
        }
        if width + char_width > max {
            break;
        }
        out.push(ch);
        width += char_width;
    }
    out
}

#[cfg(test)]
mod tests;
