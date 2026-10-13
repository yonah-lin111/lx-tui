//! prompt 右栏编辑器：纯文本缓冲区、光标、软换行视口与选区取文，无 IO。

mod list;
mod mention;
mod paste;
pub use mention::MentionRootChoice;
pub use paste::*;
mod panel;
mod refer;
mod template;
pub mod tree;

pub use tree::{
    PromptEditing, PromptFolder, PromptItem, PromptItemStatus, PromptMode, PromptPanel, PromptRow,
    PromptRowKind, PromptSection, PromptTree, TempPrompt,
};

mod text;
mod viewport;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use self::text::*;

use self::mention::MentionState;
use super::markdown::{
    self, BlockPanel, MentionEntry, MentionPanel, SlashItem, SlashPanel, WorktreeBinding,
    WorktreeCandidate,
};
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
    /// 斜杠模板命令面板；由整行 `/` 触发逼近得到，或 `/gitWorkTree` 的二级选择器。
    slash: Option<SlashPanel>,
    /// 有效自定义斜杠命令（User 与 Project 合并后）；由事件层在载入/切换时注入。
    custom: Vec<SlashItem>,
    /// 显示中工作区是否有 git 仓库（决定 `/gitWorkTree` 候选）。
    worktree_available: bool,
    /// 文件提及面板状态；含扫描缓存与在途代号。
    mention: MentionState,
    /// 粘贴提案状态机：在途读取、二级面板与待清理标记。
    paste: PasteState,
    /// 字母快捷输入面板；由正文已出现的引用候选生成。
    refer: Option<crate::app::markdown::ReferPanel>,
    /// 面板压制标记：Esc 关闭或确认插入后，等待下一次编辑/移动再重算。
    panel_suppressed: bool,
    /// 当前是否处于已保存状态；任何文本写操作置 false。
    is_saved: bool,
    /// 内容修订号：每次置脏 +1（光标移动不计数），供自动保存静置判定。
    revision: u64,
    /// 自动保存静置观察：最近一次判定时的修订号与时间；修订号变化即重置计时。
    autosave: Option<(u64, Instant)>,
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
            slash: None,
            custom: Vec::new(),
            worktree_available: false,
            mention: MentionState::default(),
            paste: PasteState::default(),
            refer: None,
            panel_suppressed: false,
            is_saved: true,
            revision: 0,
            autosave: None,
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

    /// 光标字节偏移；始终位于字符边界。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 首个可见视觉行索引。
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 内容区尺寸（列、行）。
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// 当前是否处于已保存状态。
    pub fn is_saved(&self) -> bool {
        self.is_saved
    }

    /// 内容修订号：每次置脏 +1（光标移动不计数）；保存回执按此确认。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 保存成功回执：修订号仍匹配时标记已保存并停止自动保存计时；返回是否确认。
    pub fn confirm_saved(&mut self, revision: u64) -> bool {
        if self.revision != revision {
            return false;
        }
        let changed = !self.is_saved;
        self.is_saved = true;
        self.autosave = None;
        changed
    }

    /// 保存失败后重置自动保存观察，等待下一轮静置重试。
    pub fn reset_autosave_watch(&mut self) {
        self.autosave = None;
    }

    /// 手动标记为已保存（工具栏保存点/手动保存/自动保存落库后）。
    pub fn mark_saved(&mut self) {
        self.is_saved = true;
        self.autosave = None;
    }

    /// 自动保存待触发时刻：未保存且静置观察与当前修订一致时返回 `起点 + delay`；
    /// 事件循环据此安排唤醒（空档期不轮询）。
    pub fn autosave_deadline(&self, delay: Duration) -> Option<Instant> {
        if self.is_saved {
            return None;
        }
        let (revision, since) = self.autosave?;
        (revision == self.revision).then_some(since + delay)
    }

    /// 自动保存静置判定：内容变更后静置达到 `delay` 返回 true；
    /// 每次调用记录当前修订号的观察起点，后续新修订自动重置计时。
    pub fn autosave_due(&mut self, now: Instant, delay: Duration) -> bool {
        if self.is_saved {
            self.autosave = None;
            return false;
        }
        match self.autosave {
            Some((revision, since)) if revision == self.revision => {
                now.duration_since(since) >= delay
            }
            _ => {
                self.autosave = Some((self.revision, now));
                false
            }
        }
    }

    /// 替换全文（载入提示词条目）：净化文本、清空历史与浮层并回到已保存态；光标落在文末。
    pub fn set_text(&mut self, text: &str) {
        self.text = sanitize(text);
        self.cursor = self.text.len();
        self.scroll = 0;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
        self.panel = None;
        self.slash = None;
        self.mention.clear();
        self.refer = None;
        self.paste.close();
        self.panel_suppressed = false;
        self.is_saved = true;
        self.autosave = None;
        self.scroll_cursor_into_view();
    }

    /// 是否存在可撤销的编辑。
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// 是否存在可重做的编辑。
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
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
        if self.protected_edit(end, end) {
            return;
        }
        self.record(EditKind::Other);
        self.text.insert_str(end, &insert);
        self.cursor = end + insert.len();
        self.settle();
    }

    /// 删除光标前一个字符；@ 提及与 `@[refer-*]` 引用后的空白处整块删除该标记，
    /// 紧跟列表标记时按列表上下文处理标记。
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if self.mention.panel().is_none()
            && let Some(range) = markdown::mention_deletion_range(&self.text, self.cursor)
                .or_else(|| markdown::reference_deletion_range(&self.text, self.cursor))
        {
            if self.protected_edit(range.start, range.end) {
                return;
            }
            let start = range.start;
            self.record(EditKind::Delete);
            self.text.replace_range(range, "");
            self.cursor = start;
            self.settle();
            return;
        }
        if !self.in_fence() && self.delete_list_markup() {
            return;
        }
        let previous = prev_boundary(&self.text, self.cursor);
        if self.protected_edit(previous, self.cursor) {
            return;
        }
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
        if self.protected_edit(self.cursor, next_boundary(&self.text, self.cursor)) {
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
        self.mark_dirty();
        let current = self.snapshot();
        push_bounded(&mut self.redo, current);
        self.restore(snapshot);
    }

    /// 重做；无可重做内容时 no-op。
    pub fn redo(&mut self) {
        let Some(snapshot) = self.redo.pop() else {
            return;
        };
        self.mark_dirty();
        let current = self.snapshot();
        push_bounded(&mut self.undo, current);
        self.restore(snapshot);
    }

    ///
    /// 同时作废在途的提及扫描结果，避免失焦后异步结果把面板重新弹出。
    pub fn clear_panel(&mut self) {
        self.panel = None;
        self.slash = None;
        self.mention.clear();
        self.mention.unsuppress();
        self.refer = None;
        self.paste.cancel();
        self.panel_suppressed = false;
    }

    /// 当前文件提及面板；未打开时为 None。
    pub fn mention(&self) -> Option<&MentionPanel> {
        self.mention.panel()
    }

    /// 取根选择器高亮索引；选择器未显示时为 None。
    pub fn mention_chooser(&self) -> Option<usize> {
        self.mention.chooser()
    }

    /// 提及面板（选择器或文件候选）是否可见。
    pub fn mention_active(&self) -> bool {
        self.mention.view_open()
    }

    /// 提及面板标题后缀：绑定 linked worktree 且已选定取根时为 `main` / `wt`。
    pub fn mention_root_label(&self) -> Option<&'static str> {
        self.mention.root_label(&self.text, self.cursor)
    }

    /// 提及面板打开时按偏移循环移动高亮；返回是否消费该按键。
    pub fn mention_move(&mut self, delta: isize) -> bool {
        self.mention.move_active(delta)
    }

    /// 提及面板悬停高亮：设置高亮索引；返回是否变化。
    pub fn mention_set_active(&mut self, index: usize) -> bool {
        self.mention.set_active(index)
    }

    /// 滚轮在提及面板上滚动可见窗口：视口从 `base` 起偏移、越界钳制不循环；
    /// 高亮不动；返回视口是否移动。
    pub fn mention_scroll(&mut self, delta: isize, base: usize) -> bool {
        self.mention.scroll_viewport(delta, base)
    }

    /// 提及面板点选：设置高亮并确认插入；返回是否消费。
    pub fn mention_confirm_at(&mut self, index: usize) -> bool {
        self.mention.set_active(index);
        self.mention_confirm()
    }

    /// 提及面板打开时确认高亮条目：取根选择器确认取根，文件候选替换触发区间并压制重弹；
    /// 返回是否消费该按键。
    pub fn mention_confirm(&mut self) -> bool {
        if self.mention.choose_root(&self.text, self.cursor) {
            return true;
        }
        let Some((range, insertion)) = self.mention.confirm(self.text.len()) else {
            return false;
        };
        if self.protected_edit(range.start, range.end) {
            return false;
        }
        self.break_group();
        self.record(EditKind::Other);
        self.text.replace_range(range.clone(), &insertion);
        self.cursor = range.start + insertion.len();
        self.panel_suppressed = true;
        self.scroll_cursor_into_view();
        true
    }

    /// Shift+Enter：@ 面板高亮目录可进入时把其根相对路径写回文本（`@路径/`）；返回是否消费。
    ///
    /// 进入后提及触发保持打开，面板按路径范围只列该目录候选；高亮非目录或目录无候选时返回 false。
    /// 写回按独立撤销步记录，撤销（`Ctrl+Z` / `Cmd+Z`）即回退到上一级目录。
    pub fn mention_enter_folder(&mut self) -> bool {
        let Some((range, insertion)) = self.mention.enter_folder(self.text.len()) else {
            return false;
        };
        if self.protected_edit(range.start, range.end) {
            return false;
        }
        self.break_group();
        self.record(EditKind::Other);
        self.text.replace_range(range.clone(), &insertion);
        self.cursor = range.start + insertion.len();
        self.settle();
        true
    }

    /// 提及面板打开时关闭且不改文本；返回是否消费该按键。
    pub fn mention_escape(&mut self) -> bool {
        if !self.mention.escape() {
            return false;
        }
        self.panel_suppressed = true;
        true
    }

    /// 同步提及扫描根；根变化时清除缓存并作废在途结果。
    ///
    /// `panel_suppressed` 为真（Esc/确认后尚未编辑或移动）时保持面板压制，
    /// 避免每次按键重算把面板重新弹出。
    pub fn set_mention_root(&mut self, root: Option<PathBuf>) {
        if self.panel_suppressed {
            self.mention.suppress();
        } else {
            self.mention.unsuppress();
        }
        self.mention.set_root(root, &self.text, self.cursor);
    }

    /// 同步显示中工作区的仓库根与 git 可用性；变化时作废 worktree 候选。
    /// 返回仓库根是否发生变化（调用方据此置脏重绘）。
    pub fn set_worktree_repo(&mut self, repo: Option<PathBuf>) -> bool {
        let changed = self.mention.worktree_repo() != repo.as_deref();
        let available = repo.is_some();
        if self.worktree_available != available {
            self.worktree_available = available;
            self.slash = None;
        }
        self.mention
            .set_worktree_repo(repo, &self.text, self.cursor);
        changed
    }

    /// 当前注入的自定义命令。
    pub fn custom_commands(&self) -> &[SlashItem] {
        &self.custom
    }

    /// 注入有效自定义命令；候选变化时关闭旧面板并由后续编辑重算。
    pub fn set_custom_commands(&mut self, items: Vec<SlashItem>) {
        if self.custom == items {
            return;
        }
        self.custom = items;
        self.slash = None;
        self.refresh_slash_panel();
    }

    /// 光标所在块的 worktree 绑定；无块或无绑定返回 None。
    pub fn block_binding(&self) -> Option<WorktreeBinding> {
        self.mention.block_binding(&self.text, self.cursor)
    }

    /// 需要 worktree 列表时返回仓库根；仅事件循环调用。
    pub fn take_worktree_request(&mut self) -> Option<PathBuf> {
        self.mention.take_worktree_request(&self.text, self.cursor)
    }

    /// 选择器打开期间的强制刷新请求；仅事件循环调用。
    pub fn take_worktree_refresh(&mut self) -> Option<PathBuf> {
        self.mention.request_worktree_refresh()
    }

    /// 选择器是否正在等待 worktree 列表。
    pub fn worktree_loading(&self) -> bool {
        self.mention.worktree_loading()
    }

    /// 斜杠面板是否处于 worktree 选择器模式。
    pub fn worktree_chooser_open(&self) -> bool {
        matches!(
            self.slash.as_ref().map(super::markdown::SlashPanel::kind),
            Some(super::markdown::SlashPanelKind::WorktreeChooser { .. })
        )
    }

    /// worktree 候选（已加载时）；选择器渲染用。
    pub fn worktree_candidates(&self) -> Option<&[WorktreeCandidate]> {
        self.mention.worktree_candidates()
    }

    /// 写入 worktree 列表结果并重算绑定；代号过期由 mention 状态自行判定。
    pub fn apply_worktree_entries(&mut self, candidates: Vec<WorktreeCandidate>) -> bool {
        let changed = self.mention.worktree_candidates() != Some(candidates.as_slice());
        self.mention
            .apply_worktree_entries(candidates, &self.text, self.cursor);
        changed
    }

    /// worktree 查询失败：清空候选并允许后续重试。
    pub fn worktree_failed(&mut self) {
        self.mention.worktree_failed(&self.text, self.cursor);
    }

    /// 需要新扫描时返回（代号，根路径）并标记处理中；仅事件循环调用。
    pub fn take_mention_scan_request(&mut self) -> Option<(u64, PathBuf)> {
        self.mention.take_scan_request(&self.text, self.cursor)
    }

    /// 写入扫描结果；代号过期（根变化或失焦）时丢弃；返回是否采纳。
    pub fn apply_mention_entries(&mut self, generation: u64, entries: Vec<MentionEntry>) -> bool {
        self.mention
            .apply_entries(generation, entries, &self.text, self.cursor)
    }

    /// 当前逻辑行整体右移一个缩进单位；围栏代码块内改为光标处插入空格。
    pub fn indent(&mut self) {
        if self.in_fence() {
            self.insert_at(TAB_STOP);
            return;
        }
        let start = line_start(&self.text, self.cursor);
        if self.protected_edit(start, start) {
            return;
        }
        self.record(EditKind::Other);
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
        if remove == 0 || self.protected_edit(start, start + remove) {
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
        if start < self.cursor && !self.protected_edit(start, self.cursor) {
            self.mark_dirty();
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
            self.settle();
        }
    }

    /// 删除光标到下一个词尾（readline M-d 语义）。
    pub fn delete_word_forward(&mut self) {
        let end = self.word_end_after(self.cursor);
        if end > self.cursor && !self.protected_edit(self.cursor, end) {
            self.mark_dirty();
            self.text.replace_range(self.cursor..end, "");
            self.settle();
        }
    }

    /// 删除光标到逻辑行首；光标已在行首时删除前一个换行（对齐 opencode deleteToLineStart）。
    pub fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = line_start(&self.text, self.cursor);
        if start < self.cursor {
            if self.protected_edit(start, self.cursor) {
                return;
            }
            self.record(EditKind::Delete);
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
            self.settle();
            return;
        }
        if self.protected_edit(self.cursor - 1, self.cursor) {
            return;
        }
        self.record(EditKind::Delete);
        self.text.remove(self.cursor - 1);
        self.cursor -= 1;
        self.settle();
    }

    /// 删除光标到逻辑行尾（不删除换行）。
    pub fn delete_to_line_end(&mut self) {
        let end = line_end(&self.text, self.cursor);
        if end > self.cursor && !self.protected_edit(self.cursor, end) {
            self.mark_dirty();
            self.text.replace_range(self.cursor..end, "");
            self.settle();
        }
    }

    /// 用插入文本替换字节范围（选区删除/替换）；文本会先净化，作为独立撤销步。
    pub fn replace_range(&mut self, start: usize, end: usize, insert: &str) -> bool {
        if start >= end || end > self.text.len() {
            return false;
        }
        if self.protected_edit(start, end) {
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

    /// 测试辅助：把光标移到字节偏移（不触发 settle 的面板语义）。
    #[cfg(test)]
    pub(crate) fn set_cursor_offset_for_test(&mut self, offset: usize) {
        self.cursor = offset.min(self.text.len());
    }

    /// 编辑是否触碰受保护的 `{wt:}` 绑定行；触碰时拒绝该次编辑。
    fn protected_edit(&self, from: usize, to: usize) -> bool {
        markdown::edit_touches_worktree_line(&self.text, from, to)
            || markdown::edit_touches_title_line(&self.text, from, to)
    }

    /// 在光标处插入已净化的文本并推进光标；触碰受保护绑定行时忽略。
    fn insert_at(&mut self, insert: &str) {
        if self.protected_edit(self.cursor, self.cursor) {
            return;
        }
        self.record(EditKind::Insert);
        self.text.insert_str(self.cursor, insert);
        self.cursor += insert.len();
        self.settle();
    }

    /// 记录一次编辑前的快照；连续 Insert/Delete 合并为一步，Other 独立成步。
    fn record(&mut self, kind: EditKind) {
        self.mark_dirty();
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

    /// 标记文本已修改：任何写操作（含未记录快照的删除）都置脏。
    fn mark_dirty(&mut self) {
        self.is_saved = false;
        self.revision = self.revision.wrapping_add(1);
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

    /// 编辑或光标移动后的统一收尾：解除面板压制、保持光标可见并重算面板。
    ///
    /// 面板互斥收敛优先级：文件提及 > 字母快捷 > 斜杠命令 > 块命令；
    /// 高优先级面板打开时清空其余（粘贴二级面板不由编辑重算，单独由 `clear_panel` 取消）。
    fn settle(&mut self) {
        self.panel_suppressed = false;
        self.scroll_cursor_into_view();
        self.mention.refresh(&self.text, self.cursor);
        if self.mention.panel().is_some() {
            self.slash = None;
            self.panel = None;
            self.refer = None;
            return;
        }
        self.refresh_refer_panel();
        if self.refer.is_some() {
            self.slash = None;
            self.panel = None;
            return;
        }
        self.refresh_slash_panel();
        if self.slash.is_some() {
            self.panel = None;
            return;
        }
        self.refresh_panel();
    }
}

#[cfg(test)]
mod tests;
