//! markdown 编辑领域：块命令与文件提及的触发识别、候选过滤与插入计算，纯函数、无 UI 依赖。

use std::ops::Range;

/// 块命令触发类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockTriggerKind {
    Heading,
    UnorderedList,
    OrderedList,
    Quote,
    CodeBlock,
    Table,
}

/// 触发标记区间：`from` 为标记首字符、`to` 为光标（字节偏移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockTrigger {
    pub from: usize,
    pub to: usize,
    pub kind: BlockTriggerKind,
}

/// 块命令标识；标题携带级别（1–6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockCommandId {
    Heading(u8),
    UnorderedList,
    TaskList,
    OrderedList,
    Quote,
    CodeBlock,
    Table,
}

/// 命令插入结果：替换触发区间后的文本与光标在其中的字节偏移。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInsertion {
    pub text: String,
    pub cursor: usize,
}

/// 块命令面板状态：触发区间、候选命令、高亮索引、窗口锚点与显式视口。
///
/// `anchor` 是窗口底部锚定的条目：窗口以它为底向前回退填满预算。鼠标悬停只改
/// `active` 不动 `anchor`（悬停项必然可见，窗口因此保持稳定）；键盘导航同步锚点到
/// 高亮项，让窗口跟随高亮滚动。`viewport` 是滚轮滚动后的显式窗口起点，滚轮只动它、
/// 不动高亮；键盘导航将其清空，窗口重新跟随高亮。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPanel {
    trigger: BlockTrigger,
    items: Vec<BlockCommandId>,
    active: usize,
    anchor: usize,
    viewport: Option<usize>,
}

impl BlockPanel {
    /// 组装面板状态；`items` 不可为空，`active` 调用方保证在界内。
    pub fn new(trigger: BlockTrigger, items: Vec<BlockCommandId>, active: usize) -> Self {
        Self {
            trigger,
            items,
            active,
            anchor: active,
            viewport: None,
        }
    }

    /// 触发区间。
    pub fn trigger(&self) -> BlockTrigger {
        self.trigger
    }

    /// 候选命令。
    pub fn items(&self) -> &[BlockCommandId] {
        &self.items
    }

    /// 高亮索引。
    pub fn active(&self) -> usize {
        self.active
    }

    /// 窗口锚点条目（窗口底部）。
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// 滚轮滚动后的显式视口起点；跟随高亮时为 None。
    pub fn viewport(&self) -> Option<usize> {
        self.viewport
    }

    /// 直接设置高亮索引（鼠标悬停）；越界钳到末项，窗口锚点不动。
    pub fn set_active(&mut self, index: usize) {
        self.active = index.min(self.items.len().saturating_sub(1));
    }

    /// 按偏移循环移动高亮项；窗口锚点跟随高亮，显式视口清除（窗口重新跟随高亮）。
    pub fn move_active(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        self.active = (self.active as isize + delta).rem_euclid(len) as usize;
        self.anchor = self.active;
        self.viewport = None;
    }

    /// 滚轮滚动可见窗口：显式视口从 `base` 起偏移并钳制（不循环），高亮与锚点不动。
    pub fn scroll_viewport(&mut self, delta: isize, base: usize) -> bool {
        let max = self.items.len().saturating_sub(1) as isize;
        let next = (base.min(max as usize) as isize + delta).clamp(0, max) as usize;
        let changed = next != base;
        self.viewport = Some(next);
        changed
    }
}

/// 解析光标所在行的块触发标记。
///
/// 仅在光标位于行尾、行首（可含前导空格）为已知标记、且光标不在未闭合围栏内时返回；
/// 上一行已是同类列表/引用/表格时抑制（自然续写不弹面板）。
pub fn block_trigger(text: &str, cursor: usize) -> Option<BlockTrigger> {
    let start = line_start(text, cursor);
    let end = line_end(text, cursor);
    if cursor != end {
        return None;
    }
    let line = &text[start..end];
    let (kind, marker) = match_marker(line)?;
    if open_fence(&text[..start]).is_some() {
        return None;
    }
    if continues_previous(text, start, kind) {
        return None;
    }
    Some(BlockTrigger {
        from: start + marker,
        to: cursor,
        kind,
    })
}

