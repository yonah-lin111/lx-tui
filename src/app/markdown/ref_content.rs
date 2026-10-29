//! `@[ref-content]/相对路径` 快捷引用：解析、边界识别与整块快捷删除。
//!
//! 格式：`@[ref-content]/` + 完整相对路径，文件夹补尾 `/`。
//! 纯函数、无 IO。

use std::ops::Range;

use super::{is_mention_query_char, open_fence};

/// 快捷引用的固定前缀。
pub const REF_CONTENT_PREFIX: &str = "@[ref-content]/";

/// 格式化为快捷引用标记：`@[ref-content]/<path>`，默认保留最后两个 `/` 的内容（末两段），文件夹补尾 `/`。
pub fn format_ref_content(path: &str, is_directory: bool) -> String {
    let name = super::short_ref_name(path).unwrap_or_else(|| path.trim_matches('/').to_string());
    if is_directory {
        format!("{REF_CONTENT_PREFIX}{name}/")
    } else {
        format!("{REF_CONTENT_PREFIX}{name}")
    }
}

/// 快捷引用正文长度（从 `body_start` 起）：供高亮与解析共用同一套边界规则。
pub fn ref_content_body_len(text: &str, body_start: usize) -> usize {
    let mut end = body_start;
    for ch in text[body_start..].chars() {
        if !is_ref_content_body_char(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    ref_content_body_end(text, body_start, end) - body_start
}

/// 快捷引用整块删除范围：光标紧跟其后空白时返回（起点..光标），供 Backspace 一次删净。
///
/// 与 `mention_deletion_range` / `reference_deletion_range` 同构：只有标记完整结束于
/// 光标前的空白处才命中；光标不在空白后返回 None（走普通退格）。
pub fn ref_content_deletion_range(text: &str, cursor: usize) -> Option<Range<usize>> {
    if cursor == 0 || cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let previous = text[..cursor].chars().next_back()?;
    if previous != ' ' && previous != '\t' {
        return None;
    }
    let end = cursor - previous.len_utf8();
    ref_content_ranges(text)
        .into_iter()
        .find(|range| range.end == end)
        .map(|range| range.start..cursor)
}

/// 全文中的快捷引用区间（含 `@[ref-content]/` 前缀，不含尾随空白）。
///
/// 前缀必须在行首或空白之后，且不在代码围栏内；正文由路径字符组成（`/` 与 `.` 属于正文，空白/标点终止），
/// 末尾单独成句的 `.` 视为句子标点，不计入标记。
pub fn ref_content_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find(REF_CONTENT_PREFIX) {
        let at = index + offset;
        let boundary = text[..at]
            .chars()
            .next_back()
            .is_none_or(|previous| previous.is_whitespace());
        let body_start = at + REF_CONTENT_PREFIX.len();
        let mut end = body_start;
        if boundary && open_fence(&text[..at]).is_none() {
            for ch in text[body_start..].chars() {
                if !is_ref_content_body_char(ch) {
                    break;
                }
                end += ch.len_utf8();
            }
        }
        let end = ref_content_body_end(text, body_start, end);
        if end > body_start {
            ranges.push(at..end);
            index = end;
            continue;
        }
        index = body_start;
    }
    ranges
}

/// 标记正文的实际结束位置：去掉整体只有句点的尾缀（`@[ref-content]/a/b.` → `@[ref-content]/a/b`）。
fn ref_content_body_end(text: &str, body_start: usize, end: usize) -> usize {
    let body = &text[body_start..end];
    let trimmed = body.trim_end_matches('.');
    if trimmed.is_empty() {
        return end;
    }
    body_start + trimmed.len()
}

/// 允许的正文字符：非 `@` 且为有效查询字符。
fn is_ref_content_body_char(ch: char) -> bool {
    ch != '@' && is_mention_query_char(ch)
}

/// 从正文中提取全部文本提及路径与快捷引用路径：`(相对路径, 是否目录)`。
/// 跳过代码围栏；按文本出现顺序返回。
pub fn extract_mention_paths(text: &str) -> Vec<(String, bool)> {
    let mut results = Vec::new();

    // 1. 提取 @[ref-content]/
    for range in ref_content_ranges(text) {
        let body = &text[range.start + REF_CONTENT_PREFIX.len()..range.end];
        let is_dir = body.ends_with('/');
        let path = body.trim_end_matches('/').to_string();
        if !path.is_empty() {
            results.push((path, is_dir));
        }
    }

    // 2. 提取常规 @路径（排除 @[ref-content]/ 与 @[refer-）
    let mut index = 0;
    while let Some(offset) = text[index..].find('@') {
        let at = index + offset;
        if text[at..].starts_with(REF_CONTENT_PREFIX)
            || text[at..].starts_with("@[refer-")
            || text[at..].starts_with("@ref/")
        {
            index = at + 1;
            continue;
        }
        let boundary = text[..at]
            .chars()
            .next_back()
            .is_none_or(|previous| previous.is_whitespace());
        if boundary && open_fence(&text[..at]).is_none() {
            let mut end = at + 1;
            for ch in text[end..].chars() {
                if !is_mention_query_char(ch) || ch == '@' {
                    break;
                }
                end += ch.len_utf8();
            }
            if end > at + 1 {
                let token = &text[at + 1..end];
                let is_dir = token.ends_with('/');
                let path = token.trim_end_matches('/').to_string();
                if !path.is_empty() {
                    results.push((path, is_dir));
                }
            }
            index = end;
        } else {
            index = at + 1;
        }
    }

    results
}

#[cfg(test)]
mod tests;
