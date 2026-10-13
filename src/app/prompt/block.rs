//! 补充块与变量块操作：清理、删除、合并、置顶与折叠视图状态；纯状态、无 IO。

use std::collections::BTreeSet;

use super::super::markdown::{self, BlockContext, BlockKind, BlockSpan};
use super::{EditKind, Prompt};

impl Prompt {
    /// 起始行处、类型匹配的块范围。
    fn block_span_of(&self, start_line: usize, kind: BlockKind) -> Option<BlockSpan> {
        let span = markdown::block_span_at_line(&self.text, start_line)?;
        (span.kind == kind && span.start == start_line).then_some(span)
    }

    /// 块正文区间：起止行之间的内容（未闭合延伸到文末）；空区间返回 None。
    fn block_body_span(&self, span: BlockSpan) -> Option<(usize, usize)> {
        let (_, start_end) = self.line_bounds(span.start)?;
        let (end_start, _) = self.line_bounds(span.end)?;
        let to = if span.closed {
            end_start.checked_sub(1)?
        } else {
            self.text.len()
        };
        let from = start_end + 1;
        (from <= to && to <= self.text.len()).then_some((from, to))
    }

    /// 以一次其它类编辑替换整块正文；无变化返回 false。
    fn replace_block_body(&mut self, from: usize, to: usize, cleaned: &str) -> bool {
        if cleaned == &self.text[from..to] {
            return false;
        }
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(from, to, cleaned);
        true
    }

    /// 清理补充块正文中未填写的空项（单步撤销）；返回是否产生变更。
    pub fn clean_supple_block(&mut self, start_line: usize) -> bool {
        let Some(span) = self.block_span_of(start_line, BlockKind::Supple) else {
            return false;
        };
        let Some((from, to)) = self.block_body_span(span) else {
            return false;
        };
        let cleaned = markdown::clean_template_content(&self.text[from..to]);
        self.replace_block_body(from, to, &cleaned)
    }

    /// 删除整个补充块（含两端标记，最多吸收一侧空行；单步撤销）。
    pub fn delete_supple_block(&mut self, start_line: usize) -> bool {
        let Some(span) = self.block_span_of(start_line, BlockKind::Supple) else {
            return false;
        };
        let range = markdown::block_deletion_range(&self.text, span);
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(range.start, range.end, "");
        true
    }

    /// 清理变量块正文中未填写的条目（单步撤销）；返回是否产生变更。
    pub fn clean_var_block(&mut self, start_line: usize) -> bool {
        let Some(span) = self.block_span_of(start_line, BlockKind::Var) else {
            return false;
        };
        let Some((from, to)) = self.block_body_span(span) else {
            return false;
        };
        let cleaned = markdown::clean_var_content(&self.text[from..to]);
        self.replace_block_body(from, to, &cleaned)
    }

    /// 删除整个变量块（含两端标记，最多吸收一侧空行；单步撤销）。
    pub fn delete_var_block(&mut self, start_line: usize) -> bool {
        let Some(span) = self.block_span_of(start_line, BlockKind::Var) else {
            return false;
        };
        let range = markdown::block_deletion_range(&self.text, span);
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(range.start, range.end, "");
        true
    }

    /// 把变量块内容合并到文档首个变量块；当前已是首块返回 false。
    pub fn merge_var_block(&mut self, start_line: usize) -> bool {
        let Some(span) = self.block_span_of(start_line, BlockKind::Var) else {
            return false;
        };
        match markdown::merge_var_to_first(&self.text, span) {
            Some(new) => self.apply_full_text(new),
            None => false,
        }
    }

    /// 把变量块置顶到文档开头连续变量块组之后；已在组内返回 false。
    pub fn move_var_block_to_top(&mut self, start_line: usize) -> bool {
        let Some(span) = self.block_span_of(start_line, BlockKind::Var) else {
            return false;
        };
        match markdown::move_var_to_top(&self.text, span) {
            Some(new) => self.apply_full_text(new),
            None => false,
        }
    }

    /// 以一次其它类编辑替换全文（合并/置顶）；光标钳到文末内并重算面板。
    fn apply_full_text(&mut self, new: String) -> bool {
        if new == self.text {
            return false;
        }
        self.break_group();
        self.record(EditKind::Other);
        self.text = new;
        self.cursor = self.cursor.min(self.text.len());
        self.settle();
        true
    }

    /// 折叠/展开变量块（纯视图状态，不改正文）；目标非变量块起始行返回 false。
    pub fn toggle_var_fold(&mut self, start_line: usize) -> bool {
        if self.block_span_of(start_line, BlockKind::Var).is_none() {
            return false;
        }
        if !self.folded.remove(&start_line) {
            self.folded.insert(start_line);
        }
        self.settle();
        true
    }

    /// 变量块是否处于折叠状态。
    pub fn var_folded(&self, start_line: usize) -> bool {
        self.folded.contains(&start_line)
    }

    /// 折叠起始行集合（渲染读取）。
    pub fn folded_starts(&self) -> &BTreeSet<usize> {
        &self.folded
    }

    /// 变量块键行内输入的全角冒号是否应转为半角 `:`：
    /// 仅在变量块内、不在三引号正文、且当前行尚未出现分隔冒号时。
    pub fn var_colon_converts(&self) -> bool {
        if markdown::block_context(&self.text, self.cursor) != BlockContext::Var {
            return false;
        }
        if markdown::var_in_triple_quotes(&self.text, self.cursor) {
            return false;
        }
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |at| at + 1);
        let line = &self.text[start..self.cursor];
        !line.contains(':') && !line.contains('：')
    }

    /// 变量块内 Tab / Shift-Tab 跳转到下一个/上一个键名末尾；不在变量块内返回 false。
    pub fn var_tab_jump(&mut self, forward: bool) -> bool {
        let targets = markdown::var_tab_targets(&self.text, self.cursor);
        if targets.is_empty() {
            return false;
        }
        let target = if forward {
            targets
                .iter()
                .find(|target| **target > self.cursor)
                .copied()
                .or_else(|| targets.first().copied())
        } else {
            targets
                .iter()
                .rev()
                .find(|target| **target < self.cursor)
                .copied()
                .or_else(|| targets.last().copied())
        };
        let Some(target) = target else {
            return false;
        };
        self.break_group();
        self.cursor = target;
        self.settle();
        true
    }
}
