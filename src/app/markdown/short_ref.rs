//! `@ref/短名` 引用短格式：由长路径生成短名、解析短名、整块删除识别，纯函数无 IO。
//!
//! 格式：`@ref/` + 路径末两段（`a/b`），文件夹补尾 `/`；不带绝对路径、不以 `/` 开头，
//! 避免读成"根目录"。类型不再写 `refer-*`，改由形状推断（尾 `/` 文件夹、图片扩展名图片）。
//! 短名只是"同一片段内已出现过的路径"的简称，完整路径仍以文档中已有的长标记为准。

use std::ops::Range;

use super::is_mention_query_char;

/// 短引用的固定前缀。
pub const SHORT_REF_PREFIX: &str = "@ref/";

/// 短名展示用的路径段数（末几段）。
pub const SHORT_REF_SEGMENTS: usize = 2;

/// 图片扩展名（小写比较）：短名据此推断 `image` 类型。
const IMAGE_EXTENSIONS: [&str; 9] = [
    "avif", "bmp", "gif", "jpeg", "jpg", "png", "svg", "tif", "webp",
];

/// 路径的可展示段：按 `/` 与 `\` 切分并去掉空段。
///
/// 反斜杠同时作为分隔符处理，Windows 风格路径与 POSIX 路径得到同一组段。
fn path_segments(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// 路径末两段组成的短名（`a/b`）；不足两段时用现有全部段，无段返回 None。
///
/// 尾部分隔符先剔除，`/tmp/shots/` 与 `/tmp/shots` 得到同一短名。
pub fn short_ref_name(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let segments = path_segments(trimmed);
    if segments.is_empty() {
        return None;
    }
    let tail = &segments[segments.len().saturating_sub(SHORT_REF_SEGMENTS)..];
    Some(tail.join("/"))
}

/// 短引用标记文本：`@ref/末两段`，文件夹补尾 `/`。
///
/// 路径无法给出短名时返回 None（调用方回退到原长文本）。
pub fn format_short_ref(path: &str, is_directory: bool) -> Option<String> {
    let name = short_ref_name(path)?;
    short_ref_marker(&name, is_directory)
}

/// 由已算好的短名拼标记文本；`name` 为空时返回 None。
pub fn short_ref_marker(name: &str, is_directory: bool) -> Option<String> {
    let name = name.trim_matches('/');
    if name.is_empty() {
        return None;
    }
    Some(if is_directory {
        format!("{SHORT_REF_PREFIX}{name}/")
    } else {
        format!("{SHORT_REF_PREFIX}{name}")
    })
}

/// 短引用的类型：由形状推断（尾 `/` 文件夹、图片扩展名图片、其余文件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortRefKind {
    File,
    Folder,
    Image,
}

/// 从短引用标记的正文（`@ref/` 之后）推断类型。
pub fn short_ref_kind(body: &str) -> ShortRefKind {
    if body.ends_with('/') {
        return ShortRefKind::Folder;
    }
    let name = body.rsplit('/').next().unwrap_or(body);
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
        .unwrap_or_default();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        ShortRefKind::Image
    } else {
        ShortRefKind::File
    }
}

/// 短名是否可用作面板匹配：非空且不含空白。
pub fn short_ref_matchable(body: &str) -> bool {
    !body.is_empty() && !body.chars().any(char::is_whitespace)
}

/// 短引用整块删除范围：光标紧跟其后空白时返回（起点..光标），供 Backspace 一次删净。
///
/// 与 `mention_deletion_range` / `reference_deletion_range` 同构：只有标记完整结束于
/// 光标前的空白处才命中；光标不在空白后返回 None（走普通退格）。
pub fn short_ref_deletion_range(text: &str, cursor: usize) -> Option<Range<usize>> {
    if cursor == 0 || cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let previous = text[..cursor].chars().next_back()?;
    if previous != ' ' && previous != '\t' {
        return None;
    }
    let end = cursor - previous.len_utf8();
    short_ref_ranges(text)
        .into_iter()
        .find(|range| range.end == end)
        .map(|range| range.start..cursor)
}

/// 全文中的短引用区间（含 `@ref/` 前缀，不含尾随空白）。
///
/// 前缀必须在行首或空白之后；正文由路径字符组成（`/` 与 `.` 属于正文，空白/标点终止），
/// 末尾单独成句的 `.` 视为句子标点，不计入标记。
pub fn short_ref_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find(SHORT_REF_PREFIX) {
        let at = index + offset;
        let boundary = text[..at]
            .chars()
            .next_back()
            .is_none_or(|previous| previous.is_whitespace());
        let body_start = at + SHORT_REF_PREFIX.len();
        let mut end = body_start;
        if boundary {
            for ch in text[body_start..].chars() {
                if !is_short_ref_body_char(ch) {
                    break;
                }
                end += ch.len_utf8();
            }
        }
        let end = short_ref_body_end(text, body_start, end);
        if end > body_start {
            ranges.push(at..end);
            index = end;
            continue;
        }
        index = body_start;
    }
    ranges
}

/// 标记正文的实际结束位置：去掉整体只有句点的尾缀（`@ref/a/b.` → `@ref/a/b`）。
///
/// 只在末尾 `.` 之后紧邻文末或空白时才剔除；`a.md` 这种带扩展名的保持原样。
fn short_ref_body_end(text: &str, body_start: usize, end: usize) -> usize {
    let body = &text[body_start..end];
    let trimmed = body.trim_end_matches('.');
    if trimmed.is_empty() {
        return end;
    }
    body_start + trimmed.len()
}

/// 短引用正文允许的字符：路径字符（`is_mention_query_char` 已排除 `,` `;` `:` `!` `?`
/// 与各类括号/中文标点），`.` 属于正文；`@` 与空白终止。
fn is_short_ref_body_char(ch: char) -> bool {
    ch != '@' && is_mention_query_char(ch)
}

/// 短引用正文长度（从 `body_start` 起）：供高亮与解析共用同一套边界规则。
pub fn short_ref_body_len(text: &str, body_start: usize) -> usize {
    let mut end = body_start;
    for ch in text[body_start..].chars() {
        if !is_short_ref_body_char(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    short_ref_body_end(text, body_start, end) - body_start
}

#[cfg(test)]
mod tests;
