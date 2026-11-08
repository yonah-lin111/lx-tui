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
    /// 顶部边框行（`▂`）、底部边框行（`🮂`）或内部第一行头部（`▎<lang> ... [❐]🮇`）：选区不反显且复制时整行跳过。
    Chrome,
    /// 代码块正文行：`left_bar_col` 为左侧 `▎` 相对列，`right_bar_col` 为右侧 `🮇` 相对列（选区仅作用于两边框内部）。
    Body {
        left_bar_col: u16,
        right_bar_col: u16,
    },
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
}

impl Grid {
    pub(super) fn new(area: Rect) -> Self {
        Self {
            area,
            cells: vec![Glyph::blank(); usize::from(area.width) * usize::from(area.height)],
            table_rows: vec![None; usize::from(area.height)],
            code_rows: vec![None; usize::from(area.height)],
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

    /// 把整段（`width` 列）铺成空格并套用 `style`（用户消息整块底色 / 状态覆盖底边框）。
    pub(super) fn background_row(&mut self, x: u16, y: u16, width: u16, style: Style) {
        for offset in 0..width {
            self.set(x.saturating_add(offset), y, ' ', style);
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
