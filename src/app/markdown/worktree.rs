//! 模板块的 git worktree 绑定：结束行 `{wt:<branch>}` 的读写与候选解析。
//!
//! 绑定只存在于结束行：不复制状态到别处。`{wt:}` 保存完整分支名，
//! 解析时对同一仓库的 worktree 列表要求唯一匹配，detached/bare 条目永不入选。

use super::template::{parse_template_block_at_line, parse_template_end_line};

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

/// 结束行第 `line` 行是否有 `{wt:}` 绑定；无块结构或未绑定时 None。
pub fn block_worktree_branch(text: &str, cursor: usize) -> Option<(usize, String)> {
    let start = super::template_block_start_at(text, cursor)?;
    let range = parse_template_block_at_line(text, start)?;
    let line = nth_line(text, range.end)?;
    let parsed = parse_template_end_line(line)?;
    Some((range.end, parsed.wt?.to_string()))
}

/// 指定结束行的 `{wt:}` 绑定；行号越界或无绑定时 None。
pub fn block_worktree_branch_at_line(text: &str, line: usize) -> Option<(usize, String)> {
    let current = nth_line(text, line)?;
    let parsed = parse_template_end_line(current)?;
    Some((line, parsed.wt?.to_string()))
}

/// 光标所在块的结束行索引；不在块内返回 None。
pub fn block_end_line_at(text: &str, cursor: usize) -> Option<usize> {
    let start = super::template_block_start_at(text, cursor)?;
    parse_template_block_at_line(text, start).map(|range| range.end)
}

/// 把某行结束行的 `{wt:}` 更新为 `branch`（保留 command、状态与 `{id:}`）。
///
/// 返回 `(替换范围, 新行文本)`；行不是模板结束行时返回 None。
pub fn set_line_worktree(
    text: &str,
    line: usize,
    branch: &str,
) -> Option<(std::ops::Range<usize>, String)> {
    let (range, current) = nth_line_range(text, line)?;
    let parsed = parse_template_end_line(current)?;
    let mut out = String::from(parsed.indent);
    out.push_str("&&&");
    if let Some(command) = parsed.command {
        out.push(' ');
        out.push_str(command);
    }
    out.push_str(" --end");
    out.push_str(parsed.status.suffix());
    if let Some(id) = parsed.id {
        out.push_str(" {id:");
        out.push_str(id);
        out.push('}');
    }
    out.push_str(" {wt:");
    out.push_str(branch);
    out.push('}');
    Some((range, out))
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
