//! 补充块附件会话：owner/target、草稿状态机与局部提交数据。
//!
//! 原文仍是唯一事实来源；附件列表、草稿与模态都只在内存中。保存只回写目标子块的
//! 标题值与正文，起止标记、命令名、`{id:}` / `{wt:}` 与未编辑字段逐字保留。

use std::ops::Range;
use std::path::PathBuf;

use super::markdown;
use super::overlay::TextInput;
use super::prompt::block::SupplementCommit;
use super::prompt::{Prompt, PromptEditing, PromptPageId, PromptPresentation, SupplementEntry};
use crate::layout::PaneId;

/// 附件会话的归属：打开时冻结，保存时逐项复核。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplementOwner {
    /// 显示中 prompt 编辑器标识。
    pub pane: PaneId,
    /// 显示中 prompt 所属工作区的持久化键（钉住时取被钉住工作区）。
    pub workspace: Option<PathBuf>,
    /// 编辑目标（正式条目或临时提示词）。
    pub target: Option<PromptEditing>,
    /// 目标页 id。
    pub page: Option<PromptPageId>,
    /// 打开时的显式上下文根。
    pub root: Option<PathBuf>,
}

/// 一个可编辑附件的目标快照：身份 + 原文区间 + 字段划分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplementTarget {
    /// 父任务起始逻辑行。
    pub parent_start: usize,
    /// 子块起始逻辑行。
    pub start: usize,
    /// 子块结束逻辑行。
    pub end: usize,
    /// 含起止标记的整块字节区间。
    pub range: Range<usize>,
    /// 打开时的整块原文，用于保存前逐字比对。
    pub snapshot: String,
    /// 独立标题值行（`{title: …}`）的字节区间；无标题行时为 None。
    pub title: Option<Range<usize>>,
    /// 正文核心区间（已剔除首尾空行）；无正文时为两端相等的空区间。
    pub body: Range<usize>,
    /// 列表展示标题。
    pub label: String,
}

impl SupplementTarget {
    /// 从当前文本与附件条目解析可编辑字段；结构异常返回 None。
    pub fn from_entry(text: &str, entry: &SupplementEntry) -> Option<Self> {
        let lines: Vec<&str> = text.split('\n').collect();
        let offsets = super::prompt::line_offsets(text);
        let start_offset = *offsets.get(entry.start)?;
        let end_offset = *offsets.get(entry.end)?;
        let end_line_len = lines.get(entry.end)?.len();
        let range = start_offset..end_offset + end_line_len;
        if text.get(range.clone())? != &text[entry.range.clone()] {
            return None;
        }
        let title_line = lines
            .get(entry.start + 1)
            .filter(|line| markdown::is_title_line(line))
            .map(|_| entry.start + 1);
        let slot = binding_line(&lines, entry)?;
        // 正文行：标题行（或起始行）之后到绑定行/结束行之前。
        let body_first = title_line.map_or(entry.start + 1, |line| line + 1);
        let (body, first, last) = body_core(text, &lines, &offsets, body_first, slot)?;
        let title = title_line.and_then(|line| title_value_range(text, &lines, &offsets, line));
        Some(Self {
            parent_start: entry.parent_start,
            start: entry.start,
            end: entry.end,
            range,
            snapshot: text.get(entry.range.clone())?.to_string(),
            title,
            body,
            label: entry.title.clone(),
            // 记录正文核心两端行，供保存时只替换该段。
        })
        .filter(|target| target.body.start <= target.body.end && first <= last)
    }
}

/// 子块底部绑定行（结束行上一行且为 `{wt:}`）；无绑定返回结束行。
fn binding_line(lines: &[&str], entry: &SupplementEntry) -> Option<usize> {
    let slot = entry.end.checked_sub(1)?;
    if slot <= entry.start {
        return Some(entry.end);
    }
    if lines
        .get(slot)
        .and_then(|line| markdown::parse_worktree_line(line))
        .is_some()
    {
        Some(slot)
    } else {
        Some(entry.end)
    }
}

/// 正文核心字节区间：去掉首尾纯空行，保留中间内容原样。
fn body_core(
    text: &str,
    lines: &[&str],
    offsets: &[usize],
    first: usize,
    slot: usize,
) -> Option<(Range<usize>, usize, usize)> {
    if first > slot || slot > lines.len() {
        return None;
    }
    let mut from = first;
    let mut to = slot;
    while from < to && lines.get(from).is_some_and(|line| line.trim().is_empty()) {
        from += 1;
    }
    while to > from && lines.get(to - 1).is_some_and(|line| line.trim().is_empty()) {
        to -= 1;
    }
    let body = if from == to {
        let at = *offsets.get(from).or_else(|| offsets.last())?;
        at..at
    } else {
        let start = *offsets.get(from)?;
        let end = offsets.get(to - 1).copied()? + lines.get(to - 1)?.len();
        start..end
    };
    let _ = text;
    Some((body, from, to))
}

