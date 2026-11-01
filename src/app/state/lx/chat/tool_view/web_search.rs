//! `web_search` 工具卡片详情：查询词、引擎与结果行窗口（纯数据，渲染层只读）。
//!
//! 运行阶段只有调用参数（[`WebSearchDetailView::from_arguments`]），结算与历史重载
//! 阶段才多出引擎、耗时与结果正文（[`WebSearchDetailView::from_parts`]）；两条路径
//! 共用同一份视图，渲染层不解析 JSON。

use serde_json::Value;

use crate::chat::tools::web_search::{DEFAULT_NUM_RESULTS, SearchDepth};

/// 结果区保留的最大行数（更长的部分只计数，展开也补不回来）。
const RESULT_LINES: usize = 40;

/// `web_search` 工具卡片详情视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchDetailView {
    /// 本次检索的查询词列表（去空白、去重后 1..=4 条）。
    pub queries: Vec<String>,
    /// 搜索深度模式（`"auto"` / `"fast"` / `"deep"`）。
    pub search_type: String,
    /// 每条查询请求的最大结果数。
    pub num_results: usize,
    /// 命中的搜索引擎（`"exa"` / `"tavily"`）；Pending/Running 或失败时为 None。
    pub provider: Option<String>,
    /// 结果正文行（最多保留前 [`RESULT_LINES`] 行供展开阅读，超出计数记入 `result_more`）。
    pub result: Vec<String>,
    /// 超出保留窗口的结果行数。
    pub result_more: usize,
    /// 调用耗时（毫秒）；Pending/Running 时为 None。
    pub duration_ms: Option<u64>,
    /// 结果是否被截断。
    pub truncated: bool,
}

impl WebSearchDetailView {
    /// 只由调用参数构造：工具处于 Pending/Running，或失败/取消没有结果体时使用。
    ///
    /// 参数合法但查询词缺失（模型偶发传半截对象）仍返回 `Some`：卡片先立起来，
    /// 标题按运行态显示，查询行等参数补齐后再出现。
    pub fn from_arguments(arguments_json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(arguments_json).ok()?;
        let object = value.as_object()?;
        let mut queries = Vec::new();
        if let Some(query) = object.get("query").and_then(Value::as_str) {
            push_query(&mut queries, query);
        }
        if let Some(entries) = object.get("queries").and_then(Value::as_array) {
            for entry in entries.iter().filter_map(Value::as_str) {
                push_query(&mut queries, entry);
            }
        }
        Some(Self {
            queries,
            search_type: object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_else(|| SearchDepth::default().as_str())
                .to_string(),
            num_results: num_results_of(object).unwrap_or(DEFAULT_NUM_RESULTS),
            provider: None,
            result: Vec::new(),
            result_more: 0,
            duration_ms: None,
            truncated: false,
        })
    }

    /// 历史重载：结果体（落库 JSON）+ 调用参数；非 `web_search` 结果返回 None。
    pub fn from_parts(result_json: Option<&str>, arguments_json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(result_json?).ok()?;
        if value.get("tool").and_then(Value::as_str)? != "web_search" {
            return None;
        }
        let mut view = Self::from_arguments(arguments_json).unwrap_or_else(empty);
        if let Some(queries) = value.get("queries").and_then(Value::as_array) {
            let mut parsed = Vec::new();
            for entry in queries.iter().filter_map(Value::as_str) {
                push_query(&mut parsed, entry);
            }
            if !parsed.is_empty() {
                view.queries = parsed;
            }
        }
        if let Some(num_results) = value
            .get("num_results")
            .and_then(Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
        {
            view.num_results = num_results;
        }
        if let Some(search_type) = value.get("search_type").and_then(Value::as_str) {
            view.search_type = search_type.to_string();
        }
        view.provider = value
            .get("provider")
            .and_then(Value::as_str)
            .filter(|provider| !provider.is_empty())
            .map(str::to_string);
        let text = value.get("text").and_then(Value::as_str).unwrap_or("");
        view.result = text
            .lines()
            .take(RESULT_LINES)
            .map(str::to_string)
            .collect();
        view.result_more = text.lines().count().saturating_sub(RESULT_LINES);
        view.duration_ms = value.get("duration_ms").and_then(Value::as_u64);
        view.truncated = value
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Some(view)
    }

    /// 结算与重载共用入口：结果体优先，没有结果体（失败/取消）时回退到调用参数。
    pub fn from_tool(result_json: Option<&str>, arguments_json: &str) -> Option<Self> {
        Self::from_parts(result_json, arguments_json)
            .or_else(|| Self::from_arguments(arguments_json))
    }

    /// 搜索深度是否为默认档（`auto`）。
    pub fn search_type_is_default(&self) -> bool {
        self.search_type == SearchDepth::default().as_str()
    }

    /// 结果条数是否为默认值。
    pub fn num_results_is_default(&self) -> bool {
        self.num_results == DEFAULT_NUM_RESULTS
    }
}

/// 空视图（结果体形状合法但调用参数缺失时的底稿）。
fn empty() -> WebSearchDetailView {
    WebSearchDetailView {
        queries: Vec::new(),
        search_type: SearchDepth::default().as_str().to_string(),
        num_results: DEFAULT_NUM_RESULTS,
        provider: None,
        result: Vec::new(),
        result_more: 0,
        duration_ms: None,
        truncated: false,
    }
}

/// 归一化后按出现顺序去重追加（空串丢弃）。
fn push_query(queries: &mut Vec<String>, raw: &str) {
    let query = raw.trim();
    if query.is_empty() || queries.iter().any(|seen| seen == query) {
        return;
    }
    queries.push(query.to_string());
}

/// `num_results` / `numResults` → 结果条数（缺失或非正整数为 None）。
fn num_results_of(object: &serde_json::Map<String, Value>) -> Option<usize> {
    object
        .get("num_results")
        .or_else(|| object.get("numResults"))
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .filter(|count| *count > 0)
}

#[cfg(test)]
mod tests;
