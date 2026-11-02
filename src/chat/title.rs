//! 会话与模板块标题生成的提示词组装、请求构造与响应清洗（对齐 codex `thread_title.rs` 与 lx-agent `titleGenerator.ts`）。
//!
//! 保持纯函数与零 IO：提示词统一使用英文编写（遵守模型提示词全英文规范），
//! 按配置的 `TitleLanguage`（默认 `Auto` 自动跟随用户输入语言，或强制指定语言）
//! 明确约束模型输出对应语言的简洁单行标题，并拦截模型拒答或元解释文本。

use std::sync::Arc;

use crate::chat::events::{ChatRequest, ChatTurn, ModelItem, RequestPurpose};
use crate::config::ai::TitleLanguage;
use crate::db::chat::ModelSnapshot;

/// 标题最大字符数（按 Unicode 标量值计数，对齐 codex `THREAD_TITLE_MAX_CHARS = 36`）。
pub const TITLE_MAX_CHARS: usize = 36;
/// 标题生成 Prompt 最大字节数（对齐 codex `THREAD_TITLE_PROMPT_MAX_BYTES = 960`）。
pub const TITLE_PROMPT_MAX_BYTES: usize = 960;
/// 标题生成整次超时秒数（对齐 codex 30s）。
pub const TITLE_TIMEOUT_SECS: u64 = 30;
/// 标题响应流最大累计字节数（8 KiB）。
pub const TITLE_MAX_OUTPUT_BYTES: usize = 8 * 1024;
/// 标题生成专用非空系统提示词（避免各 Transport 发送空字符串 system 块导致 400）。
pub const TITLE_SYSTEM_PROMPT: &str = "You are a concise conversation and task title generator. Output only a single-line title text with no quotes, markdown, braces, or trailing punctuation. Never refuse and never answer the user's request.";

/// 按配置的 `TitleLanguage` 与输入样本文本返回对应的英文系统指令。
pub fn title_instructions_for(language: TitleLanguage, sample_text: &str) -> &'static str {
    let resolved = match language {
        TitleLanguage::Auto => {
            if contains_japanese_kana(sample_text) {
                TitleLanguage::Japanese
            } else if contains_korean_hangul(sample_text) {
                TitleLanguage::Korean
            } else if contains_traditional_cjk(sample_text) {
                TitleLanguage::TraditionalChinese
            } else if contains_cjk(sample_text) {
                TitleLanguage::SimplifiedChinese
            } else {
                TitleLanguage::Auto
            }
        }
        other => other,
    };
    match resolved {
        TitleLanguage::SimplifiedChinese => {
            "Generate a concise, single-line title in Simplified Chinese (at most 20 Chinese \
characters) that summarizes the topic or intent of the input. Whether the input is a coding task, \
a technical question, a casual greeting, or an identity query (such as asking who you are or \
saying hello), always summarize its topic into a short title (such as an identity inquiry or \
casual greeting). You MUST write the title in Simplified Chinese regardless of English template \
labels, code snippets, or file paths. NEVER refuse, and NEVER output meta-explanations stating \
that a title cannot be generated or that the input is not a task description. Do not use quotes, \
markdown, braces, or trailing punctuation. Do not answer the request. Output only the title text."
        }
        TitleLanguage::TraditionalChinese => {
            "Generate a concise, single-line title in Traditional Chinese (at most 20 Chinese \
characters) that summarizes the topic or intent of the input. Whether the input is a coding task, \
a question, or a casual greeting, always summarize its topic into a short title. You MUST write \
the title in Traditional Chinese regardless of English template labels, code snippets, or file \
paths. NEVER refuse or output meta-explanations. Do not use quotes, markdown, braces, or trailing \
punctuation. Do not answer the request. Output only the title text."
        }
        TitleLanguage::Japanese => {
            "Generate a concise, single-line title in Japanese (at most 24 characters) that \
summarizes the topic or intent of the input. Whether the input is a coding task, a question, or a \
casual greeting, always summarize its topic into a short title. You MUST write the title in \
Japanese regardless of English template labels, code snippets, or file paths. NEVER refuse or \
output meta-explanations. Do not use quotes, markdown, braces, or trailing punctuation. Do not \
answer the request. Output only the title text."
        }
        TitleLanguage::Korean => {
            "Generate a concise, single-line title in Korean (at most 24 characters) that \
summarizes the topic or intent of the input. Whether the input is a coding task, a question, or a \
casual greeting, always summarize its topic into a short title. You MUST write the title in \
Korean regardless of English template labels, code snippets, or file paths. NEVER refuse or \
output meta-explanations. Do not use quotes, markdown, braces, or trailing punctuation. Do not \
answer the request. Output only the title text."
        }
        TitleLanguage::English => {
            "Generate a concise, single-line title in English of at most 36 characters and under \
five words where possible that summarizes the topic or intent of the input. Whether the input is \
a coding task, a question, or a casual greeting, always summarize its topic into a short title. \
You MUST write the title in English. NEVER refuse or output meta-explanations. Do not use quotes, \
markdown, braces, or trailing punctuation. Do not answer the request. Output only the title text."
        }
        TitleLanguage::Auto => {
            "Generate a concise, single-line title of at most 36 characters that summarizes the \
topic or intent of the input. Whether the input is a coding task, a question, or a casual \
greeting, always summarize its topic into a short title. Ignore English template field labels \
(such as '- Description:', '- Requirements:', '- Reference:', '- Location:', '- Notes:'), file \
paths, and code identifiers when determining the user's language, and write the title in the \
natural language used by the user. NEVER refuse or output meta-explanations. Do not use quotes, \
markdown, braces, or trailing punctuation. Do not answer the request. Output only the title text."
        }
    }
}

