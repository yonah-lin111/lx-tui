//! 长文本折叠粘贴 token：编解码、围栏感知扫描、显示投影与原子编辑边界。

use std::borrow::Cow;
use std::ops::Range;

use unicode_width::UnicodeWidthChar;

use crate::app::markdown::fence_delimiter;

/// 单个折叠粘贴块允许的最大正文 UTF-8 字节数（256 KiB）。
pub const MAX_FOLDED_BYTES: usize = 256 * 1024;

/// 折叠态首行预览的最大显示列宽。
const PREVIEW_MAX_WIDTH: usize = 40;

/// 展开态预览行的左缩进列数（2 列）。
pub const EXPANDED_PASTE_INDENT: u16 = 2;

/// 折叠 token 前缀。
const TOKEN_PREFIX: &str = "@[pasted:";

/// 一个已解析的折叠粘贴 token（`range` 指向原始缓冲区字节区间）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedToken {
    pub range: Range<usize>,
    pub line_count: usize,
    /// 已反转义的原始正文。
    pub body: String,
    /// 折叠态括号内展示的首行预览（已截断）。
    pub preview: String,
    /// 正文与行数的稳定哈希（进程内身份）。
    pub id: u64,
}

impl FoldedToken {
    /// 折叠态显示的芯片文本：`@[pasted:N](preview)`。
    pub fn label(&self) -> String {
        format!("@[pasted:{}]({})", self.line_count, self.preview)
    }
}

/// 已展开的折叠块集合：按 `(token_id, 同名出现序号)` 记录，不依赖字节偏移。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExpandedPastes {
    entries: Vec<(u64, usize)>,
}

impl ExpandedPastes {
    /// 是否没有任何展开项。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 指定身份的折叠块是否处于展开态。
    pub fn is_expanded(&self, id: u64, occurrence: usize) -> bool {
        self.entries.contains(&(id, occurrence))
    }

    /// 切换指定折叠块的展开态；返回切换后的展开状态（`true` = 已展开）。
    pub fn toggle(&mut self, id: u64, occurrence: usize) -> bool {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|&item| item == (id, occurrence))
        {
            self.entries.remove(pos);
            false
        } else {
            self.entries.push((id, occurrence));
            true
        }
    }

    /// 清空全部展开态；原先非空时返回 `true`。
    pub fn clear(&mut self) -> bool {
        let had = !self.entries.is_empty();
        self.entries.clear();
        had
    }
}

/// 投影文本中命中的折叠芯片信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipHit {
    pub token_index: usize,
    pub id: u64,
    pub occurrence: usize,
    pub expanded: bool,
    pub display_range: Range<usize>,
    pub raw_range: Range<usize>,
}

/// 投影段：记录显示文本字节范围到原始缓冲区字节范围的映射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub display: Range<usize>,
    pub raw: Range<usize>,
    pub kind: SegmentKind,
}

/// 投影段种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Plain,
    Chip {
        token: usize,
        expanded: bool,
        occurrence: usize,
    },
}

/// 投影后的显示文本及双向坐标映射。
#[derive(Debug, Clone)]
pub struct Projection<'a> {
    raw_text: &'a str,
    text: Cow<'a, str>,
    segments: Vec<Segment>,
    tokens: Vec<FoldedToken>,
}

impl<'a> Projection<'a> {
    /// 根据原始文本与展开状态构造显示投影；无 token 时零分配借用原串。
    pub fn new(raw_text: &'a str, expanded: &ExpandedPastes) -> Self {
        let toks = tokens(raw_text);
        if toks.is_empty() {
            return Self {
                raw_text,
                text: Cow::Borrowed(raw_text),
                segments: vec![Segment {
                    display: 0..raw_text.len(),
                    raw: 0..raw_text.len(),
                    kind: SegmentKind::Plain,
                }],
                tokens: Vec::new(),
            };
        }

        let mut display_text = String::with_capacity(raw_text.len());
        let mut segments = Vec::with_capacity(toks.len() * 2 + 1);
        let mut counts: Vec<(u64, usize)> = Vec::new();
        let mut raw_cursor = 0;

        for (index, tok) in toks.iter().enumerate() {
            if tok.range.start > raw_cursor {
                let slice = &raw_text[raw_cursor..tok.range.start];
                let start = display_text.len();
                display_text.push_str(slice);
                segments.push(Segment {
                    display: start..display_text.len(),
                    raw: raw_cursor..tok.range.start,
                    kind: SegmentKind::Plain,
                });
            }

            let occurrence = next_occurrence(&mut counts, tok.id);
            let is_expanded = expanded.is_expanded(tok.id, occurrence);
            let start = display_text.len();
            if is_expanded {
                display_text.push_str(&tok.body);
            } else {
                display_text.push_str(&tok.label());
            }
            segments.push(Segment {
                display: start..display_text.len(),
                raw: tok.range.clone(),
                kind: SegmentKind::Chip {
                    token: index,
                    expanded: is_expanded,
                    occurrence,
                },
            });
            raw_cursor = tok.range.end;
        }

        if raw_cursor < raw_text.len() {
            let slice = &raw_text[raw_cursor..];
            let start = display_text.len();
            display_text.push_str(slice);
            segments.push(Segment {
                display: start..display_text.len(),
                raw: raw_cursor..raw_text.len(),
                kind: SegmentKind::Plain,
            });
        }

        Self {
            raw_text,
            text: Cow::Owned(display_text),
            segments,
            tokens: toks,
        }
    }

