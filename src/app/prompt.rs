//! prompt 右栏编辑器：纯文本缓冲区、光标、软换行视口与选区取文，无 IO。

use unicode_width::UnicodeWidthChar;

use super::markdown::{self, BlockPanel};
use crate::layout::PaneId;

/// 粘贴文本中的制表符展开内容。
const TAB_STOP: &str = "  ";

/// 缩进单位宽度（空格数）；与 lx-agent 的 indentUnit 对齐。
const INDENT_UNIT: usize = 2;

/// 撤销/重做快照栈上限。
const HISTORY_LIMIT: usize = 200;

/// prompt 右栏：全局面板标识与可编辑 markdown 文本状态。
#[derive(Debug)]
pub struct Prompt {
    id: PaneId,
    text: String,
    /// 光标字节偏移；始终位于字符边界。
    cursor: usize,
    /// 首个可见视觉行索引。
    scroll: usize,
    /// 内容区尺寸（列、行），随几何同步。
    width: u16,
    height: u16,
    /// 撤销快照栈（编辑前状态）。
    undo: Vec<Snapshot>,
    /// 重做快照栈；新编辑清空。
    redo: Vec<Snapshot>,
    /// 最近一次编辑类别；连续同类编辑合并为一步。
    last_edit: Option<EditKind>,
    /// 块命令面板；由光标处触发标记逼近得到。
    panel: Option<BlockPanel>,
    /// 面板压制标记：Esc 关闭或确认插入后，等待下一次编辑/移动再重算。
    panel_suppressed: bool,
}

/// 一个视觉行：所属逻辑行与文本字节范围（不含行尾换行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// 历史快照：文本与光标位置。
#[derive(Debug)]
struct Snapshot {
    text: String,
    cursor: usize,
}

/// 编辑类别；Insert/Delete 连续执行时合并为一步，Other 独立成步。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
    Other,
}

/// 行首列表项标记类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Bullet(char),
    Ordered { number: u64, delimiter: char },
    Task(char),
}

/// 行首列表项：缩进、标记段结束偏移（含缩进与后随空白）与类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListItem {
    indent: usize,
    marker_end: usize,
    kind: ListKind,
}

impl ListItem {
    /// 下一行延续用的标记文本（不含缩进）。
    fn continuation(self) -> String {
        match self.kind {
            ListKind::Bullet(marker) => format!("{marker} "),
            ListKind::Ordered { number, delimiter } => format!("{}{delimiter} ", number + 1),
            ListKind::Task(marker) => format!("{marker} [ ] "),
        }
    }
}

impl Prompt {
    /// 创建空编辑器。
    pub fn new(id: PaneId) -> Self {
        Self {
            id,
            text: String::new(),
            cursor: 0,
            scroll: 0,
            width: 1,
            height: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            panel: None,
            panel_suppressed: false,
        }
    }

    /// 面板标识；文本选择按此标识归属。
    pub fn id(&self) -> PaneId {
        self.id
    }