/// 独立标题行的值区间：`{title: <value>}` 中 `<value>` 的字节范围。
fn title_value_range(
    _text: &str,
    lines: &[&str],
    offsets: &[usize],
    line: usize,
) -> Option<Range<usize>> {
    let raw = lines.get(line)?;
    let leading = raw.len() - raw.trim_start().len();
    let trimmed = raw.trim();
    let value = trimmed.strip_prefix("{title:")?.strip_suffix('}')?;
    let value_start = *offsets.get(line)? + leading + "{title:".len();
    // 去掉值两侧空白：只编辑值本身，括号与空白原样保留。
    let left = value.len() - value.trim_start().len();
    let right = value.trim_end().len();
    let from = value_start + left;
    let to = value_start + right.max(left);
    Some(from..to)
}

/// 未保存确认的按钮数与默认焦点（save / discard / keep editing）。
pub const UNSAVED_BUTTONS: usize = 3;
pub const UNSAVED_SAVE_INDEX: usize = 0;
pub const UNSAVED_DISCARD_INDEX: usize = 1;
pub const UNSAVED_KEEP_INDEX: usize = 2;
/// 删除确认的按钮数与默认焦点（confirm / cancel）。
pub const CONFIRM_BUTTONS: usize = 2;
pub const CONFIRM_DELETE_INDEX: usize = 0;
pub const CONFIRM_CANCEL_INDEX: usize = 1;

/// 模态视图状态：同一个模态内切换，不叠加第二个浮层。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplementView {
    List,
    Edit,
    /// 有未保存修改时的三选一。
    Unsaved,
    /// 删除确认（仅列表态进入）。
    ConfirmDelete,
}

/// 本地草稿：标题输入与正文 scratch 编辑器。
#[derive(Debug)]
pub struct SupplementDraft {
    /// 新增草稿：保存时才插入父 Prompt。
    pub new: bool,
    /// 列表来源新块的插入位置（父任务结束行）。
    pub insert_before: usize,
    /// 斜杠来源新块：冻结的触发行字节区间。
    pub trigger: Option<Range<usize>>,
    /// 新块使用的命令名（内置 `supplement` 或自定义命令名）。
    pub command: String,
    /// 标题输入。
    pub title: TextInput,
    /// 正文 scratch 编辑器（固定原文展示，不压缩附件）。
    pub body: Prompt,
    /// 打开时的标题与正文基线，用于 dirty 判定。
    pub baseline_title: String,
    pub baseline_body: String,
    /// 目标快照；新建时为 None。
    pub target: Option<SupplementTarget>,
    /// 焦点在标题行（否则在正文）。F6/Tab 在两者间切换。
    pub title_focused: bool,
}

impl SupplementDraft {
    /// 从已有附件建立草稿。
    pub fn existing(text: &str, target: SupplementTarget) -> Self {
        let title = target
            .title
            .as_ref()
            .and_then(|range| text.get(range.clone()))
            .unwrap_or_default()
            .to_string();
        let body_text = text
            .get(target.body.clone())
            .unwrap_or_default()
            .to_string();
        Self::new_draft(title, &body_text, Some(target))
    }

    /// 新建草稿：标题为空、正文为空，保存时插入完整补充块。
    pub fn blank(title: &str, body: &str, insert_before: usize) -> Self {
        let mut draft = Self::new_draft(title.to_string(), body, None);
        draft.new = true;
        draft.insert_before = insert_before;
        draft.command = crate::ui::text::SUPPLEMENTS_DEFAULT_COMMAND.to_string();
        draft
    }

    fn new_draft(title: String, body: &str, target: Option<SupplementTarget>) -> Self {
        let mut editor = Prompt::new(PaneId::alloc());
        editor.set_presentation(PromptPresentation::Source);
        editor.set_text(body);
        editor.set_cursor(0);
        // 基线取自净化后的编辑器内容：否则含制表符/控制字符的正文一打开就判脏。
        let baseline_body = editor.text().to_string();
        Self {
            new: false,
            insert_before: 0,
            trigger: None,
            command: String::new(),
            baseline_title: title.clone(),
            baseline_body,
            title: TextInput::new(title),
            body: editor,
            target,
            title_focused: true,
        }
    }

