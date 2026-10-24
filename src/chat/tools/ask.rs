//! `ask` 工具的参数模型、校验与结果构造（纯函数，无副作用）。
//!
//! 参数校验沿用 [`super::plan_args`] 的口径：手工逐字段、字段白名单、未知字段直接
//! 拒绝，错误消息是稳定英文并带题号/字段名，方便模型自我修正。**整次拒绝**：任何一条
//! 不合法都不产生部分结果。
//!
//! 校验顺序固定为 结构 → 数量 → 字符上限/控制字符 → 重复 → preview 限额。先比字符数
//! 再逐字符扫描，因此 1 MiB 级别输入不会在控制字符检查上做无用遍历。

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use super::types::{ToolExecutionResult, ToolFailure, ToolResult};

/// 稳定工具名。
pub const ASK_TOOL: &str = "ask";

/// 单次调用最多题数。
pub const MAX_ASK_QUESTIONS: usize = 3;
/// 每题最多选项数（0 合法：纯自由输入题）。
pub const MAX_ASK_OPTIONS: usize = 4;
/// `header` 字符上限。
pub const MAX_ASK_HEADER_CHARS: usize = 12;
/// 题干字符上限。
pub const MAX_ASK_QUESTION_CHARS: usize = 500;
/// 选项标签字符上限。
pub const MAX_ASK_LABEL_CHARS: usize = 80;
/// 选项说明字符上限。
pub const MAX_ASK_DESC_CHARS: usize = 200;
/// `id` 字符上限。
pub const MAX_ASK_ID_CHARS: usize = 64;
/// 单个 preview 字节上限。
pub const MAX_ASK_PREVIEW_BYTES: usize = 4096;
/// 单个 preview 行数上限。
pub const MAX_ASK_PREVIEW_LINES: usize = 40;
/// preview 单行字符上限。
pub const MAX_ASK_PREVIEW_LINE_CHARS: usize = 400;
/// 整次调用所有 preview 合计字节上限。
pub const MAX_ASK_PREVIEW_TOTAL_BYTES: usize = 24576;
/// 单题答案（含手写文本）合计字符上限。
pub const MAX_ASK_ANSWER_CHARS: usize = 8192;
/// 单题手写文本字节上限。
pub const MAX_ASK_CUSTOM_BYTES: usize = 8192;

/// 一道提问（已校验的模型参数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub multi_select: bool,
    pub options: Vec<AskOption>,
}

/// 一个选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskOption {
    pub label: String,
    pub description: String,
    /// 纯文本等宽草图；渲染时逐字保留（含前导空格）。
    pub preview: Option<String>,
}

/// 一道题的作答记录：勾选项在前，手写文本在最后。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsweredQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub answers: Vec<String>,
}

/// 按题目 `id` 汇总的答案（客户端把同题勾选与手写文本放进同一数组）。
pub type AskAnswers = BTreeMap<String, Vec<String>>;

/// 结构阶段的中间表示：借用参数 JSON 的字符串，避免逐字段提前复制。
struct RawQuestion<'a> {
    id: &'a str,
    header: &'a str,
    question: &'a str,
    multi_select: bool,
    options: Vec<RawOption<'a>>,
}

/// 结构阶段的选项中间表示。
struct RawOption<'a> {
    label: &'a str,
    description: &'a str,
    preview: Option<&'a str>,
}

/// 题目字段白名单。
const QUESTION_FIELDS: [&str; 5] = ["id", "header", "question", "multi_select", "options"];
/// 选项字段白名单。
const OPTION_FIELDS: [&str; 3] = ["label", "description", "preview"];

/// 校验 `ask` 参数并构造题目列表。
///
/// 失败返回 [`ToolFailure`]（`invalid_arguments`），不产生任何部分结果。
pub fn parse_arguments(arguments_json: &str) -> ToolResult<Vec<AskQuestion>> {
    let trimmed = arguments_json.trim();
    if trimmed.is_empty() {
        return Err(invalid("arguments must be a JSON object"));
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|error| invalid(format!("arguments are not valid JSON: {error}")))?;
    let Value::Object(object) = value else {
        return Err(invalid("arguments must be a JSON object"));
    };
    if let Some(unknown) = object.keys().find(|key| key.as_str() != "questions") {
        return Err(invalid(format!(
            "unknown argument '{unknown}'; allowed: questions"
        )));
    }

    // 数量先于逐题结构：超量题目的字段不值得再校验。
    let entries = object
        .get("questions")
        .ok_or_else(|| invalid("missing required argument 'questions'"))?
        .as_array()
        .ok_or_else(|| invalid("'questions' must be an array"))?;
    if entries.is_empty() || entries.len() > MAX_ASK_QUESTIONS {
        return Err(invalid(format!(
            "ask requires 1 to {MAX_ASK_QUESTIONS} questions"
        )));
    }

    let raw = parse_structure(entries)?;
    check_character_limits(&raw)?;
    check_duplicates(&raw)?;
    check_previews(&raw)?;

    Ok(raw
        .into_iter()
        .map(|question| AskQuestion {
            id: question.id.to_string(),
            header: question.header.to_string(),
            question: question.question.to_string(),
            multi_select: question.multi_select,
            options: question
                .options
                .into_iter()
                .map(|option| AskOption {
                    label: option.label.to_string(),
                    description: option.description.to_string(),
                    preview: option.preview.map(str::to_string),
                })
                .collect(),
        })
        .collect())
}

