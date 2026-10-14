//! prompt 编辑器纯文本工具：字符边界、行范围、显示宽度与粘贴净化。

use unicode_width::UnicodeWidthChar;

/// 词字符：Unicode 字母数字与下划线；CJK 视作词字符。
pub(super) fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// 所在逻辑行的起始字节偏移（上一个换行之后）。
pub(crate) fn line_start(text: &str, index: usize) -> usize {
    text[..index].rfind('\n').map_or(0, |offset| offset + 1)
}

/// 所在逻辑行的结束字节偏移（下一个换行之前或文末）。
pub(crate) fn line_end(text: &str, index: usize) -> usize {
    text[index..]
        .find('\n')
        .map_or(text.len(), |offset| index + offset)
}

/// 光标前一个词（含尾随分隔符）的起点。
pub(crate) fn word_start_before(text: &str, index: usize) -> usize {
    let mut boundary = 0;
    let mut in_word = false;
    let mut seen = false;
    for (offset, ch) in text[..index].char_indices().rev() {
        seen = true;
        if is_word_char(ch) {
            in_word = true;
            boundary = offset;
        } else if in_word {
            break;
        } else {
            boundary = offset;
        }
    }
    if seen { boundary } else { 0 }
}

/// 光标所在或下一个词的词尾（跳过前导分隔符）。
pub(crate) fn word_end_after(text: &str, index: usize) -> usize {
    let mut end = index;
    let mut in_word = false;
    for (offset, ch) in text[index..].char_indices() {
        if is_word_char(ch) {
            in_word = true;
            end = index + offset + ch.len_utf8();
        } else if in_word {
            break;
        }
    }
    end
}

/// 维护上限的快照入栈；超限时丢弃最旧一条。
pub(super) fn push_bounded(stack: &mut Vec<Snapshot>, snapshot: Snapshot) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(snapshot);
}

/// 围栏代码块行：至多 3 个前导空格后以 ``` 开头。
pub(super) fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start_matches(' ');
    line.len() - trimmed.len() <= 3 && trimmed.starts_with("```")
}

/// 字符显示宽度；零宽与不可打印字符按 0 处理。
pub(crate) fn char_width(ch: char) -> usize {
    ch.width().unwrap_or(0)
}

/// 文本显示宽度（列）。
pub(crate) fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 前一个字符的字节边界。
pub(crate) fn prev_boundary(text: &str, index: usize) -> usize {
    text[..index]
        .char_indices()
        .next_back()
        .map(|(offset, _)| offset)
        .unwrap_or(0)
}

/// 后一个字符的字节边界。
pub(super) fn next_boundary(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map(|ch| index + ch.len_utf8())
        .unwrap_or(index)
}

/// 行内显示列对应的字节偏移；落在宽字符中间或超出时钳到对应边界。
pub(crate) fn offset_at_col(slice: &str, col: usize) -> usize {
    let mut cell = 0;
    for (offset, ch) in slice.char_indices() {
        let width = char_width(ch);
        if col < cell + width {
            return offset;
        }
        cell += width;
    }
    slice.len()
}

/// 粘贴文本净化：换行归一、制表符展开、其余控制字符丢弃。
pub(crate) fn sanitize(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                sanitized.push('\n');
            }
            '\n' => sanitized.push('\n'),
            '\t' => sanitized.push_str(TAB_STOP),
            ch if ch.is_control() => {}
            ch => sanitized.push(ch),
        }
    }
    sanitized
}

use super::{HISTORY_LIMIT, Prompt, Snapshot, TAB_STOP};

impl Prompt {
    /// 光标前一个词（含尾随分隔符）的起点。
    pub(super) fn word_start_before(&self, index: usize) -> usize {
        word_start_before(&self.text, index)
    }
    /// 光标所在或下一个词的词尾（跳过前导分隔符）。
    pub(super) fn word_end_after(&self, index: usize) -> usize {
        word_end_after(&self.text, index)
    }
}
