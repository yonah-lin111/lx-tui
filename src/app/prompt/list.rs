//! 行首列表项解析：标记、任务项、续写标记与上文关系判定；纯函数、无 UI 依赖。

use super::{EditKind, is_fence, line_end, line_start};

/// 行首列表项标记类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListKind {
    Bullet(char),
    Ordered { number: u64, delimiter: char },
    Task(char),
}

/// 行首列表项：缩进、分隔空格结束偏移（标记+一个空格）、标记段结束偏移（含后随全部空白）与类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ListItem {
    pub(super) indent: usize,
    pub(super) sep_end: usize,
    pub(super) marker_end: usize,
    pub(super) kind: ListKind,
}

/// 当前列表项与上文列表的关系；决定 Backspace 删除标记的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListContext {
    /// 同列表已有前一项：标记替换为等宽空格。
    Sibling,
    /// 嵌套在上一级列表项内容中：删除标记、保留缩进。
    Nested,
    /// 顶层列表首项：连同缩进整段删除。
    TopLevel,
}

impl ListItem {
    /// 下一行延续用的标记文本（不含缩进）。
    pub(super) fn continuation(self) -> String {
        match self.kind {
            ListKind::Bullet(marker) => format!("{marker} "),
            ListKind::Ordered { number, delimiter } => format!("{}{delimiter} ", number + 1),
            ListKind::Task(marker) => format!("{marker} [ ] "),
        }
    }
}

/// 解析行首列表项（无序/有序/任务）；非列表返回 None。
pub(super) fn parse_list_item(line: &str) -> Option<ListItem> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let first = rest.chars().next()?;
    let (kind, marker_len) = if matches!(first, '-' | '*' | '+') {
        (ListKind::Bullet(first), 1)
    } else if first.is_ascii_digit() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let delimiter = *rest.as_bytes().get(digits)?;
        if !matches!(delimiter, b'.' | b')') {
            return None;
        }
        (
            ListKind::Ordered {
                number: rest[..digits].parse().ok()?,
                delimiter: delimiter as char,
            },
            digits + 1,
        )
    } else {
        return None;
    };
    let spaces = rest[marker_len..]
        .bytes()
        .take_while(|byte| *byte == b' ')
        .count();
    if spaces == 0 {
        return None;
    }
    let mut sep_end = indent + marker_len + 1;
    let mut marker_end = indent + marker_len + spaces;
    let mut kind = kind;
    if let ListKind::Bullet(marker) = kind {
        let after = &line.as_bytes()[marker_end..];
        if after.len() >= 4
            && after[0] == b'['
            && after[2] == b']'
            && matches!(after[1], b' ' | b'x' | b'X')
        {
            let task_spaces = after[3..].iter().take_while(|byte| **byte == b' ').count();
            if task_spaces > 0 {
                sep_end = marker_end + 3 + 1;
                marker_end += 3 + task_spaces;
                kind = ListKind::Task(marker);
            }
        }
    }
    Some(ListItem {
        indent,
        sep_end,
        marker_end,
        kind,
    })
}

/// 上溯当前行之前的列表关系。
///
/// 同缩进且标记族相同视为同列表前项；低缩进列表项视为嵌套父项；
/// 空行、低缩进非列表内容行或更高层级列表项之后的边界视为顶层首项。
pub(super) fn list_context(text: &str, line_start_at: usize, item: &ListItem) -> ListContext {
    let mut at = line_start_at;
    while at > 0 {
        let previous_end = at - 1;
        let previous_start = line_start(text, previous_end);
        let previous = &text[previous_start..previous_end];
        if previous.trim().is_empty() {
            return ListContext::TopLevel;
        }
        match parse_list_item(previous) {
            Some(previous_item) if previous_item.indent == item.indent => {
                return if same_list_family(previous_item.kind, item.kind) {
                    ListContext::Sibling
                } else {
                    ListContext::TopLevel
                };
            }
            Some(previous_item) if previous_item.indent < item.indent => {
                return ListContext::Nested;
            }
            Some(_) => {}
            None => {
                let indent = previous.len() - previous.trim_start_matches(' ').len();
                if indent <= item.indent {
                    return ListContext::TopLevel;
                }
            }
        }
        at = previous_start;
    }
    ListContext::TopLevel
}

/// 两个列表项是否属于同一列表：有序要求分隔符相同，无序/任务要求项目符号相同。
fn same_list_family(left: ListKind, right: ListKind) -> bool {
    match (left, right) {
        (
            ListKind::Ordered {
                delimiter: left, ..
            },
            ListKind::Ordered {
                delimiter: right, ..
            },
        ) => left == right,
        (ListKind::Bullet(_) | ListKind::Task(_), ListKind::Bullet(_) | ListKind::Task(_)) => {
            bullet_marker(left) == bullet_marker(right)
        }
        (ListKind::Ordered { .. }, _) | (_, ListKind::Ordered { .. }) => false,
    }
}