    /// 投影后的显示文本。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 投影段列表。
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// 已解析的折叠 token 列表。
    pub fn tokens(&self) -> &[FoldedToken] {
        &self.tokens
    }

    /// 是否包含至少一个折叠 token。
    pub fn has_tokens(&self) -> bool {
        !self.tokens.is_empty()
    }

    /// 原始字节偏移 → 显示字节偏移；落在 token 内部时夹到芯片起点。
    pub fn raw_to_display(&self, raw: usize) -> usize {
        let raw = raw.min(self.raw_text.len());
        if self.tokens.is_empty() {
            return raw;
        }
        for seg in &self.segments {
            if raw < seg.raw.end {
                return match seg.kind {
                    SegmentKind::Plain => seg.display.start + raw.saturating_sub(seg.raw.start),
                    SegmentKind::Chip { .. } => seg.display.start,
                };
            }
        }
        self.text.len()
    }

    /// 显示字节偏移 → 原始字节偏移；落在芯片内部时返回芯片起点。
    pub fn display_to_raw(&self, display: usize) -> usize {
        self.display_to_raw_directed(display, false)
    }

    /// 显示字节偏移 → 原始字节偏移；落在芯片内部时按 `after` 夹到芯片起点（`false`）或终点（`true`）。
    pub fn display_to_raw_directed(&self, display: usize, after: bool) -> usize {
        let display = display.min(self.text.len());
        if self.tokens.is_empty() {
            return display;
        }
        for seg in &self.segments {
            if display < seg.display.end {
                return match seg.kind {
                    SegmentKind::Plain => seg.raw.start + display.saturating_sub(seg.display.start),
                    SegmentKind::Chip { .. } => {
                        if after {
                            seg.raw.end
                        } else {
                            seg.raw.start
                        }
                    }
                };
            }
        }
        self.raw_text.len()
    }

    /// 视觉行 `[row_start, row_end]` 是否处于展开芯片内（若是则返回 2 列左缩进，否则返回 0）。
    pub fn row_indent(&self, row_start: usize, row_end: usize) -> u16 {
        let has_expanded = self.segments.iter().any(|seg| {
            matches!(seg.kind, SegmentKind::Chip { expanded: true, .. })
                && if row_start == row_end {
                    row_start >= seg.display.start && row_start < seg.display.end
                } else {
                    seg.display.start < row_end && seg.display.end > row_start
                }
        });
        if has_expanded {
            EXPANDED_PASTE_INDENT
        } else {
            0
        }
    }

    /// 显示字节区间 → 原始字节区间；覆盖芯片时自动扩展到完整 token 边界。
    pub fn display_range_to_raw(&self, range: Range<usize>) -> Range<usize> {
        let start = range.start.min(self.text.len());
        let end = range.end.min(self.text.len());
        if start >= end {
            let raw = self.display_to_raw(start);
            return raw..raw;
        }
        let mut raw_start = self.display_to_raw(start);
        let mut raw_end = self.display_to_raw(end);
        for seg in &self.segments {
            if seg.display.start < end && seg.display.end > start {
                raw_start = raw_start.min(seg.raw.start);
                raw_end = raw_end.max(seg.raw.end);
            }
        }
        raw_start..raw_end
    }

    /// 显示字节偏移所在的投影段。
    pub fn segment_at(&self, display: usize) -> Option<&Segment> {
        self.segments
            .iter()
            .find(|seg| display >= seg.display.start && display < seg.display.end)
    }

    /// 显示字节偏移命中的芯片（折叠或展开）。
    pub fn chip_at(&self, display: usize) -> Option<ChipHit> {
        let seg = self.segment_at(display)?;
        match seg.kind {
            SegmentKind::Plain => None,
            SegmentKind::Chip {
                token,
                expanded,
                occurrence,
            } => {
                let tok = self.tokens.get(token)?;
                Some(ChipHit {
                    token_index: token,
                    id: tok.id,
                    occurrence,
                    expanded,
                    display_range: seg.display.clone(),
                    raw_range: seg.raw.clone(),
                })
            }
        }
    }

