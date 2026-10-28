//! lx 输入框的辅助面板操作：markdown 块命令面板、`@` 提及面板与路径粘贴参照面板。

use crate::app::markdown::{
    BlockPanel, MentionEntry, block_commands, block_insertion, block_trigger,
};
use crate::app::paste::{PasteAction, PasteCandidate, PastePanel};

use super::{Lx, LxInputMode, LxMention, LxMentionView, LxPaste};

impl Lx {
    /// 重算块命令面板与 `@` 提及面板：二者互斥，提及命中时压制块面板。
    ///
    /// 编辑或移动光标后调用；同时清除输入框文本选区、解除提及面板的压制，按当前文本与根重新评估。
    pub fn refresh_panel(&mut self) {
        self.text_selection = None;
        // Shell 正文按原始脚本编辑：不弹 Markdown / 提及 / 斜杠 / 路径参照面板。
        if self.input_mode == LxInputMode::Shell {
            self.panel = None;
            self.slash = None;
            self.paste = None;
            self.mention.suppress();
            self.scroll_cursor_into_view();
            return;
        }
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
        entries: Vec<MentionEntry>,
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
        if candidates.is_empty() || self.input_mode == LxInputMode::Shell {
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