    /// 当前文本。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 首个可见视觉行索引。
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 内容区尺寸（列、行）。
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// 同步内容区尺寸；光标保持在视口内。
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.width = cols.max(1);
        self.height = rows.max(1);
        self.scroll_cursor_into_view();
    }

    /// 插入一个字符；换行与制表符按编辑器语义处理，其余控制字符忽略。
    pub fn insert_char(&mut self, ch: char) {
        match ch {
            '\n' => self.insert_at("\n"),
            '\t' => self.insert_at(TAB_STOP),
            ch if ch.is_control() => {}
            ch => {
                let mut buffer = [0_u8; 4];
                self.insert_at(ch.encode_utf8(&mut buffer));
            }
        }
    }

    /// 插入一段文本；`\r\n`/`\r` 归一为 `\n`，制表符展开，其余控制字符丢弃。
    pub fn insert_str(&mut self, text: &str) {
        self.insert_at(&sanitize(text));
    }

    /// 在光标处换行；列表项自动续写标记，空项退出层级。
    pub fn newline(&mut self) {
        if !self.in_fence() && self.continue_list() {
            return;
        }
        self.insert_at("\n");
    }

    /// 在当前逻辑行尾另起一行并保留行首缩进；不续写列表标记（lx-agent 的 Ctrl/Cmd+Shift+Enter）。
    pub fn newline_below(&mut self) {
        self.break_group();
        let start = line_start(&self.text, self.cursor);
        let end = line_end(&self.text, self.cursor);
        let indent = self.text[start..end]
            .bytes()
            .take_while(|byte| *byte == b' ')
            .count();
        let insert = format!("\n{}", " ".repeat(indent));
        self.record(EditKind::Other);
        self.text.insert_str(end, &insert);
        self.cursor = end + insert.len();
        self.settle();
    }

    /// 删除光标前一个字符；紧跟列表标记时先把标记替换为等宽空格。
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if !self.in_fence() && self.delete_list_markup() {
            return;
        }
        let previous = prev_boundary(&self.text, self.cursor);
        self.record(EditKind::Delete);
        self.text.remove(previous);
        self.cursor = previous;
        self.settle();
    }

    /// 删除光标处字符；文末为 no-op。
    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        self.record(EditKind::Delete);
        self.text.remove(self.cursor);
        self.settle();
    }

    /// 撤销上一步编辑；无可撤销内容时 no-op。
    pub fn undo(&mut self) {
        let Some(snapshot) = self.undo.pop() else {
            return;
        };
        let current = self.snapshot();
        push_bounded(&mut self.redo, current);
        self.restore(snapshot);
    }

    /// 重做；无可重做内容时 no-op。
    pub fn redo(&mut self) {
        let Some(snapshot) = self.redo.pop() else {
            return;
        };
        let current = self.snapshot();
        push_bounded(&mut self.undo, current);
        self.restore(snapshot);
    }

    /// 当前块命令面板；未打开时为 None。
    pub fn panel(&self) -> Option<&BlockPanel> {
        self.panel.as_ref()
    }

    /// 面板打开时按偏移循环移动高亮；返回是否消费该按键。
    pub fn panel_move(&mut self, delta: isize) -> bool {
        match self.panel.as_mut() {
            Some(panel) => {
                panel.move_active(delta);
                true
            }
            None => false,
        }
    }

    /// 面板打开时确认高亮命令：替换触发区间并压制重弹；返回是否消费该按键。
    pub fn panel_confirm(&mut self) -> bool {
        let Some(panel) = self.panel.as_ref() else {
            return false;
        };
        let trigger = panel.trigger();
        if trigger.from > trigger.to || trigger.to > self.text.len() {
            return false;
        }
        let Some(id) = panel.items().get(panel.active()).copied() else {
            return false;
        };
        let insertion = markdown::block_insertion(id);
        self.panel = None;
        self.break_group();
        self.record(EditKind::Other);
        self.text
            .replace_range(trigger.from..trigger.to, &insertion.text);
        self.cursor = trigger.from + insertion.cursor;
        self.panel_suppressed = true;
        self.scroll_cursor_into_view();
        true
    }

    /// 面板打开时关闭且不改文本；返回是否消费该按键。
    pub fn panel_escape(&mut self) -> bool {
        if self.panel.is_none() {
            return false;
        }
        self.panel = None;
        self.panel_suppressed = true;
        true
    }

    /// 清空面板并解除压制；prompt 失焦或折叠时调用。
    pub fn clear_panel(&mut self) {
        self.panel = None;
        self.panel_suppressed = false;
    }

    /// 当前逻辑行整体右移一个缩进单位；围栏代码块内改为光标处插入空格。
    pub fn indent(&mut self) {
        if self.in_fence() {
            self.insert_at(TAB_STOP);
            return;
        }
        self.record(EditKind::Other);
        let start = line_start(&self.text, self.cursor);
        self.text.insert_str(start, TAB_STOP);
        self.cursor += INDENT_UNIT;
        self.settle();
    }

    /// 当前逻辑行整体左移一个缩进单位；行首无空格时 no-op。
    pub fn outdent(&mut self) {
        let start = line_start(&self.text, self.cursor);
        let spaces = self.text[start..]
            .bytes()
            .take_while(|byte| *byte == b' ')
            .count();
        let remove = spaces.min(INDENT_UNIT);
        if remove == 0 {
            return;
        }
        self.record(EditKind::Other);
        let offset = self.cursor - start;
        self.text.replace_range(start..start + remove, "");
        self.cursor = start + offset.saturating_sub(remove);
        self.settle();
    }

    /// 光标左移一个字符。
    pub fn move_left(&mut self) {
        self.break_group();
        self.cursor = prev_boundary(&self.text, self.cursor);
        self.settle();
    }

    /// 光标右移一个字符。
    pub fn move_right(&mut self) {
        self.break_group();
        self.cursor = next_boundary(&self.text, self.cursor);
        self.settle();
    }

    /// 光标上移一个视觉行，保持显示列；首行时钳到行首。
    pub fn move_up(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        let (index, col) = self.cursor_visual_in(&rows);
        if index == 0 {
            self.cursor = rows[0].start;
        } else {
            let target = rows[index - 1];
            self.cursor = target.start + offset_at_col(&self.text[target.start..target.end], col);
        }
        self.settle();
    }

    /// 光标下移一个视觉行，保持显示列；末行时钳到行尾。
    pub fn move_down(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        let (index, col) = self.cursor_visual_in(&rows);
        if index + 1 >= rows.len() {
            self.cursor = rows[rows.len() - 1].end;
        } else {
            let target = rows[index + 1];
            self.cursor = target.start + offset_at_col(&self.text[target.start..target.end], col);
        }
        self.settle();
    }

    /// 光标移到当前视觉行行首。
    pub fn move_home(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = rows[index].start;
        self.settle();
    }

    /// 光标移到当前视觉行行尾。
    pub fn move_end(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = rows[index].end;
        self.settle();
    }

    /// 光标移到逻辑行行首。
    pub fn move_line_start(&mut self) {
        self.break_group();
        self.cursor = line_start(&self.text, self.cursor);
        self.settle();
    }

    /// 光标移到逻辑行行尾（换行前）。
    pub fn move_line_end(&mut self) {
        self.break_group();
        self.cursor = line_end(&self.text, self.cursor);
        self.settle();
    }

    /// 光标左移到前一个词的词首。
    pub fn move_word_backward(&mut self) {
        self.break_group();
        self.cursor = self.word_start_before(self.cursor);
        self.settle();
    }

    /// 光标右移到下一个词的词尾（readline M-f 语义）。
    pub fn move_word_forward(&mut self) {
        self.break_group();
        self.cursor = self.word_end_after(self.cursor);
        self.settle();
    }

    /// 删除光标前一个词及其后分隔符（readline ctrl+w 语义）。
    pub fn delete_word_backward(&mut self) {
        let start = self.word_start_before(self.cursor);
        if start < self.cursor {
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
            self.settle();
        }
    }

    /// 删除光标到下一个词尾（readline M-d 语义）。
    pub fn delete_word_forward(&mut self) {
        let end = self.word_end_after(self.cursor);
        if end > self.cursor {
            self.text.replace_range(self.cursor..end, "");
            self.settle();
        }
    }

    /// 删除光标到逻辑行首。
    pub fn delete_to_line_start(&mut self) {
        let start = line_start(&self.text, self.cursor);
        if start < self.cursor {
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
            self.settle();
        }
    }

    /// 删除光标到逻辑行尾（不删除换行）。
    pub fn delete_to_line_end(&mut self) {
        let end = line_end(&self.text, self.cursor);
        if end > self.cursor {
            self.text.replace_range(self.cursor..end, "");
            self.settle();
        }
    }

    /// 按内容宽度软换行得到的视觉行。
    pub fn visual_rows(&self) -> Vec<VisualRow> {
        let width = usize::from(self.width.max(1));
        let mut rows = Vec::new();
        let mut line_start = 0;
        for (line, segment) in self.text.split('\n').enumerate() {
            let line_end = line_start + segment.len();
            let mut row_start = line_start;
            let mut col = 0;
            for (offset, ch) in segment.char_indices() {
                let absolute = line_start + offset;
                let cell = char_width(ch);
                if col > 0 && col + cell > width {
                    rows.push(VisualRow {
                        line,
                        start: row_start,
                        end: absolute,
                    });
                    row_start = absolute;
                    col = 0;
                }
                col += cell;
            }
            rows.push(VisualRow {
                line,
                start: row_start,
                end: line_end,
            });
            line_start = line_end + 1;
        }
        rows
    }

    /// 光标在视口内的坐标（行、列）；滚出视口时为 None。
    pub fn cursor_cell(&self) -> Option<(u16, u16)> {
        if self.height == 0 || self.width == 0 {
            return None;
        }
        let rows = self.visual_rows();
        let (row, col) = self.cursor_visual_in(&rows);
        let visible = row.checked_sub(self.scroll)?;
        if visible >= usize::from(self.height) {
            return None;
        }
        let col = col.min(usize::from(self.width) - 1);
        Some((visible as u16, col as u16))
    }

    /// 按视觉行滚动视口；光标不动，超界时钳到可滚动范围。
    pub fn scroll_by(&mut self, lines: isize) {
        let max = self
            .visual_rows()
            .len()
            .saturating_sub(usize::from(self.height.max(1)));
        let target = (self.scroll.min(max) as isize).saturating_add(lines);
        self.scroll = target.clamp(0, max as isize) as usize;
    }

    /// 将视口单元格坐标映射为光标位置；超出文本时钳到最近行行尾。
    pub fn set_cursor_from_cell(&mut self, row: u16, col: u16) {
        self.break_group();
        self.cursor = self.viewport_offset((row, col), false);
        self.settle();
    }

    /// 选区（视口坐标，端点包含）对应的字节范围；空选区或选中内容为空时返回 None。
    pub fn selection_bounds(&self, start: (u16, u16), end: (u16, u16)) -> Option<(usize, usize)> {
        if start == end {
            return None;
        }
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let from = self.viewport_offset(first, false);
        let to = self.viewport_offset(last, true);
        (from < to).then_some((from, to))
    }

    /// 取选区文本（视口坐标，端点包含）；空选区或选中内容为空时返回 None。
    pub fn selection_text(&self, start: (u16, u16), end: (u16, u16)) -> Option<String> {
        let (from, to) = self.selection_bounds(start, end)?;
        Some(self.text[from..to].to_string())
    }

    /// 用插入文本替换字节范围（选区删除/替换）；文本会先净化，作为独立撤销步。
    pub fn replace_range(&mut self, start: usize, end: usize, insert: &str) -> bool {
        if start >= end || end > self.text.len() {
            return false;
        }
        let sanitized = sanitize(insert);
        self.break_group();
        self.record(EditKind::Insert);
        self.text.replace_range(start..end, &sanitized);
        self.cursor = start + sanitized.len();
        self.settle();
        true
    }

    /// 在光标处插入已净化的文本并推进光标。
    fn insert_at(&mut self, insert: &str) {
        self.record(EditKind::Insert);
        self.text.insert_str(self.cursor, insert);
        self.cursor += insert.len();
        self.settle();
    }

    /// 记录一次编辑前的快照；连续 Insert/Delete 合并为一步，Other 独立成步。
    fn record(&mut self, kind: EditKind) {
        if kind != EditKind::Other && self.last_edit == Some(kind) {
            return;
        }
        let snapshot = self.snapshot();
        push_bounded(&mut self.undo, snapshot);
        self.redo.clear();
        self.last_edit = (kind != EditKind::Other).then_some(kind);
    }

    /// 当前状态快照。
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
        }
    }

    /// 恢复快照并结束当前合并组。
    fn restore(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.last_edit = None;
        self.settle();
    }

    /// 结束当前合并组；光标移动或重新定位后调用。
    fn break_group(&mut self) {
        self.last_edit = None;
    }

    /// 列表项续写换行；返回是否已按列表语义处理。
    fn continue_list(&mut self) -> bool {
        let start = line_start(&self.text, self.cursor);
        let end = line_end(&self.text, self.cursor);
        let Some(item) = parse_list_item(&self.text[start..end]) else {
            return false;
        };
        if self.cursor - start < item.marker_end {
            return false;
        }
        if self.text[start + item.marker_end..end].trim().is_empty() {
            let replacement = if item.indent == 0 {
                String::new()
            } else {
                self.outer_list_prefix(start, item.indent)
                    .unwrap_or_default()
            };
            self.record(EditKind::Other);
            self.text
                .replace_range(start..start + item.marker_end, &replacement);
            self.cursor = start + replacement.len();
            self.settle();
            return true;
        }
        let insert = format!("\n{}{}", " ".repeat(item.indent), item.continuation());
        self.record(EditKind::Other);
        self.text.insert_str(self.cursor, &insert);
        self.cursor += insert.len();
        self.settle();
        true
    }

    /// 沿上文找最近的低缩进列表项，返回其缩进+标记，用于空项退出层级。
    fn outer_list_prefix(&self, line_start_offset: usize, indent: usize) -> Option<String> {
        self.text[..line_start_offset]
            .split('\n')
            .rev()
            .find_map(|line| {
                let item = parse_list_item(line)?;
                (item.indent < indent)
                    .then(|| format!("{}{}", " ".repeat(item.indent), item.continuation()))
            })
    }

    /// Backspace 的列表标记删除：光标紧跟标记段时把标记替换为等宽空格。
    fn delete_list_markup(&mut self) -> bool {
        let start = line_start(&self.text, self.cursor);
        let end = line_end(&self.text, self.cursor);
        let line = &self.text[start..end];
        let Some(item) = parse_list_item(line) else {
            return false;
        };
        let offset = self.cursor - start;
        if offset < item.marker_end {
            return false;
        }
        let has_content = !line[item.marker_end..].trim().is_empty();
        if has_content && offset != item.marker_end {
            return false;
        }
        let blanks = " ".repeat(item.marker_end - item.indent);
        self.record(EditKind::Delete);
        self.text
            .replace_range(start + item.indent..start + item.marker_end, &blanks);
        self.settle();
        true
    }

    /// 光标所在行是否位于未闭合的围栏代码块内。
    fn in_fence(&self) -> bool {
        let start = line_start(&self.text, self.cursor);
        let mut fence = false;
        for line in self.text[..start].split('\n') {
            if is_fence(line) {
                fence = !fence;
            }
        }
        fence
    }

    /// 光标前一个词（含尾随分隔符）的起点。
    fn word_start_before(&self, index: usize) -> usize {
        let mut boundary = 0;
        let mut in_word = false;
        let mut seen = false;
        for (offset, ch) in self.text[..index].char_indices().rev() {
            seen = true;
            if is_word_char(ch) {
                in_word = true;
                boundary = offset;
            } else if in_word {
                break;
            } else {
                boundary = offset;
            }
        }
        if seen { boundary } else { 0 }
    }

    /// 光标所在或下一个词的词尾（跳过前导分隔符）。
    fn word_end_after(&self, index: usize) -> usize {
        let mut end = index;
        let mut in_word = false;
        for (offset, ch) in self.text[index..].char_indices() {
            if is_word_char(ch) {
                in_word = true;
                end = index + offset + ch.len_utf8();
            } else if in_word {
                break;
            }
        }
        end
    }

    /// 光标在视觉行中的索引与显示列（跨折行边界时归入下一视觉行）。
    fn cursor_visual_in(&self, rows: &[VisualRow]) -> (usize, usize) {
        for (index, row) in rows.iter().enumerate() {
            if self.cursor < row.end {
                return (index, display_width(&self.text[row.start..self.cursor]));
            }
            if self.cursor == row.end {
                let continues = rows
                    .get(index + 1)
                    .is_some_and(|next| next.start == row.end && next.line == row.line);
                if !continues {
                    return (index, display_width(&self.text[row.start..row.end]));
                }
            }
        }
        let index = rows.len().saturating_sub(1);
        let row = rows[index];
        (index, display_width(&self.text[row.start..row.end]))
    }

    /// 视口坐标映射到文本字节偏移；`after` 为真时取该单元格字符之后。
    fn viewport_offset(&self, (row, col): (u16, u16), after: bool) -> usize {
        let rows = self.visual_rows();
        let index = (self.scroll + usize::from(row)).min(rows.len() - 1);
        let visual = rows[index];
        let slice = &self.text[visual.start..visual.end];
        let mut cell = 0;
        for (offset, ch) in slice.char_indices() {
            let width = char_width(ch);
            if usize::from(col) < cell + width {
                return visual.start
                    + if after {
                        offset + ch.len_utf8()
                    } else {
                        offset
                    };
            }
            cell += width;
        }
        visual.end
    }

    /// 编辑或光标移动后的统一收尾：解除面板压制、保持光标可见并重算面板。
    fn settle(&mut self) {
        self.panel_suppressed = false;
        self.scroll_cursor_into_view();
        self.refresh_panel();
    }

    /// 重算块命令面板：触发标记决定候选列表，同类同位置保留高亮。
    fn refresh_panel(&mut self) {
        let Some(trigger) = markdown::block_trigger(&self.text, self.cursor) else {
            self.panel = None;
            return;
        };
        let previous = self.panel.as_ref().filter(|panel| {
            panel.trigger().kind == trigger.kind && panel.trigger().to == trigger.to
        });
        let active = previous.map_or(0, BlockPanel::active);
        let items = markdown::block_commands(trigger.kind);
        let active = active.min(items.len().saturating_sub(1));
        self.panel = Some(BlockPanel::new(trigger, items, active));
    }

    /// 调整滚动量，保证光标所在视觉行可见。
    fn scroll_cursor_into_view(&mut self) {
        let rows = self.visual_rows();
        let height = usize::from(self.height.max(1));
        let (row, _) = self.cursor_visual_in(&rows);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
    }
}

