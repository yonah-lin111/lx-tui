//! 挂起的 `ask` 面板状态：题目、高亮、勾选、手写缓冲与滚动偏移。
//!
//! 边界：本模块只做**纯状态迁移**，不登记效应、不触发 IO、不读 `AppState`。作答提交
//! （推 `Effect::AnswerAsk`）与按键路由在 `app::update::chat::ask` 里完成。
//!
//! 草稿不变量：本结构持有完全独立的编辑缓冲 [`AskEditor`]，作答期间 `Lx.input`
//! 一个字节都不动。

use crate::chat::tools::ask::{AskQuestion, MAX_ASK_CUSTOM_BYTES};

use super::{Lx, LxRow, RequestKey};

/// 鼠标在 ask 面板上的命中（几何由渲染层 `layout()` 提供，事件层不重算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskHit {
    /// 当前题的某个选项行（`0..options.len()`）。
    Option(usize),
    /// 固定最后一行的「其他」。
    Other,
    /// `Submit` 行（等价 `Enter`）。
    Submit,
    /// 题目标签（切到该题，状态保留）。
    Tab(usize),
}

/// 滚轮所在分区：选项区移动高亮，预览区只滚预览。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskWheelZone {
    Options,
    Preview,
}

/// 当前挂起的一次提问。
#[derive(Debug)]
pub struct AskState {
    /// 归属键（作答 Effect 与归属校验用）。
    pub key: RequestKey,
    pub call_id: String,
    pub questions: Vec<AskQuestion>,
    pub current: usize,
    /// 多题流程的最后一步：复核 + 确认提交（`current` 此时仍停在末题）。
    pub confirm: bool,
    /// 每题高亮行：`0..options.len()`，最后一行固定是「其他」。
    pub highlight: Vec<usize>,
    /// 每题已勾选项（按下标保存，顺序即勾选顺序）。
    pub selected: Vec<Vec<usize>>,
    /// 每题手写答案（独立编辑缓冲，绝不复用 `Lx.input`）。
    pub custom: Vec<AskEditor>,
    /// 每题手写文本是否计入答案（提交编辑态即计入；清空即不计入）。
    pub custom_on: Vec<bool>,
    /// 当前题是否处于编辑态。
    pub editing: bool,
    /// 每题预览滚动偏移。
    pub preview_scroll: Vec<usize>,
    /// 预览区可视行数（由事件循环按几何同步；0 表示没有预览区）。
    ///
    /// 滚动上限必须扣掉可视行数，否则短预览也能滚出一片空白。
    pub preview_visible: usize,
}

impl AskState {
    /// 以题目列表构造面板；每题初始高亮第 0 行、无勾选、无手写。
    pub fn new(key: RequestKey, call_id: String, questions: Vec<AskQuestion>) -> Self {
        let count = questions.len();
        Self {
            key,
            call_id,
            questions,
            current: 0,
            confirm: false,
            highlight: vec![0; count],
            selected: vec![Vec::new(); count],
            custom: (0..count).map(|_| AskEditor::new()).collect(),
            custom_on: vec![false; count],
            editing: false,
            preview_scroll: vec![0; count],
            preview_visible: 0,
        }
    }

    /// 当前题。
    pub fn question(&self) -> Option<&AskQuestion> {
        self.questions.get(self.current)
    }

    /// 当前题的行数：选项 + 固定的「其他」行（0 选项题只剩这一行）。
    pub fn row_count(&self) -> usize {
        self.question()
            .map_or(0, |question| question.options.len() + 1)
    }

    /// 「其他」行的下标（恒为最后一行）。
    pub fn other_row(&self) -> usize {
        self.row_count().saturating_sub(1)
    }

    /// 当前题当前答案：勾选项按勾选顺序在前，手写文本（若有）在最后，已去重。
    pub fn current_answers(&self) -> Vec<String> {
        self.answers_at(self.current)
    }

    /// 指定题的当前答案（提交时逐题汇总，不依赖 `current`）。
    pub fn answers_at(&self, index: usize) -> Vec<String> {
        let Some(question) = self.questions.get(index) else {
            return Vec::new();
        };
        let mut answers: Vec<String> = self
            .selected
            .get(index)
            .map(|selected| {
                selected
                    .iter()
                    .filter_map(|option| question.options.get(*option))
                    .map(|option| option.label.clone())
                    .collect()
            })
            .unwrap_or_default();
        if self.custom_on.get(index).copied().unwrap_or(false) {
            let text = self
                .custom
                .get(index)
                .map(AskEditor::text)
                .unwrap_or_default();
            let text = text.trim();
            if !text.is_empty() && !answers.iter().any(|answer| answer == text) {
                answers.push(text.to_string());
            }
        }
        answers
    }