/// 触发类别对应的候选命令。
pub fn block_commands(kind: BlockTriggerKind) -> Vec<BlockCommandId> {
    match kind {
        BlockTriggerKind::Heading => (1..=6).map(BlockCommandId::Heading).collect(),
        BlockTriggerKind::UnorderedList => {
            vec![BlockCommandId::UnorderedList, BlockCommandId::TaskList]
        }
        BlockTriggerKind::OrderedList => vec![BlockCommandId::OrderedList],
        BlockTriggerKind::Quote => vec![BlockCommandId::Quote],
        BlockTriggerKind::CodeBlock => vec![BlockCommandId::CodeBlock],
        BlockTriggerKind::Table => vec![BlockCommandId::Table],
    }
}

/// 命令的替换文本与光标落点（偏移相对替换文本起点）。
pub fn block_insertion(id: BlockCommandId) -> BlockInsertion {
    match id {
        BlockCommandId::Heading(level) => {
            let text = format!("{} ", "#".repeat(usize::from(level)));
            let cursor = text.len();
            BlockInsertion { text, cursor }
        }
        BlockCommandId::UnorderedList => BlockInsertion {
            text: "- ".into(),
            cursor: 2,
        },
        BlockCommandId::TaskList => BlockInsertion {
            text: "- [ ] ".into(),
            cursor: 6,
        },
        BlockCommandId::OrderedList => BlockInsertion {
            text: "1. ".into(),
            cursor: 3,
        },
        BlockCommandId::Quote => BlockInsertion {
            text: "> ".into(),
            cursor: 2,
        },
        BlockCommandId::CodeBlock => BlockInsertion {
            text: "```\n```".into(),
            cursor: 4,
        },
        BlockCommandId::Table => BlockInsertion {
            text: "|  |  |\n| --- | --- |\n|  |  |".into(),
            cursor: 2,
        },
    }
}

/// 文件提及触发区间：`from` 为 `@` 起始、`to` 为光标，`query` 为 `@` 与光标之间的查询串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionTrigger {
    pub from: usize,
    pub to: usize,
    pub query: String,
}

/// 工作区文件候选：相对根的路径（`/` 分隔）与是否为目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionEntry {
    pub path: String,
    pub is_directory: bool,
}

/// 提及面板的显示上限。
pub const MENTION_LIMIT: usize = 100;

/// 文件提及面板状态：触发区间、过滤后的候选、高亮索引、窗口锚点与显式视口。
///
/// `anchor` 是窗口底部锚定的条目：窗口以它为底向前回退填满预算。鼠标悬停只改
/// `active` 不动 `anchor`（悬停项必然可见，窗口因此保持稳定）；键盘导航同步锚点到
/// 高亮项，让窗口跟随高亮滚动。`viewport` 是滚轮滚动后的显式窗口起点，滚轮只动它、
/// 不动高亮；键盘导航将其清空，窗口重新跟随高亮。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionPanel {
    trigger: MentionTrigger,
    items: Vec<MentionEntry>,
    active: usize,
    anchor: usize,
    viewport: Option<usize>,
}

impl MentionPanel {
    /// 组装面板状态；`items` 不可为空，`active` 调用方保证在界内。
    pub fn new(trigger: MentionTrigger, items: Vec<MentionEntry>, active: usize) -> Self {
        Self {
            trigger,
            items,
            active,
            anchor: active,
            viewport: None,
        }
    }

    /// 触发区间。
    pub fn trigger(&self) -> &MentionTrigger {
        &self.trigger
    }

    /// 过滤后的候选。
    pub fn items(&self) -> &[MentionEntry] {
        &self.items
    }

    /// 当前浏览目录的末段名（query 最后一个 `/` 之前的部分）；未进入目录返回 None。
    pub fn scope_name(&self) -> Option<&str> {
        let query = self.trigger.query.as_str();
        let scope = &query[..=query.rfind('/')?];
        let scope = scope.strip_suffix('/')?;
        (!scope.is_empty()).then(|| scope.rsplit('/').next().unwrap_or(scope))
    }

