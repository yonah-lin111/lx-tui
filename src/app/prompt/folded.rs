//! Prompt 编辑器的长文本折叠粘贴集成：面板状态、显示投影、芯片命中与展开区选区。

use crate::app::paste::{
    ChipHit, PasteAnchor, Projection, TextPasteAction, TextPastePanel, encode_pasted_text,
    mask_pasted_text, snap_pasted_range,
};
use crate::app::selection::SelectionUnit;

use super::Prompt;
use super::text::{display_width, offset_at_col};

impl Prompt {
    /// 当前长文本粘贴面板；未打开时为 `None`。
    pub fn text_paste_panel(&self) -> Option<&TextPastePanel> {
        self.text_paste.panel()
    }

    /// 发起长文本粘贴提案；不修改缓冲区。
    ///
    /// 若锚点位于代码围栏内或触碰受保护行，返回 `false`（由调用方回退为普通文本粘贴）。
    pub fn begin_text_paste(
        &mut self,
        content: String,
        full_text: String,
        anchor: PasteAnchor,
    ) -> bool {
        let (start, end) = anchor.range();
        let end = end.min(self.text.len());
        if start > end
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
            || self.protected_edit(start, end)
            || super::list::in_fence(&mask_pasted_text(&self.text), start)
        {
            return false;
        }
        self.paste_cancel();
        self.text_paste.cancel();
        let normalized_anchor = PasteAnchor {
            selection: (start < end).then_some((start, end)),
            cursor: start,
        };
        let panel = TextPastePanel::new(content, full_text, normalized_anchor);
        self.text_paste.open(panel);
        true
    }

    /// 键盘移动长文本粘贴面板高亮；返回是否消费。
    pub fn text_paste_move(&mut self, delta: isize) -> bool {
        self.text_paste
            .panel_mut()
            .is_some_and(|panel| panel.move_active(delta))
    }

    /// 鼠标悬停长文本粘贴面板条目；返回高亮是否变化。
    pub fn text_paste_set_active(&mut self, index: usize) -> bool {
        self.text_paste
            .panel_mut()
            .is_some_and(|panel| panel.set_active(index))
    }

    /// Esc 取消长文本粘贴面板：不修改缓冲区；返回是否消费。
    pub fn text_paste_escape(&mut self) -> bool {
        self.text_paste_cancel()
    }

    /// 确认长文本粘贴面板操作：`Collapse` 插入折叠 token（补空格），`Full` 插入原文；一步撤销。
    pub fn text_paste_confirm(&mut self, action: TextPasteAction) -> bool {
        let Some(panel) = self.text_paste.panel().cloned() else {
            return false;
        };
        if !panel.enabled(action) {
            return false;
        }
        let (start, end) = panel.anchor.range();
        let range = snap_pasted_range(&self.text, start..end.min(self.text.len()));
        let insertion = match action {
            TextPasteAction::Collapse => {
                let token = encode_pasted_text(&panel.content);
                self.paste_insertion(range.start, &[token])
            }
            TextPasteAction::Full => panel.full_text,
        };
        if !self.replace_at(range.start, range.end, &insertion) {
            return false;
        }
        self.text_paste.cancel();
        true
    }

    /// 关闭长文本粘贴面板（不动缓冲区）；原先打开时返回 `true`。
    pub fn text_paste_cancel(&mut self) -> bool {
        self.text_paste.cancel()
    }

    /// 当前正文的折叠/展开显示投影。
    pub fn projection(&self) -> Projection<'_> {
        Projection::new(&self.text, &self.expanded_pastes)
    }

    /// 是否有任何已展开的折叠芯片。
    pub fn has_expanded_pastes(&self) -> bool {
        !self.expanded_pastes.is_empty()
    }

    /// 切换指定身份折叠芯片的展开状态；不写缓冲区、不记撤销、不置脏。
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

    /// 视口单元格坐标命中的折叠芯片（折叠或已展开）。
    pub fn paste_hit_at(&self, viewport_row: u16, col: u16) -> Option<ChipHit> {
        let content_row = self.scroll.saturating_add(usize::from(viewport_row));
        self.paste_hit_at_content(content_row, col)
    }

    /// 内容行坐标命中的折叠芯片（折叠或已展开）。
    pub fn paste_hit_at_content(&self, content_row: usize, col: u16) -> Option<ChipHit> {
        let rows = self.visual_rows();
        let visual = *rows.get(content_row)?;
        if visual.top_border {
            return None;
        }
        let proj = self.projection();
        if !proj.has_tokens() {
            return None;
        }
        let gutter = self.line_gutter(visual.line);
        if col < gutter {
            return None;
        }
        if visual.start == visual.end {
            return proj.chip_at(visual.start).filter(|hit| hit.expanded);
        }
        let rel_col = usize::from(col.saturating_sub(gutter));
        let slice = &proj.text()[visual.start..visual.end];
        if rel_col >= display_width(slice) {
            return None;
        }
        let disp_offset = visual.start + offset_at_col(slice, rel_col);
        proj.chip_at(disp_offset)
    }

    /// 在展开芯片内按下鼠标：初始化预览选区（支持单击/双击选词/三击选行）。
    pub fn begin_preview_selection(&mut self, viewport_row: u16, col: u16, clicks: u8) -> bool {
        let Some(hit) = self.paste_hit_at(viewport_row, col) else {
            return false;
        };
        if !hit.expanded {
            return false;
        }
        let content_row = self.scroll.saturating_add(usize::from(viewport_row));
        let disp_pos = self
            .display_content_offset_at(content_row, col, false)
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
    pub fn drag_preview_selection(&mut self, viewport_row: u16, col: u16) -> bool {
        let Some((_, _, ref bounds)) = self.preview_drag else {
            return false;
        };
        let Some((anchor, _)) = self.preview_selection else {
            return false;
        };
        let bounds = bounds.clone();
        let content_row = self.scroll.saturating_add(usize::from(viewport_row));
        let probe = self.display_content_offset_at(content_row, col, false);
        let raw_cursor = if probe >= anchor {
            self.display_content_offset_at(content_row, col, true)
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

/// 辅助：计算起止字节在被折叠 token 遮蔽后的合理行首/行尾偏移。
pub(super) fn clamp_line_start(text: &str, cursor: usize) -> usize {
    let start = super::text::line_start(text, cursor);
    for tok in crate::app::paste::folded_tokens(text) {
        if tok.range.start < start && start <= tok.range.end {
            return super::text::line_start(text, tok.range.start);
        }
    }
    crate::app::paste::clamp_cursor(text, start)
}

/// 辅助：计算逻辑行行尾；若行尾换行落在多行 token 内部，延伸到该 token 所在末行的行尾。
pub(super) fn clamp_line_end(text: &str, cursor: usize) -> usize {
    let end = super::text::line_end(text, cursor);
    for tok in crate::app::paste::folded_tokens(text) {
        if tok.range.start <= end && end < tok.range.end {
            return super::text::line_end(text, tok.range.end);
        }
    }
    end
}
