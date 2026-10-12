//! 模板块的 git worktree 绑定：块顶部独立 `{wt:<branch>}` 行的读写、
//! 候选解析与编辑保护。
//!
//! 绑定只存在于独立的绑定行：起始行下一行（有标题行时为标题行的下一行）——
//! 即块顶部第二行。`{wt:}` 保存完整分支名，解析时对同一仓库的 worktree 列表
//! 要求唯一匹配，detached/bare 条目永不入选。绑定行是结构性元数据：
//! 编辑器禁止局部删除或修改。

use super::template::{
    is_title_line, parse_template_end_line, parse_template_start_line, parse_worktree_line,
};

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

/// 块顶部绑定行位置：起始行下一行；该行为标题行时顺延一行。
fn binding_line_of(text: &str, start_line: usize) -> Option<usize> {
    let after_start = start_line.checked_add(1)?;
    if nth_line(text, after_start).is_some_and(is_title_line) {
        return after_start.checked_add(1);
    }
    Some(after_start)
}

/// 结束行上方最近的模板块起始行；无起始行返回 None。
fn block_start_before(text: &str, end_line: usize) -> Option<usize> {
    (0..end_line).rev().find(|index| {
        nth_line(text, *index).is_some_and(|line| parse_template_start_line(line).is_some())
    })
}

/// 指定结束行的块绑定：返回（绑定行索引, 分支）；无绑定返回 None。
fn binding_at_end_line(text: &str, end_line: usize) -> Option<(usize, String)> {
    parse_template_end_line(nth_line(text, end_line)?)?;
    let start = block_start_before(text, end_line)?;
    let line = binding_line_of(text, start)?;
    if line >= end_line {
        return None;
    }
    let branch = parse_worktree_line(nth_line(text, line)?)?;
    Some((line, branch.to_string()))
}

/// 光标所在块的绑定：返回（绑定行索引, 分支）；无块结构或未绑定返回 None。
pub fn block_worktree_branch(text: &str, cursor: usize) -> Option<(usize, String)> {
    let start = super::template_block_start_at(text, cursor)?;
    let line = binding_line_of(text, start)?;
    let branch = parse_worktree_line(nth_line(text, line)?)?;
    Some((line, branch.to_string()))
}

/// 指定结束行的块绑定：返回（绑定行索引, 分支）；行号越界或无绑定返回 None。
pub fn block_worktree_branch_at_line(text: &str, end_line: usize) -> Option<(usize, String)> {
    binding_at_end_line(text, end_line)
}

/// 光标所在块的结束行索引；块未闭合（末行非结束行）时返回 None。
pub fn block_end_line_at(text: &str, cursor: usize) -> Option<usize> {
    let start = super::template_block_start_at(text, cursor)?;
    let range = super::template::parse_template_block_at_line(text, start)?;
    parse_template_end_line(nth_line(text, range.end)?)?;
    Some(range.end)
}

/// 把结束行所在块的绑定写为 `branch`：块顶部已有绑定行则整行替换，
/// 否则在绑定位置插入一行；绑定行下方保留一个空行分隔元数据与正文。
///
/// 返回（替换范围, 新文本）：插入时范围是绑定位置的空区间，替换时覆盖
/// 绑定行整行（含换行）；新文本自带绑定行的换行。
pub fn set_line_worktree(
    text: &str,
    end_line: usize,
    branch: &str,
) -> Option<(std::ops::Range<usize>, String)> {
    parse_template_end_line(nth_line(text, end_line)?)?;
    let start = block_start_before(text, end_line)?;
    let indent = parse_template_start_line(nth_line(text, start)?)?.indent;
    let marker_line = binding_line_of(text, start)?;
    let marker = format!("{indent}{{wt:{branch}}}");
    if marker_line < end_line
        && let Some(previous) = nth_line(text, marker_line)
        && parse_worktree_line(previous).is_some()
    {
        let (range, _) = nth_line_range(text, marker_line)?;
        let range = line_slot(text, range);
        return Some((range, binding_line_text(text, marker_line, &marker)));
    }
    let target = marker_line.min(end_line);
    let (range, _) = nth_line_range(text, target)?;
    // 插入的绑定行把原目标行下推：原行非空时补一个空行做分隔。
    let gap = follows_blank(text, target);
    Some((range.start..range.start, format!("{marker}\n{gap}")))
}

/// 行内容范围扩展为整行槽位：含尾随换行（末行无换行时保持原范围）。
fn line_slot(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    if text[range.end..].starts_with('\n') {
        range.start..range.end + 1
    } else {
        range
    }
}

/// 绑定行整行文本（含换行）加下方空行：紧随其后的原行非空时才补空行。
fn binding_line_text(text: &str, line: usize, marker: &str) -> String {
    format!("{marker}\n{}", follows_blank(text, line + 1))
}

/// 指定行非空时返回分隔用空行 `"\n"`；行缺失或已空返回空串。
fn follows_blank(text: &str, line: usize) -> &'static str {
    if nth_line(text, line).is_some_and(|next| !next.trim().is_empty()) {
        "\n"
    } else {
        ""
    }
}

/// 受保护绑定行的字节范围（整行内容，不含换行）：`{wt:}` 独占一行且位于模板块顶部。
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
    for (index, line) in lines.iter().enumerate() {
        if parse_template_start_line(line).is_none() {
            continue;
        }
        let Some(marker_line) = binding_line_of(text, index) else {
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
