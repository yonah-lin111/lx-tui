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
    /// 题目标签（切到该题，状态保留）。
    Tab(usize),
}

/// 滚轮所在分区。
///
/// 选项与角色画预览共用同一个滚动视口，两个分区语义完全一致：保留枚举只为兼容
/// 既有事件路由（`Options` / `Preview` 都是「滚动统一列表」）。
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
    /// 提问挂起开始的毫秒时间戳：结束时把用户作答耗时从本轮耗时里剔除。
    pub started_at_ms: i64,
    /// 每题高亮行：`0..options.len()`，最后一行是「其他」。
    pub highlight: Vec<usize>,
    /// 每题已勾选项（按下标保存，顺序即勾选顺序）。
    pub selected: Vec<Vec<usize>>,
    /// 每题手写答案（独立编辑缓冲，绝不复用 `Lx.input`）。
    pub custom: Vec<AskEditor>,
    /// 每题手写文本是否计入答案（提交编辑态即计入；清空即不计入）。
    pub custom_on: Vec<bool>,
    /// 当前题是否处于编辑态。
    pub editing: bool,
    /// 是否停在 `Submit` 汇总 tab（只有多题 / 单题多选才有）。
    ///
    /// 单题单选没有汇总 tab：选中即提交，`confirm` 恒为 `false`。
    pub confirm: bool,
    /// 统一列表的滚动起点、总行数与可视行数（由事件循环按几何同步）。
    ///
    /// 选项与角色画预览现在是同一份可滚列表，`PageUp`/`PageDown` 在 `app/` 层
    /// 拿不到窗格几何，只能消费这里落下的上一帧窗口。
    pub list_scroll: usize,
    pub list_total: usize,
    pub list_visible: usize,
    /// 未扣减右侧滚动条列的行内编辑框基准宽度（列）；列表溢出时自动减 1 列与渲染对齐。
    base_editor_width: Option<u16>,
    /// 选项列表的显式滚动起点（行）；`None` 表示窗口跟随高亮。
    ///
    /// 语义与 `@` 提及面板、`/` 命令面板的 `PanelWindow.viewport` 一致：
    /// **滚轮只动它，不动高亮**。键盘移动/点击选中会把它清空，窗口重新跟随高亮。
    /// 之前把滚动从高亮推导，导致「滚轮 = 移高亮」，滚到哪就选到哪。
    pub viewport: Option<usize>,
}

impl AskState {
    /// 「其他」行内输入框最多占用的视觉行数。
    pub const MAX_OTHER_EDIT_ROWS: usize = 3;

    /// 以题目列表构造面板；每题初始高亮第 0 行、无勾选、无手写。
    pub fn new(key: RequestKey, call_id: String, questions: Vec<AskQuestion>) -> Self {
        let count = questions.len();
        Self {
            key,
            call_id,
            questions,
            current: 0,
            started_at_ms: crate::db::now_ms(),
            highlight: vec![0; count],
            selected: vec![Vec::new(); count],
            custom: (0..count).map(|_| AskEditor::new()).collect(),
            custom_on: vec![false; count],
            editing: false,
            confirm: false,
            list_scroll: 0,
            list_total: 0,
            list_visible: 0,
            base_editor_width: None,
            viewport: None,
        }
    }

    /// 当前题。
    pub fn question(&self) -> Option<&AskQuestion> {
        self.questions.get(self.current)
    }

    /// 唯一一题且为单选：没有 tab 栏、没有 `Submit` 汇总，选中即提交。
    pub fn is_single_select_only(&self) -> bool {
        self.questions.len() == 1 && self.questions.first().is_some_and(|q| !q.multi_select)
    }

    /// 是否停在 `Submit` 汇总 tab（单题单选永远不是）。
    pub fn is_confirm(&self) -> bool {
        !self.is_single_select_only() && self.confirm
    }

    /// 当前题可被 `↑`/`↓` 高亮的逻辑条目数：选项 + 「其他」。
    ///
    /// `Submit` 汇总 tab 没有任何可高亮项（`↑`/`↓` 在那里是滚动）。
    pub fn row_count(&self) -> usize {
        if self.is_confirm() {
            return 0;
        }
        self.question()
            .map_or(0, |question| question.options.len() + 1)
    }

    /// 「其他」行的逻辑下标。
    pub fn other_row(&self) -> usize {
        self.question().map_or(0, |question| question.options.len())
    }