    /// 选/取消选中一行：选项行按单选/多选规则处理，最后一行进入编辑态。
    ///
    /// 单选：设为唯一勾选并清空该题手写答案（先勾后写、先写后勾都只留一个答案）。
    /// 多选：切换该选项，勾选与手写可共存。
    pub fn choose(&mut self, index: usize) -> bool {
        if self.confirm {
            return false;
        }
        let Some(question) = self.question() else {
            return false;
        };
        let other = question.options.len();
        if index > other {
            return false;
        }
        if index == other {
            return self.begin_edit();
        }
        let multi = question.multi_select;
        let current = self.current;
        if multi {
            let selected = &mut self.selected[current];
            if let Some(position) = selected.iter().position(|item| *item == index) {
                selected.remove(position);
            } else {
                selected.push(index);
            }
        } else {
            self.selected[current] = vec![index];
            self.clear_custom(current);
        }
        true
    }

    /// 鼠标点选项行：高亮该项并选中，但**绝不提交**（design §7.3）。
    ///
    /// 单选按下标独占选择，多选切换。单选若只高亮不选中，纯鼠标用户永远交不出
    /// 答案（Submit 只会得到空答案提示）——单击选中、Submit 提交是唯一可行的鼠标路径。
    pub fn click_option(&mut self, index: usize) -> bool {
        if self.confirm {
            return false;
        }
        let Some(question) = self.question() else {
            return false;
        };
        if index >= question.options.len() {
            return false;
        }
        self.choose(index)
    }

    /// 移动高亮（钳制在 `0..row_count`，不循环）；返回是否移动。
    pub fn move_highlight(&mut self, delta: isize) -> bool {
        if self.confirm {
            return false;
        }
        let len = self.row_count();
        if len == 0 {
            return false;
        }
        let max = len - 1;
        let next = (self.highlight[self.current] as isize)
            .saturating_add(delta)
            .clamp(0, max as isize) as usize;
        let moved = next != self.highlight[self.current];
        self.highlight[self.current] = next;
        moved
    }

    /// 设置高亮行（鼠标点击）；越界返回 false。
    pub fn set_highlight(&mut self, index: usize) -> bool {
        if self.confirm || index >= self.row_count() || index == self.highlight[self.current] {
            return false;
        }
        self.highlight[self.current] = index;
        true
    }

    /// 进入「其他」编辑态，光标落在文本末尾。
    pub fn begin_edit(&mut self) -> bool {
        if self.confirm || self.row_count() == 0 {
            return false;
        }
        self.highlight[self.current] = self.other_row();
        if let Some(editor) = self.custom.get_mut(self.current) {
            editor.set_cursor_end();
        }
        self.editing = true;
        true
    }

    /// 提交手写文本并退出编辑：非空即计入答案；空文本视为未作答。
    ///
    /// 单选下提交手写答案会清空该题勾选（保证「单选 = 一个答案」）。
    pub fn commit_edit(&mut self) -> bool {
        if !self.editing {
            return false;
        }
        self.editing = false;
        let current = self.current;
        let text = self
            .custom
            .get(current)
            .map(|editor| editor.text().trim().to_string())
            .unwrap_or_default();
        if text.is_empty() {
            self.custom_on[current] = false;
            return false;
        }
        self.custom_on[current] = true;
        if self
            .question()
            .is_some_and(|question| !question.multi_select)
        {
            self.selected[current].clear();
        }
        true
    }

    /// 退出编辑态，文本保留（`Esc` 语义）。
    pub fn cancel_edit(&mut self) -> bool {
        std::mem::replace(&mut self.editing, false)
    }

    /// 是否处于多题流程的复核步。
    pub fn is_confirm(&self) -> bool {
        self.confirm
    }

    /// tab 总数：多题时最后多一个 Confirm。
    pub fn tab_len(&self) -> usize {
        self.questions.len() + usize::from(self.questions.len() > 1)
    }

    /// 指定题是否已有答案（勾选或已提交的手写文本）。
    pub fn answered(&self, index: usize) -> bool {
        !self.answers_at(index).is_empty()
    }

    /// 是否全部题目都已作答（Confirm 的放行条件）。
    pub fn all_answered(&self) -> bool {
        (0..self.questions.len()).all(|index| self.answered(index))
    }