    /// 高亮索引。
    pub fn active(&self) -> usize {
        self.active
    }

    /// 窗口锚点条目（窗口底部）。
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// 滚轮滚动后的显式视口起点；跟随高亮时为 None。
    pub fn viewport(&self) -> Option<usize> {
        self.viewport
    }

    /// 直接设置高亮索引（鼠标悬停）；越界钳到末项，窗口锚点不动。
    pub fn set_active(&mut self, index: usize) {
        self.active = index.min(self.items.len().saturating_sub(1));
    }

    /// 按偏移循环移动高亮项；窗口锚点跟随高亮，显式视口清除（窗口重新跟随高亮）。
    pub fn move_active(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        self.active = (self.active as isize + delta).rem_euclid(len) as usize;
        self.anchor = self.active;
        self.viewport = None;
    }

    /// 滚轮滚动可见窗口：显式视口从 `base` 起偏移并钳制（不循环），高亮与锚点不动。
    pub fn scroll_viewport(&mut self, delta: isize, base: usize) -> bool {
        let max = self.items.len().saturating_sub(1) as isize;
        let next = (base.min(max as usize) as isize + delta).clamp(0, max) as usize;
        let changed = next != base;
        self.viewport = Some(next);
        changed
    }
}

/// 解析光标前的文件提及触发。
///
/// `@` 必须在行首、空白或 `[` 之后，且 `@` 与光标之间只能是查询字符；
/// 围栏代码块内不触发。
pub fn mention_trigger(text: &str, cursor: usize) -> Option<MentionTrigger> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    if open_fence(&text[..line_start(text, cursor)]).is_some() {
        return None;
    }
    let at = text[..cursor].rfind('@')?;
    if let Some(previous) = text[..at].chars().next_back()
        && !(previous.is_whitespace() || previous == '[')
    {
        return None;
    }
    let query = &text[at + 1..cursor];
    if query.chars().any(|ch| !is_mention_query_char(ch)) {
        return None;
    }
    Some(MentionTrigger {
        from: at,
        to: cursor,
        query: query.to_string(),
    })
}

/// 查询字符：非空白且非提及边界标点；允许中文等多字节字符，`.` 属于文件名字符。
fn is_mention_query_char(ch: char) -> bool {
    !ch.is_whitespace()
        && !matches!(
            ch,
            ',' | ';'
                | ':'
                | '!'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '，'
                | '。'
                | '；'
                | '：'
                | '！'
                | '？'
                | '、'
                | '…'
        )
}

/// 按查询过滤候选：文件名优先打分排序后截取展示上限。
///
/// query 含 `/` 时按「目录范围 + 过滤词」解释：候选必须是范围目录的后代（范围自身除外），
/// 空过滤词只保留直接子项；其余在范围内的相对路径上打分，无范围时保持全局打分。
/// 空查询按路径排序全部展示。
pub fn filter_mentions(entries: &[MentionEntry], query: &str) -> Vec<MentionEntry> {
    let query = query.trim().to_lowercase();
    let (scope, rest) = split_mention_scope(&query);
    let mut scored: Vec<(u32, &MentionEntry)> = entries
        .iter()
        .filter_map(|entry| {
            let path = match scope {
                Some(scope) => scope_relative(&entry.path, scope)?,
                None => entry.path.as_str(),
            };
            if scope.is_some() && rest.is_empty() && path.contains('/') {
                return None;
            }
            let score = mention_score(path, rest);
            (score > 0).then_some((score, entry))
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.path.cmp(&right.1.path))
    });
    scored
        .into_iter()
        .take(MENTION_LIMIT)
        .map(|(_, entry)| entry.clone())
        .collect()
}

/// 拆分提及 query 的目录范围与过滤词：`src/ui/ma` → (`Some("src/ui/")`, `"ma"`)；
/// 无 `/` → (`None`, 全部 query)。范围前缀含尾 `/`，用于限定候选目录。
fn split_mention_scope(query: &str) -> (Option<&str>, &str) {
    match query.rfind('/') {
        Some(index) => (Some(&query[..=index]), &query[index + 1..]),
        None => (None, query),
    }
}

