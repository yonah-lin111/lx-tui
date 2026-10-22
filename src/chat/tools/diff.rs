//! 文件工具的结构化行差异：真实前后行快照 → hunks + 行号 + 精确增删计数。
//!
//! 契约（见 `docs/feature/lx-file-tool-diff/design.md` §4、§5）：
//!
//! - 文本行**不含**行尾换行，缩进原样保留；末行是否有换行由 `no_final_newline` 表达；
//! - 行号 1-based：`Added` 只有 `new_line`，`Removed` 只有 `old_line`，`Context` 两者都有；
//! - 上下文半径固定 1，相邻范围合并，远距范围保持独立；
//! - 计数来自**同一份编辑脚本**，预览/存储截断不改变计数；
//! - 超限输入退化为「公共前后缀外整段替换」的线性简化脚本，`simplified = true`，
//!   计数仍是该脚本的精确统计（不是最小编辑脚本）。
//!
//! 本模块是纯计算：不读文件、不写文件、不依赖 `app`/`ui`。

use similar::{ChangeTag, TextDiff};

/// 复杂差异输入总字节上限（old + new）；超限走线性简化脚本。
pub const MAX_DIFF_INPUT_BYTES: usize = 8 * 1024 * 1024;

/// 复杂差异输入总行数上限；超限走线性简化脚本（避免大量短行造成操作对象膨胀）。
pub const MAX_DIFF_INPUT_LINES: usize = 100_000;

/// 单文件差异算法超时配置（`similar` 的尽力限制，不是严格 CPU 截止）。
pub const DIFF_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// 单个文件最多保留的差异行对象；超出即 `Truncated`，计数不受影响。
pub const MAX_DIFF_LINES: usize = 10_000;

/// 上下文半径。
const CONTEXT_RADIUS: usize = 1;

/// 差异行种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// 未变化的上下文行。
    Context,
    /// 新增行。
    Added,
    /// 删除行。
    Removed,
}

impl DiffLineKind {
    /// 稳定 JSON 值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Added => "added",
            Self::Removed => "removed",
        }
    }

    /// 由稳定 JSON 值还原。
    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "context" => Some(Self::Context),
            "added" => Some(Self::Added),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

/// 一行差异。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// 行种类。
    pub kind: DiffLineKind,
    /// 旧文件 1-based 行号（`Added` 为 None）。
    pub old_line: Option<usize>,
    /// 新文件 1-based 行号（`Removed` 为 None）。
    pub new_line: Option<usize>,
    /// 行文本（不含行尾换行，缩进原样）。
    pub text: String,
    /// 该行是文件末行且文件没有结尾换行（元信息，不计入增删）。
    pub no_final_newline: bool,
}

/// 一个连续差异块（含半径 1 的上下文）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffHunk {
    /// 块内有序行。
    pub lines: Vec<DiffLine>,
}

/// 差异不可用的稳定原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffUnavailableReason {
    /// 内容含 NUL，判定为二进制。
    Binary,
    /// 内容不是合法 UTF-8。
    NotUtf8,
    /// 读取旧内容失败。
    ReadFailed,
}

impl DiffUnavailableReason {
    /// 稳定 JSON 值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::NotUtf8 => "not_utf8",
            Self::ReadFailed => "read_failed",
        }
    }

    /// 由稳定 JSON 值还原。
    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "binary" => Some(Self::Binary),
            "not_utf8" => Some(Self::NotUtf8),
            "read_failed" => Some(Self::ReadFailed),
            _ => None,
        }
    }
}

/// 一个文件的差异正文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDiff {
    /// 完整差异（空 hunks 表示没有内容差异）。
    Complete {
        /// 已保存的差异块。
        hunks: Vec<DiffHunk>,
        /// 是否来自线性简化脚本。
        simplified: bool,
    },
    /// 正文被行数预算截断；计数仍是完整脚本的精确值。
    Truncated {
        /// 已保存的差异块。
        hunks: Vec<DiffHunk>,
        /// 是否来自线性简化脚本。
        simplified: bool,
    },
    /// 无法生成文本差异（二进制/非 UTF-8/读取失败）。
    Unavailable {
        /// 稳定原因码。
        reason: DiffUnavailableReason,
    },
}

