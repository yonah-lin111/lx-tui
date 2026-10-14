//! lx 页状态与多行文本输入编辑模型。

use crate::app::markdown::{BlockPanel, block_commands, block_insertion, block_trigger};
use crate::app::paste::{PasteAction, PasteCandidate, PastePanel};
use crate::app::prompt::{list, text};

/// lx 页三个入口：原生 agent、混合 agent、自动化 agent。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LxEntry {
    Native,
    Hybrid,
    Automation,
}

impl LxEntry {
    /// 三个入口的显示顺序。
    pub const ALL: [LxEntry; 3] = [LxEntry::Native, LxEntry::Hybrid, LxEntry::Automation];
}

/// lx 页交互状态：选中入口、多行输入缓冲与 markdown 块命令面板；每个 lx 窗格各持一份。
#[derive(Debug)]
pub struct Lx {
    /// 当前选中入口；仅鼠标点击切换，重启复位为 Native。
    pub entry: LxEntry,
    /// 多行输入内容。
    pub input: String,
    /// 光标字节偏移；恒位于字符边界。
    pub cursor: usize,
    /// 输入框内容宽度（列）；按几何同步，用于软换行与视觉行移动。未同步前视为不换行。
    width: u16,
    /// 输入框可视高度（内容行）；按几何同步，用于滚动钳制。
    height: usize,
    /// 视口首视觉行索引；滚轮与光标跟随共同维护。
    scroll: usize,
    /// markdown 块命令面板；光标处触发时打开。
    pub panel: Option<BlockPanel>,
    /// 路径粘贴参照面板；与 `panel` 互斥，同一时刻至多一个打开。
    paste: Option<LxPaste>,
    /// 撤销历史（上限 `LX_HISTORY_LIMIT`）。
    undo: Vec<LxSnapshot>,
    /// 重做历史。
    redo: Vec<LxSnapshot>,
}

/// 软换行后的一个视觉行：输入缓冲中的字节区间 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LxRow {
    pub start: usize,
    pub end: usize,
}

/// 在途路径粘贴：参照面板与占位插入现场。
#[derive(Debug)]
struct LxPaste {
    panel: PastePanel,
    /// 占位插入前的重做栈；`snapshot` 会清空它，取消必须原样还回。
    redo: Vec<LxSnapshot>,
}

/// lx 输入缓冲的一份快照（文本 + 光标），供撤销/重做。
#[derive(Debug, Clone)]
struct LxSnapshot {
    text: String,
    cursor: usize,
}

/// 撤销历史上限。
const LX_HISTORY_LIMIT: usize = 200;

impl Default for Lx {
    fn default() -> Self {
        Self::new()
    }
}