/// query 的目录范围前缀（含尾 `/`）；无 `/` 返回 None。
pub fn mention_scope(query: &str) -> Option<&str> {
    split_mention_scope(query).0
}

/// scope 前缀下的相对路径（原大小写）；不在范围内或恰为范围自身返回 None。
///
/// 大小写折叠在极少数非 ASCII 情况下会改变字节长度，此时退回整路径参与打分，
/// 候选已由前缀校验保证范围正确。
fn scope_relative<'a>(path: &'a str, scope: &str) -> Option<&'a str> {
    let normalized = path.to_lowercase();
    let rest = normalized.strip_prefix(scope)?;
    if rest.is_empty() {
        return None;
    }
    Some(path.get(scope.len()..).unwrap_or(path))
}

/// 文件名优先的模糊打分；对齐 lx-agent 的 `getProjectFileMatchScore`。
fn mention_score(path: &str, query: &str) -> u32 {
    if query.is_empty() {
        return 1;
    }
    let normalized = path.to_lowercase();
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let normalized_name = file_name.to_lowercase();
    if normalized_name == query {
        return 5000;
    }
    if normalized == query {
        return 4000;
    }
    if normalized_name.starts_with(query) {
        return 3000;
    }
    if normalized.starts_with(query) {
        return 2000;
    }
    let caps: String = file_name
        .chars()
        .filter(|ch| ch.is_ascii_uppercase())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    if !caps.is_empty() && caps.starts_with(query) {
        return 2800 + u32::from(caps.len() == query.len()) * 100;
    }
    if !caps.is_empty() && caps.contains(query) {
        return 2500;
    }
    if normalized_name.contains(query) {
        return 1800;
    }
    if let Some(span) = subsequence_span(&normalized_name, query) {
        return 1200.max(1500_u32.saturating_sub(span * 10));
    }
    if normalized.contains(query) {
        return 800;
    }
    if let Some(span) = subsequence_span(&normalized, query) {
        return 100.max(500_u32.saturating_sub(span));
    }
    0
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

/// 提及插入文本：`@相对路径` 加尾随空格；目录带 `/`。
pub fn mention_insertion(entry: &MentionEntry) -> String {
    if entry.is_directory {
        format!("@{}/ ", entry.path)
    } else {
        format!("@{} ", entry.path)
    }
}

/// @ 提及整块删除范围：光标紧跟提及后的空白时返回（提及起点..光标），供 Backspace 一次删净。
///
/// 提及必须位于行首或空白之后且路径非空；光标不在空白后时返回 None（走普通退格）。
pub fn mention_deletion_range(text: &str, cursor: usize) -> Option<Range<usize>> {
    if cursor == 0 || cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let previous = text[..cursor].chars().next_back()?;
    if previous != ' ' && previous != '\t' {
        return None;
    }
    let end = cursor - previous.len_utf8();
    mention_token_ranges(text)
        .into_iter()
        .find(|range| range.end == end)
        .map(|range| range.start..cursor)
}

/// 全文中的 @ 提及区间：`@` 位于行首或空白之后，路径由查询字符组成且非空。
fn mention_token_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find('@') {
        let at = index + offset;
        let boundary = text[..at]
            .chars()
            .next_back()
            .is_none_or(|previous| previous.is_whitespace());
        if boundary {
            let mut end = at + 1;
            for ch in text[end..].chars() {
                if !is_mention_query_char(ch) || ch == '@' {
                    break;
                }
                end += ch.len_utf8();
            }
            if end > at + 1 {
                ranges.push(at..end);
            }
            index = end;
        } else {
            index = at + 1;
        }
    }
    ranges
}

