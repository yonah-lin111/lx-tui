//! 字母快捷输入：从正文已出现的 `@` 提及与 `@[refer-*]` 引用生成候选，纯函数无 IO。
//!
//! 触发词为光标前至少 2 个路径字符；候选只来自光标所在分段（模板块正文或块外正文）已出现的
//! 引用文本，不扫描文件系统、不产生新引用目标。

use std::ops::Range;

use super::{
    MentionEntry, ParsedReference, ReferKind, TemplateLineInfo, TemplateLineRole, parse_references,
    parse_references_in, template_line_infos,
};

/// 候选展示条数上限。
pub const REFER_LIMIT: usize = 100;

/// 快捷触发：触发词字节区间与查询串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferTrigger {
    pub range: Range<usize>,
    pub query: String,
}

/// 候选类型（tag 语义）：`@` 提及按目录信息区分 file/folder，引用按标记类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferTag {
    File,
    Folder,
    Refer(ReferKind),
}

impl ReferTag {
    /// 标签稳定名；UI 负责包裹方括号。
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Refer(ReferKind::File) => "refer-file",
            Self::Refer(ReferKind::Folder) => "refer-folder",
            Self::Refer(ReferKind::Image) => "refer-image",
        }
    }
}

/// 快捷候选：完整原文（确认时整体替换触发词）、匹配名称/路径与类型标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferCandidate {
    /// 完整原文；确认后按此写入。
    pub text: String,
    /// 名称（路径末段）。
    pub name: String,
    /// 匹配用完整路径（提及为相对路径，引用为解码路径）。
    pub path: String,
    pub tag: ReferTag,
    /// 首次出现顺序；同分排序用。
    pub order: usize,
}

/// 快捷面板：触发区间、候选与高亮。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferPanel {
    trigger: ReferTrigger,
    items: Vec<ReferCandidate>,
    active: usize,
}

impl ReferPanel {
    /// 组装面板；`items` 不可为空。
    pub fn new(trigger: ReferTrigger, items: Vec<ReferCandidate>) -> Self {
        Self {
            trigger,
            items,
            active: 0,
        }
    }

    /// 触发区间。
    pub fn trigger(&self) -> &ReferTrigger {
        &self.trigger
    }

    /// 候选。
    pub fn items(&self) -> &[ReferCandidate] {
        &self.items
    }

    /// 高亮索引。
    pub fn active(&self) -> usize {
        self.active
    }

    /// 当前高亮候选。
    pub fn active_candidate(&self) -> Option<&ReferCandidate> {
        self.items.get(self.active)
    }

    /// 直接设置高亮；越界钳到末项。
    pub fn set_active(&mut self, index: usize) {
        self.active = index.min(self.items.len().saturating_sub(1));
    }

    /// 按偏移循环移动高亮。
    pub fn move_active(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        self.active = (self.active as isize + delta).rem_euclid(len) as usize;
    }
}

/// 路径字符：字母数字（含 CJK）与常见路径符号。
fn is_refer_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '~' | '+')
}

/// 光标前的快捷触发；不满足触发条件时返回 None。
///
/// 触发条件：至少 2 个路径字符、位于单词边界之后且不以 `@` 开头、不在围栏代码块、
/// 不在引用标记内部、不在模板块结构行（起止行、标题/绑定分割行）上。
pub fn refer_trigger(text: &str, cursor: usize) -> Option<ReferTrigger> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let line_start = super::line_start(text, cursor);
    if super::open_fence(&text[..line_start]).is_some() {
        return None;
    }
    let line = text[..cursor].matches('\n').count();
    if is_structural_line(text, line) {
        return None;
    }
    // 补充块内部不提供引用候选（避免改写定义/结构行）。
    if super::block_context(text, cursor) == super::BlockContext::Supple {
        return None;
    }
    let infos = template_line_infos(text);
    if let Some(info) = infos.get(line).copied().flatten()
        && (info.role != TemplateLineRole::Middle || info.separator)
    {
        return None;
    }
    let mut start = cursor;
    for (offset, ch) in text[..cursor].char_indices().rev() {
        if !is_refer_word_char(ch) {
            break;
        }
        start = offset;
    }
    let query = &text[start..cursor];
    if query.chars().count() < 2 {
        return None;
    }
    if let Some(previous) = text[..start].chars().next_back()
        && (previous == '@' || is_refer_word_char(previous))
    {
        return None;
    }
    if parse_references(text)
        .iter()
        .any(|reference| reference.range.contains(&cursor) || reference.range.contains(&start))
    {
        return None;
    }
    Some(ReferTrigger {
        range: start..cursor,
        query: query.to_string(),
    })
}

