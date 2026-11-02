//! 会话与模板块标题生成的提示词组装、请求构造与响应清洗（对齐 codex `thread_title.rs`）。
//!
//! 保持纯函数与零 IO：提示词统一使用英文指令，要求输出跟随用户语言、单行、
//! 不超过 36 个字符且不含引号、Markdown 或末尾标点。

use std::sync::Arc;

use crate::chat::events::{ChatRequest, ChatTurn, ModelItem, RequestPurpose};
use crate::db::chat::ModelSnapshot;

/// 标题最大字符数（按 Unicode 标量值计数，对齐 codex `THREAD_TITLE_MAX_CHARS = 36`）。
pub const TITLE_MAX_CHARS: usize = 36;
/// 标题生成 Prompt 最大字节数（对齐 codex `THREAD_TITLE_PROMPT_MAX_BYTES = 960`）。
pub const TITLE_PROMPT_MAX_BYTES: usize = 960;
/// 标题生成整次超时秒数（对齐 codex 30s）。
pub const TITLE_TIMEOUT_SECS: u64 = 30;
/// 标题响应流最大累计字节数（8 KiB）。
pub const TITLE_MAX_OUTPUT_BYTES: usize = 8 * 1024;

/// 基础英文指令（对齐 codex `thread_title_instructions`）。
pub fn title_instructions() -> &'static str {
    "Generate a concise, single-line task title of at most 36 characters and under five words \
where possible. Start with an imperative verb. Capitalize only the first word unless the user's \
language, proper nouns, acronyms, or code terms require otherwise. Preserve ticket references \
exactly. Write in the user's language. Do not use quotes, markdown, or trailing punctuation. \
Do not answer the request. Output only the title text."
}

/// 按 UTF-8 字符边界安全截断字符串字节数。
pub fn truncate_utf8_bytes(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &text[..boundary]
}

/// 组装新对话标题生成 Prompt（总长钳制在 960 字节内）。
pub fn session_title_prompt(user_message: &str) -> Option<String> {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return None;
    }
    let prefix = format!("{}\n\nUser prompt:\n", title_instructions());
    let budget = TITLE_PROMPT_MAX_BYTES.saturating_sub(prefix.len());
    let body = truncate_utf8_bytes(trimmed, budget).trim_end();
    if body.is_empty() {
        return None;
    }
    Some(format!("{prefix}{body}"))
}

/// 组装 Prompt 模板块（`&&&` / `+++`）标题生成 Prompt（总长钳制在 960 字节内）。
pub fn template_block_title_prompt(block_content: &str) -> Option<String> {
    let trimmed = block_content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let prefix = format!("{}\n\nTask template content:\n", title_instructions());
    let budget = TITLE_PROMPT_MAX_BYTES.saturating_sub(prefix.len());
    let body = truncate_utf8_bytes(trimmed, budget).trim_end();
    if body.is_empty() {
        return None;
    }
    Some(format!("{prefix}{body}"))
}

/// 构造无工具、无额外系统提示开销的轻量级 `ChatRequest`。
pub fn build_title_request(prompt: &str, model: &ModelSnapshot) -> ChatRequest {
    ChatRequest {
        turns: vec![ChatTurn {
            role: "user".into(),
            text: prompt.to_string(),
            reasoning_metadata: None,
        }],
        items: vec![ModelItem::UserText(prompt.to_string())],
        model: model.clone(),
        variant_params: None,
        purpose: RequestPurpose::CompactSummary,
        system_prompt: Some(String::new()),
        tools: Arc::new(Vec::new()),
    }
}

/// 剥离 `<think>...</think>` 与 `<thinking>...</thinking>` 标签块。
pub fn strip_think_blocks(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut cursor = raw;
    while let Some((start_idx, close_tag)) = next_think_tag(cursor) {
        result.push_str(&cursor[..start_idx]);
        let after_start = &cursor[start_idx..];
        if let Some(end_idx) = after_start.find(close_tag) {
            cursor = &after_start[end_idx + close_tag.len()..];
        } else {
            cursor = "";
            break;
        }
    }
    result.push_str(cursor);
    result
}

