//! 补充块附件索引与显示投影：从一次块扫描派生，纯数据、无 IO。
//!
//! 原文是唯一事实来源；附件索引与显示行都是派生结果，不持久化。索引只收
//! 「父子均闭合、子块严格位于父块内部」的 `+++` 子块；围栏内、块外与未闭合区域
//! 保持原样可见，不因三个加号被强制附件化。

use std::ops::Range;

use super::super::markdown::{
    self, BlockKind, BlockLineInfo, BlockSpan, parse_supple_start_line, parse_title_line,
};
use super::text::char_width;

/// 一个可附件化的补充子块（逻辑行与字节区间均为派生值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplementEntry {
    /// 父 `&&&` 任务块的起始逻辑行；显示入口与焦点按它归属。
    pub parent_start: usize,
    /// 子块起始逻辑行。
    pub start: usize,
    /// 子块结束逻辑行。
    pub end: usize,
    /// 含起止标记行的字节区间（不含末尾换行）。
    pub range: Range<usize>,
    /// 列表展示标题；空标题使用 `Supplement N` 兜底。
    pub title: String,
}

/// 一次扫描得到的附件索引。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupplementIndex {
    pub entries: Vec<SupplementEntry>,
}

impl SupplementIndex {
    /// 父任务可附件化的子块数量。
    pub fn count_for(&self, parent_start: usize) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.parent_start == parent_start)
            .count()
    }

    /// 有安全子块的父任务（按原文顺序，去重）。
    pub fn parents(&self) -> Vec<usize> {
        let mut parents: Vec<usize> = Vec::new();
        for entry in &self.entries {
            if parents.last() != Some(&entry.parent_start) {
                parents.push(entry.parent_start);
            }
        }
        parents
    }

    /// 指定父任务下的附件（原文顺序）。
    pub fn entries_for(&self, parent_start: usize) -> Vec<&SupplementEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.parent_start == parent_start)
            .collect()
    }

    /// 全部隐藏字节区间：集中视图下这些内容不显示、不可直接编辑。
    pub fn hidden_ranges(&self) -> impl Iterator<Item = &Range<usize>> {
        self.entries.iter().map(|entry| &entry.range)
    }

    /// 缺少父任务或已不在索引中的下一个可聚焦入口。
    pub fn contains_parent(&self, parent_start: usize) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.parent_start == parent_start)
    }
}

/// 显示行：真实文本视觉行，或父任务末尾的合成附件入口行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDisplayRow {
    Text {
        line: usize,
        start: usize,
        end: usize,
    },
    Supplements {
        parent_start: usize,
        count: usize,
    },
}

impl PromptDisplayRow {
    /// 真实文本视觉行；合成入口为 None。
    pub fn text(self) -> Option<(usize, usize, usize)> {
        match self {
            Self::Text { line, start, end } => Some((line, start, end)),
            Self::Supplements { .. } => None,
        }
    }

    /// 所属逻辑行；合成入口返回父任务起始行（供样式与边框判断）。
    pub fn line(self) -> usize {
        match self {
            Self::Text { line, .. } => line,
            Self::Supplements { parent_start, .. } => parent_start,
        }
    }
}

/// 全文行起始字节偏移（`split('\n')` 一一对应）。
pub(crate) fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        offsets.push(offset);
        offset += line.len() + 1;
    }
    offsets
}

/// 从统一块扫描派生附件索引。
pub fn supplement_index(text: &str) -> SupplementIndex {
    let scan = markdown::block_scan(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let offsets = line_offsets(text);
    let templates: Vec<BlockSpan> = scan
        .spans
        .iter()
        .copied()
        .filter(|span| span.kind == BlockKind::Template && span.closed)
        .collect();
    let mut entries = Vec::new();
    for span in scan
        .spans
        .iter()
        .copied()
        .filter(|span| span.kind == BlockKind::Supple && span.closed)
    {
        let Some(parent) = templates
            .iter()
            .find(|parent| parent.start < span.start && span.end < parent.end)
        else {
            continue;
        };
        let (Some(&from), Some(&end_at)) = (offsets.get(span.start), offsets.get(span.end)) else {
            continue;
        };
        let to = end_at + lines.get(span.end).map_or(0, |line| line.len());
        entries.push(SupplementEntry {
            parent_start: parent.start,
            start: span.start,
            end: span.end,
            range: from..to,
            title: entry_title(&lines, span.start, entries.len()),
        });
    }
    entries.sort_by_key(|entry| entry.start);
    SupplementIndex { entries }
}

/// 子块标题：起始行内联标题优先，其次下一行独立 `{title: …}`；都为空时兜底序号。
fn entry_title(lines: &[&str], start: usize, ordinal: usize) -> String {
    let inline = lines
        .get(start)
        .and_then(|line| parse_supple_start_line(line))
        .and_then(|parsed| parsed.title)
        .map(str::trim)
        .filter(|title| !title.is_empty());
    if let Some(title) = inline {
        return title.to_string();
    }
    let standalone = lines
        .get(start + 1)
        .and_then(|line| parse_title_line(line))
        .map(str::trim)
        .filter(|title| !title.is_empty());
    match standalone {
        Some(title) => title.to_string(),
        None => format!("Supplement {}", ordinal + 1),
    }
}

/// 原文模式下的显示行数（测试与几何断言共用）。
pub fn prompt_display_row_count(text: &str) -> usize {
    text.split('\n').count()
}

/// 一次投影构建结果：显示行与附件索引（共享同一份扫描，避免重复全文扫描）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub rows: Vec<PromptDisplayRow>,
    pub index: SupplementIndex,
}