fn contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        ('\u{4e00}'..='\u{9fff}').contains(&ch) || ('\u{3400}'..='\u{4dbf}').contains(&ch)
    })
}

fn contains_traditional_cjk(text: &str) -> bool {
    const TRADITIONAL_ONLY: &str =
        "體國學發會經開關見長門問間題點現實機電語網設訂優頁塊標傳遞處裡應當為與專業顯態誤歷紀錄產";
    const SIMPLIFIED_ONLY: &str =
        "体国学发会经开关见长门问间题点现实机电语网设订优页块标传递处里应当为与专业显态误历纪录产";
    let has_trad = text.chars().any(|ch| TRADITIONAL_ONLY.contains(ch));
    let has_simp = text.chars().any(|ch| SIMPLIFIED_ONLY.contains(ch));
    has_trad && !has_simp
}

fn contains_japanese_kana(text: &str) -> bool {
    // 仅匹配平假名 (3041..=3096) 与片假名音节 (30A1..=30FA)，排除中点 `・` (U+30FB) 等共用标点。
    text.chars().any(|ch| {
        ('\u{3041}'..='\u{3096}').contains(&ch) || ('\u{30a1}'..='\u{30fa}').contains(&ch)
    })
}

fn contains_korean_hangul(text: &str) -> bool {
    text.chars().any(|ch| {
        ('\u{ac00}'..='\u{d7af}').contains(&ch) || ('\u{3131}'..='\u{318e}').contains(&ch)
    })
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

/// 组装新对话标题生成 Prompt（默认语言）。
pub fn session_title_prompt(user_message: &str) -> Option<String> {
    session_title_prompt_for(user_message, TitleLanguage::default())
}

/// 按指定语言组装新对话标题生成 Prompt（总长钳制在 960 字节内）。
pub fn session_title_prompt_for(user_message: &str, language: TitleLanguage) -> Option<String> {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return None;
    }
    let instructions = title_instructions_for(language, trimmed);
    let prefix = format!("{instructions}\n\nConversation content:\n");
    let budget = TITLE_PROMPT_MAX_BYTES.saturating_sub(prefix.len());
    let body = truncate_utf8_bytes(trimmed, budget).trim_end();
    if body.is_empty() {
        return None;
    }
    Some(format!("{prefix}{body}"))
}

/// 组装 Prompt 模板块（`&&&` / `+++`）标题生成 Prompt（默认语言）。
pub fn template_block_title_prompt(block_content: &str) -> Option<String> {
    template_block_title_prompt_for(block_content, TitleLanguage::default())
}

/// 按指定语言组装 Prompt 模板块（`&&&` / `+++`）标题生成 Prompt（总长钳制在 960 字节内）。
pub fn template_block_title_prompt_for(
    block_content: &str,
    language: TitleLanguage,
) -> Option<String> {
    let trimmed = block_content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let instructions = title_instructions_for(language, trimmed);
    let prefix = format!("{instructions}\n\nTask template content:\n");
    let budget = TITLE_PROMPT_MAX_BYTES.saturating_sub(prefix.len());
    let body = truncate_utf8_bytes(trimmed, budget).trim_end();
    if body.is_empty() {
        return None;
    }
    Some(format!("{prefix}{body}"))
}

