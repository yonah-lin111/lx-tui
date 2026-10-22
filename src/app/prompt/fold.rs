//! `+++` 补充块折叠：从统一块扫描派生隐藏区间，维护展开键。
//!
//! 折叠是纯展示状态：不写正文、不置脏；展开键是起始行行首字节偏移，
//! 随正文编辑平移，并随撤销快照与视图历史往返。默认全部折叠。
//! 只有闭合的补充块可折叠；未闭合块保留逐行可见，避免吞掉后续内容。

use std::collections::BTreeSet;
use std::ops::Range;

use super::super::markdown::{self, BlockKind, BlockScan, is_title_line};
use super::Prompt;

/// 一个折叠的补充块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedBlock {
    /// 起始逻辑行（保持可见）。
    pub start_line: usize,
    /// 结束逻辑行。
    pub end_line: usize,
    /// 首个隐藏逻辑行：有独立标题行时从起始行 + 2 开始，标题行保持可见。
    pub hidden_from_line: usize,
    /// 隐藏字节区间：最后一个可见行行尾换行至结束行行尾换行之后。
    pub hidden: Range<usize>,
    /// 展开键：起始行行首字节偏移。
    pub key: usize,
}

/// 全文逻辑行行首字节偏移（与 `split('\n')` 一一对应）。
fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        offsets.push(offset);
        offset += line.len() + 1;
    }
    offsets
}

/// 由一次块扫描派生未展开的闭合补充块（按原文顺序）。
fn folded_in(scan: &BlockScan, text: &str, expanded: &BTreeSet<usize>) -> Vec<FoldedBlock> {
    let lines: Vec<&str> = text.split('\n').collect();
    let offsets = line_offsets(text);
    let mut blocks = Vec::new();
    for span in scan
        .spans
        .iter()
        .filter(|span| span.kind == BlockKind::Supple && span.closed)
    {
        let Some(&key) = offsets.get(span.start) else {
            continue;
        };
        if expanded.contains(&key) {
            continue;
        }
        // 独立标题行保持可见，其余内容整体隐藏；换行本身随隐藏区一起收走，
        // 避免删除隐藏区与可见行之间的换行。
        let hidden_from_line = if lines
            .get(span.start + 1)
            .is_some_and(|line| is_title_line(line))
        {
            span.start + 2
        } else {
            span.start + 1
        };
        let Some(&next_line) = offsets.get(hidden_from_line) else {
            continue;
        };
        let hidden_start = next_line.saturating_sub(1);
        let hidden_end = offsets.get(span.end + 1).copied().unwrap_or(text.len());
        if hidden_start >= hidden_end {
            continue;
        }
        blocks.push(FoldedBlock {
            start_line: span.start,
            end_line: span.end,
            hidden_from_line,
            hidden: hidden_start..hidden_end,
            key,
        });
    }
    blocks
}

impl Prompt {
    /// 当前折叠的补充块（按原文顺序）。
    pub fn folded_blocks(&self) -> Vec<FoldedBlock> {
        let scan = markdown::block_scan(&self.text);
        folded_in(&scan, &self.text, &self.expanded_supplements)
    }

    /// 起始逻辑行是否为折叠的补充块；非闭合块恒为 false。
    pub fn supple_folded_at(&self, start_line: usize) -> bool {
        self.folded_blocks()
            .iter()
            .any(|block| block.start_line == start_line)
    }

    /// 切换补充块折叠；返回是否命中闭合补充块起始行。
    ///
    /// 不修改正文、不入撤销历史；收起时把落在隐藏区内的光标归一到起始行行尾。
    pub fn toggle_supple_fold(&mut self, start_line: usize) -> bool {
        let Some(&key) = line_offsets(&self.text).get(start_line) else {
            return false;
        };
        let closed = markdown::block_scan(&self.text)
            .spans
            .iter()
            .any(|span| span.kind == BlockKind::Supple && span.closed && span.start == start_line);
        if !closed {
            return false;
        }
        self.break_group();
        if !self.expanded_supplements.remove(&key) {
            self.expanded_supplements.insert(key);
        }
        self.normalize_fold_cursor();
        self.settle();
        true
    }

    /// 正文区间被替换后平移展开键；键落入被删区间时丢弃。
    pub(super) fn shift_folds(&mut self, at: usize, removed: usize, inserted: usize) {
        if self.expanded_supplements.is_empty() {
            return;
        }
        let end = at.saturating_add(removed);
        let mut shifted = BTreeSet::new();
        for key in std::mem::take(&mut self.expanded_supplements) {
            if key < at {
                shifted.insert(key);
            } else if key >= end {
                shifted.insert(key - removed + inserted);
            }
        }
        self.expanded_supplements = shifted;
    }

    /// 光标落在折叠区内部时归一到起始行行尾（可见位置）。
    pub(super) fn normalize_fold_cursor(&mut self) {
        let Some(target) = self
            .folded_blocks()
            .into_iter()
            .find(|block| self.cursor >= block.hidden.start && self.cursor < block.hidden.end)
            .map(|block| block.hidden.start)
        else {
            return;
        };
        self.cursor = target;
    }

    /// 光标位于折叠块起始行（行首除外）或隐藏区内部时，回车落点改为整块之后。
    ///
    /// 行首回车仍按普通编辑在块前插入空行；起始行内换行会把结构标记拆开，因此统一后移。
    /// 返回 `(插入位置, 回车后的光标位置)`；不在折叠块上时返回 None。
    pub(super) fn fold_newline_target(&self, cursor: usize) -> Option<(usize, usize)> {
        let block = self
            .folded_blocks()
            .into_iter()
            .find(|block| cursor > block.key && cursor < block.hidden.end)?;
        let offsets = line_offsets(&self.text);
        match offsets.get(block.end_line + 1) {
            // 块后已有下一行：在下一行行首插入一个空行，光标停在该空行。
            Some(&at) => Some((at, at)),
            // 块是最后一行：追加换行，光标落在新行行首。
            None => {
                let at = self.text.len();
                Some((at, at + 1))
            }
        }
    }

    /// 编辑区间是否与折叠区相交（写入拒绝判定）。
    pub(super) fn edit_hits_fold(&self, from: usize, to: usize) -> bool {
        self.folded_blocks().iter().any(|block| {
            if from == to {
                from >= block.hidden.start && from < block.hidden.end
            } else {
                from < block.hidden.end && to > block.hidden.start
            }
        })
    }

    /// 把落在折叠区内部的字符偏移吸附到可见一侧。
    ///
    /// `forward` 为真时吸附到块后（右移/正向按词），否则吸附到起始行行尾（左移/反向按词）。
    pub(super) fn snap_visible(&self, at: usize, forward: bool) -> usize {
        let mut at = at;
        for block in self.folded_blocks() {
            if at >= block.hidden.start && at < block.hidden.end {
                at = if forward {
                    block.hidden.end
                } else {
                    block.hidden.start
                };
            }
        }
        at
    }
}

/// 折叠派生入口：供视口投影复用同一次块扫描。
pub(super) fn folded_blocks_from(
    scan: &BlockScan,
    text: &str,
    expanded: &BTreeSet<usize>,
) -> Vec<FoldedBlock> {
    folded_in(scan, text, expanded)
}

#[cfg(test)]
mod tests;