/// 快捷候选作用域：可参与候选的正文分段（字节区间，可多段）。
///
/// 模板块起止行是分段边界，因此块内出现的提及/引用只在同一分段内可见——不跨块、
/// 也不泄漏到块外（未闭合的外层块不会把内层块并入自己的正文）。光标在块外时取
/// 全部块外分段。模板块起止行、标题、`{wt:}` 与结构空行不属于正文，作用域为 None。
pub fn refer_scope(text: &str, cursor: usize) -> Option<Vec<Range<usize>>> {
    let line = text[..cursor.min(text.len())].matches('\n').count();
    if is_structural_line(text, line) {
        return None;
    }
    let infos = template_line_infos(text);
    let segment = body_segment_at(text, line)?;
    match infos.get(line).copied().flatten() {
        Some(info) if info.separator => None,
        Some(_) => Some(vec![segment]),
        None => Some(plain_segments(text, &infos)),
    }
}

/// 光标所在正文分段：以模板块起止行为界，向两侧扩展连续的正文行。
///
/// 起止行本身不是正文（返回 None）。
fn body_segment_at(text: &str, line: usize) -> Option<Range<usize>> {
    let body = |index: usize| {
        line_bounds(text, index).is_some_and(|(start, end)| {
            let content = &text[start..end];
            super::parse_template_start_line(content).is_none()
                && super::parse_template_end_line(content).is_none()
        })
    };
    if !body(line) {
        return None;
    }
    let from = (0..=line).rev().take_while(|index| body(*index)).last()?;
    let to = (line..).take_while(|index| body(*index)).last()?;
    let (from, _) = line_bounds(text, from)?;
    let (_, to) = line_bounds(text, to)?;
    (from <= to).then_some(from..to)
}

/// 块外正文分段：连续的非模板块行合并为一段。
fn plain_segments(text: &str, infos: &[Option<TemplateLineInfo>]) -> Vec<Range<usize>> {
    let mut segments = Vec::new();
    let mut current: Option<Range<usize>> = None;
    let mut offset = 0;
    for (index, content) in text.split('\n').enumerate() {
        let end = offset + content.len();
        if infos.get(index).copied().flatten().is_none() {
            match &mut current {
                Some(segment) => segment.end = end,
                None => current = Some(offset..end),
            }
        } else if let Some(segment) = current.take() {
            segments.push(segment);
        }
        offset = end + 1;
    }
    segments.extend(current);
    segments
}

/// 结构元数据行（标题行、`{wt:}` 绑定行）：不属于模板块正文，不参与快捷匹配。
fn is_structural_line(text: &str, line: usize) -> bool {
    let Some((start, end)) = line_bounds(text, line) else {
        return false;
    };
    let content = &text[start..end];
    super::is_title_line(content) || super::parse_worktree_line(content).is_some()
}

/// 逻辑行字节范围（不含换行）。
fn line_bounds(text: &str, line: usize) -> Option<(usize, usize)> {
    let mut start = 0;
    for (index, content) in text.split('\n').enumerate() {
        let end = start + content.len();
        if index == line {
            return Some((start, end));
        }
        start = end + 1;
    }
    None
}

