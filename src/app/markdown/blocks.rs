//! 统一块扫描：一次性识别代码围栏与顶层 `&&&`，再识别 `&&&` 内部的 `+++`。
//!
//! 纯函数、无 IO。行角色用 `BlockKind` + `Start/Middle/End` 表示；`&&&` 内的 `+++`
//! 覆盖父块角色。未闭合块延伸到其合法父范围末行。

use super::supple::{parse_supple_end_line, parse_supple_start_line};
use super::template::{
    TemplateStatus, is_title_line, parse_template_end_line, parse_template_start_line,
    parse_worktree_line,
};

/// 块类型：任务块 `&&&`、补充子块 `+++`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Template,
    Supple,
}

/// 块内行角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockRole {
    Start,
    Middle,
    End,
}

/// 单行块归属信息（`start`/`end` 为所属块的逻辑行索引，端点含）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockLineInfo {
    pub kind: BlockKind,
    pub role: BlockRole,
    pub start: usize,
    pub end: usize,
    /// `&&&` 状态；`+++` 恒为 `Todo`。
    pub status: TemplateStatus,
    /// `&&&` 结构分割行（标题下或绑定行上的空行）；其余块恒为 false。
    pub separator: bool,
    /// 所属块是否已闭合。
    pub closed: bool,
}

/// 一个已识别的块范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSpan {
    pub kind: BlockKind,
    pub start: usize,
    pub end: usize,
    pub status: TemplateStatus,
    /// 是否已闭合；未闭合块不产生 End 角色。
    pub closed: bool,
}

/// 一次全文扫描结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockScan {
    pub infos: Vec<Option<BlockLineInfo>>,
    pub spans: Vec<BlockSpan>,
}

/// 光标所处的块上下文；决定斜杠命令的可见性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockContext {
    /// 普通正文（不在任何块内）。
    Normal,
    /// `&&&` 任务块内（不含 `+++`）。
    Template,
    /// `+++` 补充子块内。
    Supple,
}

/// 扫描全文，返回每行块归属与块范围。
pub fn block_scan(text: &str) -> BlockScan {
    let lines: Vec<&str> = text.split('\n').collect();
    let infos = vec![None; lines.len()];
    let mut spans: Vec<BlockSpan> = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut template: Option<usize> = None;
    let mut supple: Option<usize> = None;

    for (index, line) in lines.iter().enumerate() {
        if let Some((marker, length)) = fence {
            if fence_delimiter(line).is_some_and(|(m, l)| m == marker && l >= length) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = fence_delimiter(line) {
            fence = Some(open);
            continue;
        }
        if let Some(start) = template {
            if let Some(end) = parse_template_end_line(line) {
                spans.push(BlockSpan {
                    kind: BlockKind::Template,
                    start,
                    end: index,
                    status: end.status,
                    closed: true,
                });
                template = None;
                supple = None;
            } else if let Some(s_start) = supple {
                if parse_supple_end_line(line).is_some() {
                    spans.push(BlockSpan {
                        kind: BlockKind::Supple,
                        start: s_start,
                        end: index,
                        status: TemplateStatus::Todo,
                        closed: true,
                    });
                    supple = None;
                }
            } else if parse_supple_start_line(line).is_some() {
                supple = Some(index);
            }
        } else if parse_template_start_line(line).is_some() {
            template = Some(index);
        }
    }

    let last = lines.len().saturating_sub(1);
    if let Some(start) = template {
        spans.push(BlockSpan {
            kind: BlockKind::Template,
            start,
            end: last,
            status: TemplateStatus::Todo,
            closed: false,
        });
    }
    if let Some(start) = supple {
        spans.push(BlockSpan {
            kind: BlockKind::Supple,
            start,
            end: last,
            status: TemplateStatus::Todo,
            closed: false,
        });
    }

    let mut scan = BlockScan { infos, spans };
    fill_roles(&mut scan);
    mark_separators(&lines, &mut scan);
    scan
}

/// 逐行块归属（便捷入口）。
pub fn block_line_infos(text: &str) -> Vec<Option<BlockLineInfo>> {
    block_scan(text).infos
}

/// 按固定优先级填充行归属：`&&&` 最外，`+++` 覆盖。
fn fill_roles(scan: &mut BlockScan) {
    for kind in [BlockKind::Template, BlockKind::Supple] {
        let spans: Vec<BlockSpan> = scan
            .spans
            .iter()
            .copied()
            .filter(|span| span.kind == kind)
            .collect();
        for span in spans {
            for line in span.start..=span.end {
                let Some(slot) = scan.infos.get_mut(line) else {
                    continue;
                };
                let role = if line == span.start {
                    BlockRole::Start
                } else if span.closed && line == span.end {
                    BlockRole::End
                } else {
                    BlockRole::Middle
                };
                *slot = Some(BlockLineInfo {
                    kind,
                    role,
                    start: span.start,
                    end: span.end,
                    status: span.status,
                    separator: false,
                    closed: span.closed,
                });
            }
        }
    }
}

/// 标记块结构分割行：标题行之下或 `{wt:}` 绑定行之上的块内空行。
fn mark_separators(lines: &[&str], scan: &mut BlockScan) {
    for index in 0..scan.infos.len() {
        let Some(info) = scan.infos[index] else {
            continue;
        };
        if info.role != BlockRole::Middle
            || !lines.get(index).is_some_and(|line| line.trim().is_empty())
        {
            continue;
        }
        let after_title = index > 0
            && scan
                .infos
                .get(index - 1)
                .copied()
                .flatten()
                .is_some_and(|prev| {
                    prev.kind == info.kind
                        && lines.get(index - 1).is_some_and(|line| is_title_line(line))
                });
        let before_binding = scan
            .infos
            .get(index + 1)
            .copied()
            .flatten()
            .is_some_and(|next| {
                next.kind == info.kind
                    && lines
                        .get(index + 1)
                        .is_some_and(|line| parse_worktree_line(line).is_some())
            });
        if (after_title || before_binding)
            && let Some(Some(slot)) = scan.infos.get_mut(index)
        {
            slot.separator = true;
        }
    }
}

/// 光标处所在行（按换行计数）。
fn line_index(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count()
}

/// offset 所在行的块归属；不在块内返回 None。
pub fn block_at(text: &str, offset: usize) -> Option<BlockLineInfo> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let line = line_index(text, offset);
    block_line_infos(text).get(line).copied().flatten()
}