impl FileDiff {
    /// 空差异（没有内容变化）。
    pub fn unchanged() -> Self {
        Self::Complete {
            hunks: Vec::new(),
            simplified: false,
        }
    }

    /// 计数是否已知（不可用即未知）。
    pub fn counts_known(&self) -> bool {
        !matches!(self, Self::Unavailable { .. })
    }

    /// 是否使用线性简化脚本。
    pub fn simplified(&self) -> bool {
        matches!(
            self,
            Self::Complete {
                simplified: true,
                ..
            } | Self::Truncated {
                simplified: true,
                ..
            }
        )
    }

    /// 正文是否被截断。
    pub fn truncated(&self) -> bool {
        matches!(self, Self::Truncated { .. })
    }

    /// 已保存的 hunks（不可用时为空）。
    pub fn hunks(&self) -> &[DiffHunk] {
        match self {
            Self::Complete { hunks, .. } | Self::Truncated { hunks, .. } => hunks,
            Self::Unavailable { .. } => &[],
        }
    }
}

/// 一次差异生成的计数结果（与正文同源）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffCounts {
    /// 新增行数。
    pub added: usize,
    /// 删除行数。
    pub removed: usize,
    /// 正文是否被截断。
    pub truncated: bool,
    /// 是否来自线性简化脚本。
    pub simplified: bool,
}

/// 由真实旧/新行快照生成差异与完整计数。
///
/// `old_bytes`/`new_bytes` 是两侧原始文本字节数；与行数任一超限时走线性简化脚本。
/// 新建/删除文件分别等价于 old 为空 / new 为空。调用方必须提供**实际**写入的最终内容。
pub fn diff_lines(
    old: &[String],
    new: &[String],
    old_final_newline: bool,
    new_final_newline: bool,
    old_bytes: usize,
    new_bytes: usize,
) -> (FileDiff, DiffCounts) {
    let over_budget = old_bytes.saturating_add(new_bytes) > MAX_DIFF_INPUT_BYTES
        || old.len().saturating_add(new.len()) > MAX_DIFF_INPUT_LINES;
    if over_budget {
        return simplified_diff(old, new, old_final_newline, new_final_newline);
    }

    let old_refs: Vec<&str> = old.iter().map(String::as_str).collect();
    let new_refs: Vec<&str> = new.iter().map(String::as_str).collect();
    let diff = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_slices(&old_refs, &new_refs);

    let mut ops: Vec<Change> = diff
        .iter_all_changes()
        .map(|change| Change {
            tag: change.tag(),
            old_index: change.old_index(),
            new_index: change.new_index(),
            text: change.value().to_string(),
        })
        .collect();
    let mut added = ops.iter().filter(|op| op.tag == ChangeTag::Insert).count();
    let mut removed = ops.iter().filter(|op| op.tag == ChangeTag::Delete).count();

    // 只差结尾换行：`similar` 按行看不出变化，但这是真实变化，必须展示 EOF 元信息。
    if added == 0 && removed == 0 && old_final_newline != new_final_newline && !old.is_empty() {
        ops = eof_only_ops(old, new);
        added = 1;
        removed = 1;
    }

    let hunks = build_hunks(&ops, old, new, old_final_newline, new_final_newline);
    let (hunks, truncated) = truncate_hunks(hunks, MAX_DIFF_LINES);
    let diff = if truncated {
        FileDiff::Truncated {
            hunks,
            simplified: false,
        }
    } else {
        FileDiff::Complete {
            hunks,
            simplified: false,
        }
    };
    (
        diff,
        DiffCounts {
            added,
            removed,
            truncated,
            simplified: false,
        },
    )
}