/// 结构阶段：对象形状、字段白名单、必填字段与类型、选项数量。
fn parse_structure(entries: &[Value]) -> ToolResult<Vec<RawQuestion<'_>>> {
    let mut raw = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let number = index + 1;
        let object = entry
            .as_object()
            .ok_or_else(|| invalid(format!("question {number} must be an object")))?;
        if let Some(unknown) = object
            .keys()
            .find(|key| !QUESTION_FIELDS.contains(&key.as_str()))
        {
            return Err(invalid(format!(
                "question {number}: unknown field '{unknown}'; allowed: {}",
                QUESTION_FIELDS.join(", ")
            )));
        }
        let id = required_text(object, "id", number)?;
        let header = required_text(object, "header", number)?;
        let question = required_text(object, "question", number)?;
        let multi_select = match object.get("multi_select") {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(_) => {
                return Err(invalid(format!(
                    "question {number}: \"multi_select\" must be a boolean"
                )));
            }
        };
        let options = match object.get("options") {
            None => Vec::new(),
            Some(Value::Array(items)) => {
                if items.len() > MAX_ASK_OPTIONS {
                    return Err(invalid(format!(
                        "question {number}: at most {MAX_ASK_OPTIONS} options"
                    )));
                }
                parse_options(items, number)?
            }
            Some(_) => {
                return Err(invalid(format!(
                    "question {number}: \"options\" must be an array"
                )));
            }
        };
        raw.push(RawQuestion {
            id,
            header,
            question,
            multi_select,
            options,
        });
    }
    Ok(raw)
}

/// 选项结构：字段白名单、必填字段与类型。
fn parse_options<'a>(items: &'a [Value], number: usize) -> ToolResult<Vec<RawOption<'a>>> {
    let mut options = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let option_number = index + 1;
        let object = item.as_object().ok_or_else(|| {
            invalid(format!(
                "question {number}: option {option_number} must be an object"
            ))
        })?;
        if let Some(unknown) = object
            .keys()
            .find(|key| !OPTION_FIELDS.contains(&key.as_str()))
        {
            return Err(invalid(format!(
                "question {number}: option {option_number}: unknown field '{unknown}'; allowed: {}",
                OPTION_FIELDS.join(", ")
            )));
        }
        let label = required_field(object, "label", number, Some(option_number))?;
        let description = required_field(object, "description", number, Some(option_number))?;
        let preview = match object.get("preview") {
            None => None,
            Some(Value::String(text)) => Some(text.as_str()),
            Some(_) => {
                return Err(invalid(format!(
                    "question {number}: option {option_number}: \"preview\" must be a string"
                )));
            }
        };
        options.push(RawOption {
            label,
            description,
            preview,
        });
    }
    Ok(options)
}

