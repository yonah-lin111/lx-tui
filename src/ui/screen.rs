//! 最终可见画面取样：把 ratatui `Buffer` 转成只读屏幕文本，供选择、复制与高亮定位。
//!
//! 只读取已渲染 Buffer，不修改状态、不执行 IO；宽字符续格不重复输出。

use ratatui::buffer::Buffer;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::selection::{FrozenRow, ScreenPoint, SelectedCell, SelectedCells, SelectionUnit};

/// 最终画面的一格；宽字符续格 `width == 0` 且符号为空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCell {
    pub symbol: String,
    pub width: u8,
}

/// 一帧最终可见文本。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenText {
    width: u16,
    height: u16,
    cells: Vec<ScreenCell>,
}

impl ScreenText {
    /// 从最终 Buffer 取样；调用方须在所有图层绘制完成、选择反显之前调用。
    pub fn from_buffer(buffer: &Buffer) -> Self {
        let area = buffer.area;
        let width = area.width;
        let height = area.height;
        let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
        for y in area.top()..area.bottom() {
            let mut skip = 0u16;
            for x in area.left()..area.right() {
                if skip > 0 {
                    skip -= 1;
                    cells.push(ScreenCell {
                        symbol: String::new(),
                        width: 0,
                    });
                    continue;
                }
                let symbol = buffer[(x, y)].symbol().to_string();
                let width = symbol.width().max(1) as u8;
                skip = u16::from(width).saturating_sub(1);
                cells.push(ScreenCell { symbol, width });
            }
        }
        Self {
            width,
            height,
            cells,
        }
    }