    /// 切到指定 tab（题目或 Confirm）；同位置返回 false。
    pub fn goto_tab(&mut self, index: usize) -> bool {
        if index >= self.tab_len() {
            return false;
        }
        let confirm = index >= self.questions.len();
        if confirm == self.confirm && (confirm || index == self.current) {
            return false;
        }
        if !confirm {
            self.current = index;
        }
        self.confirm = confirm;
        self.editing = false;
        self.restore_highlight();
        true
    }

    /// 回到某题时把高亮落到已选答案上（opencode 的 `pickFocus`）：
    /// 否则回车会默认选中第 1 项，悄悄覆盖用户之前的选择。
    fn restore_highlight(&mut self) {
        if self.confirm {
            return;
        }
        let current = self.current;
        let highlight = if self.custom_on.get(current).copied().unwrap_or(false) {
            self.other_row()
        } else {
            self.selected
                .get(current)
                .and_then(|picked| picked.first().copied())
                .unwrap_or(0)
        };
        if let Some(slot) = self.highlight.get_mut(current) {
            *slot = highlight;
        }
    }

    /// 前进到下一题；末题之后进入 Confirm（仅多题）。已是 Confirm 返回 false。
    pub fn next_step(&mut self) -> bool {
        if self.confirm {
            return false;
        }
        if self.current + 1 < self.questions.len() {
            self.current += 1;
            self.editing = false;
            self.restore_highlight();
            return true;
        }
        if self.questions.len() > 1 {
            self.confirm = true;
            self.editing = false;
            return true;
        }
        false
    }

    /// 后退：Confirm → 末题；末题 → 上一题。
    pub fn prev_step(&mut self) -> bool {
        if self.confirm {
            self.confirm = false;
            return true;
        }
        if self.current == 0 {
            return false;
        }
        self.current -= 1;
        self.editing = false;
        self.restore_highlight();
        true
    }

    /// 当前高亮选项的预览文本（Confirm 步没有预览）。
    pub fn preview(&self) -> Option<&str> {
        if self.confirm {
            return None;
        }
        let question = self.question()?;
        question
            .options
            .get(self.highlight[self.current])?
            .preview
            .as_deref()
    }

    /// 当前预览的最大滚动偏移：总行数减可视行数（不足一屏时为 0）。
    ///
    /// 只按 `lines - 1` 钳制会让短预览也能滚出一片空白：内容没溢出时滚动毫无意义。
    pub fn preview_max_scroll(&self) -> usize {
        let Some(preview) = self.preview() else {
            return 0;
        };
        let total = preview.split('\n').count();
        total.saturating_sub(self.preview_visible.max(1))
    }

    /// 同步预览区可视行数（只有真的溢出才允许滚动，返回值表示偏移被收紧）。
    pub fn resize_preview(&mut self, visible: usize) -> bool {
        self.preview_visible = visible;
        let mut moved = false;
        for index in 0..self.preview_scroll.len() {
            let total = self
                .questions
                .get(index)
                .and_then(|question| {
                    question
                        .options
                        .get(self.highlight.get(index).copied().unwrap_or(0))
                })
                .and_then(|option| option.preview.as_deref())
                .map_or(0, |preview| preview.split('\n').count());
            let max = total.saturating_sub(visible.max(1));
            if self.preview_scroll[index] > max {
                self.preview_scroll[index] = max;
                moved = true;
            }
        }
        moved
    }

    /// 滚动预览（正数向下）；返回偏移是否变化。
    pub fn scroll_preview(&mut self, delta: isize) -> bool {
        if self.confirm {
            return false;
        }
        let max = self.preview_max_scroll();
        let next = (self.preview_scroll[self.current] as isize)
            .saturating_add(delta)
            .clamp(0, max as isize) as usize;
        let moved = next != self.preview_scroll[self.current];
        self.preview_scroll[self.current] = next;
        moved
    }

    /// 该题手写缓冲（供编辑态按键/粘贴使用）。
    pub fn editor_mut(&mut self) -> Option<&mut AskEditor> {
        self.custom.get_mut(self.current)
    }

    /// 当前题手写缓冲（只读；渲染编辑框用）。
    pub fn editor(&self) -> Option<&AskEditor> {
        self.custom.get(self.current)
    }

    /// 同步全部手写缓冲的软换行宽度与可视高度。
    ///
    /// 渲染宽度与折行宽度必须同源：创建面板时按输入框宽度初始化，窗格 resize 时
    /// 由 `resize_panes` 用面板内编辑框的真实宽度再同步一次。
    pub fn resize_editor(&mut self, width: u16, height: u16) {
        for editor in &mut self.custom {
            editor.resize(width, height);
        }
    }