/// 差异计算的一个原始操作（已从 `similar` 复制出文本，不再持有借用）。
struct Change {
    tag: ChangeTag,
    old_index: Option<usize>,
    new_index: Option<usize>,
    text: String,
}

/// 按半径 1 分组：相邻/重叠范围合并，远距范围独立。
fn build_hunks(
    ops: &[Change],
    old: &[String],
    new: &[String],
    old_final_newline: bool,
    new_final_newline: bool,
) -> Vec<DiffHunk> {
    let last = match ops.len().checked_sub(1) {
        Some(last) => last,
        None => return Vec::new(),
    };
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        if op.tag == ChangeTag::Equal {
            continue;
        }
        let start = index.saturating_sub(CONTEXT_RADIUS);
        let end = (index + CONTEXT_RADIUS).min(last);
        match ranges.last_mut() {
            // 重叠或紧邻时合并（否则只隔一行的两处改动会拆成两个 hunk）。
            Some((_, previous_end)) if start <= previous_end.saturating_add(1) => {
                *previous_end = (*previous_end).max(end);
            }
            _ => ranges.push((start, end)),
        }
    }
    if ranges.is_empty() {
        return Vec::new();
    }
    ranges
        .into_iter()
        .map(|(start, end)| DiffHunk {
            lines: ops[start..=end]
                .iter()
                .map(|op| line_of(op, old, new, old_final_newline, new_final_newline))
                .collect(),
        })
        .collect()
}

/// 单个操作 → 差异行（含真实行号与 EOF 标记）。
fn line_of(
    op: &Change,
    old: &[String],
    new: &[String],
    old_final_newline: bool,
    new_final_newline: bool,
) -> DiffLine {
    let old_last = op.old_index.is_some_and(|index| index + 1 == old.len());
    let new_last = op.new_index.is_some_and(|index| index + 1 == new.len());
    let no_final_newline = match op.tag {
        ChangeTag::Equal => (old_last && !old_final_newline) || (new_last && !new_final_newline),
        ChangeTag::Delete => old_last && !old_final_newline,
        ChangeTag::Insert => new_last && !new_final_newline,
    };
    DiffLine {
        kind: match op.tag {
            ChangeTag::Equal => DiffLineKind::Context,
            ChangeTag::Insert => DiffLineKind::Added,
            ChangeTag::Delete => DiffLineKind::Removed,
        },
        old_line: op.old_index.map(|index| index + 1),
        new_line: op.new_index.map(|index| index + 1),
        text: op.text.clone(),
        no_final_newline,
    }
}

/// 只保留前 `limit` 个差异行对象；返回正文与是否截断。
fn truncate_hunks(hunks: Vec<DiffHunk>, limit: usize) -> (Vec<DiffHunk>, bool) {
    let total: usize = hunks.iter().map(|hunk| hunk.lines.len()).sum();
    if total <= limit {
        return (hunks, false);
    }
    let mut remaining = limit;
    let mut kept = Vec::with_capacity(hunks.len());
    for mut hunk in hunks {
        if remaining == 0 {
            break;
        }
        if hunk.lines.len() <= remaining {
            remaining -= hunk.lines.len();
            kept.push(hunk);
        } else {
            hunk.lines.truncate(remaining);
            remaining = 0;
            kept.push(hunk);
        }
    }
    (kept, true)
}

/// 末行换行状态变化：把末行表达为一次删除 + 一次新增（附带前一行上下文）。
fn eof_only_ops(old: &[String], new: &[String]) -> Vec<Change> {
    let last = old.len().saturating_sub(1);
    let mut ops = Vec::with_capacity(3);
    if last > 0 {
        ops.push(Change {
            tag: ChangeTag::Equal,
            old_index: Some(last - 1),
            new_index: Some(last - 1),
            text: old[last - 1].clone(),
        });
    }
    ops.push(Change {
        tag: ChangeTag::Delete,
        old_index: Some(last),
        new_index: None,
        text: old[last].clone(),
    });
    ops.push(Change {
        tag: ChangeTag::Insert,
        old_index: None,
        new_index: Some(new.len().saturating_sub(1)),
        text: new.last().cloned().unwrap_or_default(),
    });
    ops
}

