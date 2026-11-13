//! 补充块操作：清理、删除与插入；纯状态、无 IO。

use crate::app::paste::mask_pasted_text;

use super::super::markdown::{self, BlockKind, BlockSpan};
use super::{EditKind, Prompt};

impl Prompt {
    /// 起始行处、类型匹配的块范围。
    fn block_span_of(&self, start_line: usize, kind: BlockKind) -> Option<BlockSpan> {
        let masked = mask_pasted_text(&self.text);
        let span = markdown::block_span_at_line(&masked, start_line)?;
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

    /// 删除整个补充块（含两端标记，最多吸收一侧空行；单步撤销）。
    pub fn delete_supple_block(&mut self, start_line: usize) -> bool {
        let Some(span) = self.block_span_of(start_line, BlockKind::Supple) else {
            return false;
        };
        let masked = mask_pasted_text(&self.text);
        let range = markdown::block_deletion_range(&masked, span);
        self.break_group();
        self.record(EditKind::Other);
        self.replace_span(range.start, range.end, "");
        true
    }

    /// 清空斜杠命令行内容（保留该空行）后，把补充块插到下一个块之前；
    /// 没有下一个块时按旧落点追加；调用方已记录撤销快照。
    ///
    /// 下一个块按文档顺序取起始行在命令行之后的第一个已闭合块（`&&&` 与 `+++`
    /// 同等参与）；有下一个块时插在其起始行之前，无下一个块时追加到最后一个
    /// `+++` 结束行之后 → `&&&` 结束行之后 → 文末。新块与前后内容各隔一个空行，
    /// 光标落在新块标题值（`{title: }` 的 `}` 前）。
    pub(super) fn insert_supple_block(
        &mut self,
        trigger: &markdown::SlashTrigger,
        content: &str,
        cursor: usize,
    ) {
        // 命令行在文首（前面只有空白）时，清空后要把这段前导空行一并折叠。
        let at_document_start = self.text[..trigger.line_start].trim().is_empty();
        // 只删命令本身（`/` + query），保留行首缩进或同一行已有的粘贴 chip。
        let command_start = trigger
            .line_end
            .saturating_sub(trigger.query.len() + 1)
            .max(trigger.line_start);
        self.splice_text(command_start, trigger.line_end, "");
        // 先落到清空后的命令行末尾；未闭合守卫提前返回时也不会留下越界光标。
        self.cursor = command_start;

        let masked = mask_pasted_text(&self.text);
        let scan = markdown::block_scan(&masked);
        // 页内存在未闭合块时不插入：落点会落在未闭合块内部，新块会被吞掉或误闭合前一块。
        if scan.spans.iter().any(|span| !span.closed) {
            return;
        }
        let command_line = self.text[..trigger.line_start].matches('\n').count();
        let next_start = scan
            .spans
            .iter()
            .find(|span| span.start > command_line)
            .map(|span| span.start);
        let anchor_line = scan
            .spans
            .iter()
            .rev()
            .find(|span| span.kind == BlockKind::Supple && span.closed)
            .or_else(|| {
                scan.spans
                    .iter()
                    .find(|span| span.kind == BlockKind::Template && span.closed)
            })
            .map(|span| span.end);

        // 没有任何块锚点时只能追加到文末；文末若在未闭合代码围栏内，新块会被围栏吞掉。
        if next_start.is_none() && anchor_line.is_none() && markdown::open_fence(&masked).is_some()
        {
            return;
        }
        let mut block_start = match next_start {
            Some(line) => match self.insert_before_line(line, content) {
                Some(start) => start,
                None => return,
            },
            None => self.append_after_anchor(anchor_line, content),
        };
        if at_document_start {
            let removed = self.trim_leading_blank_lines();
            block_start = block_start.saturating_sub(removed);
        }

        self.cursor = block_start + cursor;
        let start_line = self.text[..block_start].matches('\n').count();
        let end_line = start_line + content.matches('\n').count();
        self.scroll_lines_into_view(start_line, end_line);
    }

    /// 在逻辑行 `line` 起始位置插入块正文，前后各保留一个空行；返回正文起始字节偏移。
    /// 行号越界返回 None（不插入）。
    ///
    /// 文首只有空行时清掉前导空行，避免文档以空行开头；否则从最后一个非空白行
    /// 的末端开始替换，连同中间只含空白的行一起折叠成一个空行。
    fn insert_before_line(&mut self, line: usize, content: &str) -> Option<usize> {
        let (line_start, _) = self.line_bounds(line)?;
        let prefix = &self.text[..line_start];
        if prefix.trim().is_empty() {
            self.splice_text(0, line_start, &format!("{content}\n\n"));
            Some(0)
        } else {
            let base = last_content_end(prefix);
            self.splice_text(base, line_start, &format!("\n\n{content}\n\n"));
            Some(base + 2)
        }
    }

    /// 删除文首连续空白行（含只含空白的行）；返回删除的字节数。
    fn trim_leading_blank_lines(&mut self) -> usize {
        let mut removed = 0;
        for line in self.text.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            if !content.trim().is_empty() {
                break;
            }
            removed += line.len();
        }
        if removed > 0 {
            self.splice_text(0, removed, "");
        }
        removed
    }

    /// 无下一个块时的旧落点：最后一个已闭合 `+++` 结束行之后 → 首个已闭合 `&&&`
    /// 结束行之后 → 文末；返回正文起始字节偏移。
    fn append_after_anchor(&mut self, anchor_line: Option<usize>, content: &str) -> usize {
        match anchor_line.and_then(|line| self.line_bounds(line)) {
            Some((_, anchor_end)) => match first_content_line_start(&self.text, anchor_end) {
                Some(tail_start) => {
                    let insert = format!("\n\n{content}\n\n");
                    self.splice_text(anchor_end, tail_start, &insert);
                    anchor_end + 2
                }
                None => {
                    let base = last_content_end(&self.text);
                    let insert = format!("\n\n{content}");
                    self.splice_text(base, self.text.len(), &insert);
                    base + 2
                }
            },
            None => {
                if self.text.trim_matches('\n').is_empty() {
                    self.splice_text(0, self.text.len(), content);
                    0
                } else {
                    let base = last_content_end(&self.text);
                    let insert = format!("\n\n{content}");
                    self.splice_text(base, self.text.len(), &insert);
                    base + 2
                }
            }
        }
    }
}

/// 最后一个非空白行内容末端的字节偏移；保留该行自身的尾随空格。
fn last_content_end(text: &str) -> usize {
    let mut offset = 0;
    let mut end = 0;
    for line in text.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if !content.trim().is_empty() {
            end = offset + content.len();
        }
        offset += line.len();
    }
    end
}

/// 从 `from` 起第一个非空白行的行首字节偏移；没有内容行时返回 None。
fn first_content_line_start(text: &str, from: usize) -> Option<usize> {
    let mut offset = from;
    for line in text.get(from..)?.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if !content.trim().is_empty() {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
mod tests;
