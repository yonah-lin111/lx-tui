//! markdown 块命令领域：触发识别、候选命令与插入计算，纯函数、无 UI 依赖。

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

/// 块命令面板状态：触发区间、候选命令与高亮索引。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPanel {
    trigger: BlockTrigger,
    items: Vec<BlockCommandId>,
    active: usize,
}

impl BlockPanel {
    /// 组装面板状态；`items` 不可为空，`active` 调用方保证在界内。
    pub fn new(trigger: BlockTrigger, items: Vec<BlockCommandId>, active: usize) -> Self {
        Self {
            trigger,
            items,
            active,
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

    /// 按偏移循环移动高亮项。
    pub fn move_active(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        self.active = (self.active as isize + delta).rem_euclid(len) as usize;
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