impl Lx {
    pub fn new() -> Self {
        Self {
            entry: LxEntry::Native,
            input: String::new(),
            cursor: 0,
            width: u16::MAX,
            height: usize::MAX,
            scroll: 0,
            panel: None,
            paste: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// 同步输入框内容宽度（列）与可视高度（内容行）；用于软换行、视觉行移动与滚动。
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width.max(1);
        self.height = usize::from(height.max(1));
        self.scroll_cursor_into_view();
    }

    /// 输入框视口首视觉行索引。
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 滚轮滚动输入框视口（正数向下）；返回视口是否移动。
    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let max = self.max_scroll();
        let next = (self.scroll.min(max) as isize)
            .saturating_add(delta)
            .clamp(0, max as isize) as usize;
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    /// 真实内容的最大滚动偏移。
    fn max_scroll(&self) -> usize {
        self.visual_rows().len().saturating_sub(self.height.max(1))
    }

    /// 调整滚动量，保证光标所在视觉行可见。
    fn scroll_cursor_into_view(&mut self) {
        let (row, _) = self.cursor_visual();
        let height = self.height.max(1);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll.saturating_add(height) {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// 按内容宽度软换行得到的视觉行。
    pub fn visual_rows(&self) -> Vec<LxRow> {
        let width = usize::from(self.width.max(1));
        let mut rows = Vec::new();
        let mut line_start = 0;
        for segment in self.input.split('\n') {
            let line_end = line_start + segment.len();
            let mut row_start = line_start;
            let mut col = 0;
            for (offset, ch) in segment.char_indices() {
                let absolute = line_start + offset;
                let cell = text::char_width(ch);
                if col > 0 && col + cell > width {
                    rows.push(LxRow {
                        start: row_start,
                        end: absolute,
                    });
                    row_start = absolute;
                    col = 0;
                }
                col += cell;
            }
            rows.push(LxRow {
                start: row_start,
                end: line_end,
            });
            line_start = line_end + 1;
        }
        rows
    }

    /// 光标在视觉行中的索引与显示列（跨折行边界归入下一视觉行）。
    pub fn cursor_visual(&self) -> (usize, usize) {
        let rows = self.visual_rows();
        self.cursor_visual_in(&rows)
    }

    fn cursor_visual_in(&self, rows: &[LxRow]) -> (usize, usize) {
        for (index, row) in rows.iter().enumerate() {
            if self.cursor < row.end {
                return (
                    index,
                    text::display_width(&self.input[row.start..self.cursor]),
                );
            }
            if self.cursor == row.end {
                let continues = rows
                    .get(index + 1)
                    .is_some_and(|next| next.start == row.end);
                if !continues {
                    return (index, text::display_width(&self.input[row.start..row.end]));
                }
            }
        }
        let index = rows.len().saturating_sub(1);
        let row = rows[index];
        (index, text::display_width(&self.input[row.start..row.end]))
    }

    /// 记录一次编辑前的快照，并清空重做栈。
    fn snapshot(&mut self) {
        if self.undo.len() == LX_HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(LxSnapshot {
            text: self.input.clone(),
            cursor: self.cursor,
        });
        self.redo.clear();
    }

    /// 在光标处插入一个字符。
    pub fn insert_char(&mut self, ch: char) {
        self.snapshot();
        self.input.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.refresh_panel();
    }

    /// 在光标处插入一段文本；净化规则同 prompt 编辑器（换行归一、Tab 展开、丢弃控制字符）。
    ///
    /// 整段粘贴作为一步撤销。粘贴不是“正在输入”：不刷新 md 块面板，避免粘贴文本
    /// 尾部的触发标记紧接着弹出面板、被下一次 Enter 误采纳（对齐 prompt 的粘贴语义）。
    pub fn insert_str(&mut self, text: &str) {
        let sanitized = text::sanitize(text);
        if sanitized.is_empty() {
            return;
        }
        self.snapshot();
        self.input.insert_str(self.cursor, &sanitized);
        self.cursor += sanitized.len();
        self.panel = None;
        self.scroll_cursor_into_view();
    }

    /// 在光标处换行；列表项自动续写标记（对齐 prompt 编辑器）。
    pub fn newline(&mut self) {
        if !list::in_fence(&self.input, self.cursor)
            && let Some(edit) = list::list_continuation(&self.input, self.cursor)
        {
            self.snapshot();
            self.input.replace_range(edit.range, &edit.replacement);
            self.cursor = edit.cursor;
            self.refresh_panel();
            return;
        }
        self.snapshot();
        self.input.insert(self.cursor, '\n');
        self.cursor += 1;
        self.refresh_panel();
    }

    /// 删除光标前一个字符。
    pub fn backspace(&mut self) {
        if let Some(prev) = self.input[..self.cursor].chars().next_back() {
            self.snapshot();
            self.cursor -= prev.len_utf8();
            self.input.remove(self.cursor);
            self.refresh_panel();
        }
    }

    /// 删除光标处字符。
    pub fn delete(&mut self) {
        if self.cursor < self.input.len() {
            self.snapshot();
            self.input.remove(self.cursor);
            self.refresh_panel();
        }
    }

    /// 光标左移一个字符。
    pub fn move_left(&mut self) {
        if let Some(prev) = self.input[..self.cursor].chars().next_back() {
            self.cursor -= prev.len_utf8();
            self.refresh_panel();
        }
    }

    /// 光标右移一个字符。
    pub fn move_right(&mut self) {
        if let Some(next) = self.input[self.cursor..].chars().next() {
            self.cursor += next.len_utf8();
            self.refresh_panel();
        }
    }

    /// 光标移到当前视觉行行首。
    pub fn home(&mut self) {
        let rows = self.visual_rows();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = rows[index].start;
        self.refresh_panel();
    }

    /// 光标移到当前视觉行行尾。
    pub fn end(&mut self) {
        let rows = self.visual_rows();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = rows[index].end;
        self.refresh_panel();
    }

    /// 光标上移一个视觉行（保持显示列，越界钳到行尾）。
    pub fn move_up(&mut self) {
        let rows = self.visual_rows();
        let (index, col) = self.cursor_visual_in(&rows);
        if index == 0 {
            self.cursor = rows[0].start;
        } else {
            let target = rows[index - 1];
            self.cursor =
                target.start + text::offset_at_col(&self.input[target.start..target.end], col);
        }
        self.refresh_panel();
    }

    /// 光标下移一个视觉行（保持显示列，越界钳到行尾）。
    pub fn move_down(&mut self) {
        let rows = self.visual_rows();
        let (index, col) = self.cursor_visual_in(&rows);
        if index + 1 >= rows.len() {
            self.cursor = rows[rows.len() - 1].end;
        } else {
            let target = rows[index + 1];
            self.cursor =
                target.start + text::offset_at_col(&self.input[target.start..target.end], col);
        }
        self.refresh_panel();
    }

    /// 光标左移一个词。
    pub fn word_left(&mut self) {
        self.cursor = text::word_start_before(&self.input, self.cursor);
        self.refresh_panel();
    }

    /// 光标右移一个词。
    pub fn word_right(&mut self) {
        self.cursor = text::word_end_after(&self.input, self.cursor);
        self.refresh_panel();
    }

    /// 删除光标到逻辑行首；光标已在行首时删除前一个换行（与上一行合并）。
    pub fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = text::line_start(&self.input, self.cursor);
        self.snapshot();
        if start < self.cursor {
            self.input.replace_range(start..self.cursor, "");
            self.cursor = start;
        } else {
            let previous = text::prev_boundary(&self.input, self.cursor);
            self.input.replace_range(previous..self.cursor, "");
            self.cursor = previous;
        }
        self.refresh_panel();
    }

    /// 删除光标到逻辑行尾。
    pub fn delete_to_line_end(&mut self) {
        let end = text::line_end(&self.input, self.cursor);
        if self.cursor < end {
            self.snapshot();
            self.input.replace_range(self.cursor..end, "");
            self.refresh_panel();
        }
    }

    /// 向前删除一个词。
    pub fn delete_word_backward(&mut self) {
        let start = text::word_start_before(&self.input, self.cursor);
        if start < self.cursor {
            self.snapshot();
            self.input.replace_range(start..self.cursor, "");
            self.cursor = start;
        }
    }

    /// 向后删除一个词。
    pub fn delete_word_forward(&mut self) {
        let end = text::word_end_after(&self.input, self.cursor);
        if self.cursor < end {
            self.snapshot();
            self.input.replace_range(self.cursor..end, "");
            self.cursor = end;
        }
    }

    /// 清空输入框内容。
    pub fn clear(&mut self) {
        if self.input.is_empty() {
            return;
        }
        self.snapshot();
        self.input.clear();
        self.cursor = 0;
        self.refresh_panel();
    }

    /// 撤销一步。
    pub fn undo(&mut self) {
        if let Some(snapshot) = self.undo.pop() {
            let current = LxSnapshot {
                text: std::mem::replace(&mut self.input, snapshot.text),
                cursor: self.cursor,
            };
            self.redo.push(current);
            self.cursor = snapshot.cursor;
            self.refresh_panel();
        }
    }

    /// 重做一步。
    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo.pop() {
            let current = LxSnapshot {
                text: std::mem::replace(&mut self.input, snapshot.text),
                cursor: self.cursor,
            };
            self.undo.push(current);
            self.cursor = snapshot.cursor;
            self.refresh_panel();
        }
    }

    /// 重算 markdown 块命令面板：光标处命中触发标记时打开，否则关闭。
    pub fn refresh_panel(&mut self) {
        self.panel = block_trigger(&self.input, self.cursor).map(|trigger| {
            let items = block_commands(trigger.kind);
            BlockPanel::new(trigger, items, 0)
        });
        self.scroll_cursor_into_view();
    }

    /// 面板高亮项循环移动。
    pub fn panel_move(&mut self, delta: isize) {
        if let Some(panel) = self.panel.as_mut() {
            panel.move_active(delta);
        }
    }

    /// 采纳面板高亮项：替换触发区间并落下光标；无面板返回 false。
    pub fn panel_accept(&mut self) -> bool {
        let Some(panel) = self.panel.take() else {
            return false;
        };
        let Some(id) = panel.items().get(panel.active()).copied() else {
            return false;
        };
        let trigger = panel.trigger();
        let insertion = block_insertion(id);
        self.input
            .replace_range(trigger.from..trigger.to, &insertion.text);
        self.cursor = trigger.from + insertion.cursor;
        self.refresh_panel();
        true
    }

    /// 关闭面板。
    pub fn panel_close(&mut self) {
        self.panel = None;
    }

    /// 当前路径粘贴参照面板。
    pub fn paste_panel(&self) -> Option<&PastePanel> {
        self.paste.as_ref().map(|paste| &paste.panel)
    }

    /// 打开路径粘贴参照面板：在光标处插入纯路径占位文本并把高亮落在首个可用项。
    ///
    /// 占位插入与后续引用改写同属一步撤销；打开时关闭 md 块面板（互斥）。
    /// 候选为空返回 false 且不改动缓冲区。
    pub fn paste_open(&mut self, candidates: Vec<PasteCandidate>) -> bool {
        if candidates.is_empty() {
            return false;
        }
        let plain: Vec<String> = candidates
            .iter()
            .map(|candidate| candidate.path.to_string_lossy().into_owned())
            .collect();
        let reference = candidates
            .iter()
            .map(PasteCandidate::reference)
            .collect::<Option<Vec<_>>>()
            .map(|references| self.paste_insertion(&references))
            .filter(|reference| !reference.is_empty());
        let insertion = self.paste_insertion(&plain);
        let redo = std::mem::take(&mut self.redo);
        self.snapshot();
        let start = self.cursor;
        self.input.insert_str(start, &insertion);
        self.cursor = start + insertion.len();
        self.panel = None;
        self.paste = Some(LxPaste {
            panel: PastePanel::new(0, candidates, start..start + insertion.len(), reference),
            redo,
        });
        self.scroll_cursor_into_view();
        true
    }

    /// 键盘移动粘贴面板高亮；返回是否消费。
    pub fn paste_move(&mut self, delta: isize) -> bool {
        self.paste
            .as_mut()
            .is_some_and(|paste| paste.panel.move_active(delta))
    }

    /// 确认粘贴面板：引用改写占位文本，路径保持原样；两者都结束面板。
    pub fn paste_confirm(&mut self, action: PasteAction) -> bool {
        let Some(paste) = self.paste.as_ref() else {
            return false;
        };
        if !paste.panel.enabled(action) {
            return false;
        }
        if action == PasteAction::Reference {
            let Some(reference) = paste.panel.reference.clone() else {
                return false;
            };
            let range = paste.panel.inserted.clone();
            self.input.replace_range(range.clone(), &reference);
            self.cursor = range.start + reference.len();
        }
        self.paste = None;
        self.panel = None;
        self.scroll_cursor_into_view();
        true
    }

    /// 取消粘贴面板：回滚占位插入，不残留撤销步；返回是否消费。
    pub fn paste_cancel(&mut self) -> bool {
        let Some(paste) = self.paste.take() else {
            return false;
        };
        if let Some(snapshot) = self.undo.pop() {
            self.input = snapshot.text;
            self.cursor = snapshot.cursor;
        }
        self.redo = paste.redo;
        self.panel = None;
        self.scroll_cursor_into_view();
        true
    }

    /// 构造粘贴插入文本：左邻非空白先补空格，末尾补空格。
    fn paste_insertion(&self, parts: &[String]) -> String {
        let mut insert = parts.join(" ");
        let left_blank = self.input[..self.cursor]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if !left_blank {
            insert.insert(0, ' ');
        }
        insert.push(' ');
        insert
    }
}

#[cfg(test)]
mod tests;
