//! `@[refer-type](path)` 引用语法：无损格式化、解析与保护区过滤，纯函数无 IO。

use std::ops::Range;
use std::path::Path;

/// 引用类型；与 `@[refer-*]` 标记一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferKind {
    File,
    Folder,
    Image,
}

impl ReferKind {
    /// 标记中的稳定类型名。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Image => "image",
        }
    }

    /// 从标记类型名解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "folder" => Some(Self::Folder),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

/// 光标处已出现的引用标记：字节区间、类型、完整原文与解码路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReference {
    pub range: Range<usize>,
    pub kind: ReferKind,
    /// 原文（含 `@[refer-…](…)`）。
    pub raw: String,
    /// 解码后的路径文本。
    pub path: String,
}

/// 引用标记的固定前缀。
const REFER_PREFIX: &str = "@[refer-";

/// 路径是否可用引用语法无损表达：不接受换行（无法转义）与 NUL。
pub fn reference_representable(path: &str) -> bool {
    !path.contains(['\n', '\r', '\0'])
}

/// 引用标记路径的转义：`\` → `\\`、`(` → `\(`、`)` → `\)`。
///
/// 与 `decode_path` 构成往返：对任意可表达路径 `decode(encode(p)) == p`。
pub fn encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for ch in path.chars() {
        match ch {
            '\\' => encoded.push_str("\\\\"),
            '(' => encoded.push_str("\\("),
            ')' => encoded.push_str("\\)"),
            ch => encoded.push(ch),
        }
    }
    encoded
}

/// 引用标记路径的解转义；未知转义按字面保留，与 `encode_path` 往返一致。
pub fn decode_path(encoded: &str) -> String {
    let mut decoded = String::with_capacity(encoded.len());
    let mut chars = encoded.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }
        match chars.next() {
            Some(next @ ('\\' | '(' | ')')) => decoded.push(next),
            Some(next) => {
                decoded.push('\\');
                decoded.push(next);
            }
            None => decoded.push('\\'),
        }
    }
    decoded
}

/// 完整引用标记文本；路径不可无损表达时返回 None。
pub fn format_reference(kind: ReferKind, path: &Path) -> Option<String> {
    let path = path.to_str()?;
    if !reference_representable(path) {
        return None;
    }
    Some(format!("@[refer-{}]({})", kind.as_str(), encode_path(path)))
}

/// 解析文本中全部引用标记；跳过代码围栏内的内容，坏标记跳过不产生错误。
pub fn parse_references(text: &str) -> Vec<ParsedReference> {
    parse_references_in(text, 0, text.len())
}

/// 解析 `[from, to)` 范围内的引用标记（字节区间按原文计）。
pub fn parse_references_in(text: &str, from: usize, to: usize) -> Vec<ParsedReference> {
    let to = to.min(text.len());
    if from >= to || !text.is_char_boundary(from) || !text.is_char_boundary(to) {
        return Vec::new();
    }
    let mut references = Vec::new();
    let mut index = from;
    while let Some(offset) = text[index..to].find(REFER_PREFIX) {
        let at = index + offset;
        if super::open_fence(&text[..at]).is_some() {
            index = at + REFER_PREFIX.len();
            continue;
        }
        match parse_reference_at(text, at, to) {
            Some(reference) => {
                index = reference.range.end;
                references.push(reference);
            }
            None => index = at + REFER_PREFIX.len(),
        }
    }
    references
}

/// 解析起点处的引用标记；结构完整且类型合法时返回。
fn parse_reference_at(text: &str, at: usize, limit: usize) -> Option<ParsedReference> {
    let rest = text.get(at + REFER_PREFIX.len()..limit)?;
    let type_end = rest.find(']')?;
    let kind = ReferKind::parse(rest.get(..type_end)?)?;
    if rest.as_bytes().get(type_end + 1) != Some(&b'(') {
        return None;
    }
    let body_start = at + REFER_PREFIX.len() + type_end + 2;
    let body_end = find_closing_paren(text, body_start, limit)?;
    let encoded = text.get(body_start..body_end)?;
    Some(ParsedReference {
        range: at..body_end + 1,
        kind,
        raw: text[at..body_end + 1].to_string(),
        path: decode_path(encoded),
    })
}

/// 从 `body_start` 起查找未转义的 `)`；找不到返回 None。
fn find_closing_paren(text: &str, body_start: usize, limit: usize) -> Option<usize> {
    let mut escaped = false;
    for (offset, ch) in text.get(body_start..limit)?.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            ')' => return Some(body_start + offset),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests;