    /// 清空某题手写答案（文本 + 计入标记）。
    fn clear_custom(&mut self, index: usize) {
        if let Some(editor) = self.custom.get_mut(index) {
            editor.clear();
        }
        self.custom_on[index] = false;
    }
}

/// 独立编辑缓冲：包一层空 `Lx`，只暴露文本编辑 API。
///
/// 复用理由：软换行、宽字符、撤销/重做、词移动、粘贴等逻辑全部已在 [`Lx`] 里，
/// 另写一份必然漂移。这里构造一个空 `Lx`（chat/rewind/task 等字段恒为空），
/// 只把编辑命令转发进去。**不实现 `DerefMut`**：防止误写 `chat`/`rewind` 等字段。
#[derive(Debug)]
pub struct AskEditor(Lx);

impl Default for AskEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl AskEditor {
    pub fn new() -> Self {
        Self(Lx::plain())
    }

    /// 当前文本：与 [`Self::visual_rows`]/[`Self::cursor_visual`] 同一份原始缓冲。
    ///
    /// 不用 `serialized_input()`：那是给 shell 草稿用的（会补 `!` 前缀），与按
    /// 原始 `input` 计算的视觉行字节偏移不同源，切出来的片段会错位甚至整行丢失。
    pub fn text(&self) -> String {
        self.0.input.clone()
    }

    /// 光标字节偏移（渲染层计算编辑框内光标用；只读，禁止外部改写）。
    pub fn cursor(&self) -> usize {
        self.0.cursor
    }

    /// 软换行后的视觉行（渲染编辑框用；宽度由 [`Self::resize`] 同步）。
    pub fn visual_rows(&self) -> Vec<LxRow> {
        self.0.visual_rows()
    }

    /// 光标在视觉行中的位置 `(行, 列)`。
    pub fn cursor_visual(&self) -> (usize, usize) {
        self.0.cursor_visual()
    }

    /// 同步软换行宽度与可视高度（渲染与折行同一份几何）。
    pub fn resize(&mut self, width: u16, height: u16) {
        self.0.resize(width, height);
    }

    /// 当前软换行宽度（列）；渲染编辑框时以它为准，保证折行与画出的框同源。
    pub fn width(&self) -> u16 {
        self.0.input_width()
    }

    /// 光标落到文本末尾（进入编辑态时调用）。
    pub fn set_cursor_end(&mut self) {
        self.0.end();
    }

    /// 还能否再插入 `bytes` 字节：手写答案的硬上限在**写入处**执行，
    /// 键入、粘贴、换行共用同一条规则（超限拒绝，绝不截断）。
    pub fn can_insert(&self, bytes: usize) -> bool {
        self.text().len().saturating_add(bytes) <= MAX_ASK_CUSTOM_BYTES
    }

    pub fn insert_char(&mut self, ch: char) {
        if self.can_insert(ch.len_utf8()) {
            self.0.insert_char(ch);
        }
    }

    pub fn insert_str(&mut self, text: &str) {
        if self.can_insert(text.len()) {
            self.0.insert_str(text);
        }
    }

    pub fn newline(&mut self) {
        if self.can_insert(1) {
            self.0.newline();
        }
    }

    pub fn backspace(&mut self) {
        self.0.backspace();
    }

    pub fn delete(&mut self) {
        self.0.delete();
    }

    pub fn move_left(&mut self) {
        self.0.move_left();
    }

    pub fn move_right(&mut self) {
        self.0.move_right();
    }

    pub fn move_up(&mut self) {
        self.0.move_up();
    }

    pub fn move_down(&mut self) {
        self.0.move_down();
    }

    pub fn home(&mut self) {
        self.0.home();
    }

    pub fn end(&mut self) {
        self.0.end();
    }

    pub fn word_left(&mut self) {
        self.0.word_left();
    }

    pub fn word_right(&mut self) {
        self.0.word_right();
    }

    pub fn delete_to_line_start(&mut self) {
        self.0.delete_to_line_start();
    }

    pub fn delete_to_line_end(&mut self) {
        self.0.delete_to_line_end();
    }

    pub fn delete_word_backward(&mut self) {
        self.0.delete_word_backward();
    }

    pub fn delete_word_forward(&mut self) {
        self.0.delete_word_forward();
    }

    pub fn undo(&mut self) {
        self.0.undo();
    }

    pub fn redo(&mut self) {
        self.0.redo();
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests;
