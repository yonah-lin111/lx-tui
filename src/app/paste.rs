//! 粘贴提案的共享纯类型：候选识别、引用/路径动作与选择面板。
//!
//! prompt 编辑器与 lx 输入框共用同一套“路径粘贴 → 占位 → 参照面板”语义；
//! 这里只放纯数据与面板导航，具体文本改写由各编辑器各自实现。

use std::ops::Range;
use std::path::PathBuf;

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

    /// 引用格式文本；路径不可无损表达时返回 None。
    pub fn reference(&self) -> Option<String> {
        crate::app::markdown::format_reference(refer_kind(self.kind), &self.path)
    }
}

/// 候选种类对应的引用类型。
pub(crate) fn refer_kind(kind: PasteCandidateKind) -> ReferKind {
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

/// 粘贴二级面板：候选、占位插入范围与引用替换文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastePanel {
    pub request: u64,
    pub candidates: Vec<PasteCandidate>,
    /// 占位插入（纯路径文本）的字节范围。
    pub(crate) inserted: Range<usize>,
    /// 引用格式的完整替换文本；全部候选可无损表达时为 Some。
    pub(crate) reference: Option<String>,
    pub(crate) active: usize,
}

impl PastePanel {
    /// 组装面板并把高亮落在首个可用项。
    pub(crate) fn new(
        request: u64,
        candidates: Vec<PasteCandidate>,
        inserted: Range<usize>,
        reference: Option<String>,
    ) -> Self {
        let mut panel = Self {
            request,
            candidates,
            inserted,
            reference,
            active: 0,
        };
        panel.active = panel.first_enabled();
        panel
    }

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
