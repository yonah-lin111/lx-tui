//! prompt 粘贴提案：候选识别、占位插入与引用/路径二级面板的纯状态转换。
//!
//! 路径粘贴先写入纯路径占位文本，再由面板把占位文本改写成引用格式或保持原样；
//! 取消（Esc/外部点击）回滚到粘贴前，不留下多余撤销步。位图不进面板，直接以
//! `@[refer-image](path)` 引用插入。

use std::ops::Range;
use std::path::{Path, PathBuf};

use super::{EditKind, Prompt};
use crate::app::markdown::ReferKind;

/// 粘贴候选种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteCandidateKind {
    File,
    Folder,
    Image,
}

/// 粘贴候选：路径、类型与引用可表达性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteCandidate {
    pub path: PathBuf,
    pub kind: PasteCandidateKind,
    /// 引用语法是否可无损表达该路径。
    pub reference_ok: bool,
}

impl PasteCandidate {
    /// 外部文件系统候选。
    pub fn external(path: PathBuf, kind: PasteCandidateKind) -> Self {
        let reference_ok =
            crate::app::markdown::format_reference(refer_kind(kind), &path).is_some();
        Self {
            path,
            kind,
            reference_ok,
        }
    }

    /// 本应用管理的截图候选（已登记资产，引用即插入其路径）。
    pub fn staged_image(path: PathBuf) -> Self {
        let reference_ok =
            crate::app::markdown::format_reference(ReferKind::Image, &path).is_some();
        Self {
            path,
            kind: PasteCandidateKind::Image,
            reference_ok,
        }
    }
}

/// 候选种类对应的引用类型。
fn refer_kind(kind: PasteCandidateKind) -> ReferKind {
    match kind {
        PasteCandidateKind::File => ReferKind::File,
        PasteCandidateKind::Folder => ReferKind::Folder,
        PasteCandidateKind::Image => ReferKind::Image,
    }
}

/// 粘贴处理方式；面板两项固定顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteAction {
    Reference,
    Path,
}

impl PasteAction {
    /// 面板固定顺序。
    pub const ALL: [Self; 2] = [Self::Reference, Self::Path];
}

/// 提案锚点：请求时刻的选区（优先）与光标字节偏移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteAnchor {
    pub selection: Option<(usize, usize)>,
    pub cursor: usize,
}

impl PasteAnchor {
    /// 当前光标锚点。
    pub fn at_cursor(cursor: usize) -> Self {
        Self {
            selection: None,
            cursor,
        }
    }

    /// 替换范围：有选区取选区，否则空区间落在光标处。
    pub fn range(&self) -> (usize, usize) {
        self.selection.unwrap_or((self.cursor, self.cursor))
    }
}

/// 占位插入现场；取消时据此回滚（含撤销步与保存态）。
#[derive(Debug, Clone, Copy)]
struct Staged {
    was_saved: bool,
    revision: u64,
}

/// 粘贴二级面板：候选、占位插入范围与引用替换文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastePanel {
    pub request: u64,
    pub candidates: Vec<PasteCandidate>,
    /// 占位插入（纯路径文本）的字节范围。
    inserted: Range<usize>,
    /// 引用格式的完整替换文本；全部候选可无损表达时为 Some。
    reference: Option<String>,
    active: usize,
}

impl PastePanel {
    /// 引用是否可用：全部候选路径可无损表达。
    pub fn reference_enabled(&self) -> bool {
        !self.candidates.is_empty() && self.reference.is_some()
    }

    /// 操作是否可用；路径模式只要候选非空。
    pub fn enabled(&self, action: PasteAction) -> bool {
        match action {
            PasteAction::Reference => self.reference_enabled(),
            PasteAction::Path => !self.candidates.is_empty(),
        }
    }

    /// 高亮索引。
    pub fn active(&self) -> usize {
        self.active
    }

    /// 高亮操作。
    pub fn active_action(&self) -> Option<PasteAction> {
        PasteAction::ALL.get(self.active).copied()
    }

    /// 鼠标悬停：仅接受可用项。
    pub fn set_active(&mut self, index: usize) -> bool {
        let Some(action) = PasteAction::ALL.get(index).copied() else {
            return false;
        };
        if !self.enabled(action) || self.active == index {
            return false;
        }
        self.active = index;
        true
    }

    /// 键盘移动高亮：跳过禁用项；无可移动项返回 false。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = PasteAction::ALL.len() as isize;
        let mut index = self.active as isize;
        for _ in 0..len {
            index = (index + delta.signum()).rem_euclid(len);
            if self.enabled(PasteAction::ALL[index as usize]) {
                let next = index as usize;
                if next == self.active {
                    return false;
                }
                self.active = next;
                return true;
            }
        }
        false
    }

    /// 首个可用项索引；全禁用时为 0。
    fn first_enabled(&self) -> usize {
        PasteAction::ALL
            .iter()
            .position(|action| self.enabled(*action))
            .unwrap_or(0)
    }
}

