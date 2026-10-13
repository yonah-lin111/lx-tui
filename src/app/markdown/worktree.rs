//! 块（`&&&` 任务块与 `+++` 补充块）的 git worktree 绑定：块底部独立 `{wt:<branch>}` 行的
//! 读写、候选解析与编辑保护。
//!
//! 绑定只存在于独立的绑定行：结束行的上一行——即块内最后一行，上方固定空一行
//! （渲染为块边框分割线，与标题行和正文之间的一致）。`{wt:}` 保存完整分支名，
//! 解析时对同一仓库的 worktree 列表要求唯一匹配，detached/bare 条目永不入选。
//! 绑定行是结构性元数据：编辑器禁止局部删除或修改。

use super::blocks::{BlockKind, block_scan};
use super::template::parse_worktree_line;

/// 候选 worktree：展示用分支名与 checkout 路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCandidate {
    /// 完整分支短名（`refs/heads/` 已剥离）。
    pub branch: String,
    pub path: std::path::PathBuf,
    /// 是否为主 checkout（列表首项）。
    pub is_main: bool,
}

/// 绑定解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeBinding {
    /// 未绑定 `{wt:}`。
    None,
    /// 已绑定且唯一解析到 checkout。
    Bound(WorktreeCandidate),
    /// 绑定的分支在列表中不存在。
    Missing(String),
    /// 多个 checkout 使用同一分支。
    Ambiguous(String),
}

/// 从候选列表解析分支绑定；列表只含非 detached/bare 条目。
pub fn resolve_binding(candidates: &[WorktreeCandidate], branch: &str) -> WorktreeBinding {
    let matches: Vec<&WorktreeCandidate> = candidates
        .iter()
        .filter(|candidate| candidate.branch == branch)
        .collect();
    match matches.as_slice() {
        [] => WorktreeBinding::Missing(branch.to_string()),
        [only] => WorktreeBinding::Bound((*only).clone()),
        _ => WorktreeBinding::Ambiguous(branch.to_string()),
    }
}

/// 结束行所在的块绑定行：`&&&` 或 `+++` 的结束行上一行（且大于起始行）；否则 None。
/// 返回（块类型, 起始行, 绑定行）。
fn binding_line_for(text: &str, end_line: usize) -> Option<(BlockKind, usize, usize)> {
    let span = block_scan(text).spans.into_iter().find(|span| {
        span.end == end_line
            && span.closed
            && matches!(span.kind, BlockKind::Template | BlockKind::Supple)
    })?;
    let line = end_line.checked_sub(1).filter(|line| *line > span.start)?;
    Some((span.kind, span.start, line))
}

/// 光标所在块（`&&&`/`+++`）的结束行索引；块未闭合或不在块内时返回 None。
pub fn block_end_line_at(text: &str, cursor: usize) -> Option<usize> {
    let info = super::block_at(text, cursor)?;
    if !info.closed {
        return None;
    }
    matches!(info.kind, BlockKind::Template | BlockKind::Supple).then_some(info.end)
}

/// 光标所在块的绑定：返回（绑定行索引, 分支）；无块结构或未绑定返回 None。
pub fn block_worktree_branch(text: &str, cursor: usize) -> Option<(usize, String)> {
    let end = block_end_line_at(text, cursor)?;
    block_worktree_branch_at_line(text, end)
}

/// 指定结束行的块绑定：返回（绑定行索引, 分支）；行号越界或无绑定返回 None。
pub fn block_worktree_branch_at_line(text: &str, end_line: usize) -> Option<(usize, String)> {
    let (_, _, line) = binding_line_for(text, end_line)?;
    let branch = parse_worktree_line(nth_line(text, line)?)?;
    Some((line, branch.to_string()))
}