/// 行首标记匹配；返回类别与标记在行内的起始偏移（跳过前导空格）。
fn match_marker(line: &str) -> Option<(BlockTriggerKind, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    if heading_marker(rest) {
        return Some((BlockTriggerKind::Heading, indent));
    }
    if unordered_marker(rest) {
        return Some((BlockTriggerKind::UnorderedList, indent));
    }
    if ordered_marker(rest) {
        return Some((BlockTriggerKind::OrderedList, indent));
    }
    if quote_marker(rest) {
        return Some((BlockTriggerKind::Quote, indent));
    }
    if fence_marker(rest) {
        return Some((BlockTriggerKind::CodeBlock, indent));
    }
    if rest == "|" {
        return Some((BlockTriggerKind::Table, indent));
    }
    None
}

/// 标题标记：1–6 个 `#`，可后随一个空格或行尾。
fn heading_marker(rest: &str) -> bool {
    let hashes = rest.bytes().take_while(|byte| *byte == b'#').count();
    (1..=6).contains(&hashes) && matches!(&rest[hashes..], "" | " ")
}
/// 无序列表标记：单个 `-`/`+`/`*`，可后随一个空格或行尾。
fn unordered_marker(rest: &str) -> bool {
    matches!(rest, "-" | "- " | "+" | "+ " | "*" | "* ")
}

/// 有序列表标记：`1.` 或 `1)`，可后随一个空格或行尾。
fn ordered_marker(rest: &str) -> bool {
    matches!(rest, "1." | "1. " | "1)" | "1) ")
}

/// 引用标记：单个 `>`，可后随一个空格或行尾。
fn quote_marker(rest: &str) -> bool {
    matches!(rest, ">" | "> ")
}

/// 围栏标记：至少 3 个同种 ` 或 `~`，行尾结束。
fn fence_marker(rest: &str) -> bool {
    let mut bytes = rest.bytes();
    let Some(marker) = bytes.next() else {
        return false;
    };
    if marker != b'`' && marker != b'~' {
        return false;
    }
    rest.len() >= 3 && bytes.all(|byte| byte == marker)
}

/// 上一行是否已是同类块头部；用于抑制自然续写时的弹窗。
fn continues_previous(text: &str, line_start_at: usize, kind: BlockTriggerKind) -> bool {
    let Some(before) = text[..line_start_at].strip_suffix('\n') else {
        return false;
    };
    let previous = before.rsplit('\n').next().unwrap_or_default();
    let rest = previous.trim_start_matches(' ');
    match kind {
        BlockTriggerKind::UnorderedList => {
            let bytes = rest.as_bytes();
            matches!(bytes.first(), Some(b'-' | b'+' | b'*'))
                && matches!(bytes.get(1), None | Some(b' '))
        }
        BlockTriggerKind::OrderedList => {
            let bytes = rest.as_bytes();
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            digits > 0
                && matches!(bytes.get(digits), Some(b'.' | b')'))
                && matches!(bytes.get(digits + 1), None | Some(b' '))
        }
        BlockTriggerKind::Quote => {
            let bytes = rest.as_bytes();
            bytes.first() == Some(&b'>') && matches!(bytes.get(1), None | Some(b' '))
        }
        BlockTriggerKind::Table => rest.starts_with('|'),
        BlockTriggerKind::Heading | BlockTriggerKind::CodeBlock => false,
    }
}

/// 前文未闭合的围栏（标记字符与长度）；无则 None。
fn open_fence(text: &str) -> Option<(u8, usize)> {
    let mut open: Option<(u8, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() > 3 {
            continue;
        }
        let Some(marker) = trimmed.bytes().next() else {
            continue;
        };
        if marker != b'`' && marker != b'~' {
            continue;
        }
        let len = trimmed.bytes().take_while(|byte| *byte == marker).count();
        if len < 3 {
            continue;
        }
        open = match open {
            None => Some((marker, len)),
            Some((open_marker, open_len)) if open_marker == marker && len >= open_len => None,
            current => current,
        };
    }
    open
}

/// 所在逻辑行的起始字节偏移。
fn line_start(text: &str, index: usize) -> usize {
    text[..index].rfind('\n').map_or(0, |offset| offset + 1)
}

/// 所在逻辑行的结束字节偏移。
fn line_end(text: &str, index: usize) -> usize {
    text[index..]
        .find('\n')
        .map_or(text.len(), |offset| index + offset)
}

#[cfg(test)]
mod tests;