    /// 画面宽高。
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// 读取一格；越界返回 None。
    pub fn cell(&self, x: u16, y: u16) -> Option<&ScreenCell> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.cells
            .get(usize::from(y) * usize::from(self.width) + usize::from(x))
    }

    /// 一整行的可见文本（宽字符只保留首格）。
    pub fn row_text(&self, y: u16) -> Option<String> {
        if y >= self.height {
            return None;
        }
        let start = usize::from(y) * usize::from(self.width);
        let mut line = String::with_capacity(usize::from(self.width));
        for cell in &self.cells[start..start + usize::from(self.width)] {
            if cell.width > 0 {
                line.push_str(&cell.symbol);
            }
        }
        Some(line)
    }

    /// 冻结 `start..=end` 覆盖的画面片段；按屏幕阅读顺序，端点包含。
    pub fn selection_cells(&self, start: ScreenPoint, end: ScreenPoint) -> SelectedCells {
        let Some((start, end)) = normalize(start, end) else {
            return SelectedCells::default();
        };
        if start.row >= self.height || end.row >= self.height {
            return SelectedCells::default();
        }
        let mut rows = Vec::new();
        for row in start.row..=end.row {
            let (col_start, col_end) = if start.row == end.row {
                (start.col, end.col)
            } else if row == start.row {
                // 首行只延伸到按下点所在容器右边界；末行从终点所在容器左边界
                // 开始。中间行取起止点跨越的容器段，不能直接铺满整个屏幕。
                let right = self
                    .container_right_shared(row, start.col, start.row)
                    .map(|col| col.saturating_sub(1))
                    .unwrap_or_else(|| self.width.saturating_sub(1));
                let end = self.trim_row_end(row, start.col, right);
                (start.col, end.unwrap_or(start.col.saturating_sub(1)))
            } else if row == end.row {
                let left = self
                    .container_left_shared(row, end.col, start.row)
                    .map(|col| col.saturating_add(1))
                    .unwrap_or(0);
                (left, end.col)
            } else {
                let low = start.col.min(end.col);
                let high = start.col.max(end.col);
                let (left_ref, right_ref) = if start.col <= end.col {
                    (start.row, end.row)
                } else {
                    (end.row, start.row)
                };
                let left = self
                    .container_left_shared(row, low, left_ref)
                    .map(|col| col.saturating_add(1))
                    .unwrap_or(0);
                let right = self
                    .container_right_shared(row, high, right_ref)
                    .map(|col| col.saturating_sub(1))
                    .unwrap_or_else(|| self.width.saturating_sub(1));
                (
                    left,
                    self.trim_row_end(row, left, right)
                        .unwrap_or(left.saturating_sub(1)),
                )
            };
            let full = self.row_text(row).unwrap_or_default();
            if col_start > col_end {
                rows.push(FrozenRow {
                    full,
                    start_col: col_start,
                    end_col: col_start,
                    cells: Vec::new(),
                });
                continue;
            }
            let (col_start, col_end) = self.widen_columns(row, col_start, col_end);
            let mut cells = Vec::new();
            for col in col_start..=col_end {
                if let Some(cell) = self.cell(col, row)
                    && cell.width > 0
                {
                    cells.push(SelectedCell {
                        symbol: cell.symbol.clone(),
                        width: cell.width,
                    });
                }
            }
            rows.push(FrozenRow {
                full,
                start_col: col_start,
                end_col: col_end,
                cells,
            });
        }
        SelectedCells {
            rows,
            start: Some(start),
            end: Some(end),
        }
    }

    /// 双击选词：命中词字符时取连续词；否则取该单格。
    pub fn word_range(&self, point: ScreenPoint) -> (ScreenPoint, ScreenPoint) {
        let text = self.row_text(point.row).unwrap_or_default();
        let chars: Vec<char> = text.chars().collect();
        let mut col = 0usize;
        let mut index = chars.len();
        for (i, ch) in chars.iter().enumerate() {
            let width = ch.width().unwrap_or(1).max(1);
            if col >= usize::from(point.col) {
                index = i;
                break;
            }
            col += width;
        }
        if index >= chars.len() {
            return (point, point);
        }
        let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
        let hit = chars[index];
        let (mut from, mut to) = (index, index);
        if is_word(hit) {
            while from > 0 && is_word(chars[from - 1]) {
                from -= 1;
            }
            while to + 1 < chars.len() && is_word(chars[to + 1]) {
                to += 1;
            }
        }
        let start_col = chars[..from]
            .iter()
            .map(|ch| ch.width().unwrap_or(1).max(1) as u16)
            .sum();
        let end_col = start_col
            + chars[from..=to]
                .iter()
                .map(|ch| ch.width().unwrap_or(1).max(1) as u16)
                .sum::<u16>()
                .saturating_sub(1);
        (
            ScreenPoint::new(point.row, start_col),
            ScreenPoint::new(point.row, end_col),
        )
    }

    /// 行内最后一个非空白列；全空白返回 None。
    fn trim_row_end(&self, row: u16, start: u16, end: u16) -> Option<u16> {
        if start > end || end >= self.width {
            return None;
        }
        let mut col = end;
        loop {
            if self.char_at(col, row).is_some_and(|ch| !ch.is_whitespace()) {
                return Some(col);
            }
            if col == start {
                return None;
            }
            col -= 1;
        }
    }

    /// 两行共有的、位于 `col` 左侧最近的竖边界。
    ///
    /// 只认跨行都存在的边界，避免把卡片树里的单行 `└─` 误当成容器边界。
    fn container_left_shared(&self, row: u16, col: u16, reference: u16) -> Option<u16> {
        (0..=col.min(self.width.saturating_sub(1)))
            .rev()
            .find(|col| {
                self.is_vertical_boundary(*col, row) && self.is_vertical_boundary(*col, reference)
            })
    }

    /// 两行共有的、位于 `col` 右侧最近的竖边界。
    fn container_right_shared(&self, row: u16, col: u16, reference: u16) -> Option<u16> {
        (col.min(self.width.saturating_sub(1))..self.width).find(|col| {
            self.is_vertical_boundary(*col, row) && self.is_vertical_boundary(*col, reference)
        })
    }

    /// 三击选中当前视觉行：以最近的左右边框为界，不跨面板/表格列。
    ///
    /// 普通内容行保留行首缩进、裁掉行尾填充空白；空白行不形成选区。竖边框列
    /// 不进入选区；纯横边框行（如 `├───┤`）返回空选区。边框上的标题与状态文字
    /// （`╭─ Prompt ─╮`、`╰ model … ─╯`）只取文字本身，不复制边框线。
    pub fn line_range(&self, point: ScreenPoint) -> (ScreenPoint, ScreenPoint) {
        if point.row >= self.height || self.width == 0 {
            return (point, point);
        }
        let left = (0..point.col.min(self.width))
            .rev()
            .find(|col| self.is_vertical_boundary(*col, point.row));
        let right = (point.col.saturating_add(1).min(self.width)..self.width)
            .find(|col| self.is_vertical_boundary(*col, point.row));
        let start = left.map_or(0, |col| col.saturating_add(1));
        let end = right.map_or(self.width.saturating_sub(1), |col| col.saturating_sub(1));
        if start > end || end >= self.width {
            return (point, point);
        }

        // 只有“首/尾非空白格是横边框”的行才按边框标题处理；普通内容行保
        // 留行首缩进并裁掉行尾填充空白。
        let glyphs: Vec<(u16, char)> = (start..=end)
            .filter_map(|col| self.char_at(col, point.row).map(|ch| (col, ch)))
            .filter(|(_, ch)| !ch.is_whitespace())
            .collect();
        let border_row = glyphs.iter().any(|(_, ch)| is_horizontal_border(*ch))
            && (glyphs
                .first()
                .is_some_and(|(_, ch)| is_horizontal_border(*ch))
                || glyphs
                    .last()
                    .is_some_and(|(_, ch)| is_horizontal_border(*ch)));
        if !border_row {
            // 普通内容行保留行首缩进，但裁掉行尾填充空白：后台任务输出/弹窗
            // 内部没有右侧竖边框，若一直选到容器右缘会把整行填充一起复制。
            let mut text_end = end;
            while text_end >= start
                && self
                    .char_at(text_end, point.row)
                    .is_some_and(char::is_whitespace)
            {
                if text_end == start {
                    return (point, point);
                }
                text_end -= 1;
            }
            return (
                ScreenPoint::new(point.row, start),
                ScreenPoint::new(point.row, text_end),
            );
        }

        // 边框行只取点击处的连续文字段；点到边框线本身不形成选区。
        if self.char_at(point.col, point.row).is_some_and(is_box_glyph) {
            return (point, point);
        }
        let mut text_start = point.col.clamp(start, end);
        while text_start > start {
            let previous = text_start - 1;
            match self.char_at(previous, point.row) {
                Some(ch) if !is_box_glyph(ch) => text_start = previous,
                Some(_) => break,
                None => text_start = previous,
            }
        }
        let mut text_end = point.col.clamp(start, end);
        while text_end < end {
            let next = text_end + 1;
            match self.char_at(next, point.row) {
                Some(ch) if !is_box_glyph(ch) => text_end = next,
                Some(_) => break,
                None => text_end = next,
            }
        }
        while text_start <= text_end
            && self
                .char_at(text_start, point.row)
                .is_some_and(char::is_whitespace)
        {
            text_start = text_start.saturating_add(1);
        }
        while text_end >= text_start
            && self
                .char_at(text_end, point.row)
                .is_some_and(char::is_whitespace)
        {
            text_end = text_end.saturating_sub(1);
        }
        if text_start > text_end {
            (point, point)
        } else {
            (
                ScreenPoint::new(point.row, text_start),
                ScreenPoint::new(point.row, text_end),
            )
        }
    }

    /// 读取单字符格；宽字符续格与多字符 grapheme 不算边框。
    fn char_at(&self, x: u16, y: u16) -> Option<char> {
        let cell = self.cell(x, y)?;
        if cell.width == 0 {
            return None;
        }
        let mut chars = cell.symbol.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        }
    }

    /// 该格是否具备左右分界意义（竖线、角、┬/┴/┼、滚动条 thumb `▐` 等；横线不算）。
    fn is_vertical_boundary(&self, x: u16, y: u16) -> bool {
        self.char_at(x, y).is_some_and(is_vertical_boundary)
    }

    /// 按单位扩展一次点击。
    pub fn unit_range(
        &self,
        point: ScreenPoint,
        unit: SelectionUnit,
    ) -> (ScreenPoint, ScreenPoint) {
        match unit {
            SelectionUnit::Character => (point, point),
            SelectionUnit::Word => self.word_range(point),
            SelectionUnit::Line => self.line_range(point),
        }
    }

    /// 拖动时把初始单位扩展到新的拖动点，反向拖动也不切断起始词/行。
    pub fn drag_range(
        &self,
        base: ScreenPoint,
        cursor: ScreenPoint,
        unit: SelectionUnit,
    ) -> (ScreenPoint, ScreenPoint) {
        let (base_start, base_end) = self.unit_range(base, unit);
        let (cursor_start, cursor_end) = self.unit_range(cursor, unit);
        if cursor < base {
            (cursor_start, base_end)
        } else {
            (base_start, cursor_end)
        }
    }

    /// 在当前画面中按内容重新定位冻结选区；找不到返回 None。
    ///
    /// 只匹配完整视觉行文本，避免把同样文字误认成新内容；重复行取离旧位置最近的一处。
    pub fn reanchor(&self, cells: &SelectedCells) -> Option<(ScreenPoint, ScreenPoint)> {
        if cells.rows.is_empty() {
            return None;
        }
        let old = cells.start?;
        let height = usize::from(self.height);
        let needle = cells.rows.len();
        if needle > height {
            return None;
        }
        let mut best: Option<(u16, (ScreenPoint, ScreenPoint))> = None;
        for start_row in 0..=self.height.saturating_sub(cells.rows.len() as u16) {
            let mut matched = true;
            for (offset, row) in cells.rows.iter().enumerate() {
                let y = start_row + offset as u16;
                if self.row_text(y).as_deref() != Some(row.full.as_str()) {
                    matched = false;
                    break;
                }
            }
            if !matched {
                continue;
            }
            let first = &cells.rows[0];
            let last = cells.rows.last()?;
            let distance = start_row.abs_diff(old.row);
            let candidate = (
                ScreenPoint::new(start_row, first.start_col),
                ScreenPoint::new(start_row + needle as u16 - 1, last.end_col),
            );
            if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                best = Some((distance, candidate));
            }
        }
        best.map(|(_, range)| range)
    }

    /// 把落在宽字符续格上的端点向前扩到宽字符首格，并保证终点覆盖首格。
    fn widen_columns(&self, row: u16, mut start: u16, mut end: u16) -> (u16, u16) {
        while start > 0 && self.cell(start, row).is_some_and(|cell| cell.width == 0) {
            start -= 1;
        }
        if end + 1 < self.width && self.cell(end, row).is_some_and(|cell| cell.width > 1) {
            end += 1;
        }
        (start, end.min(self.width.saturating_sub(1)))
    }
}

/// 具备左右分界意义的边框字形；横线不参与左右扫描，避免沿顶边框一路吞到屏幕边。
fn is_vertical_boundary(ch: char) -> bool {
    matches!(
        ch,
        '│' | '┃'
            | '║'
            | '╎'
            | '╏'
            | '╭'
            | '╮'
            | '╰'
            | '╯'
            | '┌'
            | '┐'
            | '└'
            | '┘'
            | '├'
            | '┤'
            | '┬'
            | '┴'
            | '┼'
            | '▐'
    )
}

/// 横边框线：用于识别边框行并从复制片段中剔除。
fn is_horizontal_border(ch: char) -> bool {
    matches!(ch, '─' | '━' | '═' | '┄' | '┈' | '╌' | '╍')
}

/// 任意制表/边框字形；边框行取标题文字时以它作为分隔。
fn is_box_glyph(ch: char) -> bool {
    is_vertical_boundary(ch) || is_horizontal_border(ch)
}

fn normalize(start: ScreenPoint, end: ScreenPoint) -> Option<(ScreenPoint, ScreenPoint)> {
    if start == end {
        return None;
    }
    Some(if start <= end {
        (start, end)
    } else {
        (end, start)
    })
}

#[cfg(test)]
mod tests;
