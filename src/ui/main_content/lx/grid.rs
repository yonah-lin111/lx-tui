//! lx 页绘制网格：一份单元格网格既用于绘制，也用于整页取词。
//!
//! 坐标均为绝对屏幕坐标；宽字符占位列写入 [`CONTINUATION`] 标记，复制/取词时跳过。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::markdown_render::TableLineRef;

/// 宽字符占位单元格的标记；不参与绘制与取词。
pub(super) const CONTINUATION: char = '\0';

/// 网格中单行的表格几何（列偏移相对 `Grid.area.x`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GridTableRow {
    /// 表格左边框在内容网格中的起始列（相对 `area.x`）。
    pub(crate) base_col: u16,
    /// 表格结构元数据与行内偏移。
    pub(crate) line_ref: TableLineRef,
}

/// 网格中单行的代码块装饰元数据（列偏移相对 `Grid.area.x`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GridCodeRow {
    /// 顶部边框行（`▂`）或下一行头部（`▎ <lang> ... [❐]`）：选区不反显且复制时整行跳过。
    Chrome,
    /// 代码块正文行：`bar_col` 为左侧 `▎` 单栏所在的相对列偏移（选区仅作用于 `bar_col + 1` 右侧）。
    Body { bar_col: u16 },
}

/// 网格中单行的超链接热区（列偏移相对 `Grid.area.x`，左闭右开 `start_col..end_col`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GridLinkSpan {
    pub(crate) start_col: u16,
    pub(crate) end_col: u16,
    pub(crate) url: String,
}

/// 一个待绘制单元格。
#[derive(Clone, Copy)]
pub(super) struct Glyph {
    pub(super) ch: char,
    pub(super) style: Style,
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
pub(crate) struct Grid {
    area: Rect,
    cells: Vec<Glyph>,
    table_rows: Vec<Option<GridTableRow>>,
    code_rows: Vec<Option<GridCodeRow>>,
    link_rows: Vec<Vec<GridLinkSpan>>,
}

impl Grid {
    pub(super) fn new(area: Rect) -> Self {
        Self {
            area,
            cells: vec![Glyph::blank(); usize::from(area.width) * usize::from(area.height)],
            table_rows: vec![None; usize::from(area.height)],
            code_rows: vec![None; usize::from(area.height)],
            link_rows: vec![Vec::new(); usize::from(area.height)],
        }
    }

    /// 网格对应的屏幕矩形。
    pub(super) fn area(&self) -> Rect {
        self.area
    }

    /// 登记第 `y` 行的表格结构元数据（`x` 为表格左边框的绝对屏幕列坐标）。
    pub(super) fn set_table_row(&mut self, x: u16, y: u16, line_ref: TableLineRef) {
        if x < self.area.x || y < self.area.y {
            return;
        }
        let dy = y - self.area.y;
        if dy < self.area.height {
            self.table_rows[usize::from(dy)] = Some(GridTableRow {
                base_col: x - self.area.x,
                line_ref,
            });
        }
    }

    /// 读取相对行 `dy`（`0..area.height`）的表格行元数据。
    pub(super) fn table_row_at(&self, dy: u16) -> Option<&GridTableRow> {
        self.table_rows
            .get(usize::from(dy))
            .and_then(Option::as_ref)
    }

    /// 登记第 `y` 行的代码块装饰元数据。
    pub(super) fn set_code_row(&mut self, y: u16, row: GridCodeRow) {
        if y < self.area.y {
            return;
        }
        let dy = y - self.area.y;
        if dy < self.area.height {
            self.code_rows[usize::from(dy)] = Some(row);
        }
    }

    /// 读取相对行 `dy`（`0..area.height`）的代码块行元数据。
    pub(super) fn code_row_at(&self, dy: u16) -> Option<GridCodeRow> {
        self.code_rows.get(usize::from(dy)).copied().flatten()
    }

