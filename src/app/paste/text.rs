//! 长文本粘贴二级面板状态机与候选检测。

use super::PasteAnchor;
use super::folded::MAX_FOLDED_BYTES;

/// 触发长文本粘贴面板的最小逻辑行数（`>= 3`）。
pub const FOLD_LINE_THRESHOLD: usize = 3;

/// 触发长文本粘贴面板的字符数阈值（`> 150` Unicode 字符）。
pub const FOLD_CHAR_THRESHOLD: usize = 150;

/// 长文本粘贴面板操作；固定顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPasteAction {
    Collapse,
    Full,
}

impl TextPasteAction {
    /// 面板固定顺序。
    pub const ALL: [Self; 2] = [Self::Collapse, Self::Full];
}

/// 经过净化与阈值判定后的长文本粘贴候选。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPasteCandidate {
    /// 用于折叠编码的 `trim()` 后正文（已 sanitize）。
    pub content: String,
    /// 用于 `Paste full text` 的完整正文（已 sanitize，保留首尾空白）。
    pub full_text: String,
    /// `content` 的逻辑行数。
    pub line_count: usize,
    /// `content` 是否超过 `MAX_FOLDED_BYTES`（256 KiB）。
    pub too_large: bool,
}

/// 检测一段粘贴文本是否满足长文本面板触发阈值（`>= 3` 行或 `> 150` Unicode 字符）。
pub fn text_paste_candidate(raw: &str) -> Option<TextPasteCandidate> {
    let full_text = crate::app::prompt::text::sanitize(raw);
    let trimmed = full_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let line_count = trimmed.split('\n').count();
    let char_count = trimmed.chars().count();
    if line_count < FOLD_LINE_THRESHOLD && char_count <= FOLD_CHAR_THRESHOLD {
        return None;
    }
    let too_large = trimmed.len() > MAX_FOLDED_BYTES;
    Some(TextPasteCandidate {
        content: trimmed.to_string(),
        full_text,
        line_count,
        too_large,
    })
}

/// 长文本粘贴二级面板。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPastePanel {
    /// 用于折叠的 `trim()` 后正文。
    pub content: String,
    /// 用于原文插入的归一化完整文本。
    pub full_text: String,
    /// 正文逻辑行数。
    pub line_count: usize,
    /// 提案锚点（请求时刻的选区或光标）。
    pub anchor: PasteAnchor,
    /// 是否超过单块折叠上限（256 KiB）。
    pub too_large: bool,
    active: usize,
}

impl TextPastePanel {
    /// 组装长文本粘贴面板并把高亮落在首个可用项。
    pub fn new(content: String, full_text: String, anchor: PasteAnchor) -> Self {
        let line_count = content.split('\n').count().max(1);
        let too_large = content.len() > MAX_FOLDED_BYTES;
        let mut panel = Self {
            content,
            full_text,
            line_count,
            anchor,
            too_large,
            active: 0,
        };
        panel.active = panel.first_enabled();
        panel
    }

    /// 折叠操作是否可用（正文非空且未超 256 KiB）。
    pub fn collapse_enabled(&self) -> bool {
        !self.content.is_empty() && !self.too_large
    }

    /// 指定操作是否可用。
    pub fn enabled(&self, action: TextPasteAction) -> bool {
        match action {
            TextPasteAction::Collapse => self.collapse_enabled(),
            TextPasteAction::Full => !self.full_text.is_empty(),
        }
    }

    /// 供 `CommandPanelView.disabled` 使用的禁用掩码。
    pub fn disabled_mask(&self) -> [bool; 2] {
        [
            !self.enabled(TextPasteAction::Collapse),
            !self.enabled(TextPasteAction::Full),
        ]
    }

    /// 当前高亮索引。
    pub fn active(&self) -> usize {
        self.active
    }

    /// 当前高亮操作（仅返回可用项）。
    pub fn active_action(&self) -> Option<TextPasteAction> {
        TextPasteAction::ALL
            .get(self.active)
            .copied()
            .filter(|&action| self.enabled(action))
    }

    /// 鼠标悬停：仅接受可用项。
    pub fn set_active(&mut self, index: usize) -> bool {
        let Some(action) = TextPasteAction::ALL.get(index).copied() else {
            return false;
        };
        if !self.enabled(action) || self.active == index {
            return false;
        }
        self.active = index;
        true
    }

    /// 键盘上下移动高亮：跳过禁用项；无可移动项返回 `false`。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = TextPasteAction::ALL.len() as isize;
        let mut index = self.active as isize;
        for _ in 0..len {
            index = (index + delta.signum()).rem_euclid(len);
            if self.enabled(TextPasteAction::ALL[index as usize]) {
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
        TextPasteAction::ALL
            .iter()
            .position(|action| self.enabled(*action))
            .unwrap_or(0)
    }
}

/// 长文本粘贴状态机（不预先改写缓冲区，仅维护选择面板）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TextPasteState {
    panel: Option<TextPastePanel>,
}

impl TextPasteState {
    /// 打开面板。
    pub fn open(&mut self, panel: TextPastePanel) {
        self.panel = Some(panel);
    }

    /// 当前面板。
    pub fn panel(&self) -> Option<&TextPastePanel> {
        self.panel.as_ref()
    }

    /// 当前面板（可变）。
    pub fn panel_mut(&mut self) -> Option<&mut TextPastePanel> {
        self.panel.as_mut()
    }

    /// 取走当前面板（确认时使用）。
    pub fn take(&mut self) -> Option<TextPastePanel> {
        self.panel.take()
    }

    /// 关闭面板；原先打开时返回 `true`。
    pub fn cancel(&mut self) -> bool {
        self.panel.take().is_some()
    }
}

#[cfg(test)]
mod tests;
