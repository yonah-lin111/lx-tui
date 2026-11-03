//! `Lx` 输入框的长文本折叠粘贴集成：面板状态、显示投影、展开文本解码与预览选区。

use crate::app::paste::{
    ChipHit, PasteAnchor, Projection, TextPasteAction, TextPastePanel, clamp_cursor,
    encode_pasted_text, expand_pasted_text, folded_tokens, mask_pasted_text, snap_pasted_range,
};
use crate::app::prompt::{list, text};
use crate::app::selection::SelectionUnit;

use super::{Lx, LxInputMode, LxTextPaste};

impl Lx {
    /// 当前长文本粘贴面板；未打开时为 `None`。
    pub fn text_paste_panel(&self) -> Option<&TextPastePanel> {
        self.text_paste.as_ref().map(|paste| &paste.panel)
    }

    /// 发起长文本粘贴提案；不修改缓冲区。
    ///
    /// 仅在非 plain 的 Chat 模式、且光标不在代码围栏内时生效。
    pub fn begin_text_paste(&mut self, content: String, full_text: String) -> bool {
        if self.plain || self.input_mode == LxInputMode::Shell {
            return false;
        }
        let anchor = PasteAnchor {
            selection: self.text_selection_range(),
            cursor: self.cursor,
        };
        let (start, _) = anchor.range();
        if list::in_fence(&mask_pasted_text(&self.input), start.min(self.input.len())) {
            return false;
        }
        self.paste_cancel();
        self.panel = None;
        self.mention.suppress();
        let panel = TextPastePanel::new(content, full_text, anchor);
        self.text_paste = Some(LxTextPaste { panel });
        true
    }

    /// 键盘移动长文本粘贴面板高亮；返回是否消费。
    pub fn text_paste_move(&mut self, delta: isize) -> bool {
        self.text_paste
            .as_mut()
            .is_some_and(|paste| paste.panel.move_active(delta))
    }

    /// 鼠标悬停长文本粘贴面板条目；返回高亮是否变化。
    pub fn text_paste_set_active(&mut self, index: usize) -> bool {
        self.text_paste
            .as_mut()
            .is_some_and(|paste| paste.panel.set_active(index))
    }

    /// 确认长文本粘贴面板：`Collapse` 插入折叠 token（补空格），`Full` 插入原文；一步撤销。
    pub fn text_paste_confirm(&mut self, action: TextPasteAction) -> bool {
        let Some(paste) = self.text_paste.as_ref() else {
            return false;
        };
        if !paste.panel.enabled(action) {
            return false;
        }
        let panel = paste.panel.clone();
        self.text_paste = None;
        let (start, end) = panel.anchor.range();
        let range = snap_pasted_range(&self.input, start..end.min(self.input.len()));
        let insertion = match action {
            TextPasteAction::Collapse => {
                let token = encode_pasted_text(&panel.content);
                self.paste_insertion_at(range.start, &[token])
            }
            TextPasteAction::Full => panel.full_text,
        };
        self.snapshot();
        self.input.replace_range(range.clone(), &insertion);
        self.cursor = range.start + insertion.len();
        self.adopt_shell_prefix();
        if self.input_mode == LxInputMode::Chat {
            self.panel = None;
            self.refer = None;
            self.refer_suppressed = true;
            self.mention.suppress();
        }
        self.scroll_cursor_into_view();
        true
    }

    /// 取消长文本粘贴面板（不修改缓冲区）；原先打开时返回 `true`。
    pub fn text_paste_cancel(&mut self) -> bool {
        self.text_paste.take().is_some()
    }