/// 收集作用域内已出现的候选：`@` 提及与 `@[refer-*]` 引用，按原文去重保序。
///
/// 两类来源按原文位置合并后再去重（`order` 必须反映正文首次出现顺序，同分排序依赖它）。
/// 过滤触发词本身与 `@start` / `@end` 结构标记；重复原文只保留首次出现。
pub fn refer_candidates(
    text: &str,
    scopes: &[Range<usize>],
    trigger: &Range<usize>,
    entries: &[MentionEntry],
) -> Vec<ReferCandidate> {
    /// 一条来源：`@` 提及的整段区间或完整的引用标记。
    enum Source<'a> {
        Mention(Range<usize>),
        Reference(&'a ParsedReference),
    }

    let in_scope = |range: &Range<usize>| {
        scopes
            .iter()
            .any(|scope| range.start >= scope.start && range.end <= scope.end)
    };
    let overlaps = |range: &Range<usize>| range.start < trigger.end && trigger.start < range.end;
    let references: Vec<ParsedReference> = parse_references_in(text, 0, text.len())
        .into_iter()
        .filter(|reference| in_scope(&reference.range))
        .collect();
    let mut sources: Vec<(usize, Source<'_>)> = super::mention_token_ranges(text)
        .into_iter()
        .filter(|range| in_scope(range) && !overlaps(range))
        .map(|range| (range.start, Source::Mention(range)))
        .collect();
    sources.extend(
        references
            .iter()
            .filter(|reference| !overlaps(&reference.range))
            .map(|reference| (reference.range.start, Source::Reference(reference))),
    );
    sources.sort_by_key(|(at, _)| *at);
    let mut candidates: Vec<ReferCandidate> = Vec::new();
    for (_, source) in sources {
        match source {
            Source::Mention(range) => {
                let raw = &text[range];
                let (path, folder) = match raw.strip_prefix('@') {
                    Some(rest) => match rest.strip_suffix('/') {
                        Some(dir) => (dir, true),
                        None => (rest, directory_entry(rest, entries)),
                    },
                    None => continue,
                };
                if path == "start" || path == "end" {
                    continue;
                }
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                let tag = if folder {
                    ReferTag::Folder
                } else {
                    ReferTag::File
                };
                push_unique(&mut candidates, raw, &name, path, tag);
            }
            Source::Reference(reference) => {
                let name = reference
                    .path
                    .rsplit(['/', '\\'])
                    .find(|part| !part.is_empty())
                    .unwrap_or(reference.path.as_str())
                    .to_string();
                push_unique(
                    &mut candidates,
                    &reference.raw,
                    &name,
                    &reference.path,
                    ReferTag::Refer(reference.kind),
                );
            }
        }
    }
    candidates
}

/// 提及路径在扫描缓存中可确定为目录时返回 true（`@foo/` 已由尾斜杠判定）。
fn directory_entry(path: &str, entries: &[MentionEntry]) -> bool {
    entries
        .iter()
        .any(|entry| entry.is_directory && entry.path == path)
}

/// 原文去重后追加候选；order 取首次出现序。
fn push_unique(
    candidates: &mut Vec<ReferCandidate>,
    raw: &str,
    name: &str,
    path: &str,
    tag: ReferTag,
) {
    if candidates.iter().any(|candidate| candidate.text == raw) {
        return;
    }
    let order = candidates.len();
    candidates.push(ReferCandidate {
        text: raw.to_string(),
        name: name.to_string(),
        path: path.to_string(),
        tag,
        order,
    });
}

/// 过滤、评分并按稳定顺序排序候选。
///
/// 优先级：名称完全匹配、名称前缀、路径前缀、名称包含、名称子序列、路径包含；
/// 同分按首次出现顺序。查询为空返回空表（不显示面板）。
pub fn filter_refer_candidates(
    candidates: Vec<ReferCandidate>,
    query: &str,
) -> Vec<ReferCandidate> {
    let query = query.to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(u32, ReferCandidate)> = candidates
        .into_iter()
        .filter_map(|candidate| refer_score(&candidate, &query).map(|score| (score, candidate)))
        .collect();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.order.cmp(&right.1.order))
    });
    scored
        .into_iter()
        .take(REFER_LIMIT)
        .map(|(_, candidate)| candidate)
        .collect()
}

/// 单个候选的匹配分；未命中返回 None。
fn refer_score(candidate: &ReferCandidate, query: &str) -> Option<u32> {
    let name = candidate.name.to_lowercase();
    let path = candidate.path.to_lowercase();
    if name == query {
        return Some(6000);
    }
    if name.starts_with(query) {
        return Some(5000);
    }
    if path.starts_with(query) {
        return Some(4000);
    }
    if name.contains(query) {
        return Some(3000);
    }
    if let Some(span) = subsequence_span(&name, query) {
        return Some(2000_u32.saturating_sub(span));
    }
    if path.contains(query) {
        return Some(1000);
    }
    None
}

/// 子序列匹配的字符跨度（首末距离 + 1）；未命中返回 None。
fn subsequence_span(haystack: &str, needle: &str) -> Option<u32> {
    let mut expected = needle.chars();
    let mut current = expected.next()?;
    let mut first: Option<u32> = None;
    for (position, ch) in haystack.chars().enumerate() {
        if ch != current {
            continue;
        }
        let position = u32::try_from(position).ok()?;
        let start = *first.get_or_insert(position);
        match expected.next() {
            Some(next) => current = next,
            None => return Some(position - start + 1),
        }
    }
    None
}

/// 确认替换文本：候选完整原文加尾随空格，便于继续输入。
pub fn refer_insertion(candidate: &ReferCandidate) -> String {
    format!("{} ", candidate.text)
}

#[cfg(test)]
mod tests;