/// 字符上限、`id` 格式与控制字符。
///
/// 先比字符数再逐字符扫描：超长输入在长度检查就被拒，不会做无用的遍历。
fn check_character_limits(raw: &[RawQuestion<'_>]) -> ToolResult<()> {
    for (index, question) in raw.iter().enumerate() {
        let number = index + 1;
        check_field("id", question.id, number, None, MAX_ASK_ID_CHARS)?;
        check_id_format(question.id, number)?;
        check_field(
            "header",
            question.header,
            number,
            None,
            MAX_ASK_HEADER_CHARS,
        )?;
        check_field(
            "question",
            question.question,
            number,
            None,
            MAX_ASK_QUESTION_CHARS,
        )?;
        for (option_index, option) in question.options.iter().enumerate() {
            let option_number = option_index + 1;
            check_field(
                "label",
                option.label,
                number,
                Some(option_number),
                MAX_ASK_LABEL_CHARS,
            )?;
            check_field(
                "description",
                option.description,
                number,
                Some(option_number),
                MAX_ASK_DESC_CHARS,
            )?;
        }
    }
    Ok(())
}

/// `id` 只允许小写字母、数字与下划线（跨题映射键必须稳定）。
fn check_id_format(id: &str, number: usize) -> ToolResult<()> {
    if id.is_empty() {
        return Err(invalid(format!(
            "question {number}: \"id\" must not be empty"
        )));
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(invalid(format!(
            "question {number}: \"id\" must use lowercase letters, digits, and underscores only"
        )));
    }
    Ok(())
}

/// 单个文本字段：字符上限 → 控制字符。
fn check_field(
    name: &str,
    text: &str,
    number: usize,
    option: Option<usize>,
    limit: usize,
) -> ToolResult<()> {
    if text.chars().count() > limit {
        return Err(invalid(format!(
            "{} \"{name}\" is longer than {limit} characters",
            question_prefix(number, option)
        )));
    }
    if has_control(text) {
        return Err(invalid(format!(
            "{} \"{name}\" contains a control character",
            question_prefix(number, option)
        )));
    }
    Ok(())
}

/// 重复：题内 `label`（大小写敏感）与跨题 `id`，先题内后跨题、按题号顺序报告。
fn check_duplicates(raw: &[RawQuestion<'_>]) -> ToolResult<()> {
    let mut ids: HashSet<&str> = HashSet::with_capacity(raw.len());
    for (index, question) in raw.iter().enumerate() {
        let number = index + 1;
        if !ids.insert(question.id) {
            return Err(invalid(format!(
                "question {number}: duplicate question id \"{}\"",
                question.id
            )));
        }
        let mut labels: HashSet<&str> = HashSet::with_capacity(question.options.len());
        for option in &question.options {
            if !labels.insert(option.label) {
                return Err(invalid(format!(
                    "question {number}: duplicate option label \"{}\"",
                    option.label
                )));
            }
        }
    }
    Ok(())
}

/// preview：单条字节/控制字符/行数/单行字符，以及整次合计字节。
fn check_previews(raw: &[RawQuestion<'_>]) -> ToolResult<()> {
    let mut total = 0usize;
    for (index, question) in raw.iter().enumerate() {
        let number = index + 1;
        for (option_index, option) in question.options.iter().enumerate() {
            let option_number = option_index + 1;
            let Some(preview) = option.preview else {
                continue;
            };
            if preview.len() > MAX_ASK_PREVIEW_BYTES {
                return Err(invalid(format!(
                    "question {number}: preview exceeds {MAX_ASK_PREVIEW_BYTES} bytes"
                )));
            }
            if has_control(preview) {
                return Err(invalid(format!(
                    "question {number}: option {option_number} preview contains a control character"
                )));
            }
            if preview_line_count(preview) > MAX_ASK_PREVIEW_LINES {
                return Err(invalid(format!(
                    "question {number}: preview has more than {MAX_ASK_PREVIEW_LINES} lines"
                )));
            }
            for (line_index, line) in preview.split('\n').enumerate() {
                if line.chars().count() > MAX_ASK_PREVIEW_LINE_CHARS {
                    return Err(invalid(format!(
                        "question {number}: preview line {} is longer than {MAX_ASK_PREVIEW_LINE_CHARS} characters",
                        line_index + 1
                    )));
                }
            }
            total = total.saturating_add(preview.len());
            if total > MAX_ASK_PREVIEW_TOTAL_BYTES {
                return Err(invalid(format!(
                    "ask previews exceed {MAX_ASK_PREVIEW_TOTAL_BYTES} bytes in total"
                )));
            }
        }
    }
    Ok(())
}

/// 按题目顺序组装作答结果。
///
/// 缺失的 `id` 视为空数组（事件层保证全答，这里只做防御）；答案按首次出现去重，
/// 手写文本由客户端约定放在数组最后，因此顺序天然保留在末尾。
pub fn answered_result(questions: &[AskQuestion], answers: &AskAnswers) -> ToolExecutionResult {
    let answered = questions
        .iter()
        .map(|question| AnsweredQuestion {
            id: question.id.clone(),
            header: question.header.clone(),
            question: question.question.clone(),
            answers: dedupe(answers.get(&question.id).map(Vec::as_slice).unwrap_or(&[])),
        })
        .collect();
    ToolExecutionResult::Ask {
        questions: answered,
    }
}

/// 去重并保留首次出现顺序。
fn dedupe(answers: &[String]) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(answers.len());
    answers
        .iter()
        .filter(|answer| seen.insert(answer.as_str()))
        .cloned()
        .collect()
}

/// 必填字符串字段（题目层）。
fn required_text<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
    number: usize,
) -> ToolResult<&'a str> {
    required_field(object, key, number, None)
}

/// 必填字符串字段（题目层或选项层）。
fn required_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
    number: usize,
    option: Option<usize>,
) -> ToolResult<&'a str> {
    match object.get(key) {
        Some(Value::String(text)) => Ok(text),
        Some(_) => Err(invalid(format!(
            "{} \"{key}\" must be a string",
            question_prefix(number, option)
        ))),
        None => Err(invalid(format!(
            "{} missing required field \"{key}\"",
            question_prefix(number, option)
        ))),
    }
}

/// `question {n}` 或 `question {n}: option {m}` 前缀。
fn question_prefix(number: usize, option: Option<usize>) -> String {
    match option {
        Some(option) => format!("question {number}: option {option}:"),
        None => format!("question {number}:"),
    }
}

/// 是否含不可见控制字符（`\n` 与 `\t` 允许）。
fn has_control(text: &str) -> bool {
    text.chars()
        .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
}

/// preview 行数（空串为 0，`\n` 分隔）。
fn preview_line_count(preview: &str) -> usize {
    if preview.is_empty() {
        0
    } else {
        preview.split('\n').count()
    }
}

/// 参数非法（消息只描述字段与期望，不回显参数正文）。
fn invalid(message: impl Into<String>) -> ToolFailure {
    ToolFailure::invalid(message)
}

#[cfg(test)]
mod tests;