    /// 当前输入的折叠/展开显示投影。
    pub fn projection(&self) -> Projection<'_> {
        Projection::new(&self.input, &self.expanded_pastes)
    }

    /// 是否有任何已展开的折叠芯片。
    pub fn has_expanded_pastes(&self) -> bool {
        !self.expanded_pastes.is_empty()
    }

    /// 切换指定身份折叠芯片的展开状态；不写缓冲区、不记撤销。
    pub fn toggle_paste(&mut self, id: u64, occurrence: usize) -> bool {
        self.preview_selection = None;
        self.preview_drag = None;
        let expanded = self.expanded_pastes.toggle(id, occurrence);
        self.scroll = self.scroll.min(self.max_scroll());
        expanded
    }

    /// 清空全部折叠芯片的展开状态与预览选区；原先有展开项或预览选区时返回 `true`。
    pub fn clear_expanded_pastes(&mut self) -> bool {
        let had_sel = self.clear_preview_selection();
        let had_exp = self.expanded_pastes.clear();
        if had_exp {
            self.scroll = self.scroll.min(self.max_scroll());
        }
        had_sel || had_exp
    }

    /// 发送/排队/标题/历史使用的展开文本：将所有折叠 token 解码替换为原始正文。
    pub fn expanded_input(&self) -> String {
        expand_pasted_text(&self.input)
    }

    /// 展开区预览选区（已按起止排序的非空投影文本字节范围）。
    pub fn preview_selection(&self) -> Option<(usize, usize)> {
        self.preview_selection
            .and_then(|(a, b)| (a != b).then_some((a.min(b), a.max(b))))
    }

    /// 直接设置展开区预览选区。
    pub fn set_preview_selection(&mut self, range: Option<(usize, usize)>) {
        self.preview_selection = range;
    }

    /// 清空展开区预览选区与拖拽状态；原先存在非空预览选区时返回 `true`。
    pub fn clear_preview_selection(&mut self) -> bool {
        let had = self.preview_selection().is_some();
        self.preview_selection = None;
        self.preview_drag = None;
        had
    }

    /// 当前是否正在拖拽展开区预览选区。
    pub fn is_preview_dragging(&self) -> bool {
        self.preview_drag.is_some()
    }

    /// 视觉行索引与内容列命中的折叠芯片（折叠或已展开）。
    pub fn paste_hit_at(&self, row: usize, col: u16) -> Option<ChipHit> {
        let rows = self.visual_rows();
        let visual = *rows.get(row)?;
        let proj = self.projection();
        if !proj.has_tokens() {
            return None;
        }
        if let Some(hit) = proj.chip_at(visual.start)
            && hit.expanded
        {
            return Some(hit);
        }
        let indent = proj.row_indent(visual.start, visual.end);
        let rel_col = usize::from(col.saturating_sub(indent));
        let slice = &proj.text()[visual.start..visual.end];
        if rel_col >= text::display_width(slice) {
            return None;
        }
        let disp_offset = visual.start + text::offset_at_col(slice, rel_col);
        proj.chip_at(disp_offset)
    }

    /// 视觉行索引与内容列 → 投影显示文本字节偏移（`after` 为真时取该单元格字符之后）。
    pub fn display_offset_at_cell(&self, row: usize, col: u16, after: bool) -> usize {
        let rows = self.visual_rows();
        if rows.is_empty() {
            return 0;
        }
        let proj = self.projection();
        let index = row.min(rows.len() - 1);
        let visual = rows[index];
        let indent = proj.row_indent(visual.start, visual.end);
        let col = col.saturating_sub(indent);
        let slice = &proj.text()[visual.start..visual.end];
        let mut cell = 0;
        for (offset, ch) in slice.char_indices() {
            let width = text::char_width(ch);
            if usize::from(col) < cell + width {
                return visual.start
                    + if after {
                        offset + ch.len_utf8()
                    } else {
                        offset
                    };
            }
            cell += width;
        }
        visual.end
    }

    /// 在展开芯片内按下鼠标：初始化预览选区（支持单击/双击选词/三击选行）。
    pub fn begin_preview_selection(&mut self, row: usize, col: u16, clicks: u8) -> bool {
        let Some(hit) = self.paste_hit_at(row, col) else {
            return false;
        };
        if !hit.expanded {
            return false;
        }
        self.text_selection = None;
        let disp_pos = self
            .display_offset_at_cell(row, col, false)
            .clamp(hit.display_range.start, hit.display_range.end);
        let unit = SelectionUnit::from_clicks(clicks);
        match unit {
            SelectionUnit::Character => {
                self.preview_selection = Some((disp_pos, disp_pos));
            }
            SelectionUnit::Word | SelectionUnit::Line => {
                let proj = self.projection();
                let body_slice = &proj.text()[hit.display_range.clone()];
                let rel = disp_pos.saturating_sub(hit.display_range.start);
                let (s, e) = unit.byte_range(body_slice, rel);
                self.preview_selection =
                    Some((hit.display_range.start + s, hit.display_range.start + e));
            }
        }
        self.preview_drag = Some((hit.id, hit.occurrence, hit.display_range));
        true
    }

    /// 拖拽延伸展开芯片内的预览选区（限制在同一展开芯片范围内）。
    pub fn drag_preview_selection(&mut self, row: usize, col: u16) -> bool {
        let Some((_, _, ref bounds)) = self.preview_drag else {
            return false;
        };
        let Some((anchor, _)) = self.preview_selection else {
            return false;
        };
        let bounds = bounds.clone();
        let probe = self.display_offset_at_cell(row, col, false);
        let raw_cursor = if probe >= anchor {
            self.display_offset_at_cell(row, col, true)
        } else {
            probe
        };
        let cursor = raw_cursor.clamp(bounds.start, bounds.end);
        self.preview_selection = Some((anchor, cursor));
        true
    }

    /// 结束展开芯片内的鼠标按下/拖拽：未发生位移（空选区）时视为单击收起芯片，否则保留选区。
    pub fn finish_preview_selection(&mut self) -> bool {
        let Some((id, occurrence, _)) = self.preview_drag.take() else {
            return false;
        };
        if self.preview_selection().is_none() {
            self.preview_selection = None;
            self.toggle_paste(id, occurrence);
        }
        true
    }
}

