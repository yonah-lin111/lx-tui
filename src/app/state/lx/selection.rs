//! lx 输入框文本选区：全选、取文、删除与剪切。

impl super::Lx {
    /// 在指定视觉行/内容列开始输入框文本选择：光标落到该单元格并登记锚点。
    pub fn begin_text_selection(&mut self, row: usize, col: u16) {
        let offset = self.offset_at_cell(row, col);
        self.cursor = offset;
        self.text_anchor = Some(offset);
        self.panel = None;
        self.mention.suppress();
        self.scroll_cursor_into_view();
    }

    /// 拖拽扩展输入框选区：光标跟到单元格、锚点不动。
    pub fn drag_text_selection(&mut self, row: usize, col: u16) {
        if self.text_anchor.is_none() {
            return;
        }
        self.cursor = self.offset_at_cell(row, col);
        self.scroll_cursor_into_view();
    }

    /// 清除输入框文本选区。
    pub fn clear_text_selection(&mut self) {
        self.text_anchor = None;
    }

    /// 是否存在输入框文本选区锚点。
    pub fn has_text_selection(&self) -> bool {
        self.text_anchor.is_some()
    }

    /// 输入框选区字节范围 `[start, end)`；空选区返回 None。
    pub fn text_selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.text_anchor?;
        let (start, end) = if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        (start < end).then_some((start, end))
    }

    /// 取输入框选中文本；空选区返回 None。
    pub fn selected_text(&self) -> Option<String> {
        self.text_selection_range()
            .map(|(start, end)| self.input[start..end].to_string())
    }

    /// 全选输入框文本。
    pub fn select_all(&mut self) {
        if self.input.is_empty() {
            self.text_anchor = None;
            self.cursor = 0;
            return;
        }
        self.text_anchor = Some(0);
        self.cursor = self.input.len();
        self.scroll_cursor_into_view();
    }

    /// 删除输入框选区；无选区返回 false。一次删除是一条撤销记录。
    pub fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.text_selection_range() else {
            return false;
        };
        self.snapshot();
        self.input.replace_range(start..end, "");
        self.cursor = start;
        self.text_anchor = None;
        true
    }

    /// 剪切输入框选区：返回选中文本；无选区返回 None。
    pub fn cut_selection(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.delete_selection();
        Some(text)
    }
}