    /// 当前题是否有任何选项配置了非空 preview（决定预览区是否占位）。
    pub fn question_has_preview(&self) -> bool {
        self.question().is_some_and(|question| {
            question.options.iter().any(|option| {
                option
                    .preview
                    .as_deref()
                    .is_some_and(|p| !p.trim().is_empty())
            })
        })
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
        self.viewport = None;
        true
    }

    /// 移动高亮：在 `0..row_count` 上**循环**（对齐 `opencode` 的
    /// `(selected + delta + total) % total`），滚动视口交还给高亮。
    ///
    /// 必须清视口：高亮滚出可视区后按到边界方向键时只会绕回另一端，若不清，
    /// 窗口不会回来——键盘看起来"完全没反应"，用户唯一的出路是继续盲目滚轮。
    pub fn move_highlight(&mut self, delta: isize) -> bool {
        let len = self.row_count();
        if len == 0 {
            return false;
        }
        let current = self.highlight[self.current].min(len - 1);
        let next = (current as isize)
            .saturating_add(delta)
            .rem_euclid(len as isize) as usize;
        let moved = next != self.highlight[self.current];
        self.highlight[self.current] = next;
        let had_viewport = self.viewport.take().is_some();
        moved || had_viewport
    }

    /// 设置高亮行（鼠标点击 / 快捷键）；越界返回 false。
    pub fn set_highlight(&mut self, index: usize) -> bool {
        if index >= self.row_count() {
            return false;
        }
        // 选中/悬停高亮后窗口跟随高亮，滚轮留下的显式视口作废。
        let scrolled = self.viewport.take().is_some();
        if index == self.highlight[self.current] {
            return scrolled;
        }
        self.highlight[self.current] = index;
        true
    }