    /// 草稿是否有未保存修改（按基线比较；本地撤销回到原内容即 clean）。
    pub fn dirty(&self) -> bool {
        self.title.text() != self.baseline_title || self.body.text() != self.baseline_body
    }
}

/// 附件模态：列表/编辑/未保存确认/删除确认共享同一份会话。
#[derive(Debug)]
pub struct Supplements {
    pub owner: SupplementOwner,
    /// 打开时的父编辑器修订号。
    pub revision: u64,
    /// 父任务起始逻辑行（入口归属）。
    pub parent_start: usize,
    /// 打开时的父编辑器全文快照（草稿正文与列表复制取自此快照）。
    pub text_snapshot: String,
    /// 打开时冻结的附件列表（按原文顺序）。
    pub entries: Vec<SupplementTarget>,
    pub selected: usize,
    /// 滚轮滚动后的显式窗口起点；键盘移动后清空，窗口跟随选中。
    pub viewport: Option<usize>,
    /// 指针悬停行。
    pub hover: Option<usize>,
    pub view: SupplementView,
    pub draft: Option<SupplementDraft>,
    /// 模态内提示（目标失效、保存失败等）。
    pub hint: Option<&'static str>,
    /// 列表可见行数（事件层按模态几何刷新；键盘/滚轮分页共用）。
    pub list_rows: usize,
    /// 当前终端是否足以编辑（外框小于最小尺寸时为 false）：
    /// 此时只提示放大，忽略写操作，草稿原样保留。
    pub editable: bool,
    /// 确认态按钮焦点下标：未保存确认默认 `keep editing`，删除确认默认 `cancel`。
    pub button_focus: usize,
}

impl Supplements {
    /// 列表行数：附件项 + 末尾的 `+ add supplement` 动作行。
    pub fn row_count(&self) -> usize {
        self.entries.len() + 1
    }

    /// 当前是否停在新增动作行。
    pub fn on_add_row(&self) -> bool {
        self.selected >= self.entries.len()
    }

    /// 新会话：冻结列表与归属，进入列表态。
    pub fn new(
        owner: SupplementOwner,
        revision: u64,
        parent_start: usize,
        text_snapshot: String,
        entries: Vec<SupplementTarget>,
    ) -> Self {
        Self {
            owner,
            revision,
            parent_start,
            text_snapshot,
            entries,
            selected: 0,
            viewport: None,
            hover: None,
            view: SupplementView::List,
            draft: None,
            hint: None,
            list_rows: 20,
            editable: true,
            button_focus: 0,
        }
    }

    /// 当前选中附件；停在新增动作行或列表为空时返回 None。
    pub fn selected_entry(&self) -> Option<&SupplementTarget> {
        self.entries.get(self.selected)
    }

    /// 列表可见窗口起点（窗口固定高度由渲染给定）。
    pub fn window_start(&self, visible: usize) -> usize {
        let visible = visible.max(1);
        let max = self.row_count().saturating_sub(visible);
        match self.viewport {
            Some(start) => start.min(max),
            None => self
                .selected
                .saturating_sub(visible.saturating_sub(1))
                .min(max),
        }
    }

    /// 键盘选择移动（含末尾新增动作行）：到端点钳制；返回是否产生变化。
    pub fn move_selection(&mut self, delta: isize) -> bool {
        let next = (self.selected as isize + delta)
            .clamp(0, self.row_count().saturating_sub(1) as isize) as usize;
        let changed = next != self.selected;
        self.selected = next;
        self.viewport = None;
        changed
    }

    /// 选中首行/末行（末行为新增动作行）；返回是否产生变化。
    pub fn select_edge(&mut self, last: bool) -> bool {
        let next = if last { self.row_count() - 1 } else { 0 };
        let changed = next != self.selected;
        self.selected = next;
        self.viewport = None;
        changed
    }

    /// 滚轮滚动列表视口（不改变选中）；返回是否移动。
    pub fn scroll_viewport(&mut self, delta: isize, visible: usize) -> bool {
        let visible = visible.max(1);
        if self.row_count() <= visible {
            return false;
        }
        let max = self.row_count() - visible;
        let current = self.window_start(visible);
        let next = (current as isize + delta).clamp(0, max as isize) as usize;
        let moved = next != current;
        if moved {
            self.viewport = Some(next);
        }
        moved
    }

    /// 进入未保存确认：默认焦点停在 `keep editing`（最安全项）。
    pub fn enter_unsaved(&mut self) {
        self.view = SupplementView::Unsaved;
        self.button_focus = UNSAVED_KEEP_INDEX;
    }

