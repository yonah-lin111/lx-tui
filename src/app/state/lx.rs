//! lx 页状态与多行文本输入编辑模型。

use crate::app::markdown::{BlockPanel, block_commands, block_insertion, block_trigger};
use crate::app::paste::{PasteAction, PasteCandidate, PastePanel};
use crate::app::prompt::{list, text};

pub mod chat;
mod history;
pub mod mention;
pub mod slash;
pub mod task;

use history::HistoryBrowse;
pub(crate) use history::HistoryMove;

pub use chat::{
    AssistantStepView, ChatMessageView, ChatPartView, CommandView, DiffHunkView, DiffLineView,
    FileChangeView, FileDiffDetailView, FileDiffView, FrozenModel, InputHint, InputHintLevel,
    InteractionAction, LxChat, ModelsSnapshot, PlanSnapshotView, PlanStepView,
    ProcessInteractionView, ProcessStateView, RequestKey, RequestState, ToolCallView,
    ToolDetailView, ToolStatus, ToolSummary,
};
pub use mention::{LxMention, LxMentionView};
pub use slash::{
    HistoryEntry, HistoryPanel, ModelCandidate, ModelPicker, ModelPickerStage, PanelWindow,
    SlashCommand, SlashPanel,
};
pub use task::{ProcessEntry, TaskPage, TaskTab};

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
    /// 输入框文本选择锚点（字节偏移）；拖拽选择期间为 Some。
    text_anchor: Option<usize>,
    /// markdown 块命令面板；光标处触发时打开。
    pub panel: Option<BlockPanel>,
    /// 路径粘贴参照面板；与 `panel` 互斥，同一时刻至多一个打开。
    paste: Option<LxPaste>,
    /// `@` 文件提及面板：扫描根、缓存与候选；与块面板/粘贴面板互斥。
    mention: LxMention,
    /// `/` 命令面板（model/resume/new）；与块面板/粘贴/提及面板互斥。
    pub slash: Option<super::lx::SlashPanel>,
    /// 聊天会话状态：会话引用、历史视图与请求状态机。
    pub chat: LxChat,
    /// 后台任务页面；`Some` 表示当前窗格正停在该页（整页替换 lx 页的绘制，
    /// 但 `Lx` 其余字段——会话、消息、草稿、滚动——全部原样保留，退出页面即复原）。
    pub task: Option<TaskPage>,
    /// 本会话正在运行的后台任务数（底部状态行指标）。
    ///
    /// 由事件循环按运行实例刷新，与页面是否打开无关——用户不打开后台任务页也要
    /// 能看见"还有几个服务在跑"，否则忘了清理就是这样发生的。
    pub background_tasks: usize,
    /// 撤销历史（上限 `LX_HISTORY_LIMIT`）。
    undo: Vec<LxSnapshot>,
    /// 重做历史。
    redo: Vec<LxSnapshot>,
    /// 已发送 prompt 的历史浏览态（游标 + 进入浏览前的草稿）。
    history: HistoryBrowse,
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
            text_anchor: None,
            panel: None,
            paste: None,
            mention: LxMention::default(),
            slash: None,
            chat: LxChat::new(),
            task: None,
            background_tasks: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            history: HistoryBrowse::default(),
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

    /// 直接设置视口滚动偏移（滚动条点击/拖拽）；越界钳到可滚动范围。
    pub fn scroll_to(&mut self, offset: usize) {
        self.scroll = offset.min(self.max_scroll());
    }

    /// 在指定视觉行/内容列开始输入框文本选择：光标落到该单元格并登记锚点。
    pub fn begin_text_selection(&mut self, row: usize, col: u16) {
        let offset = self.offset_at_cell(row, col);
        self.cursor = offset;
        self.text_anchor = Some(offset);
        self.panel = None;
        self.mention.suppress();
        self.scroll_cursor_into_view();
    }

    /// 拖拽扩展输入框选区：光标跟到单元格、锚点不动。
    pub fn drag_text_selection(&mut self, row: usize, col: u16) {
        if self.text_anchor.is_none() {
            return;
        }
        self.cursor = self.offset_at_cell(row, col);
        self.scroll_cursor_into_view();
    }

    /// 清除输入框文本选区。
    pub fn clear_text_selection(&mut self) {
        self.text_anchor = None;
    }

    /// 是否存在输入框文本选区锚点。
    pub fn has_text_selection(&self) -> bool {
        self.text_anchor.is_some()
    }

    /// 输入框选区字节范围 `[start, end)`；空选区返回 None。
    pub fn text_selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.text_anchor?;
        let (start, end) = if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        (start < end).then_some((start, end))
    }

    /// 取输入框选中文本；空选区返回 None。
    pub fn selected_text(&self) -> Option<String> {
        self.text_selection_range()
            .map(|(start, end)| self.input[start..end].to_string())
    }

    /// 视口单元格（视觉行、内容列）→ 输入缓冲字节偏移；越界钳到最近位置。
    pub fn offset_at_cell(&self, row: usize, col: u16) -> usize {
        let rows = self.visual_rows();
        let index = row.min(rows.len().saturating_sub(1));
        let visual = rows[index];
        let mut cell = 0;
        for (offset, ch) in self.input[visual.start..visual.end].char_indices() {
            let width = text::char_width(ch);
            if usize::from(col) < cell + width {
                return visual.start + offset;
            }
            cell += width;
        }
        visual.end
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
        self.text_anchor = None;
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
        self.mention.suppress();
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
    ///
    /// 光标紧跟 `@路径 ` 提及之后的空白、且提及面板未打开时，一次删净整块提及
    /// （对齐 prompt 的 `mention_deletion_range` 语义）；面板打开时按普通退格编辑查询。
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if !self.mention.is_open()
            && let Some(range) =
                crate::app::markdown::mention_deletion_range(&self.input, self.cursor)
        {
            self.snapshot();
            self.cursor = range.start;
            self.input.replace_range(range, "");
            self.refresh_panel();
            return;
        }
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

    /// 是否正在浏览已发送 prompt 的历史。
    pub(crate) fn history_browsing(&self) -> bool {
        self.history.browsing()
    }

    /// 结束历史浏览（不改输入框文本）。
    ///
    /// 任何编辑操作、`Esc` 和发送成功回执都会调用它；浏览态不保留任何隐藏文本。
    pub(crate) fn end_history_browse(&mut self) {
        self.history.end();
    }

    /// 只读判定：这次方向键是否由历史接管，以及要显示哪一条。
    ///
    /// `entries` 是全局历史（旧 → 新，末尾最新）。接管条件见 [`history::decide`]：
    /// `↑` 只在光标位于全文最开头、`↓` 只在光标位于全文末尾时生效，否则返回 None
    /// 交给调用方回落到光标移动。
    pub(crate) fn pending_history_move(
        &self,
        entries: &[String],
        direction: i8,
    ) -> Option<HistoryMove> {
        history::decide(&self.history, entries, direction, &self.input, self.cursor)
    }

    /// 应用一次历史切换：更新游标并按 [`Self::apply_history_text`] 写回文本。
    pub(crate) fn apply_history(&mut self, next: HistoryMove) {
        match next {
            HistoryMove::Entry {
                index,
                text,
                cursor_at_start,
            } => {
                // 首次向上进入浏览：先暂存当前草稿，供越过末尾时恢复。
                if !self.history.browsing() {
                    self.history.begin(self.input.clone());
                }
                self.history.set_index(index);
                self.apply_history_text(&text);
                // 光标落在文首/文末：连续同向按键才能继续切换（对齐 opencode）。
                if cursor_at_start {
                    self.cursor = 0;
                }
                self.scroll_cursor_into_view();
            }
            HistoryMove::Draft => {
                // 越过最新一条：恢复进入浏览前的草稿（光标置末尾）。
                let draft = self.history.take_draft();
                self.apply_history_text(&draft);
            }
        }
    }

    /// 把历史文本写回输入框：压一次撤销快照、光标置末尾、重算面板。
    ///
    /// 文本与当前内容完全相同时只移动光标，不产生无意义的撤销步。
    fn apply_history_text(&mut self, text: &str) {
        if self.input == text {
            self.cursor = self.input.len();
            self.text_anchor = None;
            self.scroll_cursor_into_view();
            return;
        }
        self.snapshot();
        self.input.clear();
        self.input.push_str(text);
        self.cursor = self.input.len();
        self.text_anchor = None;
        self.scroll_cursor_into_view();
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

    /// 重算块命令面板与 `@` 提及面板：二者互斥，提及命中时压制块面板。
    ///
    /// 编辑或移动光标后调用；同时解除提及面板的压制，按当前文本与根重新评估。
    pub fn refresh_panel(&mut self) {
        self.mention.unsuppress();
        self.mention.refresh(&self.input, self.cursor);
        if self.mention.is_open() {
            self.panel = None;
        } else {
            self.panel = block_trigger(&self.input, self.cursor).map(|trigger| {
                let items = block_commands(trigger.kind);
                BlockPanel::new(trigger, items, 0)
            });
        }
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

    /// `@` 提及面板状态。
    pub fn mention(&self) -> &LxMention {
        &self.mention
    }

    /// 同步 `@` 提及扫描根；根变化时作废旧候选并重算面板；返回根是否变化。
    pub fn set_mention_root(&mut self, root: Option<std::path::PathBuf>) -> bool {
        self.mention.set_root(root, &self.input, self.cursor)
    }

    /// 需要新扫描时返回（代号，扫描根）；仅事件层调用。
    pub fn take_mention_scan_request(&mut self) -> Option<(u64, std::path::PathBuf)> {
        self.mention.take_scan_request(&self.input, self.cursor)
    }

    /// 写入扫描结果；代号或根过期时丢弃；返回是否采纳。
    pub fn apply_mention_entries(
        &mut self,
        generation: u64,
        root: &std::path::Path,
        entries: Vec<crate::app::markdown::MentionEntry>,
    ) -> bool {
        let accepted =
            self.mention
                .apply_entries(generation, root, entries, &self.input, self.cursor);
        if accepted && self.mention.is_open() {
            self.panel = None;
        }
        accepted
    }

    /// 提及面板高亮循环移动；返回是否消费。
    pub fn mention_move(&mut self, delta: isize) -> bool {
        self.mention.move_active(delta)
    }

    /// 提及面板悬停高亮；返回是否变化。
    pub fn mention_set_active(&mut self, index: usize) -> bool {
        self.mention.set_active(index)
    }

    /// 滚轮滚动提及面板视口；返回是否移动。
    pub fn mention_scroll(&mut self, delta: isize, base: usize) -> bool {
        self.mention.scroll_viewport(delta, base)
    }

    /// 确认高亮提及条目：替换触发区间、落下光标并结束面板；返回是否消费。
    pub fn mention_confirm(&mut self) -> bool {
        let Some((range, insertion)) = self.mention.confirm(self.input.len()) else {
            return false;
        };
        self.snapshot();
        self.input.replace_range(range.clone(), &insertion);
        self.cursor = range.start + insertion.len();
        self.panel = None;
        self.scroll_cursor_into_view();
        true
    }

    /// 进入高亮目录：写回 `@目录/`（一步可撤销），保持面板并按新范围过滤；返回是否消费。
    pub fn mention_enter_folder(&mut self) -> bool {
        let Some((range, insertion)) = self.mention.enter_folder(self.input.len()) else {
            return false;
        };
        self.snapshot();
        self.input.replace_range(range.clone(), &insertion);
        self.cursor = range.start + insertion.len();
        self.mention.refresh(&self.input, self.cursor);
        self.panel = None;
        self.scroll_cursor_into_view();
        true
    }

    /// 关闭提及面板且保留 `@` 文本并压制重开；返回是否消费。
    pub fn mention_escape(&mut self) -> bool {
        self.mention.escape()
    }

    /// 提及面板是否可交互（有候选）。
    pub fn mention_open(&self) -> bool {
        self.mention.is_open()
    }

    /// 提及面板当前是否可见（含扫描中/无匹配状态行）；决定 Shift+Enter 是否吞键。
    pub fn mention_visible(&self) -> bool {
        self.mention.view(&self.input, self.cursor) != LxMentionView::Hidden
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
        self.mention.suppress();
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
