//! prompt 模板块操作：正文提取、空项清理、整块删除与状态轮转，均接入撤销栈。

use crate::app::paste::mask_pasted_text;

use super::super::markdown::{self, TemplateBlockRange};
use super::EditKind;
use super::Prompt;

impl Prompt {
    /// 模板块纯净正文（起止行之间的内容）；非模板块或空正文返回 None。
    pub fn template_block_body(&self, start_line: usize) -> Option<String> {
        let masked = mask_pasted_text(&self.text);
        let range = markdown::parse_template_block_at_line(&masked, start_line)?;
        let (from, to, _) = self.inner_span(range)?;
        Some(self.text[from..to].to_string())
    }

    /// 提取起始行处的 `+++` 补充块正文（支持独立块或嵌套在 `&&&` 内的补充块）。
    pub fn supple_body_at_start(&self, start_line: usize) -> Option<String> {
        let (_, start_end) = self.line_bounds(start_line)?;
        let masked = mask_pasted_text(&self.text);
        let lines: Vec<&str> = masked.split('\n').collect();
        markdown::parse_supple_start_line(lines.get(start_line)?)?;
        let mut end_line = lines.len().saturating_sub(1);
        let mut closed = false;
        let mut fence: Option<(u8, usize)> = None;
        for (idx, line) in lines.iter().enumerate().skip(start_line + 1) {
            if let Some((marker, length)) = fence {
                if markdown::fence_delimiter(line).is_some_and(|(m, l)| m == marker && l >= length)
                {
                    fence = None;
                }
                continue;
            }
            if let Some(delim) = markdown::fence_delimiter(line) {
                fence = Some(delim);
                continue;
            }
            if markdown::parse_supple_end_line(line).is_some() {
                end_line = idx;
                closed = true;
                break;
            }
        }
        let (end_start, _) = self.line_bounds(end_line)?;
        let to = if closed {
            end_start.checked_sub(1)?
        } else {
            self.text.len()
        };
        let from = start_end + 1;
        (from <= to && to <= self.text.len()).then_some(self.text[from..to].to_string())
    }

    /// 更新或恢复指定块起始行下一行的 `{title: ...}`，并自动平移光标。
    pub fn set_block_title(&mut self, start_line: usize, new_title: Option<&str>) -> bool {
        let masked = mask_pasted_text(&self.text);
        let Some((range, replacement)) =
            markdown::set_block_title_edit(&masked, start_line, new_title)
        else {
            return false;
        };
        if self.text.get(range.clone()) == Some(replacement.as_str()) {
            return false;
        }
        self.replace_span(range.start, range.end, &replacement);
        true
    }

    /// 复制快捷键用正文：光标所在模板块或补充块按复制语义提取；不在块内返回 None。
    ///
    /// `+++` 是 `&&&` 的内层块：光标在内层时优先复制内层正文。
    pub fn copy_text_at_cursor(&self) -> Option<String> {
        let masked = mask_pasted_text(&self.text);
        if let Some(start) = markdown::supple_block_start_at(&masked, self.cursor)
            && let Some(body) = self.supple_block_body(start)
        {
            return Some(markdown::copy_template_content(&body));
        }
        let start = markdown::template_block_start_at(&masked, self.cursor)?;
        let body = self.template_block_body(start)?;
        Some(markdown::copy_template_content(&body))
    }

    /// 光标在模板块内时循环块状态（todo → in_progress → done）；返回是否已处理。
    ///
    /// 切换当前页模板块状态（todo → in_progress → done → todo，单步撤销）；
    /// 无论光标位于页内何处，只要本页存在闭合的 &&& 任务块即轮转并返回 true；
    /// 本页无闭合 &&& 任务块时返回 false，由调用方回退为反缩进。
    pub fn cycle_template_status(&mut self) -> bool {
        let masked = mask_pasted_text(&self.text);
        let scan = markdown::block_scan(&masked);
        let Some(template) = scan
            .spans
            .iter()
            .find(|span| span.kind == markdown::BlockKind::Template && span.closed)
        else {
            return false;
        };
        self.toggle_template_status(template.start)
    }

    /// 清理模板块内未填写的空项（单步撤销）；返回是否产生变更。
    pub fn clean_template_block(&mut self, start_line: usize) -> bool {
        let masked = mask_pasted_text(&self.text);
        let Some(range) = markdown::parse_template_block_at_line(&masked, start_line) else {
            return false;
        };
        let Some((from, to, closed)) = self.inner_span(range) else {
            return false;
        };
        let mut cleaned = markdown::clean_template_content(&self.text[from..to]);
        if closed {
            cleaned = markdown::ensure_bottom_gap(cleaned);
        }
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
        let masked = mask_pasted_text(&self.text);
        let Some(range) = markdown::parse_template_block_at_line(&masked, start_line) else {
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
        let masked = mask_pasted_text(&self.text);
        let Some(range) = markdown::parse_template_block_at_line(&masked, start_line) else {
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

    /// 在指定字节偏移（模板块起始行行尾）处插入 `\n{title: }`，并将光标置于 `}` 之前，返回新光标字节偏移。
    ///
    /// 新行继承起始行的前导空格缩进，模板块缩进与正文保持一致。
    pub fn insert_template_title_line(&mut self, line_end_byte: usize) -> usize {
        let raw_pos = self
            .text
            .floor_char_boundary(line_end_byte.min(self.text.len()));
        let pos = super::folded::clamp_line_end(&self.text, raw_pos);
        let line_start = super::folded::clamp_line_start(&self.text, pos);
        let indent: String = self.text[line_start..pos]
            .chars()
            .take_while(|ch| *ch == ' ')
            .collect();
        self.break_group();
        self.record(EditKind::Other);
        let snippet = format!("\n{indent}{{title: }}");
        let cursor_offset = pos + 1 + indent.len() + "{title: ".len();
        self.splice_text(pos, pos, &snippet);
        self.cursor = cursor_offset;
        self.settle();
        self.cursor
    }

    /// 替换字节区间并按区间位置调整光标（区间之后的偏移平移，区间内钳到插入后）；
    /// 调用方负责记录撤销快照。
    pub(super) fn replace_span(&mut self, from: usize, to: usize, replacement: &str) {
        self.splice_text(from, to, replacement);
        self.cursor = if self.cursor >= to {
            self.cursor - (to - from) + replacement.len()
        } else if self.cursor > from {
            from + replacement.len()
        } else {
            self.cursor
        };
        self.settle();
    }

    /// 模板块正文区间与闭合态：起始行末换行之后到结束行前换行之前；
    /// 未闭合（末行非结束行）时到文末；无可清理内容返回 None。
    pub(super) fn inner_span(&self, range: TemplateBlockRange) -> Option<(usize, usize, bool)> {
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
        (from <= to && to <= self.text.len()).then_some((from, to, closed))
    }

    /// 逻辑行索引的字节范围（不含换行）；越界返回 None。
    pub(super) fn line_bounds(&self, line: usize) -> Option<(usize, usize)> {
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