/// 在途粘贴读取请求：请求代号、发起时修订号与锚点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingPaste {
    request: u64,
    revision: u64,
    anchor: PasteAnchor,
}

/// 粘贴状态机：在途请求、选择面板与占位插入现场。
#[derive(Debug, Default)]
pub struct PasteState {
    pending: Option<PendingPaste>,
    panel: Option<PastePanel>,
    staged: Option<Staged>,
    next_request: u64,
}

impl PasteState {
    /// 发起读取请求：取消旧面板/在途任务并登记新请求代号。
    pub fn begin(&mut self, revision: u64, anchor: PasteAnchor) -> u64 {
        self.panel = None;
        let request = self.next_request.wrapping_add(1);
        self.next_request = request;
        self.pending = Some(PendingPaste {
            request,
            revision,
            anchor,
        });
        request
    }

    /// 请求是否仍是在途目标（用于结果登记前的快速判定）。
    pub fn request_matches(&self, request: u64) -> bool {
        self.pending
            .is_some_and(|pending| pending.request == request)
    }

    /// 收束不进入面板的结果：请求在途且修订未变时清除在途状态并返回登记锚点。
    pub fn settle(&mut self, request: u64, revision: u64) -> Option<PasteAnchor> {
        let pending = self.pending?;
        if pending.request != request || pending.revision != revision {
            return None;
        }
        self.pending = None;
        Some(pending.anchor)
    }

    /// 记录占位插入现场。
    fn set_staged(&mut self, staged: Staged) {
        self.staged = Some(staged);
    }

    /// 取走占位插入现场。
    fn take_staged(&mut self) -> Option<Staged> {
        self.staged.take()
    }

    /// 丢弃占位插入现场（占位文本已被改写或整篇替换）。
    pub fn clear_staged(&mut self) {
        self.staged = None;
    }

    /// 写入面板（占位插入已完成后调用）。
    pub fn open(&mut self, panel: PastePanel) {
        self.pending = None;
        self.panel = Some(panel);
    }

    /// 当前面板。
    pub fn panel(&self) -> Option<&PastePanel> {
        self.panel.as_ref()
    }

    /// 当前面板（可变）。
    pub fn panel_mut(&mut self) -> Option<&mut PastePanel> {
        self.panel.as_mut()
    }

    /// 关闭面板与在途请求（整篇替换时使用；占位文本随整篇一起被替换）。
    pub fn close(&mut self) -> bool {
        let had = self.pending.is_some() || self.panel.is_some();
        self.pending = None;
        self.panel = None;
        self.staged = None;
        had
    }

    /// 关闭面板并作废在途请求；占位现场保留给调用方回滚。
    pub fn cancel(&mut self) -> bool {
        let had = self.pending.is_some() || self.panel.is_some();
        self.pending = None;
        self.panel = None;
        had
    }
}

impl Prompt {
    /// 当前粘贴面板。
    pub fn paste_panel(&self) -> Option<&PastePanel> {
        self.paste.panel()
    }

    /// 发起粘贴读取请求；返回请求代号。上一次占位插入先回滚。
    pub fn begin_paste_read(&mut self, anchor: PasteAnchor) -> u64 {
        self.paste_cancel();
        self.paste.begin(self.revision, anchor)
    }

    /// 结果仍在途且目标是当前编辑器时返回 true；登记 managed 资源前校验。
    pub fn paste_request_matches(&self, request: u64) -> bool {
        self.paste.request_matches(request)
    }

    /// 收束不进入面板的读取结果（直接插入的截图）：请求在途且修订未变时返回锚点。
    pub fn paste_settle(&mut self, request: u64) -> Option<PasteAnchor> {
        self.paste.settle(request, self.revision)
    }

    /// 丢弃在途请求（不触碰正文；普通文本回退用）；返回是否有在途状态。
    pub fn paste_discard(&mut self) -> bool {
        self.paste.close()
    }

