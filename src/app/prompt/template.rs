//! prompt 模板块操作：正文提取、空项清理、整块删除与状态轮转，均接入撤销栈。

use super::super::markdown::{self, TemplateBlockRange};
use super::EditKind;
use super::Prompt;

impl Prompt {
    /// 模板块纯净正文（起止行之间的内容）；非模板块或空正文返回 None。
    pub fn template_block_body(&self, start_line: usize) -> Option<String> {
        let range = markdown::parse_template_block_at_line(&self.text, start_line)?;
        let (from, to) = self.inner_span(range)?;
        Some(self.text[from..to].to_string())
    }

    /// 复制快捷键用正文：光标所在模板块按复制语义提取；不在块内返回 None。
    pub fn copy_text_at_cursor(&self) -> Option<String> {
        let start = markdown::template_block_start_at(&self.text, self.cursor)?;
        let body = self.template_block_body(start)?;
        Some(markdown::copy_template_content(&body))
    }

    /// 清理模板块内未填写的空项（单步撤销）；返回是否产生变更。
    pub fn clean_template_block(&mut self, start_line: usize) -> bool {
        let Some(range) = markdown::parse_template_block_at_line(&self.text, start_line) else {
            return false;
        };
        let Some((from, to)) = self.inner_span(range) else {
            return false;
        };
        let cleaned = markdown::clean_template_content(&self.text[from..to]);
        if cleaned == self.text[from..to] {
            return false;
        }
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(from, to, &cleaned);
        true
    }

    /// 删除整个模板块（含结束行换行，单步撤销）；返回是否产生变更。
    pub fn delete_template_block(&mut self, start_line: usize) -> bool {
        let Some(range) = markdown::parse_template_block_at_line(&self.text, start_line) else {
            return false;
        };
        let Some((from, _)) = self.line_bounds(range.start) else {
            return false;
        };
        let Some((_, end)) = self.line_bounds(range.end) else {
            return false;
        };
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(from, (end + 1).min(self.text.len()), "");
        true
    }

    /// 循环切换模板块状态 todo → in_progress → done（单步撤销）；返回是否产生变更。
    pub fn toggle_template_status(&mut self, start_line: usize) -> bool {
        let Some(range) = markdown::parse_template_block_at_line(&self.text, start_line) else {
            return false;
        };
        let Some((from, to)) = self.line_bounds(range.end) else {
            return false;
        };
        let Some(next) = markdown::cycle_template_status(&self.text[from..to]) else {
            return false;
        };
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(from, to, &next);
        true
    }

    /// 替换字节区间并按区间位置调整光标（区间之后的偏移平移，区间内钳到插入后）；
    /// 调用方负责记录撤销快照。
    fn replace_span(&mut self, from: usize, to: usize, replacement: &str) {
        self.text.replace_range(from..to, replacement);
        self.cursor = if self.cursor >= to {
            self.cursor - (to - from) + replacement.len()
        } else if self.cursor > from {
            from + replacement.len()
        } else {
            self.cursor
        };
        self.settle();
    }

    /// 模板块正文区间：起始行末换行之后到结束行前换行之前；
    /// 未闭合（末行非结束行）时到文末；无可清理内容返回 None。
    fn inner_span(&self, range: TemplateBlockRange) -> Option<(usize, usize)> {
        let (_, start_end) = self.line_bounds(range.start)?;
        let (end_start, end_line_end) = self.line_bounds(range.end)?;
        let closed =
            markdown::parse_template_end_line(&self.text[end_start..end_line_end]).is_some();
        let to = if closed {
            end_start.checked_sub(1)?
        } else {
            self.text.len()
        };
        let from = start_end + 1;
        (from <= to && to <= self.text.len()).then_some((from, to))
    }

    /// 逻辑行索引的字节范围（不含换行）；越界返回 None。
    fn line_bounds(&self, line: usize) -> Option<(usize, usize)> {
        let mut start = 0;
        for (index, content) in self.text.split('\n').enumerate() {
            let end = start + content.len();
            if index == line {
                return Some((start, end));
            }
            start = end + 1;
        }
        None
    }
}