/// 超限输入的线性简化脚本：剥离公共前后缀，剩余旧区间全部删除、新区间全部新增。
fn simplified_diff(
    old: &[String],
    new: &[String],
    old_final_newline: bool,
    new_final_newline: bool,
) -> (FileDiff, DiffCounts) {
    let prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let max_suffix = old.len().min(new.len()).saturating_sub(prefix);
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(left, right)| left == right)
        .count();
    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];
    let has_change = !old_mid.is_empty()
        || !new_mid.is_empty()
        || (old_final_newline != new_final_newline && !old.is_empty());
    let mut lines = Vec::new();
    if prefix > 0 && has_change {
        lines.push(DiffLine {
            kind: DiffLineKind::Context,
            old_line: Some(prefix),
            new_line: Some(prefix),
            text: old[prefix - 1].clone(),
            no_final_newline: false,
        });
    }
    for (offset, text) in old_mid.iter().enumerate() {
        let line = prefix + offset + 1;
        lines.push(DiffLine {
            kind: DiffLineKind::Removed,
            old_line: Some(line),
            new_line: None,
            text: text.clone(),
            no_final_newline: !old_final_newline && line == old.len(),
        });
    }
    for (offset, text) in new_mid.iter().enumerate() {
        let line = prefix + offset + 1;
        lines.push(DiffLine {
            kind: DiffLineKind::Added,
            old_line: None,
            new_line: Some(line),
            text: text.clone(),
            no_final_newline: !new_final_newline && line == new.len(),
        });
    }
    if suffix > 0 && has_change {
        let index = old.len() - suffix;
        lines.push(DiffLine {
            kind: DiffLineKind::Context,
            old_line: Some(index + 1),
            new_line: Some(index + 1),
            text: old[index].clone(),
            no_final_newline: (index + 1 == old.len() && !old_final_newline)
                || (index + 1 == new.len() && !new_final_newline),
        });
    }
    // 只差 EOF 换行时上面不会产生任何行：补一次末行替换，计数才是精确的。
    let (mut added, mut removed) = (new_mid.len(), old_mid.len());
    if added == 0 && removed == 0 && old_final_newline != new_final_newline && !old.is_empty() {
        let last = old.len() - 1;
        lines.clear();
        if last > 0 {
            lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(last),
                new_line: Some(last),
                text: old[last - 1].clone(),
                no_final_newline: false,
            });
        }
        lines.push(DiffLine {
            kind: DiffLineKind::Removed,
            old_line: Some(last + 1),
            new_line: None,
            text: old[last].clone(),
            no_final_newline: !old_final_newline,
        });
        lines.push(DiffLine {
            kind: DiffLineKind::Added,
            old_line: None,
            new_line: Some(new.len()),
            text: new.last().cloned().unwrap_or_default(),
            no_final_newline: !new_final_newline,
        });
        added = 1;
        removed = 1;
    }
    let hunks = if lines.is_empty() {
        Vec::new()
    } else {
        vec![DiffHunk { lines }]
    };
    let (hunks, truncated) = truncate_hunks(hunks, MAX_DIFF_LINES);
    let diff = if truncated {
        FileDiff::Truncated {
            hunks,
            simplified: true,
        }
    } else {
        FileDiff::Complete {
            hunks,
            simplified: true,
        }
    };
    (
        diff,
        DiffCounts {
            added,
            removed,
            truncated,
            simplified: true,
        },
    )
}

#[cfg(test)]
mod tests;