/// 构建集中视图投影：隐藏安全子块与直接相邻的块内空行，每个父任务只留一个入口行。
///
/// `width` 为内容列宽；块行扣除左右边框槽后换行，与渲染/命中口径一致。
pub fn project_compact(text: &str, width: usize) -> Projection {
    let index = supplement_index(text);
    let scan = markdown::block_scan(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let offsets = line_offsets(text);
    let mut hidden = vec![false; lines.len()];
    for entry in &index.entries {
        for line in entry.start..=entry.end {
            if let Some(slot) = hidden.get_mut(line) {
                *slot = true;
            }
        }
    }
    collapse_adjacent_blanks(&lines, &scan.infos, &mut hidden);
    let mut rows = Vec::new();
    let parents = index.parents();
    let mut line_start = 0;
    for (line, segment) in lines.iter().enumerate() {
        if !hidden.get(line).copied().unwrap_or(false) {
            let info = scan.infos.get(line).copied().flatten();
            let line_end = line_start + segment.len();
            push_wrapped(&mut rows, line, line_start, line_end, segment, width, info);
        }
        line_start += segment.len() + 1;
    }
    // 入口统一落在父任务结束行之前：按逻辑行定位，保持原文顺序。
    let parent_ends: Vec<(usize, usize)> = scan
        .spans
        .iter()
        .filter(|span| span.kind == BlockKind::Template && span.closed)
        .map(|span| (span.start, span.end))
        .collect();
    for parent_start in parents.into_iter().rev() {
        let count = index.count_for(parent_start);
        let end_line = parent_ends
            .iter()
            .find(|(start, _)| *start == parent_start)
            .map_or(parent_start, |(_, end)| *end);
        let insert_at = row_before_line(&rows, &offsets, end_line);
        rows.insert(
            insert_at,
            PromptDisplayRow::Supplements {
                parent_start,
                count,
            },
        );
    }
    Projection { rows, index }
}

/// 隐藏区两侧直接相邻的块内空行一并收敛；其余用户空行不动。
fn collapse_adjacent_blanks(lines: &[&str], infos: &[Option<BlockLineInfo>], hidden: &mut [bool]) {
    for line in 0..hidden.len() {
        if hidden[line] {
            continue;
        }
        if !lines.get(line).is_some_and(|text| text.trim().is_empty()) {
            continue;
        }
        if infos.get(line).copied().flatten().is_none() {
            continue;
        }
        let before = line
            .checked_sub(1)
            .is_some_and(|line| hidden.get(line).copied().unwrap_or(false));
        let after = hidden.get(line + 1).copied().unwrap_or(false);
        if before || after {
            hidden[line] = true;
        }
    }
}

/// 父任务结束行之前的插入位置：该行首个视觉行；行被隐藏（异常数据）时退到末尾。
fn row_before_line(rows: &[PromptDisplayRow], offsets: &[usize], line: usize) -> usize {
    let Some(&offset) = offsets.get(line) else {
        return rows.len();
    };
    rows.iter()
        .position(|row| matches!(row, PromptDisplayRow::Text { start, .. } if *start >= offset))
        .unwrap_or(rows.len())
}

/// 按内容宽度把一个逻辑行切成视觉行（与原 `visual_rows` 口径一致）。
fn push_wrapped(
    rows: &mut Vec<PromptDisplayRow>,
    line: usize,
    line_start: usize,
    line_end: usize,
    segment: &str,
    width: usize,
    info: Option<BlockLineInfo>,
) {
    let width = if info.is_some() {
        width
            .saturating_sub(usize::from(
                crate::layout::PROMPT_TEMPLATE_GUTTER_LEFT
                    + crate::layout::PROMPT_TEMPLATE_GUTTER_RIGHT,
            ))
            .max(1)
    } else {
        width.max(1)
    };
    let mut row_start = line_start;
    let mut col = 0;
    for (offset, ch) in segment.char_indices() {
        let absolute = line_start + offset;
        let cell = char_width(ch);
        if col > 0 && col + cell > width {
            rows.push(PromptDisplayRow::Text {
                line,
                start: row_start,
                end: absolute,
            });
            row_start = absolute;
            col = 0;
        }
        col += cell;
    }
    rows.push(PromptDisplayRow::Text {
        line,
        start: row_start,
        end: line_end,
    });
}

/// 源文本模式（原文视图 / scratch）的投影：全部可见，不生成合成入口。
pub fn project_source(text: &str, width: usize) -> Projection {
    let scan = markdown::block_scan(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut rows = Vec::new();
    let mut line_start = 0;
    for (line, segment) in lines.iter().enumerate() {
        let info = scan.infos.get(line).copied().flatten();
        let line_end = line_start + segment.len();
        push_wrapped(&mut rows, line, line_start, line_end, segment, width, info);
        line_start = segment.len() + line_start + 1;
    }
    Projection {
        rows,
        index: supplement_index(text),
    }
}

/// 插入位置是否落在隐藏区内部（区间端点不算）。
pub fn insertion_in_hidden(index: &SupplementIndex, at: usize) -> bool {
    index
        .hidden_ranges()
        .any(|range| at > range.start && at < range.end)
}

/// 编辑区间是否与隐藏附件相交。
pub fn range_hits_hidden(index: &SupplementIndex, from: usize, to: usize) -> bool {
    if from == to {
        return insertion_in_hidden(index, from);
    }
    index
        .hidden_ranges()
        .any(|range| from < range.end && to > range.start)
}

#[cfg(test)]
mod tests;