/// 词字符：Unicode 字母数字与下划线；CJK 视作词字符。
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// 所在逻辑行的起始字节偏移（上一个换行之后）。
fn line_start(text: &str, index: usize) -> usize {
    text[..index].rfind('\n').map_or(0, |offset| offset + 1)
}

/// 所在逻辑行的结束字节偏移（下一个换行之前或文末）。
fn line_end(text: &str, index: usize) -> usize {
    text[index..]
        .find('\n')
        .map_or(text.len(), |offset| index + offset)
}

/// 维护上限的快照入栈；超限时丢弃最旧一条。
fn push_bounded(stack: &mut Vec<Snapshot>, snapshot: Snapshot) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(snapshot);
}

/// 解析行首列表项（无序/有序/任务）；非列表返回 None。
fn parse_list_item(line: &str) -> Option<ListItem> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let first = rest.chars().next()?;
    let (kind, marker_len) = if matches!(first, '-' | '*' | '+') {
        (ListKind::Bullet(first), 1)
    } else if first.is_ascii_digit() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let delimiter = *rest.as_bytes().get(digits)?;
        if !matches!(delimiter, b'.' | b')') {
            return None;
        }
        (
            ListKind::Ordered {
                number: rest[..digits].parse().ok()?,
                delimiter: delimiter as char,
            },
            digits + 1,
        )
    } else {
        return None;
    };
    let spaces = rest[marker_len..]
        .bytes()
        .take_while(|byte| *byte == b' ')
        .count();
    if spaces == 0 {
        return None;
    }
    let mut marker_end = indent + marker_len + spaces;
    let mut kind = kind;
    if let ListKind::Bullet(marker) = kind {
        let after = &line.as_bytes()[marker_end..];
        if after.len() >= 4
            && after[0] == b'['
            && after[2] == b']'
            && matches!(after[1], b' ' | b'x' | b'X')
        {
            let task_spaces = after[3..].iter().take_while(|byte| **byte == b' ').count();
            if task_spaces > 0 {
                marker_end += 3 + task_spaces;
                kind = ListKind::Task(marker);
            }
        }
    }
    Some(ListItem {
        indent,
        marker_end,
        kind,
    })
}