    /// 登记第 `y` 行屏幕列 `[x, x + cols)` 的超链接热区（自动裁剪到 `area` 范围内）。
    pub(super) fn set_link_span(&mut self, x: u16, y: u16, cols: u16, url: &str) {
        if cols == 0 || y < self.area.y || x >= self.area.right() {
            return;
        }
        let dy = y - self.area.y;
        if dy >= self.area.height {
            return;
        }
        let start_col = x.saturating_sub(self.area.x);
        let end_col = x
            .saturating_add(cols)
            .min(self.area.right())
            .saturating_sub(self.area.x);
        if start_col < end_col
            && let Some(links) = self.link_rows.get_mut(usize::from(dy))
        {
            links.push(GridLinkSpan {
                start_col,
                end_col,
                url: url.to_string(),
            });
        }
    }

    /// 读取相对坐标 `(dx, dy)` 命中的超链接 URL。
    pub(super) fn link_at(&self, dx: u16, dy: u16) -> Option<&str> {
        self.link_rows
            .get(usize::from(dy))?
            .iter()
            .find(|span| dx >= span.start_col && dx < span.end_col)
            .map(|span| span.url.as_str())
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

    pub(super) fn cell(&self, x: u16, y: u16) -> Glyph {
        self.index(x, y)
            .map(|index| self.cells[index])
            .unwrap_or_else(Glyph::blank)
    }

    pub(super) fn set(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if let Some(index) = self.index(x, y) {
            self.cells[index] = Glyph { ch, style };
        }
    }

    /// 逐字符写入；宽字符按显示宽度占位，后续列为占位标记，零宽组合字符不独立占格。
    pub(super) fn put(&mut self, x: u16, y: u16, content: &str, style: Style) {
        let mut cx = x;
        for ch in content.chars() {
            let width = ch.width().unwrap_or(0);
            if width == 0 {
                continue;
            }
            self.set(cx, y, ch, style);
            for offset in 1..width {
                self.set(cx.saturating_add(offset as u16), y, CONTINUATION, style);
            }
            cx = cx.saturating_add(width as u16);
        }
    }

    /// 裁剪写入：超过 `width` 列的内容截断（宽字符不跨列半写）。
    pub(super) fn put_clipped(&mut self, x: u16, y: u16, content: &str, style: Style, width: u16) {
        let right = x.saturating_add(width);
        let mut cx = x;
        for ch in content.chars() {
            let cell = ch.width().unwrap_or(0) as u16;
            if cell == 0 {
                continue;
            }
            if cx.saturating_add(cell) > right {
                break;
            }
            self.set(cx, y, ch, style);
            for offset in 1..cell {
                self.set(cx.saturating_add(offset), y, CONTINUATION, style);
            }
            cx = cx.saturating_add(cell);
        }
    }

    /// 单字符写入（越界忽略）。
    pub(super) fn put_char(&mut self, x: u16, y: u16, ch: char, style: Style) {
        let width = ch.width().unwrap_or(1).max(1);
        self.set(x, y, ch, style);
        for offset in 1..width {
            self.set(x.saturating_add(offset as u16), y, CONTINUATION, style);
        }
    }

    /// 把整段（`width` 列）铺成空格并套用 `style`（用户消息整块底色）。
    pub(super) fn background_row(&mut self, x: u16, y: u16, width: u16, style: Style) {
        for offset in 0..width {
            self.set(x.saturating_add(offset), y, ' ', style);
        }
    }

    /// 为整段（`width` 列）叠底色，保留已有字符与前景样式（系统事件卡片选中态）。
    pub(super) fn tint_row_bg(&mut self, x: u16, y: u16, width: u16, style: Style) {
        for offset in 0..width {
            if let Some(index) = self.index(x.saturating_add(offset), y) {
                self.cells[index].style = self.cells[index].style.patch(style);
            }
        }
    }

    pub(super) fn put_centered(
        &mut self,
        x0: u16,
        width: u16,
        y: u16,
        content: &str,
        style: Style,
    ) {
        let content_width = u16::try_from(content.width()).unwrap_or(u16::MAX);
        if content_width > width {
            return;
        }
        self.put(x0 + (width - content_width) / 2, y, content, style);
    }
}
