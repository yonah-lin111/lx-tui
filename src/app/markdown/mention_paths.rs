//! 从正文提取普通 `@路径` 提及：供历史消息的引用路径汇总使用。
//!
//! 跳过 `@ref/` 短引用与 `@[refer-*](...)` 显式引用（它们由附件元数据承担），
//! 围栏内不解析。纯函数、无 IO。

use super::mention_token_ranges;

/// 从正文中提取全部文本提及路径：`(相对路径, 是否目录)`。
/// 跳过代码围栏；按文本出现顺序返回。
pub fn extract_mention_paths(text: &str) -> Vec<(String, bool)> {
    mention_token_ranges(text)
        .into_iter()
        .filter_map(|range| {
            let raw = &text[range];
            if raw.starts_with("@[refer-") || raw.starts_with(super::SHORT_REF_PREFIX) {
                return None;
            }
            let token = &raw[1..];
            let is_dir = token.ends_with('/');
            let path = token.trim_end_matches('/').to_string();
            (!path.is_empty()).then_some((path, is_dir))
        })
        .collect()
}

#[cfg(test)]
mod tests;