    /// 显示字节偏移对应的原始逻辑行号（0-based）。
    pub fn raw_line_of_display(&self, display: usize) -> usize {
        let raw = self.display_to_raw(display).min(self.raw_text.len());
        self.raw_text.as_bytes()[..raw]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    }

    /// 复制显示区间文本：普通文本与展开芯片取所见切片，折叠芯片整块取原始 token。
    pub fn copy_display_range(&self, range: Range<usize>) -> String {
        let start = self
            .text
            .floor_char_boundary(range.start.min(self.text.len()));
        let end = self
            .text
            .floor_char_boundary(range.end.min(self.text.len()));
        if start >= end {
            return String::new();
        }
        if self.tokens.is_empty() {
            return self.text[start..end].to_string();
        }
        let mut out = String::new();
        for seg in &self.segments {
            let ov_start = start.max(seg.display.start);
            let ov_end = end.min(seg.display.end);
            if ov_start >= ov_end {
                continue;
            }
            match seg.kind {
                SegmentKind::Plain | SegmentKind::Chip { expanded: true, .. } => {
                    let s = self.text.floor_char_boundary(ov_start);
                    let e = self.text.floor_char_boundary(ov_end);
                    if s < e {
                        out.push_str(&self.text[s..e]);
                    }
                }
                SegmentKind::Chip {
                    expanded: false, ..
                } => {
                    out.push_str(&self.raw_text[seg.raw.clone()]);
                }
            }
        }
        out
    }
}

/// 原文 → token 正文（只转义 `\` 与 `)`）。
pub fn escape_body(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ')' => out.push_str("\\)"),
            _ => out.push(ch),
        }
    }
    out
}

