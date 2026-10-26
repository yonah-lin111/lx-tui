//! `&&&` 模板块聚焦：范围派生、切换、进入/退出与光标落点。
//!
//! 只记块起始逻辑行；块范围每次用 [`markdown::block_scan`] 现算，
//! 视口过滤统一由 [`Prompt::visual_rows`](super::Prompt::visual_rows) 完成。

use super::super::markdown::{self, BlockKind, BlockScan};
use super::Prompt;

/// 块聚焦范围（逻辑行，端点含）。
pub(crate) type FocusRange = (usize, usize);

impl Prompt {
    /// 当前聚焦的顶层 `&&&` 块行区间；未进入模式或聚焦行已失效时为 None。
    pub(crate) fn focused_block_range(&self, scan: &BlockScan) -> Option<FocusRange> {
        let line = self.focus_block?;
        scan.spans
            .iter()
            .find(|span| span.kind == BlockKind::Template && span.start == line)
            .map(|span| (span.start, span.end))
    }

    /// 本页全部顶层 `&&&` 起始行（文档顺序，含未闭合块）。
    pub(super) fn template_starts(scan: &BlockScan) -> Vec<usize> {
        scan.spans
            .iter()
            .filter(|span| span.kind == BlockKind::Template)
            .map(|span| span.start)
            .collect()
    }

    /// 是否处于块聚焦模式（聚焦行仍有效）。
    pub fn block_focus_active(&self) -> bool {
        let scan = markdown::block_scan(&self.text);
        self.focused_block_range(&scan).is_some()
    }

    /// 进入块聚焦：光标所在块优先，否则第一个块；无块返回 false 且不改状态。
    pub fn enter_block_focus(&mut self) -> bool {
        let scan = markdown::block_scan(&self.text);
        let starts = Self::template_starts(&scan);
        let Some(&first) = starts.first() else {
            return false;
        };
        let cursor_line = line_of_offset(&self.text, self.cursor);
        let containing = scan.spans.iter().find(|span| {
            span.kind == BlockKind::Template && span.start <= cursor_line && cursor_line <= span.end
        });
        match containing {
            Some(span) => {
                self.focus_block = Some(span.start);
                self.focus_scroll_to_block(span.start);
            }
            None => {
                self.focus_block = Some(first);
                self.locate_focus_start(first);
            }
        }
        true
    }

    /// 退出块聚焦；返回是否原本处于模式中。
    pub fn exit_block_focus(&mut self) -> bool {
        let changed = self.focus_block.take().is_some();
        if changed {
            // 视口从「单块行集合」切回全文：重算滚动，避免光标/当前块被甩出视口。
            self.scroll_cursor_into_view();
        }
        changed
    }

    /// 切到相邻块：-1 上一个、+1 下一个；两端钳制不循环；返回是否移动。
    pub fn focus_adjacent_block(&mut self, delta: isize) -> bool {
        let scan = markdown::block_scan(&self.text);
        let starts = Self::template_starts(&scan);
        if starts.is_empty() {
            return false;
        }
        let current = self
            .focus_block
            .and_then(|focus| starts.iter().position(|start| *start == focus))
            .unwrap_or_else(|| {
                let cursor_line = line_of_offset(&self.text, self.cursor);
                scan.spans
                    .iter()
                    .find(|span| {
                        span.kind == BlockKind::Template
                            && span.start <= cursor_line
                            && cursor_line <= span.end
                    })
                    .and_then(|span| starts.iter().position(|start| *start == span.start))
                    .unwrap_or(0)
            });
        let target = (current as isize).saturating_add(delta);
        let target = target.clamp(0, starts.len() as isize - 1) as usize;
        if target == current {
            return false;
        }
        let start = starts[target];
        self.focus_block = Some(start);
        self.locate_focus_start(start);
        true
    }

    /// 删除块后的接续：下一个 → 上一个 → 退出（调用方保证删除前处于块模式）。
    pub(crate) fn focus_after_block_removed(&mut self, removed_start: usize) {
        let scan = markdown::block_scan(&self.text);
        let starts = Self::template_starts(&scan);
        // 删除后其后的块会顶到 `removed_start`，因此下一个块用 `>=`；
        // 用 `>` 会在两个块紧邻时跳过真正的下一个块。
        let next = starts
            .iter()
            .find(|start| **start >= removed_start)
            .copied();
        let previous = starts
            .iter()
            .rev()
            .find(|start| **start < removed_start)
            .copied();
        match next.or(previous) {
            Some(start) => {
                self.focus_block = Some(start);
                self.locate_focus_start(start);
            }
            None => self.focus_block = None,
        }
    }

    /// 编辑/撤销后按光标重定位；出块即退出。普通视图零开销。
    pub(super) fn reconcile_block_focus(&mut self) {
        let Some(focus) = self.focus_block else {
            return;
        };
        let scan = markdown::block_scan(&self.text);
        let starts = Self::template_starts(&scan);
        let cursor_line = line_of_offset(&self.text, self.cursor);
        let cursor_span = scan.spans.iter().find(|span| {
            span.kind == BlockKind::Template && span.start <= cursor_line && cursor_line <= span.end
        });
        if starts.contains(&focus) {
            match cursor_span {
                Some(span) if span.start == focus => {}
                Some(span) => self.focus_block = Some(span.start),
                None => self.focus_block = None,
            }
        } else {
            self.focus_block = cursor_span.map(|span| span.start);
        }
        // 聚焦范围可能刚切换或失效；视口行集合随之变化，重新保证光标可见。
        self.scroll_cursor_into_view();
    }

    /// 把光标落到块标题值（无标题落正文首行、正文为空落起始行）并保持可见。
    fn locate_focus_start(&mut self, start: usize) {
        self.break_group();
        self.cursor = title_value_offset(&self.text, start);
        self.focus_scroll_to_block(start);
    }

    /// 光标进入视口后，把滚动钳到不超过块首视觉行，保证块顶可见。
    fn focus_scroll_to_block(&mut self, start: usize) {
        self.scroll_cursor_into_view();
        let row = self
            .visual_rows()
            .iter()
            .position(|row| row.line == start)
            .unwrap_or(0);
        self.scroll = self.scroll.min(row);
    }
}

/// 光标偏移所在逻辑行（0 起）。
fn line_of_offset(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count()
}

/// 块起始行的标题值落点：`{title: …}` 的 `}` 前；无标题落正文首行行首，
/// 正文为空（下一行已是结束行或文末）落起始行行首。
pub(super) fn title_value_offset(text: &str, start_line: usize) -> usize {
    let mut offset = 0;
    for (line, content) in text.split('\n').enumerate() {
        if line != start_line {
            offset += content.len() + 1;
            continue;
        }
        let next_start = offset + content.len() + 1;
        let Some(next) = text
            .get(next_start..)
            .and_then(|rest| rest.split('\n').next())
        else {
            return offset;
        };
        if markdown::parse_title_line(next).is_some() {
            return next_start + next.find('}').unwrap_or(next.len());
        }
        if markdown::parse_template_end_line(next).is_some() {
            return offset;
        }
        return next_start;
    }
    text.len()
}