/// offset 的块上下文。
pub fn block_context(text: &str, offset: usize) -> BlockContext {
    match block_at(text, offset).map(|info| info.kind) {
        Some(BlockKind::Template) => BlockContext::Template,
        Some(BlockKind::Supple) => BlockContext::Supple,
        None => BlockContext::Normal,
    }
}

/// offset 所在的块范围；不在块内返回 None。
pub fn block_span_at(text: &str, offset: usize) -> Option<BlockSpan> {
    let info = block_at(text, offset)?;
    Some(BlockSpan {
        kind: info.kind,
        start: info.start,
        end: info.end,
        status: info.status,
        closed: info.closed,
    })
}

/// 全部块范围（文档顺序，按结束行）。
pub fn block_spans(text: &str) -> Vec<BlockSpan> {
    block_scan(text).spans
}

/// 指定逻辑行所在的块范围。
pub fn block_span_at_line(text: &str, line: usize) -> Option<BlockSpan> {
    let info = block_scan(text).infos.get(line).copied().flatten()?;
    Some(BlockSpan {
        kind: info.kind,
        start: info.start,
        end: info.end,
        status: info.status,
        closed: info.closed,
    })
}

/// 删除整个块并最多吸收一侧空行的字节范围：优先吞掉块后一个换行与其后的空行，
/// 文末块则吞掉块前一个换行。
pub fn block_deletion_range(text: &str, span: BlockSpan) -> std::ops::Range<usize> {
    let mut from = line_start_at(text, span.start);
    let mut to = line_end_at(text, span.end);
    if to < text.len() {
        to += 1;
        if text[to..].starts_with('\n') {
            to += 1;
        }
    } else if from > 0 {
        from -= 1;
        if from > 0 && text[..from].ends_with('\n') {
            from -= 1;
        }
    }
    from..to
}

/// 逻辑行起始字节偏移（越界返回文末）。
pub(super) fn line_start_at(text: &str, line: usize) -> usize {
    let mut start = 0;
    for (index, content) in text.split('\n').enumerate() {
        if index == line {
            return start;
        }
        start += content.len() + 1;
    }
    text.len()
}

/// 逻辑行结束字节偏移（不含换行；越界返回文末）。
pub(super) fn line_end_at(text: &str, line: usize) -> usize {
    let mut start = 0;
    for (index, content) in text.split('\n').enumerate() {
        if index == line {
            return start + content.len();
        }
        start += content.len() + 1;
    }
    text.len()
}

/// 代码围栏定界行：至多 3 个前导空格后的至少 3 个同种 `` ` ``/`~`。
fn fence_delimiter(line: &str) -> Option<(u8, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = trimmed.bytes().next()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let length = trimmed.bytes().take_while(|byte| *byte == marker).count();
    (length >= 3).then_some((marker, length))
}
