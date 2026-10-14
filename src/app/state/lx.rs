//! lx 页状态与多行文本输入编辑模型。

use crate::app::markdown::{BlockPanel, block_commands, block_insertion, block_trigger};
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
    /// markdown 块命令面板；光标处触发时打开。
    pub panel: Option<BlockPanel>,
    /// 撤销历史（上限 `LX_HISTORY_LIMIT`）。
    undo: Vec<LxSnapshot>,
    /// 重做历史。
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
            panel: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
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

    /// 光标移到行首。
    pub fn home(&mut self) {
        self.cursor = self.line_start();
        self.refresh_panel();
    }

    /// 光标移到行尾。
    pub fn end(&mut self) {
        self.cursor = self.line_end();
        self.refresh_panel();
    }

    /// 光标上移一行（保持行内字节列，越界钳到行尾）。
    pub fn move_up(&mut self) {
        let start = self.line_start();
        if start == 0 {
            self.cursor = 0;
        } else {
            let column = self.cursor - start;
            let previous_end = start - 1;
            let previous_start = self.input[..previous_end]
                .rfind('\n')
                .map_or(0, |offset| offset + 1);
            self.cursor =
                self.clamp_boundary(previous_start + column.min(previous_end - previous_start));
        }
        self.refresh_panel();
    }

    /// 光标下移一行（保持行内字节列，越界钳到行尾）。
    pub fn move_down(&mut self) {
        let end = self.line_end();
        if end >= self.input.len() {
            self.cursor = self.input.len();
        } else {
            let start = self.line_start();
            let column = self.cursor - start;
            let next_start = end + 1;
            let next_end = self.input[next_start..]
                .find('\n')
                .map_or(self.input.len(), |offset| next_start + offset);
            self.cursor = self.clamp_boundary(next_start + column.min(next_end - next_start));
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

    /// 光标所在逻辑行起始字节偏移。
    fn line_start(&self) -> usize {
        self.input[..self.cursor]
            .rfind('\n')
            .map_or(0, |offset| offset + 1)
    }

    /// 光标所在逻辑行结束字节偏移。
    fn line_end(&self) -> usize {
        self.input[self.cursor..]
            .find('\n')
            .map_or(self.input.len(), |offset| self.cursor + offset)
    }

    /// 把偏移回退到最近的字符边界。
    fn clamp_boundary(&self, mut index: usize) -> usize {
        while index > 0 && !self.input.is_char_boundary(index) {
            index -= 1;
        }
        index
    }
}