/// 围栏代码块行：至多 3 个前导空格后以 ``` 开头。
fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start_matches(' ');
    line.len() - trimmed.len() <= 3 && trimmed.starts_with("```")
}

/// 字符显示宽度；零宽与不可打印字符按 0 处理。
fn char_width(ch: char) -> usize {
    ch.width().unwrap_or(0)
}

/// 文本显示宽度（列）。
fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 前一个字符的字节边界。
fn prev_boundary(text: &str, index: usize) -> usize {
    text[..index]
        .char_indices()
        .next_back()
        .map(|(offset, _)| offset)
        .unwrap_or(0)
}

/// 后一个字符的字节边界。
fn next_boundary(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map(|ch| index + ch.len_utf8())
        .unwrap_or(index)
}

/// 行内显示列对应的字节偏移；落在宽字符中间或超出时钳到对应边界。
fn offset_at_col(slice: &str, col: usize) -> usize {
    let mut cell = 0;
    for (offset, ch) in slice.char_indices() {
        let width = char_width(ch);
        if col < cell + width {
            return offset;
        }
        cell += width;
    }
    slice.len()
}

/// 粘贴文本净化：换行归一、制表符展开、其余控制字符丢弃。
fn sanitize(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                sanitized.push('\n');
            }
            '\n' => sanitized.push('\n'),
            '\t' => sanitized.push_str(TAB_STOP),
            ch if ch.is_control() => {}
            ch => sanitized.push(ch),
        }
    }
    sanitized
}

#[cfg(test)]
mod tests;