    /// 进入「其他」编辑态，光标落在文本末尾。
    pub fn begin_edit(&mut self) -> bool {
        if self.row_count() == 0 {
            return false;
        }
        self.highlight[self.current] = self.other_row();
        self.viewport = None;
        if let Some(editor) = self.custom.get_mut(self.current) {
            editor.clear_text_selection();
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
        if let Some(editor) = self.custom.get_mut(current) {
            editor.clear_text_selection();
        }
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
        if let Some(editor) = self.custom.get_mut(self.current) {
            editor.clear_text_selection();
        }
        std::mem::replace(&mut self.editing, false)
    }

    /// tab 总数：单题单选没有 tab（恒 1），其余为「题目数 + Submit 汇总」。
    pub fn tab_len(&self) -> usize {
        if self.is_single_select_only() {
            1
        } else {
            self.questions.len().saturating_add(1)
        }
    }

    /// 指定题是否已有答案（勾选或已提交的手写文本）。
    pub fn answered(&self, index: usize) -> bool {
        !self.answers_at(index).is_empty()
    }

    /// 是否全部题目都已作答（提交汇总的放行条件）。
    pub fn all_answered(&self) -> bool {
        (0..self.questions.len()).all(|index| self.answered(index))
    }

    /// 切到指定 tab；题目越界钳到末题，`questions.len()` 是 `Submit` 汇总。
    ///
    /// 与 `opencode` 的 `selectTab` 一致：切 tab 一律退出编辑、清滚动视口，
    /// 并恢复该题的高亮（否则回车会默认选中第 1 项、悄悄覆盖用户之前的选择）。
    pub fn goto_tab(&mut self, index: usize) -> bool {
        if !self.is_single_select_only() && index >= self.questions.len() {
            if self.confirm {
                return false;
            }
            if let Some(editor) = self.custom.get_mut(self.current) {
                editor.clear_text_selection();
            }
            self.confirm = true;
            self.editing = false;
            self.viewport = None;
            return true;
        }
        let index = index.min(self.questions.len().saturating_sub(1));
        if index == self.current && !self.confirm {
            return false;
        }
        if let Some(editor) = self.custom.get_mut(self.current) {
            editor.clear_text_selection();
        }
        self.current = index;
        self.confirm = false;
        self.editing = false;
        self.viewport = None;
        self.restore_highlight();
        true
    }

    /// 回到某题时把高亮落到已选答案上（opencode 的 `pickFocus`）：
    /// 否则回车会默认选中第 1 项，悄悄覆盖用户之前的选择。
    fn restore_highlight(&mut self) {
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

    /// 前进一个 tab：题目 → 下一题 → `Submit` 汇总 → 回到第 1 题（循环）。
    ///
    /// 单题单选没有 tab 可切，返回 false。
    pub fn next_step(&mut self) -> bool {
        if self.is_single_select_only() {
            return false;
        }
        if self.is_confirm() {
            return self.goto_tab(0);
        }
        if self.current + 1 < self.questions.len() {
            self.goto_tab(self.current + 1)
        } else {
            self.goto_tab(self.questions.len())
        }
    }

    /// 后退一个 tab：`Submit` 汇总 → 末题 → 上一题 → `Submit` 汇总（循环）。
    pub fn prev_step(&mut self) -> bool {
        if self.is_single_select_only() {
            return false;
        }
        if self.is_confirm() {
            return self.goto_tab(self.questions.len().saturating_sub(1));
        }
        if self.current > 0 {
            self.goto_tab(self.current - 1)
        } else {
            self.goto_tab(self.questions.len())
        }
    }

    /// 当前高亮选项的预览文本（高亮在 `Other` 时没有预览）。
    pub fn preview(&self) -> Option<&str> {
        let highlight = *self.highlight.get(self.current)?;
        self.question()?.options.get(highlight)?.preview.as_deref()
    }

    /// 同步统一列表的 `(滚动起点, 总行数, 可视行数)`（事件循环按几何写入）。
    ///
    /// 列表溢出出现右侧滚动条时，行宽收窄 1 列：这里同步调整 `AskEditor` 的软折行宽度，
    /// 保证 `editor.visual_rows()` 与 `draw_other` 的正文列宽严格同源。
    pub fn sync_list_window(&mut self, scroll: usize, total: usize, visible: usize) -> bool {
        let mut changed =
            self.list_scroll != scroll || self.list_total != total || self.list_visible != visible;
        self.list_scroll = scroll;
        self.list_total = total;
        self.list_visible = visible;
        if let Some(base) = self.base_editor_width {
            let effective = if visible > 0 && total > visible {
                base.saturating_sub(1).max(1)
            } else {
                base
            };
            changed |= self.sync_editor_width(effective);
        }
        changed
    }

    /// 直接将手写缓冲宽度同步为渲染层给出的精确正文列宽（已含滚动条列扣减）。
    pub fn sync_editor_width(&mut self, width: u16) -> bool {
        let width = width.max(1);
        let rows = Self::MAX_OTHER_EDIT_ROWS as u16;
        let mut changed = false;
        for editor in &mut self.custom {
            if editor.width() != width {
                editor.resize(width, rows);
                changed = true;
            }
        }
        changed
    }

    /// 按行滚动统一列表（`Submit` 汇总 tab 的 `↑`/`↓`）；返回视口是否变化。
    pub fn scroll_window(&mut self, delta: isize) -> bool {
        let base = self.viewport.unwrap_or(self.list_scroll);
        self.scroll_viewport(delta, base, self.list_total, self.list_visible)
    }

    /// 按一屏滚动统一列表（`PageUp`/`PageDown`）；返回视口是否变化。
    pub fn scroll_page(&mut self, direction: isize) -> bool {
        let step = isize::try_from(self.list_visible.max(1)).unwrap_or(isize::MAX);
        self.scroll_window(direction.saturating_mul(step))
    }

    /// 滚轮滚动选项列表：只动显式视口，**不动高亮**（与 `@`/`/` 面板同语义）。
    ///
    /// `base` 是当前窗口起点（由渲染层给出，事件层不重算）；`total` 是列表总行数。
    /// 返回视口是否变化。列表装得下时返回 false，滚轮被吞掉但不产生虚假位移。
    pub fn scroll_viewport(
        &mut self,
        delta: isize,
        base: usize,
        total: usize,
        visible: usize,
    ) -> bool {
        if total == 0 || visible == 0 || total <= visible {
            return false;
        }
        let max = total.saturating_sub(visible);
        let next = (base.min(max) as isize)
            .saturating_add(delta)
            .clamp(0, max as isize) as usize;
        let changed = next != base;
        self.viewport = Some(next);
        changed
    }

    /// 选项列表的显式视口起点；`None` 表示窗口跟随高亮。
    pub fn viewport(&self) -> Option<usize> {
        self.viewport
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
    /// 由 `resize_panes` 用面板内行内输入框的基准宽度同步，绘制前再由 `sync_list_window`
    /// 按是否溢出出现滚动条扣减 1 列。
    ///
    /// 可视高度恒夹在 `1..=MAX_OTHER_EDIT_ROWS`：`Other` 原地最多展开 3 行。
    pub fn resize_editor(&mut self, width: u16, height: u16) {
        let base = width.max(1);
        self.base_editor_width = Some(base);
        let effective = if self.list_visible > 0 && self.list_total > self.list_visible {
            base.saturating_sub(1).max(1)
        } else {
            base
        };
        let rows = height.clamp(1, Self::MAX_OTHER_EDIT_ROWS as u16);
        for editor in &mut self.custom {
            editor.resize(effective, rows);
        }
    }

    /// 把用户作答等待时长补偿进本轮起点：`ask` 期间不计入 Agent 耗时与 tok/s。
    pub fn compensate_turn_clock(&self, chat: &mut super::LxChat) {
        let paused_ms = crate::db::now_ms()
            .saturating_sub(self.started_at_ms)
            .max(0);
        if let Some(started) = chat.turn_started_at_ms.as_mut() {
            *started = started.saturating_add(paused_ms);
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
        let mut lx = Lx::plain();
        lx.resize(u16::MAX, AskState::MAX_OTHER_EDIT_ROWS as u16);
        Self(lx)
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

    /// 视口首视觉行索引（由内部 `Lx` 按 3 行滑动窗口维护）。
    pub fn scroll(&self) -> usize {
        self.0.scroll()
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
        self.0.cursor = self.0.input.len();
        self.0.scroll_cursor_into_view();
    }

    /// 还能否再插入 `bytes` 字节：手写答案的硬上限在**写入处**执行，
    /// 键入、粘贴、换行共用同一条规则（超限拒绝，绝不截断）。
    ///
    /// 传入的必须是**净化后**的字节数：`Lx::insert_str` 内部会把 `\t` 展开成两个
    /// 空格、丢弃控制字符，按原始输入判长会让上限翻倍。
    pub fn can_insert(&self, bytes: usize) -> bool {
        self.can_replace(bytes)
    }

    /// 判定在替换当前可能存在的选区后，是否仍在字节上限内。
    pub fn can_replace(&self, bytes: usize) -> bool {
        let selected_bytes = self
            .text_selection_range()
            .map_or(0, |(s, e)| e.saturating_sub(s));
        self.text()
            .len()
            .saturating_sub(selected_bytes)
            .saturating_add(bytes)
            <= MAX_ASK_CUSTOM_BYTES
    }

    pub fn insert_char(&mut self, ch: char) {
        let ch_len = ch.len_utf8();
        if !self.can_replace(ch_len) {
            return;
        }
        let selection = self.0.text_selection_range();
        self.0.snapshot();
        if let Some((start, end)) = selection {
            self.0.input.replace_range(start..end, "");
            self.0.cursor = start;
        }
        self.0.input.insert(self.0.cursor, ch);
        self.0.cursor += ch_len;
        self.0.touch();
        self.0.scroll_cursor_into_view();
    }

    /// 粘贴：先按与写入完全相同的净化规则算长度，再决定是否写入。
    ///
    /// 判定与写入必须看同一份文本，否则 `\t`（展开为两空格）这类字符会让
    /// 「预检通过、实际超限」。
    pub fn insert_str(&mut self, text: &str) -> bool {
        let sanitized = crate::app::prompt::text::sanitize(text);
        if sanitized.is_empty() {
            return false;
        }
        let sanitized_len = sanitized.len();
        if !self.can_replace(sanitized_len) {
            return false;
        }
        let selection = self.0.text_selection_range();
        self.0.snapshot();
        if let Some((start, end)) = selection {
            self.0.input.replace_range(start..end, "");
            self.0.cursor = start;
        }
        self.0.input.insert_str(self.0.cursor, &sanitized);
        self.0.cursor += sanitized_len;
        self.0.touch();
        self.0.scroll_cursor_into_view();
        true
    }

    pub fn newline(&mut self) {
        if !self.can_replace(1) {
            return;
        }
        let selection = self.0.text_selection_range();
        self.0.snapshot();
        if let Some((start, end)) = selection {
            self.0.input.replace_range(start..end, "");
            self.0.cursor = start;
        }
        self.0.input.insert(self.0.cursor, '\n');
        self.0.cursor += 1;
        self.0.touch();
        self.0.scroll_cursor_into_view();
    }

    pub fn backspace(&mut self) {
        if self.delete_selected() {
            return;
        }
        self.0.backspace();
    }

    pub fn delete(&mut self) {
        if self.delete_selected() {
            return;
        }
        self.0.delete();
    }

    pub fn move_left(&mut self) {
        self.clear_text_selection();
        self.0.move_left();
    }

    pub fn move_right(&mut self) {
        self.clear_text_selection();
        self.0.move_right();
    }

    pub fn move_up(&mut self) {
        self.clear_text_selection();
        self.0.move_up();
    }

    pub fn move_down(&mut self) {
        self.clear_text_selection();
        self.0.move_down();
    }

    /// Shift+←：向左扩展选区。
    pub fn select_left(&mut self) {
        self.0.select_left();
    }

    /// Shift+→：向右扩展选区。
    pub fn select_right(&mut self) {
        self.0.select_right();
    }

    /// Shift+Ctrl/Alt+←：向左扩展一个词。
    pub fn select_word_left(&mut self) {
        self.0.select_word_left();
    }

    /// Shift+Ctrl/Alt+→：向右扩展一个词。
    pub fn select_word_right(&mut self) {
        self.0.select_word_right();
    }

    /// Shift+Home：扩展到视觉行首。
    pub fn select_home(&mut self) {
        self.0.select_home();
    }

    /// Shift+End：扩展到视觉行尾。
    pub fn select_end(&mut self) {
        self.0.select_end();
    }

    /// Shift+↑：向上扩展一个视觉行。
    pub fn select_up(&mut self) {
        self.0.select_up();
    }

    /// Shift+↓：向下扩展一个视觉行。
    pub fn select_down(&mut self) {
        self.0.select_down();
    }

    /// Ctrl+A：全选。
    pub fn select_all(&mut self) {
        self.0.select_all();
    }

    pub fn home(&mut self) {
        self.clear_text_selection();
        self.0.home();
    }

    pub fn end(&mut self) {
        self.clear_text_selection();
        self.0.end();
    }

    pub fn word_left(&mut self) {
        self.clear_text_selection();
        self.0.word_left();
    }

    pub fn word_right(&mut self) {
        self.clear_text_selection();
        self.0.word_right();
    }

    pub fn delete_to_line_start(&mut self) {
        if self.delete_selected() {
            return;
        }
        self.0.delete_to_line_start();
    }

    pub fn delete_to_line_end(&mut self) {
        if self.delete_selected() {
            return;
        }
        self.0.delete_to_line_end();
    }

    pub fn delete_word_backward(&mut self) {
        if self.delete_selected() {
            return;
        }
        self.0.delete_word_backward();
    }

    pub fn delete_word_forward(&mut self) {
        if self.delete_selected() {
            return;
        }
        self.0.delete_word_forward();
    }

    pub fn undo(&mut self) {
        self.clear_text_selection();
        self.0.undo();
    }

    pub fn redo(&mut self) {
        self.clear_text_selection();
        self.0.redo();
    }

    pub fn clear(&mut self) {
        self.clear_text_selection();
        self.0.clear();
    }

    /// 在指定视觉行/内容列开始输入框文本选择。
    pub fn begin_text_selection(&mut self, row: usize, col: u16) {
        self.0.begin_text_selection(row, col);
    }

    /// 按连续点击次数（1=单字、2=选词、3=全选/选行）在指定视觉行/内容列开始文本选择。
    pub fn begin_text_selection_with_clicks(&mut self, row: usize, col: u16, clicks: u8) {
        self.0.begin_text_selection_with_clicks(row, col, clicks);
    }

    /// 拖拽扩展文本选区。
    pub fn drag_text_selection(&mut self, row: usize, col: impl TryInto<u16>) {
        let col = col.try_into().unwrap_or(u16::MAX);
        self.0.drag_text_selection(row, col);
    }

    /// 清除输入框文本选区。
    pub fn clear_text_selection(&mut self) {
        self.0.clear_text_selection();
    }

    /// 是否存在选区。
    pub fn has_text_selection(&self) -> bool {
        self.0.has_text_selection()
    }

    /// 选区字节范围 `[start, end)`；空选区返回 None。
    pub fn text_selection_range(&self) -> Option<(usize, usize)> {
        self.0.text_selection_range()
    }

    /// 取选中文本；空选区返回 None。
    pub fn selected_text(&self) -> Option<String> {
        self.0.selected_text()
    }

    /// 删除选中的文本区间；返回是否实际删除了选区。
    pub fn delete_selected(&mut self) -> bool {
        let Some((start, end)) = self.0.text_selection_range() else {
            self.0.clear_text_selection();
            return false;
        };
        self.0.snapshot();
        self.0.input.replace_range(start..end, "");
        self.0.cursor = start;
        self.0.clear_text_selection();
        self.0.touch();
        self.0.scroll_cursor_into_view();
        true
    }

    /// 剪切选中文本：返回被剪切文本并将删除记入一步撤销；无选区返回 None。
    pub fn cut_selected(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.delete_selected();
        Some(text)
    }
}

#[cfg(test)]
mod tests;