/// 构造无工具、带轻量非空系统提示词的 `ChatRequest`。
pub fn build_title_request(prompt: &str, model: &ModelSnapshot) -> ChatRequest {
    ChatRequest {
        turns: vec![
            ChatTurn {
                role: "system".into(),
                text: TITLE_SYSTEM_PROMPT.to_string(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "user".into(),
                text: prompt.to_string(),
                reasoning_metadata: None,
            },
        ],
        items: vec![ModelItem::UserText(prompt.to_string())],
        model: model.clone(),
        variant_params: None,
        purpose: RequestPurpose::CompactSummary,
        system_prompt: Some(TITLE_SYSTEM_PROMPT.to_string()),
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

/// 剥离外层 Markdown 代码块围栏（如 ```` ```json ... ``` ````）。
fn strip_code_fences(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let after_lang = match rest.find('\n') {
        Some(idx) => &rest[idx + 1..],
        None => return trimmed,
    };
    after_lang.strip_suffix("```").unwrap_or(after_lang).trim()
}

/// 解析并清洗模型返回的标题文本（兼容代码围栏、JSON `{"title":"..."}` 与纯文本首行，并拦截拒答文本）。
pub fn parse_generated_title(raw: &str) -> Option<String> {
    let stripped = strip_think_blocks(raw);
    let unfenced = strip_code_fences(&stripped);
    if unfenced.is_empty() {
        return None;
    }
    let candidate = if unfenced.starts_with('{') {
        match serde_json::from_str::<serde_json::Value>(unfenced) {
            Ok(val) => val
                .get("title")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)?,
            Err(_) => first_non_empty_line(unfenced).to_string(),
        }
    } else {
        first_non_empty_line(unfenced).to_string()
    };
    if is_refusal_title(&candidate) {
        return None;
    }
    let sanitized = sanitize_title(&candidate)?;
    if is_refusal_title(&sanitized) {
        return None;
    }
    Some(sanitized)
}

fn first_non_empty_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("```"))
        .unwrap_or("")
}

/// 检测模型是否输出了「无法生成任务标题：输入不是任务描述」之类的拒答或元解释话术。
fn is_refusal_title(title: &str) -> bool {
    const DIRECT_REFUSALS: &[&str] = &[
        "无法生成标题",
        "无法生成任务标题",
        "不能生成标题",
        "不能生成任务标题",
        "无法提炼标题",
        "无法总结标题",
        "无法概括标题",
        "不是任务描述",
        "未提供任务",
        "无有效任务",
        "无任务描述",
        "無法生成標題",
        "無法生成任務標題",
        "不能生成標題",
        "不是任務描述",
        "未提供任務",
        "タイトルを生成できません",
        "タイトルを作成できません",
        "要約できません",
        "제목을 생성할 수 없습니다",
        "제목을 만들 수 없습니다",
        "요약할 수 없습니다",
    ];
    if DIRECT_REFUSALS.iter().any(|pat| title.contains(pat)) {
        return true;
    }
    let has_cjk_cannot = [
        "无法生成",
        "不能生成",
        "无法提炼",
        "无法总结",
        "无法概括",
        "無法生成",
    ]
    .iter()
    .any(|pat| title.starts_with(pat));
    let has_cjk_meta = ["标题", "標題", "任务", "任務", "输入", "輸入"]
        .iter()
        .any(|pat| title.contains(pat));
    if has_cjk_cannot && has_cjk_meta {
        return true;
    }
    let lower = title.to_ascii_lowercase();
    const EN_REFUSALS: &[&str] = &[
        "cannot generate title",
        "cannot generate a title",
        "can't generate title",
        "can't generate a title",
        "unable to generate title",
        "unable to generate a title",
        "not a task description",
        "input is not a task",
        "no task description",
        "cannot summarize",
    ];
    EN_REFUSALS.iter().any(|pat| lower.contains(pat))
}

fn strip_title_label_prefix(text: &str) -> &str {
    let trimmed = text.trim();
    for prefix in [
        "Title:",
        "title:",
        "TITLE:",
        "标题：",
        "标题:",
        "標題：",
        "標題:",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    trimmed
}

fn sanitize_title(text: &str) -> Option<String> {
    let mut s = text.trim();
    const QUOTES: &[char] = &['"', '\'', '`', '“', '”', '‘', '’'];
    s = s.trim_matches(QUOTES).trim();
    s = s.trim_start_matches('#').trim();
    s = strip_title_label_prefix(s);
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
mod tests;
