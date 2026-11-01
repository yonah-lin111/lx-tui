//! 内置联网搜索工具 `web_search`：参数模型、边界常量与结果格式化。
//!
//! 本模块只有纯数据（不发网络请求、不碰文件），执行链在 [`client`]：Exa MCP
//! 优先、Tavily REST 兜底，逐条查询并发。参数校验与格式化独立成纯函数，
//! 不依赖传输层即可测试。

mod client;

#[cfg(test)]
mod tests;

pub use client::{WebSearchEndpoints, execute, execute_with_endpoints};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::{ToolFailure, ToolResult};

/// 稳定工具名（schema、错误消息与提示共用）。
pub const WEB_SEARCH_TOOL: &str = "web_search";

/// 单次调用的查询条数上限（去重后，并发执行）。
pub const MAX_QUERIES: usize = 4;

/// 去重前的原始查询条数防御上限（防止模型塞入超大数组）。
pub const MAX_RAW_QUERIES: usize = 16;

/// 单条查询的字符上限。
pub const MAX_QUERY_CHARS: usize = 500;

/// 单条查询输出的字符预算（超出截断并显式标记）。
pub const MAX_QUERY_OUTPUT_CHARS: usize = 12_000;

/// 无结果时的统一文案（与 lx-agent 一致）。
pub const NO_RESULTS: &str = "No search results found. Please try a different query.";

/// 每条查询的结果数下限。
pub const MIN_NUM_RESULTS: usize = 1;

/// 每条查询的结果数上限。
pub const MAX_NUM_RESULTS: usize = 10;

/// 每条查询的结果数默认值。
pub const DEFAULT_NUM_RESULTS: usize = 8;

/// 单次引擎请求的超时（毫秒）。
pub const WEB_SEARCH_TIMEOUT_MS: u64 = 25_000;

/// `web_search` 参数白名单（`numResults` 兼容模型偶发的 camelCase）。
const ALLOWED_FIELDS: [&str; 5] = ["query", "queries", "num_results", "numResults", "type"];

/// 搜索深度档位（对齐 Exa 的 `type` 参数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchDepth {
    #[default]
    Auto,
    Fast,
    Deep,
}

impl SearchDepth {
    /// 稳定字符串值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Fast => "fast",
            Self::Deep => "deep",
        }
    }

    /// 解析稳定值；未知值返回 None。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "fast" => Some(Self::Fast),
            "deep" => Some(Self::Deep),
            _ => None,
        }
    }
}

/// 归一化后的 `web_search` 请求参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchArgs {
    /// 去空白、去重后的查询列表（1..=MAX_QUERIES）。
    pub queries: Vec<String>,
    /// 每条查询的最大结果数（1..=10，默认 8）。
    pub num_results: usize,
    /// 搜索深度模式（默认 Auto）。
    pub search_type: SearchDepth,
}

/// 实际返回结果的搜索引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchProvider {
    Exa,
    Tavily,
}

impl WebSearchProvider {
    /// 稳定字符串值（落库 JSON 与 UI 共用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exa => "exa",
            Self::Tavily => "tavily",
        }
    }

    /// 解析稳定值；未知值返回 None。
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "exa" => Some(Self::Exa),
            "tavily" => Some(Self::Tavily),
            _ => None,
        }
    }
}

/// 单条查询的执行结局（部分成功时失败项也保留）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySearchOutcome {
    pub query: String,
    /// 成功时的提供商；失败时为 None。
    pub provider: Option<WebSearchProvider>,
    /// 成功时的格式化 Markdown 检索文本。
    pub output: String,
    /// 失败时的单行错误说明。
    pub error: Option<String>,
    /// 该查询是否因引擎超时而失败（全量失败时用于选择 `Timeout` 错误码）。
    #[serde(default)]
    pub timed_out: bool,
}

/// 整次 `web_search` 调用的结构化结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchToolResult {
    pub queries: Vec<String>,
    pub num_results: usize,
    pub search_type: SearchDepth,
    /// 首个成功查询命中的提供商。
    pub provider: WebSearchProvider,
    /// 按查询顺序对应的结局。
    pub outcomes: Vec<QuerySearchOutcome>,
    /// 模型可见的组合输出（亦用于 UI 结果区）。
    pub text: String,
    pub truncated: bool,
    pub duration_ms: u64,
}

impl WebSearchToolResult {
    /// 展示用提供商标签：同批次混合命中 Exa 与 Tavily 时返回 `"exa+tavily"`。
    pub fn provider_label(&self) -> &'static str {
        let mut exa = false;
        let mut tavily = false;
        for provider in self.outcomes.iter().filter_map(|outcome| outcome.provider) {
            match provider {
                WebSearchProvider::Exa => exa = true,
                WebSearchProvider::Tavily => tavily = true,
            }
        }
        if exa && tavily {
            "exa+tavily"
        } else {
            self.provider.as_str()
        }
    }
}