fn next_think_tag(text: &str) -> Option<(usize, &'static str)> {
    let think = text.find("<think>").map(|idx| (idx, "</think>"));
    let thinking = text.find("<thinking>").map(|idx| (idx, "</thinking>"));
    match (think, thinking) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// 解析并清洗模型返回的标题文本（兼容 JSON `{"title":"..."}` 与纯文本首行）。
pub fn parse_generated_title(raw: &str) -> Option<String> {
    let stripped = strip_think_blocks(raw);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if trimmed.starts_with('{') {
        serde_json::from_str::<serde_json::Value>(trimmed)
            .ok()
            .and_then(|val| {
                val.get("title")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| first_non_empty_line(trimmed).to_string())
    } else {
        first_non_empty_line(trimmed).to_string()
    };
    sanitize_title(&candidate)
}

fn first_non_empty_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

fn sanitize_title(text: &str) -> Option<String> {
    let mut s = text.trim();
    const QUOTES: &[char] = &['"', '\'', '`', '“', '”', '‘', '’'];
    s = s.trim_matches(QUOTES).trim();
    s = s.trim_start_matches('#').trim();
    s = s.trim_matches(QUOTES).trim();
    const TRAILING_PUNCT: &[char] = &['.', '?', '!', '。', '？', '！', ':', '：', ';', '；'];
    s = s.trim_end_matches(TRAILING_PUNCT).trim();
    if s.is_empty() {
        return None;
    }
    let mut cleaned = String::new();
    let mut last_space = false;
    for ch in s.chars() {
        if ch == '{' || ch == '}' {
            continue;
        }
        if ch.is_whitespace() {
            if !last_space {
                cleaned.push(' ');
                last_space = true;
            }
        } else {
            cleaned.push(ch);
            last_space = false;
        }
    }
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return None;
    }
    let truncated: String = cleaned.chars().take(TITLE_MAX_CHARS).collect();
    let final_title = truncated.trim_end_matches(TRAILING_PUNCT).trim();
    if final_title.is_empty() {
        None
    } else {
        Some(final_title.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_clamps_within_byte_limit_on_utf8_boundary() {
        let long_cjk = "修复设置页模型与供应商拆分以及自动生成标题问题".repeat(40);
        let prompt = session_title_prompt(&long_cjk).expect("prompt");
        assert!(prompt.len() <= TITLE_PROMPT_MAX_BYTES);
        assert!(prompt.starts_with(title_instructions()));

        let tpl_prompt = template_block_title_prompt(&long_cjk).expect("template prompt");
        assert!(tpl_prompt.len() <= TITLE_PROMPT_MAX_BYTES);
        assert!(tpl_prompt.contains("Task template content:"));
    }

    #[test]
    fn empty_input_returns_none() {
        assert!(session_title_prompt("   \n\t ").is_none());
        assert!(template_block_title_prompt("").is_none());
        assert!(parse_generated_title("   ").is_none());
        assert!(parse_generated_title("<think>only thoughts</think>").is_none());
    }

    #[test]
    fn parse_generated_title_handles_think_tags_json_and_punctuation() {
        let raw = "<think>\nLet me summarize this.\n</think>\n\"# Fix login token refresh!\"";
        assert_eq!(
            parse_generated_title(raw).as_deref(),
            Some("Fix login token refresh")
        );

        let json_raw = r#"{"title": "重构设置页 Model 与 Provider 标签。"}"#;
        assert_eq!(
            parse_generated_title(json_raw).as_deref(),
            Some("重构设置页 Model 与 Provider 标签")
        );
    }

    #[test]
    fn parse_generated_title_strips_braces_and_truncates_to_36_chars() {
        let raw = "Implement {title: test} with a very long title exceeding thirty six characters";
        let parsed = parse_generated_title(raw).expect("parsed");
        assert!(!parsed.contains('{') && !parsed.contains('}'));
        assert!(parsed.chars().count() <= TITLE_MAX_CHARS);
    }
}
