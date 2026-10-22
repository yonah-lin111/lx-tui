//! 补充块操作：清理与删除；纯状态、无 IO。

use super::super::markdown::{self, BlockKind, BlockSpan};
use super::{EditKind, Prompt};

/// 附件保存结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplementCommit {
    /// 已提交，形成一条父编辑器撤销记录。
    Changed,
    /// 内容与原文一致，不改文本、不入历史。
    Unchanged,
    /// 区间或结构不合法，拒绝写入。
    Rejected,
}

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
        let mut cleaned = markdown::clean_template_content(&self.text[from..to]);
        if span.closed {
            cleaned = markdown::ensure_bottom_gap(cleaned);
        }
        self.replace_block_body(from, to, &cleaned)
    }

    /// 补充块纯净正文（起止行之间的内容）；非补充块或空正文返回 None。
    pub fn supple_block_body(&self, start_line: usize) -> Option<String> {
        let span = self.block_span_of(start_line, BlockKind::Supple)?;
        let (from, to) = self.block_body_span(span)?;
        Some(self.text[from..to].to_string())
    }

    /// 附件保存结果：实际变更、内容未变、目标或结构不合法被拒。
    pub fn commit_supplement(
        &mut self,
        from: usize,
        to: usize,
        replacement: &str,
    ) -> SupplementCommit {
        if from >= to
            || to > self.text.len()
            || !self.text.is_char_boundary(from)
            || !self.text.is_char_boundary(to)
        {
            return SupplementCommit::Rejected;
        }
        if replacement == &self.text[from..to] {
            return SupplementCommit::Unchanged;
        }
        // 附件本身就是隐藏区：这里只校验结构保护（标题/绑定行），不再套用隐藏区拒绝。
        if markdown::edit_touches_worktree_line(&self.text, from, to)
            || markdown::edit_touches_title_line(&self.text, from, to)
        {
            return SupplementCommit::Rejected;
        }
        // 清理旧屏幕选区与候选，避免保存后突然重弹面板。
        self.panel = None;
        self.slash = None;
        self.refer = None;
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(from, to, replacement);
        self.normalize_hidden_focus();
        SupplementCommit::Changed
    }

    /// 在文末追加补充块文本（未闭合父任务的新增路径；单步撤销）；返回是否插入。
    pub fn append_supplement(&mut self, block: &str) -> bool {
        let separator = if self.text.is_empty() || self.text.ends_with("\n\n") {
            ""
        } else if self.text.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let at = self.text.len();
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(at, at, &format!("{separator}{block}\n"));
        self.normalize_hidden_focus();
        true
    }

    /// 在指定逻辑行之前整块插入补充块文本（单步撤销）；返回是否插入。
    ///
    /// `block` 必须是完整 `+++ … --start … --end` 文本（无首尾换行）。
    pub fn insert_supplement_before(&mut self, line: usize, block: &str) -> bool {
        let Some((from, _)) = self.line_bounds(line) else {
            return false;
        };
        if !self.text.is_char_boundary(from) {
            return false;
        }
        let separator = if from > 0 && !self.text[..from].ends_with("\n\n") {
            "\n"
        } else {
            ""
        };
        let payload = format!("{separator}{block}\n");
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(from, from, &payload);
        self.normalize_hidden_focus();
        true
    }

    /// 父任务块范围；起始行不是 `&&&` 块时返回 None（未闭合块也返回范围）。
    pub fn template_block(&self, start_line: usize) -> Option<BlockSpan> {
        self.block_span_of(start_line, BlockKind::Template)
    }

    /// 斜杠来源新建：用完整补充块替换冻结的触发行区间（单步撤销）。
    pub fn replace_supplement_trigger(
        &mut self,
        trigger: &std::ops::Range<usize>,
        block: &str,
    ) -> bool {
        if trigger.start >= trigger.end
            || trigger.end > self.text.len()
            || !self.text.is_char_boundary(trigger.start)
            || !self.text.is_char_boundary(trigger.end)
        {
            return false;
        }
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(trigger.start, trigger.end, block);
        self.normalize_hidden_focus();
        true
    }

    /// 删除整个补充块（含两端标记，最多吸收一侧空行；单步撤销）。
    ///
    /// 集中视图下补充块是隐藏区，删除是显式领域操作，不走普通编辑的相交拒绝。
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
}