    /// 进入删除确认：默认焦点停在 `cancel`。
    pub fn enter_confirm_delete(&mut self) {
        self.view = SupplementView::ConfirmDelete;
        self.button_focus = CONFIRM_CANCEL_INDEX;
    }

    /// 确认态按钮数量。
    pub fn button_count(&self) -> usize {
        match self.view {
            SupplementView::Unsaved => UNSAVED_BUTTONS,
            SupplementView::ConfirmDelete => CONFIRM_BUTTONS,
            _ => 0,
        }
    }

    /// 确认态横向移动按钮焦点；越界钳制。返回是否变化。
    pub fn move_button_focus(&mut self, delta: isize) -> bool {
        let count = self.button_count();
        if count == 0 {
            return false;
        }
        let next = (self.button_focus as isize + delta).clamp(0, count as isize - 1) as usize;
        let changed = next != self.button_focus;
        self.button_focus = next;
        changed
    }

    /// 删除后按新列表钳制选中：删末项回退到剩下的最后一项，删空后停在新增动作行。
    ///
    /// 与 [`Self::move_selection`] 不同——后者的端点包含动作行，是用户显式导航的目标。
    pub fn clamp_after_removal(&mut self) {
        self.selected = match self.entries.len() {
            0 => 0,
            len => self.selected.min(len - 1),
        };
        self.viewport = None;
    }
}

/// 保存结果：用于区分未变化、结构拒绝与目标失效。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// 已写入父 Prompt，形成一条撤销记录。
    Changed,
    /// 与原文一致，未产生变更。
    Unchanged,
    /// 结构保护或区间不合法。
    Rejected,
    /// 目标文档身份、修订或原文快照已变化。
    TargetChanged,
}

/// 把草稿正文与标题写回父编辑器。
///
/// 以打开时的整块快照为基准重建块文本：起止标记、命令名、`{id:}` / `{wt:}`
/// 与未编辑的空白逐字保留，只替换标题值与正文。整块作为一次 `Other` 提交。
pub fn apply_draft(prompt: &mut Prompt, draft: &SupplementDraft) -> SaveOutcome {
    let Some(target) = draft.target.as_ref() else {
        return SaveOutcome::Rejected;
    };
    let current = prompt.text();
    let Some(snapshot) = current.get(target.range.clone()) else {
        return SaveOutcome::TargetChanged;
    };
    if snapshot != target.snapshot {
        return SaveOutcome::TargetChanged;
    }
    let Some(replacement) = rebuild_block(target, draft) else {
        return SaveOutcome::Rejected;
    };
    if replacement == snapshot {
        return SaveOutcome::Unchanged;
    }
    match prompt.commit_supplement(target.range.start, target.range.end, &replacement) {
        SupplementCommit::Changed => SaveOutcome::Changed,
        SupplementCommit::Unchanged => SaveOutcome::Unchanged,
        SupplementCommit::Rejected => SaveOutcome::Rejected,
    }
}

/// 以块快照重建替换文本；字段区间相对块起点，从后往前替换避免偏移互相影响。
fn rebuild_block(target: &SupplementTarget, draft: &SupplementDraft) -> Option<String> {
    let base = target.range.start;
    let relative = |range: &Range<usize>| {
        range
            .start
            .checked_sub(base)
            .zip(range.end.checked_sub(base))
            .filter(|(from, to)| from <= to && *to <= target.snapshot.len())
    };
    let mut block = target.snapshot.clone();
    // 正文在后：先替换正文。
    match relative(&target.body) {
        Some((from, to)) => {
            let body = draft.body.text();
            if from == to && !body.is_empty() {
                // 空正文插入：保持与结束标记之间的结构留白。
                block.replace_range(from..to, &format!("{body}\n\n"));
            } else {
                block.replace_range(from..to, body);
            }
        }
        None => return None,
    }
    match target.title.as_ref() {
        Some(range) => {
            let (from, to) = relative(range)?;
            block.replace_range(from..to, draft.title.text());
        }
        // 原块无独立标题行：只有非空标题才补一行，插在起始行之后。
        None if !draft.title.text().is_empty() => {
            let newline = block.find('\n')?;
            let insert = format!("\n{{title: {}}}", draft.title.text());
            block.insert_str(newline, &insert);
        }
        None => {}
    }
    Some(block)
}

/// 新建草稿要插入的完整补充块文本：沿用生成器外壳，标题与正文来自草稿。
pub fn new_block_text(draft: &SupplementDraft, command: &str) -> String {
    let (content, _) =
        markdown::supple_block_content(command, Some(draft.title.text()), draft.body.text());
    content
}

#[cfg(test)]
mod tests;