/// 解析并校验 `web_search` 参数（严格白名单，不信任模型遵守 schema）。
pub fn parse_arguments(arguments_json: &str) -> ToolResult<WebSearchArgs> {
    let trimmed = arguments_json.trim();
    if trimmed.is_empty() {
        return Err(ToolFailure::invalid("arguments must be a JSON object"));
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|error| ToolFailure::invalid(format!("arguments are not valid JSON: {error}")))?;
    let Value::Object(object) = value else {
        return Err(ToolFailure::invalid("arguments must be a JSON object"));
    };
    if let Some(unknown) = object
        .keys()
        .find(|key| !ALLOWED_FIELDS.contains(&key.as_str()))
    {
        return Err(ToolFailure::invalid(format!(
            "unknown argument '{unknown}'; allowed: {}",
            ALLOWED_FIELDS.join(", ")
        )));
    }

    let mut raw_queries: Vec<&str> = Vec::new();
    if let Some(value) = object.get("query") {
        let Value::String(text) = value else {
            return Err(ToolFailure::invalid("'query' must be a string"));
        };
        raw_queries.push(text);
    }
    if let Some(value) = object.get("queries") {
        let Value::Array(entries) = value else {
            return Err(ToolFailure::invalid(
                "'queries' must be an array of strings",
            ));
        };
        for entry in entries {
            let Value::String(text) = entry else {
                return Err(ToolFailure::invalid(
                    "'queries' must be an array of strings",
                ));
            };
            raw_queries.push(text);
        }
    }

    if raw_queries.len() > MAX_RAW_QUERIES {
        return Err(ToolFailure::invalid(format!(
            "web_search accepts at most {MAX_RAW_QUERIES} raw queries per call"
        )));
    }

    let mut queries: Vec<String> = Vec::with_capacity(raw_queries.len());
    for raw in &raw_queries {
        let query = validate_query(raw)?;
        if !queries.contains(&query) {
            queries.push(query);
        }
    }
    if queries.is_empty() {
        return Err(ToolFailure::invalid(
            "web_search requires `query` or non-empty `queries`",
        ));
    }
    // 上限按去重后的条数计算：重复项不占预算。
    if queries.len() > MAX_QUERIES {
        return Err(ToolFailure::invalid(format!(
            "web_search accepts at most {MAX_QUERIES} queries per call"
        )));
    }

    let num_results = match (
        read_num_results(object.get("num_results"))?,
        read_num_results(object.get("numResults"))?,
    ) {
        (Some(primary), Some(alias)) if primary != alias => {
            return Err(ToolFailure::invalid(
                "'num_results' and 'numResults' must match when both are provided",
            ));
        }
        (Some(value), _) | (_, Some(value)) => value,
        (None, None) => DEFAULT_NUM_RESULTS,
    };

    let search_type = match object.get("type") {
        None | Some(Value::Null) => SearchDepth::Auto,
        Some(Value::String(text)) => SearchDepth::parse(text)
            .ok_or_else(|| ToolFailure::invalid("type must be one of: auto, fast, deep"))?,
        Some(_) => return Err(ToolFailure::invalid("'type' must be a string")),
    };

    Ok(WebSearchArgs {
        queries,
        num_results,
        search_type,
    })
}

/// 单条查询的规范化：去空白、限长、禁控制字符（`\t` 除外）。
fn validate_query(raw: &str) -> ToolResult<String> {
    let query = raw.trim();
    if query.is_empty() {
        return Err(ToolFailure::invalid("search query must not be empty"));
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(ToolFailure::invalid(format!(
            "search query exceeds {MAX_QUERY_CHARS} characters"
        )));
    }
    if query.chars().any(|ch| ch.is_control() && ch != '\t') {
        return Err(ToolFailure::invalid(
            "search query contains control characters",
        ));
    }
    Ok(query.to_string())
}

/// 读取 `num_results` 类字段：只接受 1..=10 的整数。
fn read_num_results(value: Option<&Value>) -> ToolResult<Option<usize>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::Number(number) = value else {
        return Err(ToolFailure::invalid("num_results must be an integer"));
    };
    let out_of_range = || {
        ToolFailure::invalid(format!(
            "num_results must be between {MIN_NUM_RESULTS} and {MAX_NUM_RESULTS}"
        ))
    };
    let raw = number.as_u64().ok_or_else(out_of_range)?;
    let value = usize::try_from(raw).map_err(|_| out_of_range())?;
    if !(MIN_NUM_RESULTS..=MAX_NUM_RESULTS).contains(&value) {
        return Err(out_of_range());
    }
    Ok(Some(value))
}

/// 组合多条查询的模型可见文本：单条直接返回正文，多条按查询编号分段。
///
/// 每条查询的正文按 [`MAX_QUERY_OUTPUT_CHARS`] 单独裁剪，返回
/// `(text, truncated)`；任一条被裁剪即 `truncated = true`。
pub fn format_outcomes(outcomes: &[QuerySearchOutcome]) -> (String, bool) {
    let mut truncated = false;
    let text = match outcomes {
        [single] => {
            let (body, cut) = bounded_body(single);
            truncated |= cut;
            body
        }
        _ => outcomes
            .iter()
            .enumerate()
            .map(|(index, outcome)| {
                let (body, cut) = bounded_body(outcome);
                truncated |= cut;
                format!("### {}. \"{}\"\n{}", index + 1, outcome.query, body)
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
    };
    (text, truncated)
}

/// 单条结局的正文：失败项显式写出原因，不静默丢弃。
fn outcome_body(outcome: &QuerySearchOutcome) -> String {
    if let Some(error) = &outcome.error {
        return format!("[Search failed: {error}]");
    }
    if outcome.output.is_empty() {
        return NO_RESULTS.to_string();
    }
    outcome.output.clone()
}

/// 按单条查询预算裁剪正文（按字符计，不切断 UTF-8），超出追加截断标记。
fn bounded_body(outcome: &QuerySearchOutcome) -> (String, bool) {
    let body = outcome_body(outcome);
    if body.chars().count() <= MAX_QUERY_OUTPUT_CHARS {
        return (body, false);
    }
    let mut capped: String = body.chars().take(MAX_QUERY_OUTPUT_CHARS).collect();
    capped.push_str("\n[...truncated]");
    (capped, true)
}