/// token 正文 → 原文（只反转义 `\\` 与 `\)`，其余反斜杠序列原样保留）。
pub fn unescape_body(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some(')') => out.push(')'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// 原文 → 自包含折叠 token `@[pasted:N](转义正文)`。
pub fn encode(content: &str) -> String {
    let line_count = content.split('\n').count().max(1);
    format!("@[pasted:{line_count}]({})", escape_body(content))
}

/// 扫描全文，返回所有在代码围栏外的合法折叠 token。
///
/// 扫描器自行维护行首围栏状态：围栏内的 `@[pasted:...]` 字面量被忽略，
/// 而 token 正文内部换行后的 ``` 不会开启或关闭外层围栏。
pub fn tokens(text: &str) -> Vec<FoldedToken> {
    if !text.contains(TOKEN_PREFIX) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut pos = 0;
    let mut at_line_start = true;
    let len = text.len();

    while pos < len {
        if at_line_start {
            let line_end = text[pos..].find('\n').map_or(len, |rel| pos + rel);
            let line = &text[pos..line_end];
            if let Some((marker, fence_len)) = fence {
                if fence_delimiter(line).is_some_and(|(m, l)| m == marker && l >= fence_len) {
                    fence = None;
                }
                pos = if line_end < len { line_end + 1 } else { len };
                at_line_start = true;
                continue;
            }
            if let Some(delim) = fence_delimiter(line) {
                fence = Some(delim);
                pos = if line_end < len { line_end + 1 } else { len };
                at_line_start = true;
                continue;
            }
        }

        if fence.is_none()
            && text[pos..].starts_with(TOKEN_PREFIX)
            && let Some(token) = try_parse_token(text, pos)
        {
            pos = token.range.end;
            at_line_start = false;
            out.push(token);
            continue;
        }

        let Some(ch) = text[pos..].chars().next() else {
            break;
        };
        pos += ch.len_utf8();
        at_line_start = ch == '\n';
    }

    out
}

/// 把全文中所有折叠 token 解码替换为原始正文（发送给模型前使用）。
pub fn expand_all(text: &str) -> String {
    let toks = tokens(text);
    if toks.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for tok in toks {
        if tok.range.start > cursor {
            out.push_str(&text[cursor..tok.range.start]);
        }
        out.push_str(&tok.body);
        cursor = tok.range.end;
    }
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    out
}

/// 把全文中所有折叠 token 替换为等长空格（保留 `\n`），供 Markdown/触发词/受保护行扫描使用。
pub fn mask(text: &str) -> Cow<'_, str> {
    let toks = tokens(text);
    if toks.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut bytes = text.as_bytes().to_vec();
    for tok in toks {
        for byte in &mut bytes[tok.range] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    match String::from_utf8(bytes) {
        Ok(masked) => Cow::Owned(masked),
        Err(_) => Cow::Borrowed(text),
    }
}

/// 把原始字节区间吸附到 token 边界：部分覆盖 token 时扩展为整块。
pub fn snap_range(text: &str, range: Range<usize>) -> Range<usize> {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    let toks = tokens(text);
    if toks.is_empty() {
        return start..end;
    }
    for tok in toks {
        if start == end {
            if tok.range.start < start && start < tok.range.end {
                start = tok.range.start;
                end = tok.range.end;
            }
        } else if tok.range.start < end && tok.range.end > start {
            start = start.min(tok.range.start);
            end = end.max(tok.range.end);
        }
    }
    start..end
}

/// 若偏移严格落在某个 token 内部，夹到该 token 起点。
pub fn clamp_cursor(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    for tok in tokens(text) {
        if tok.range.start < offset && offset < tok.range.end {
            return tok.range.start;
        }
    }
    offset
}

/// 若偏移位于某个 token 内部或紧跟在 token 末尾，返回该 token（供 Backspace 整块删除）。
pub fn token_ending_at(text: &str, offset: usize) -> Option<FoldedToken> {
    let offset = offset.min(text.len());
    tokens(text)
        .into_iter()
        .find(|tok| offset > tok.range.start && offset <= tok.range.end)
}

/// 若偏移位于某个 token 起点或内部，返回该 token（供 Delete 整块删除）。
pub fn token_starting_at(text: &str, offset: usize) -> Option<FoldedToken> {
    let offset = offset.min(text.len());
    tokens(text)
        .into_iter()
        .find(|tok| offset >= tok.range.start && offset < tok.range.end)
}

/// 光标向左移动一步：遇到 token 时整块跨过。
pub fn prev_boundary(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if offset == 0 {
        return 0;
    }
    if let Some(tok) = token_ending_at(text, offset) {
        return tok.range.start;
    }
    let prev = crate::app::prompt::text::prev_boundary(text, offset);
    for tok in tokens(text) {
        if tok.range.start < prev && prev < tok.range.end {
            return tok.range.start;
        }
    }
    prev
}

/// 光标向右移动一步：遇到 token 时整块跨过。
pub fn next_boundary(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if offset >= text.len() {
        return text.len();
    }
    if let Some(tok) = token_starting_at(text, offset) {
        return tok.range.end;
    }
    let next = text[offset..]
        .chars()
        .next()
        .map_or(offset, |ch| offset + ch.len_utf8());
    for tok in tokens(text) {
        if tok.range.start < next && next < tok.range.end {
            return tok.range.end;
        }
    }
    next
}

/// 尝试在 `start` 处解析一个 `@[pasted:N](...)` token。
fn try_parse_token(text: &str, start: usize) -> Option<FoldedToken> {
    let after_prefix = start.checked_add(TOKEN_PREFIX.len())?;
    let rest = text.get(after_prefix..)?;
    let digit_len = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_len == 0 {
        return None;
    }
    let line_count: usize = rest.get(..digit_len)?.parse().ok()?;
    if line_count == 0 {
        return None;
    }
    let after_digits = rest.get(digit_len..)?;
    if !after_digits.starts_with("](") {
        return None;
    }
    let body_start = after_prefix + digit_len + 2;
    let body_rest = text.get(body_start..)?;

    let mut chars = body_rest.char_indices();
    let mut body_end = None;
    while let Some((rel_idx, ch)) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            ')' => {
                body_end = Some(body_start + rel_idx);
                break;
            }
            _ => {}
        }
    }

    let body_end = body_end?;
    if body_end <= body_start {
        return None;
    }
    let token_end = body_end + 1;
    let body = unescape_body(&text[body_start..body_end]);
    if body.is_empty() {
        return None;
    }
    let preview = compute_preview(&body, line_count);
    let id = compute_id(line_count, &body);

    Some(FoldedToken {
        range: start..token_end,
        line_count,
        body,
        preview,
        id,
    })
}

/// 计算折叠态首行预览：取第一条非空行并截断到 40 列。
fn compute_preview(body: &str, line_count: usize) -> String {
    let first_non_empty = body
        .split('\n')
        .map(|line| line.trim_end())
        .find(|line| !line.trim().is_empty());
    let Some(line) = first_non_empty else {
        return format!("{line_count} lines");
    };
    let mut out = String::new();
    let mut width = 0;
    let mut truncated = false;
    for ch in line.chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > PREVIEW_MAX_WIDTH {
            truncated = true;
            break;
        }
        width += w;
        out.push(ch);
    }
    if truncated {
        out.push('…');
    }
    out
}

/// 稳定 FNV-1a 64 位哈希：相同 `(line_count, body)` 恒产出相同 `id`。
fn compute_id(line_count: usize, body: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in (line_count as u64).to_le_bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    for &b in body.as_bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// 递增同名 token 的出现序号。
fn next_occurrence(counts: &mut Vec<(u64, usize)>, id: u64) -> usize {
    if let Some(entry) = counts.iter_mut().find(|(existing, _)| *existing == id) {
        let current = entry.1;
        entry.1 += 1;
        current
    } else {
        counts.push((id, 1));
        0
    }
}

#[cfg(test)]
mod tests;