/// 辅助：计算起止字节在被折叠 token 遮蔽后的合理行首偏移。
pub(super) fn clamp_line_start(text: &str, cursor: usize) -> usize {
    let start = text::line_start(text, cursor);
    for tok in folded_tokens(text) {
        if tok.range.start < start && start <= tok.range.end {
            return text::line_start(text, tok.range.start);
        }
    }
    clamp_cursor(text, start)
}

/// 辅助：计算逻辑行行尾；若行尾换行落在多行 token 内部，延伸到该 token 所在末行的行尾。
pub(super) fn clamp_line_end(text: &str, cursor: usize) -> usize {
    let end = text::line_end(text, cursor);
    for tok in folded_tokens(text) {
        if tok.range.start <= end && end < tok.range.end {
            return text::line_end(text, tok.range.end);
        }
    }
    end
}

impl super::LxChat {
    /// 用户消息是否处于展开状态。
    pub fn user_message_expanded(&self, message_id: &str) -> bool {
        self.expanded_user_messages.contains(message_id)
    }

    /// 切换用户消息展开状态；返回是否改变。
    pub fn toggle_user_message_expanded(&mut self, message_id: &str) -> bool {
        if self.expanded_user_messages.contains(message_id) {
            self.expanded_user_messages.remove(message_id);
        } else {
            self.expanded_user_messages.insert(message_id.to_string());
        }
        true
    }

    /// 用户消息内指定折叠粘贴芯片是否处于展开状态（存 `detail_expanded` 的 `paste:<id>:<occ>` 槽位）。
    pub fn user_paste_expanded(&self, message_id: &str, paste_id: u64, occurrence: usize) -> bool {
        let slot = format!("paste:{paste_id}:{occurrence}");
        self.detail_is_expanded(message_id, &slot)
    }

    /// 切换用户消息内指定折叠粘贴芯片的展开状态；返回切换后的状态（`true` = 已展开）。
    pub fn toggle_user_paste(
        &mut self,
        message_id: &str,
        paste_id: u64,
        occurrence: usize,
    ) -> bool {
        let slot = format!("paste:{paste_id}:{occurrence}");
        self.toggle_detail(message_id, &slot)
    }

    /// 构造指定用户消息当前已展开的折叠粘贴芯片集合。
    pub fn user_expanded_pastes(&self, message_id: &str) -> crate::app::paste::ExpandedPastes {
        let mut expanded = crate::app::paste::ExpandedPastes::default();
        for (msg, slot, part, is_exp) in &self.detail_expanded {
            if msg == message_id
                && *part == super::CardSection::Body
                && *is_exp
                && let Some(rest) = slot.strip_prefix("paste:")
                && let Some((id_str, occ_str)) = rest.split_once(':')
                && let (Ok(id), Ok(occ)) = (id_str.parse::<u64>(), occ_str.parse::<usize>())
            {
                expanded.toggle(id, occ);
            }
        }
        expanded
    }
}