    /// 写入读取结果：请求与修订匹配时按登记的锚点先占位插入纯路径文本再打开面板。
    ///
    /// 占位插入与后续格式改写同属一步撤销；触碰受保护行时不写入也不打开面板。
    pub fn paste_stage(&mut self, request: u64, candidates: Vec<PasteCandidate>) -> bool {
        if candidates.is_empty() {
            return false;
        }
        let Some(anchor) = self.paste.settle(request, self.revision) else {
            return false;
        };
        let (start, end) = anchor.range();
        let end = end.min(self.text.len());
        if start > end
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
            || self.protected_edit(start, end)
        {
            return false;
        }
        let plain: Vec<String> = candidates
            .iter()
            .map(|candidate| candidate.path.to_string_lossy().into_owned())
            .collect();
        let references: Option<Vec<String>> = candidates
            .iter()
            .map(|candidate| {
                crate::app::markdown::format_reference(refer_kind(candidate.kind), &candidate.path)
            })
            .collect();
        let insertion = self.paste_insertion(start, &plain);
        let was_saved = self.is_saved;
        let revision = self.revision;
        if !self.replace_at(start, end, &insertion) {
            return false;
        }
        let reference = references
            .map(|references| self.paste_insertion(start, &references))
            .filter(|reference| !reference.is_empty());
        self.paste.set_staged(Staged {
            was_saved,
            revision,
        });
        let mut panel = PastePanel {
            request,
            candidates,
            inserted: start..start + insertion.len(),
            reference,
            active: 0,
        };
        panel.active = panel.first_enabled();
        self.paste.open(panel);
        true
    }

    /// 在锚点直接插入引用（截图粘贴）；路径不可无损表达或触碰受保护行时返回 false。
    pub fn insert_reference(&mut self, anchor: &PasteAnchor, kind: ReferKind, path: &Path) -> bool {
        let Some(reference) = crate::app::markdown::format_reference(kind, path) else {
            return false;
        };
        let (start, end) = anchor.range();
        let end = end.min(self.text.len());
        if start > end || !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return false;
        }
        let insertion = self.paste_insertion(start, &[reference]);
        self.replace_at(start, end, &insertion)
    }

    /// 键盘移动粘贴面板高亮；返回是否消费。
    pub fn paste_move(&mut self, delta: isize) -> bool {
        self.paste
            .panel_mut()
            .is_some_and(|panel| panel.move_active(delta))
    }

    /// 鼠标悬停粘贴面板条目；返回是否变化。
    pub fn paste_set_active(&mut self, index: usize) -> bool {
        self.paste
            .panel_mut()
            .is_some_and(|panel| panel.set_active(index))
    }

    /// Esc 取消粘贴面板：回滚占位插入；返回是否消费。
    pub fn paste_escape(&mut self) -> bool {
        self.paste_cancel()
    }

    /// 确认粘贴面板操作：引用改写占位文本，路径保持占位文本。
    pub fn paste_confirm(&mut self, action: PasteAction) -> bool {
        let Some(panel) = self.paste.panel().cloned() else {
            return false;
        };
        if !panel.enabled(action) {
            return false;
        }
        if action == PasteAction::Reference {
            let Some(reference) = panel.reference else {
                return false;
            };
            if !self.rewrite_at(&panel.inserted, &reference) {
                return false;
            }
        }
        self.paste.cancel();
        self.paste.clear_staged();
        true
    }

    /// 取消粘贴面板：回滚占位插入并关闭；返回是否消费。
    pub fn paste_cancel(&mut self) -> bool {
        let staged = self.paste.take_staged();
        let had = self.paste.cancel();
        if let Some(staged) = staged {
            self.restore_staged(staged);
            return true;
        }
        had
    }

    /// 回滚占位插入：弹出该步快照、恢复保存态与修订号，编辑痕迹归零。
    fn restore_staged(&mut self, staged: Staged) {
        if let Some(snapshot) = self.undo.pop() {
            self.restore(snapshot);
        }
        self.is_saved = staged.was_saved;
        self.revision = staged.revision;
        self.autosave = None;
    }

    /// 构造粘贴插入文本：左邻非空白先补空格，末尾补空格。
    fn paste_insertion(&self, at: usize, parts: &[String]) -> String {
        let mut insert = parts.join(" ");
        let left_blank = self.text[..at]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if !left_blank {
            insert.insert(0, ' ');
        }
        insert.push(' ');
        insert
    }

    /// 占位插入/直接插入：一次可撤销编辑，并压制其他面板。
    fn replace_at(&mut self, start: usize, end: usize, insert: &str) -> bool {
        if start > end
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
            || self.protected_edit(start, end)
        {
            return false;
        }
        self.break_group();
        self.record(EditKind::Other);
        self.text.replace_range(start..end, insert);
        self.cursor = start + insert.len();
        self.clear_other_panels();
        self.scroll_cursor_into_view();
        true
    }

    /// 同一步内的格式改写（占位文本 → 引用文本）：不新增撤销步。
    fn rewrite_at(&mut self, range: &Range<usize>, insert: &str) -> bool {
        if range.end > self.text.len()
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
        {
            return false;
        }
        self.text.replace_range(range.clone(), insert);
        self.cursor = range.start + insert.len();
        self.clear_other_panels();
        self.scroll_cursor_into_view();
        true
    }

    /// 清空互斥的编辑器面板；粘贴面板自身不由这里管理。
    fn clear_other_panels(&mut self) {
        self.panel = None;
        self.slash = None;
        self.mention.clear();
        self.refer = None;
        self.panel_suppressed = true;
    }
}
