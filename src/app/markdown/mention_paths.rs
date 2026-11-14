//! 从正文提取普通 `@路径` 提及：供历史消息的引用路径汇总使用。
//!
//! 跳过 `@ref/` 短引用与 `@[refer-*](...)` 显式引用（它们由附件元数据承担），
//! 围栏内不解析。纯函数、无 IO。

use super::{is_mention_query_char, open_fence};

/// 从正文中提取全部文本提及路径：`(相对路径, 是否目录)`。
/// 跳过代码围栏；按文本出现顺序返回。
pub fn extract_mention_paths(text: &str) -> Vec<(String, bool)> {
    let mut results = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find('@') {
        let at = index + offset;
        if text[at..].starts_with("@[refer-") || text[at..].starts_with("@ref/") {
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
