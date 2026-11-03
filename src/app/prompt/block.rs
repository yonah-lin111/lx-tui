//! 补充块操作：清理与删除；纯状态、无 IO。

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

    /// 清空斜杠命令行内容（保留该空行）后，把补充块追加到固定落点；调用方已记录撤销快照。
    ///
    /// 落点：最后一个 `+++` 结束行之后 → `&&&` 结束行之后 → 文末；
    /// 与前文之间恰好隔一个空行，光标落在新块标题值（`{title: }` 的 `}` 前）。
    pub(super) fn append_supple_block(
        &mut self,
        trigger: &markdown::SlashTrigger,
        content: &str,
        cursor: usize,
    ) {
        self.splice_text(trigger.line_start, trigger.line_end, "");

        let masked = mask_pasted_text(&self.text);
        let scan = markdown::block_scan(&masked);
        // 页内存在未闭合块时不插入：落点会落在未闭合块内部，新块会被吞掉或误闭合前一块。
        if scan.spans.iter().any(|span| !span.closed) {
            return;
        }
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

        let block_start = match anchor_line.and_then(|line| self.line_bounds(line)) {
            Some((_, anchor_end)) => {
                let after = &self.text[anchor_end..];
                let newlines = after.bytes().take_while(|byte| *byte == b'\n').count();
                let tail = &after[newlines..];
                if tail.is_empty() {
                    let base = self.text[..anchor_end + newlines]
                        .trim_end_matches('\n')
                        .len();
                    let insert = format!("\n\n{content}");
                    self.splice_text(base, self.text.len(), &insert);
                    base + 2
                } else {
                    let insert = format!("\n\n{content}\n\n");
                    self.splice_text(anchor_end, anchor_end + newlines, &insert);
                    anchor_end + 2
                }
            }
            None => {
                if self.text.trim_matches('\n').is_empty() {
                    self.splice_text(0, self.text.len(), content);
                    0
                } else {
                    let base = self.text.trim_end_matches('\n').len();
                    let insert = format!("\n\n{content}");
                    self.splice_text(base, self.text.len(), &insert);
                    base + 2
                }
            }
        };

        self.cursor = block_start + cursor;
        let start_line = self.text[..block_start].matches('\n').count();
        let end_line = start_line + content.matches('\n').count();
        self.scroll_lines_into_view(start_line, end_line);
    }
}

#[cfg(test)]
mod tests;