/// 无序/任务列表项的项目符号字符。
fn bullet_marker(kind: ListKind) -> Option<char> {
    match kind {
        ListKind::Bullet(marker) | ListKind::Task(marker) => Some(marker),
        ListKind::Ordered { .. } => None,
    }
}

use super::Prompt;

/// 列表续写编辑：把 `range` 替换为 `replacement`，并把光标落到 `cursor`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListEdit {
    pub(crate) range: core::ops::Range<usize>,
    pub(crate) replacement: String,
    pub(crate) cursor: usize,
}

/// 列表项续写换行；非列表语义返回 None（调用方回退为普通换行）。
///
/// 空列表项退出层级（顶层清空标记、嵌套回退到上级前缀），非空项在光标处
/// 插入换行 + 缩进 + 续写标记（有序序号 +1）。
pub(crate) fn list_continuation(text: &str, cursor: usize) -> Option<ListEdit> {
    let start = line_start(text, cursor);
    let end = line_end(text, cursor);
    let item = parse_list_item(&text[start..end])?;
    if cursor - start < item.marker_end {
        return None;
    }
    if text[start + item.marker_end..end].trim().is_empty() {
        let replacement = if item.indent == 0 {
            String::new()
        } else {
            outer_list_prefix(text, start, item.indent).unwrap_or_default()
        };
        let cursor = start + replacement.len();
        return Some(ListEdit {
            range: start..start + item.marker_end,
            replacement,
            cursor,
        });
    }
    let insert = format!("\n{}{}", " ".repeat(item.indent), item.continuation());
    let cursor = cursor + insert.len();
    Some(ListEdit {
        range: cursor - insert.len()..cursor - insert.len(),
        replacement: insert,
        cursor,
    })
}

/// 沿上文找最近的低缩进列表项，返回其缩进 + 标记，用于空项退出层级。
fn outer_list_prefix(text: &str, line_start_offset: usize, indent: usize) -> Option<String> {
    text[..line_start_offset]
        .split('\n')
        .rev()
        .find_map(|line| {
            let item = parse_list_item(line)?;
            (item.indent < indent)
                .then(|| format!("{}{}", " ".repeat(item.indent), item.continuation()))
        })
}

/// 光标所在行是否位于未闭合的围栏代码块内。
pub(crate) fn in_fence(text: &str, cursor: usize) -> bool {
    let start = line_start(text, cursor);
    let mut fence = false;
    for line in text[..start].split('\n') {
        if is_fence(line) {
            fence = !fence;
        }
    }
    fence
}

impl Prompt {
    /// 列表项续写换行；返回是否已按列表语义处理。
    pub(super) fn continue_list(&mut self) -> bool {
        let Some(edit) = list_continuation(&self.text, self.cursor) else {
            return false;
        };
        self.record(EditKind::Other);
        self.text.replace_range(edit.range, &edit.replacement);
        self.cursor = edit.cursor;
        self.settle();
        true
    }
    /// Backspace 的列表标记删除；对齐 lx-agent（CodeMirror `deleteMarkupBackward`）。
    ///
    /// 光标与分隔空格之间只剩空格时先删这些多余空格；在分隔空格处，
    /// 同列表非首项替换为等宽空格，列表首项整段删除（顶层含缩进，嵌套保留缩进）。
    pub(super) fn delete_list_markup(&mut self) -> bool {
        let start = line_start(&self.text, self.cursor);
        let end = line_end(&self.text, self.cursor);
        let line = &self.text[start..end];
        let Some(item) = parse_list_item(line) else {
            return false;
        };
        let offset = self.cursor - start;
        if offset > item.sep_end {
            if !line[item.sep_end..offset].bytes().all(|byte| byte == b' ') {
                return false;
            }
            self.record(EditKind::Delete);
            self.text
                .replace_range(start + item.sep_end..self.cursor, "");
            self.cursor = start + item.sep_end;
            self.settle();
            return true;
        }
        if offset < item.sep_end {
            return false;
        }
        self.record(EditKind::Delete);
        match list_context(&self.text, start, &item) {
            ListContext::Sibling => {
                let blanks = " ".repeat(item.sep_end - item.indent);
                self.text
                    .replace_range(start + item.indent..start + item.sep_end, &blanks);
                self.cursor = start + item.sep_end;
            }
            ListContext::Nested => {
                self.text
                    .replace_range(start + item.indent..start + item.sep_end, "");
                self.cursor = start + item.indent;
            }
            ListContext::TopLevel => {
                self.text.replace_range(start..start + item.sep_end, "");
                self.cursor = start;
            }
        }
        self.settle();
        true
    }
    /// 光标所在行是否位于未闭合的围栏代码块内。
    pub(super) fn in_fence(&self) -> bool {
        in_fence(&self.text, self.cursor)
    }
}