/// 把结束行所在块的绑定写为 `branch`：块底部已有绑定行、或 `occupy` 指定的
/// 命令行恰占该行时整行替换，否则在结束行之前插入一行；绑定行上方固定为一条
/// 分割线（空行被其替换，已是分割线时不重复补）。
///
/// `occupy` 为占用绑定位置的行号（`/gitWorkTree` 命令行所在行），不占用传 None。
/// 返回（替换范围, 新文本）。
pub fn set_line_worktree(
    text: &str,
    end_line: usize,
    branch: &str,
    occupy: Option<usize>,
) -> Option<(std::ops::Range<usize>, String)> {
    let (_, start, slot) = binding_line_for(text, end_line)?;
    let start_line = nth_line(text, start)?;
    let indent = &start_line[..start_line.len() - start_line.trim_start_matches(' ').len()];
    let marker = format!("{indent}{{wt:{branch}}}");
    let (range, current) = nth_line_range(text, slot)?;
    if parse_worktree_line(current).is_some() || occupy == Some(slot) {
        let gap = gap_before(text, slot);
        return Some((range, format!("{gap}{marker}")));
    }
    let (range, _) = nth_line_range(text, end_line)?;
    // 插入到结束行之前：原末行非空时其前补一个空行（渲染为分割线）。
    let gap = gap_before(text, end_line);
    Some((range.start..range.start, format!("{gap}{marker}\n")))
}

/// 绑定行上方需要的空行：其上一行非空且自身不是首个位置时返回 `"\n"`。
fn gap_before(text: &str, line: usize) -> &'static str {
    let Some(previous) = line.checked_sub(1) else {
        return "";
    };
    if nth_line(text, previous).is_some_and(|above| !above.trim().is_empty()) {
        "\n"
    } else {
        ""
    }
}

/// 移除结束行所在块的绑定：删除绑定行与其上方空行（若在）；无绑定返回 None。
///
/// 返回（替换范围, 新文本）。
pub fn clear_line_worktree(
    text: &str,
    end_line: usize,
) -> Option<(std::ops::Range<usize>, String)> {
    let (_, _, line) = binding_line_for(text, end_line)?;
    parse_worktree_line(nth_line(text, line)?)?;
    let (range, _) = nth_line_range(text, line)?;
    let to = if text[range.end..].starts_with('\n') {
        range.end + 1
    } else {
        range.end
    };
    let mut from = range.start;
    if let Some(above) = line.checked_sub(1)
        && nth_line(text, above).is_some_and(|line| line.trim().is_empty())
    {
        from = nth_line_range(text, above)?.0.start;
    }
    Some((from..to, String::new()))
}

/// 受保护绑定行的字节范围（整行内容，不含换行）：`{wt:}` 独占一行且为块内末行。
pub fn worktree_line_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut ranges = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in &lines {
        let range = offset..offset + line.len();
        offset = range.end + 1;
        ranges.push(range);
    }
    let mut spans = Vec::new();
    for span in block_scan(text).spans {
        if !span.closed || !matches!(span.kind, BlockKind::Template | BlockKind::Supple) {
            continue;
        }
        let Some(marker_line) = span.end.checked_sub(1).filter(|line| *line > span.start) else {
            continue;
        };
        let Some(candidate) = lines.get(marker_line) else {
            continue;
        };
        if parse_worktree_line(candidate).is_none() {
            continue;
        }
        if let Some(range) = ranges.get(marker_line) {
            spans.push(range.clone());
        }
    }
    spans
}

/// 目标编辑是否触碰受保护的绑定行：局部修改/删除（含相邻换行）被拒绝；
/// 仅当区间完整包住整行（两端都超出）时才放行，如清空全文或删除整个块。
pub fn edit_touches_worktree_line(text: &str, from: usize, to: usize) -> bool {
    let insertion = from == to;
    for span in worktree_line_spans(text) {
        if insertion {
            if from >= span.start && from <= span.end {
                return true;
            }
            continue;
        }
        if from < span.start && to > span.end {
            continue;
        }
        if from < span.end && to > span.start {
            return true;
        }
        if to == span.start || from == span.end {
            return true;
        }
    }
    false
}

/// 第 `line` 行的字节范围；越界返回 None。
fn nth_line_range(text: &str, line: usize) -> Option<(std::ops::Range<usize>, &str)> {
    let mut offset = 0;
    for (index, segment) in text.split('\n').enumerate() {
        let range = offset..offset + segment.len();
        if index == line {
            return Some((range, segment));
        }
        offset = range.end + 1;
    }
    None
}

/// 第 `line` 行的文本；越界返回 None。
fn nth_line(text: &str, line: usize) -> Option<&str> {
    text.split('\n').nth(line)
}
