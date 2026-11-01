//! `web_search` 双引擎执行链：Exa MCP 优先、Tavily REST 兜底。
//!
//! 单条查询按 `Exa (带 Key) → Exa (匿名) → Tavily` 顺序回退；匿名 Exa 被
//! `401/403` 拒绝后进程内熔断，只有匿名调用真正成功才解除。多条查询并发执行，
//! 至少一条成功即算工具成功；收到取消信号立即返回 `Cancelled`。
//!
//! 安全约束：端点 URL 可能带 `exaApiKey` 查询参数。错误消息只报静态端点标签、
//! HTTP 状态码与响应体里脱敏后的简短原因，绝不回显完整 URL 或 Key。

use std::future::Future;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::{
    NO_RESULTS, QuerySearchOutcome, SearchDepth, WEB_SEARCH_TIMEOUT_MS, WebSearchProvider,
    WebSearchToolResult, format_outcomes, parse_arguments,
};
use crate::chat::tools::types::{ToolErrorCode, ToolExecutionResult, ToolFailure};

/// Exa MCP 默认端点。
pub const DEFAULT_EXA_URL: &str = "https://mcp.exa.ai/mcp";

/// Tavily REST 默认端点。
pub const DEFAULT_TAVILY_URL: &str = "https://api.tavily.com/search";

/// 单次响应体硬上限：异常端点返回无限流时立即失败，不撑爆内存。
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// 错误摘要的最大字符数（非 2xx 响应体只保留一小段可读原因）。
const MAX_ERROR_DETAIL_CHARS: usize = 160;

/// 匿名 Exa 被 `401/403` 拒绝后的进程内熔断标记。
static EXA_ANON_BLOCKED: AtomicBool = AtomicBool::new(false);

/// 进程级复用的 HTTP 客户端（连接池跨调用共享）。
fn http_client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(concat!("lx-tui/", env!("CARGO_PKG_VERSION")))
                .build()
                .ok()
        })
        .as_ref()
}

/// 可注入的引擎端点（默认指向公网；测试注入本地回环 server）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchEndpoints {
    pub exa_url: String,
    pub tavily_url: String,
}

impl Default for WebSearchEndpoints {
    fn default() -> Self {
        Self {
            exa_url: DEFAULT_EXA_URL.to_string(),
            tavily_url: DEFAULT_TAVILY_URL.to_string(),
        }
    }
}

/// 单引擎失败：`status` 用于判断是否值得回退，`timed_out` 用于汇总错误码。
#[derive(Debug)]
struct SearchError {
    message: String,
    status: Option<u16>,
    timed_out: bool,
}

impl SearchError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
            timed_out: false,
        }
    }

    /// 非 2xx：状态码 + 响应体里已脱敏的简短原因。
    fn http(provider: &str, status: reqwest::StatusCode, body: &str, secrets: &[&str]) -> Self {
        let mut message = format!("{provider} returned HTTP {status}");
        if let Some(detail) = error_detail(body, secrets) {
            message.push_str(": ");
            message.push_str(&detail);
        }
        Self {
            message,
            status: Some(status.as_u16()),
            timed_out: false,
        }
    }

    fn timed_out(provider: &str) -> Self {
        Self {
            message: format!("{provider} did not respond within {WEB_SEARCH_TIMEOUT_MS} ms"),
            status: None,
            timed_out: true,
        }
    }

    /// 鉴权/额度类失败：可以尝试匿名 Exa，再回退 Tavily。
    fn is_credential_failure(&self) -> bool {
        matches!(self.status, Some(401 | 402 | 403 | 429))
    }
}

/// 从非 2xx 响应体提取 `error.message` / `error` / `detail.error` / `message`，
/// 剥掉控制字符、脱敏 Key 并截断到 [`MAX_ERROR_DETAIL_CHARS`]。
fn error_detail(body: &str, secrets: &[&str]) -> Option<String> {
    let value: Value = serde_json::from_str(body.trim()).ok()?;
    let candidate = value
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .or_else(|| {
            value
                .get("detail")
                .and_then(|detail| detail.get("error").and_then(Value::as_str))
        })
        .or_else(|| value.get("message").and_then(Value::as_str))?
        .trim();
    if candidate.is_empty() {
        return None;
    }
    Some(sanitize_detail(candidate, secrets))
}

/// 单行化错误摘要：去除控制字符、脱敏 Key 查询串与 Key 值本身。
fn sanitize_detail(detail: &str, secrets: &[&str]) -> String {
    let mut text: String = detail.chars().filter(|ch| !ch.is_control()).collect();
    for secret in secrets.iter().copied().filter(|secret| !secret.is_empty()) {
        text = text.replace(secret, "<redacted>");
    }
    let text = text.replace("exaApiKey=", "<redacted>");
    let text = text.trim();
    if text.chars().count() <= MAX_ERROR_DETAIL_CHARS {
        return text.to_string();
    }
    let mut capped: String = text.chars().take(MAX_ERROR_DETAIL_CHARS).collect();
    capped.push('…');
    capped
}

/// 执行一次 `web_search`（生产入口：使用公网端点）。
pub async fn execute(
    arguments_json: &str,
    exa_api_key: &str,
    tavily_api_key: &str,
    cancel: watch::Receiver<bool>,
) -> ToolExecutionResult {
    execute_with_endpoints(
        arguments_json,
        exa_api_key,
        tavily_api_key,
        &WebSearchEndpoints::default(),
        cancel,
    )
    .await
}

/// 带端点注入的版本（测试用本地回环 server 覆盖全部分支）。
pub async fn execute_with_endpoints(
    arguments_json: &str,
    exa_api_key: &str,
    tavily_api_key: &str,
    endpoints: &WebSearchEndpoints,
    mut cancel: watch::Receiver<bool>,
) -> ToolExecutionResult {
    // 已取消的调用不解析参数、不建任务、不发请求。
    if *cancel.borrow() {
        return cancelled_result();
    }
    let args = match parse_arguments(arguments_json) {
        Ok(args) => args,
        Err(failure) => return failure.into_result(Vec::new()),
    };
    let Some(client) = http_client() else {
        return ToolFailure::new(ToolErrorCode::Io, "failed to initialize the HTTP client")
            .into_result(Vec::new());
    };

    let started = Instant::now();
    let mut tasks = JoinSet::new();
    for (index, query) in args.queries.iter().enumerate() {
        let request = QueryRequest {
            client,
            endpoints: endpoints.clone(),
            exa_api_key: exa_api_key.trim().to_string(),
            tavily_api_key: tavily_api_key.trim().to_string(),
            query: query.clone(),
            num_results: args.num_results,
            search_type: args.search_type,
        };
        tasks.spawn(async move { (index, request.run().await) });
    }

    let mut slots: Vec<Option<QuerySearchOutcome>> =
        (0..args.queries.len()).map(|_| None).collect();
    loop {
        tokio::select! {
            biased;
            () = wait_cancelled(&mut cancel) => {
                tasks.abort_all();
                return cancelled_result();
            }
            joined = tasks.join_next() => match joined {
                Some(Ok((index, outcome))) => slots[index] = Some(outcome),
                Some(Err(error)) => {
                    tracing::warn!(
                        target: "lx::web_search",
                        %error,
                        "web search task failed"
                    );
                }
                None => break,
            },
        }
    }

    let outcomes: Vec<QuerySearchOutcome> = slots
        .into_iter()
        .zip(&args.queries)
        .map(|(slot, query)| {
            slot.unwrap_or_else(|| QuerySearchOutcome {
                query: query.clone(),
                provider: None,
                output: String::new(),
                error: Some("search task failed".to_string()),
                timed_out: false,
            })
        })
        .collect();

    if outcomes.iter().all(|outcome| outcome.error.is_some()) {
        return all_failed_failure(&outcomes).into_result(Vec::new());
    }

    let provider = outcomes
        .iter()
        .find_map(|outcome| outcome.provider)
        .unwrap_or(WebSearchProvider::Exa);
    let (text, truncated) = format_outcomes(&outcomes);
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    ToolExecutionResult::WebSearch(Box::new(WebSearchToolResult {
        queries: args.queries,
        num_results: args.num_results,
        search_type: args.search_type,
        provider,
        outcomes,
        text,
        truncated,
        duration_ms,
    }))
}

/// 取消结果（入口短路与运行中取消共用同一文案与错误码）。
fn cancelled_result() -> ToolExecutionResult {
    ToolFailure::new(ToolErrorCode::Cancelled, "cancelled").into_result(Vec::new())
}

/// 全量失败：保留去重后的真实错误摘要；全部超时则返回 `Timeout`。
pub(super) fn all_failed_failure(outcomes: &[QuerySearchOutcome]) -> ToolFailure {
    let mut details: Vec<&str> = Vec::new();
    for outcome in outcomes {
        let Some(error) = outcome.error.as_deref().map(str::trim) else {
            continue;
        };
        if !error.is_empty() && !details.contains(&error) {
            details.push(error);
        }
    }
    let all_timed_out = outcomes.iter().all(|outcome| outcome.timed_out);
    let code = if all_timed_out {
        ToolErrorCode::Timeout
    } else {
        ToolErrorCode::Io
    };
    let message = format!(
        "Web search failed across all providers (Exa, Tavily): {}. Please try again later or \
         check your Exa/Tavily API keys in Settings -> General.",
        details.join(" | ")
    );
    ToolFailure::new(code, message)
}

/// 单条查询的一次执行（拥有所有权，便于放进并发任务）。
struct QueryRequest {
    client: &'static reqwest::Client,
    endpoints: WebSearchEndpoints,
    exa_api_key: String,
    tavily_api_key: String,
    query: String,
    num_results: usize,
    search_type: SearchDepth,
}

impl QueryRequest {
    async fn run(self) -> QuerySearchOutcome {
        match self.search().await {
            Ok((provider, output)) => QuerySearchOutcome {
                query: self.query.clone(),
                provider: Some(provider),
                output,
                error: None,
                timed_out: false,
            },
            Err(error) => QuerySearchOutcome {
                query: self.query.clone(),
                provider: None,
                output: String::new(),
                error: Some(error.message),
                timed_out: error.timed_out,
            },
        }
    }

    /// 逐级回退：带 Key 的 Exa → 匿名 Exa → Tavily。
    ///
    /// 匿名熔断只在匿名 Exa 真正成功时解除；带 Key 的请求无论成败都不清零，
    /// 否则失效 Key 会让每次调用都重复白跑一次注定 401/403 的匿名请求。
    async fn search(&self) -> Result<(WebSearchProvider, String), SearchError> {
        let mut exa_error: Option<SearchError> = None;
        if self.exa_api_key.is_empty() {
            if !EXA_ANON_BLOCKED.load(Ordering::Acquire) {
                match self.call_exa(None).await {
                    Ok(text) => return Ok((WebSearchProvider::Exa, text)),
                    Err(error) => exa_error = Some(error),
                }
            }
        } else {
            match self.call_exa(Some(&self.exa_api_key)).await {
                Ok(text) => return Ok((WebSearchProvider::Exa, text)),
                Err(error) => {
                    let retry_anonymous =
                        error.is_credential_failure() && !EXA_ANON_BLOCKED.load(Ordering::Acquire);
                    exa_error = Some(error);
                    if retry_anonymous {
                        match self.call_exa(None).await {
                            Ok(text) => return Ok((WebSearchProvider::Exa, text)),
                            Err(error) => exa_error = Some(error),
                        }
                    }
                }
            }
        }

        match self.call_tavily().await {
            Ok(text) => Ok((WebSearchProvider::Tavily, text)),
            Err(tavily_error) => {
                let timed_out = exa_error.as_ref().is_some_and(|error| error.timed_out)
                    && tavily_error.timed_out;
                let exa_message = exa_error.map_or_else(
                    || {
                        "Exa anonymous access is disabled after an authentication failure"
                            .to_string()
                    },
                    |error| error.message,
                );
                Err(SearchError {
                    message: format!("{exa_message}; {}", tavily_error.message),
                    status: None,
                    timed_out,
                })
            }
        }
    }

    /// Exa MCP `tools/call`（`web_search_exa`）；`api_key` 为 None 时匿名直连。
    async fn call_exa(&self, api_key: Option<&str>) -> Result<String, SearchError> {
        let mut url = reqwest::Url::parse(&self.endpoints.exa_url)
            .map_err(|_| SearchError::new("Exa endpoint URL is invalid"))?;
        if let Some(key) = api_key.filter(|key| !key.is_empty()) {
            // 百分号编码交给 `Url`：Key 里的 `+`/`/`/`=` 不会破坏查询串。
            url.query_pairs_mut().append_pair("exaApiKey", key);
        }
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "web_search_exa",
                "arguments": {
                    "query": self.query,
                    "numResults": self.num_results,
                    "type": self.search_type.as_str(),
                    "livecrawl": "fallback",
                }
            }
        });
        let request = self
            .client
            .post(url)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(&body);
        let (status, text) = with_timeout("Exa", post_json("Exa", request)).await?;
        if !status.is_success() {
            // 匿名被拒：熔断，避免后续每次调用都白跑一次 401。
            if api_key.is_none() && matches!(status.as_u16(), 401 | 403) {
                EXA_ANON_BLOCKED.store(true, Ordering::Release);
            }
            return Err(self.http_error("Exa", status, &text));
        }
        let output = parse_exa_body(&text).map_err(SearchError::new)?;
        // 匿名通道确实可用时才解除熔断。
        if api_key.is_none() {
            EXA_ANON_BLOCKED.store(false, Ordering::Release);
        }
        Ok(output)
    }

    /// 非 2xx 状态错误：始终用两个 Key 做脱敏集合。
    fn http_error(&self, provider: &str, status: reqwest::StatusCode, body: &str) -> SearchError {
        let secrets = [self.exa_api_key.as_str(), self.tavily_api_key.as_str()];
        SearchError::http(provider, status, body, &secrets)
    }

    /// Tavily REST 搜索；`Deep` 映射为 `advanced`，其余为 `basic`。
    ///
    /// Tavily 没有匿名通道：空 Key 直接短路，不发明知必败的无认证请求。
    async fn call_tavily(&self) -> Result<String, SearchError> {
        if self.tavily_api_key.is_empty() {
            return Err(SearchError::new("Tavily API key is not configured"));
        }
        let body = json!({
            "query": self.query,
            "max_results": self.num_results,
            "search_depth": match self.search_type {
                SearchDepth::Deep => "advanced",
                SearchDepth::Auto | SearchDepth::Fast => "basic",
            },
            "include_answer": false,
        });
        let request = self
            .client
            .post(&self.endpoints.tavily_url)
            .header(reqwest::header::ACCEPT, "application/json")
            .bearer_auth(&self.tavily_api_key)
            .json(&body);
        let (status, text) = with_timeout("Tavily", post_json("Tavily", request)).await?;
        if !status.is_success() {
            return Err(self.http_error("Tavily", status, &text));
        }
        parse_tavily_body(&text).map_err(SearchError::new)
    }
}

/// 单引擎请求的超时包装（覆盖发送与读取响应体）。
async fn with_timeout<F>(
    provider: &str,
    future: F,
) -> Result<(reqwest::StatusCode, String), SearchError>
where
    F: Future<Output = Result<(reqwest::StatusCode, String), SearchError>>,
{
    match tokio::time::timeout(Duration::from_millis(WEB_SEARCH_TIMEOUT_MS), future).await {
        Ok(result) => result,
        Err(_) => Err(SearchError::timed_out(provider)),
    }
}

/// 发送请求并读回完整响应体（有硬上限）。
async fn post_json(
    provider: &'static str,
    request: reqwest::RequestBuilder,
) -> Result<(reqwest::StatusCode, String), SearchError> {
    let response = request
        .send()
        .await
        .map_err(|error| SearchError::new(transport_message(provider, &error)))?;
    let status = response.status();
    let body = read_body(provider, response).await?;
    Ok((status, body))
}

/// 传输层错误只报原因分类：`reqwest` 的 `Display` 会带上完整 URL（可能含 Key）。
fn transport_message(provider: &str, error: &reqwest::Error) -> String {
    let reason = if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_builder() {
        "request could not be built"
    } else {
        "request failed"
    };
    format!("{provider}: {reason}")
}

/// 逐块读取响应体；超过上限直接失败，不截断（截断的 JSON 无法解析）。
async fn read_body(provider: &str, mut response: reqwest::Response) -> Result<String, SearchError> {
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| SearchError::new(transport_message(provider, &error)))?;
        let Some(chunk) = chunk else {
            break;
        };
        if buffer.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(SearchError::new(format!(
                "{provider}: response body exceeds 2 MiB"
            )));
        }
        buffer.extend_from_slice(&chunk);
    }
    String::from_utf8(buffer)
        .map_err(|_| SearchError::new(format!("{provider}: response is not valid UTF-8")))
}

/// 解析 Exa 响应：直接 JSON 与 `data: ` SSE 行都支持。
pub(super) fn parse_exa_body(body: &str) -> Result<String, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(NO_RESULTS.to_string());
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return exa_text(&value).map(|text| text.unwrap_or_else(|| NO_RESULTS.to_string()));
    }
    // MCP 流式传输：逐行取首个 `data:` 事件里的文本。
    let mut parsed_any = false;
    for line in trimmed.lines() {
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        parsed_any = true;
        if let Some(text) = exa_text(&value)? {
            return Ok(text);
        }
    }
    if parsed_any {
        Ok(NO_RESULTS.to_string())
    } else {
        Err("Exa returned an unreadable response".to_string())
    }
}

/// 从一个 JSON-RPC 响应里取首个非空文本；JSON-RPC `error` 直接上抛。
fn exa_text(value: &Value) -> Result<Option<String>, String> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Exa returned an error");
        return Err(message.to_string());
    }
    let Some(content) = value
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
    else {
        return Ok(None);
    };
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let text = item.get("text").and_then(Value::as_str).unwrap_or("");
        if !text.trim().is_empty() {
            return Ok(Some(text.trim().to_string()));
        }
    }
    Ok(None)
}

/// 解析 Tavily 响应：`answer` 在前的编号结果列表。
pub(super) fn parse_tavily_body(body: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(body.trim())
        .map_err(|_| "Tavily returned an unreadable response".to_string())?;
    if let Some(detail) = value.get("detail") {
        let message = detail
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| detail.as_str())
            .unwrap_or("Tavily returned an error");
        return Err(message.to_string());
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(answer) = value
        .get("answer")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        parts.push(answer.to_string());
    }
    if let Some(results) = value.get("results").and_then(Value::as_array) {
        for (index, item) in results.iter().enumerate() {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let content = item
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let mut block = format!("{}. {title}\n{url}", index + 1);
            if !content.is_empty() {
                block.push('\n');
                block.push_str(content);
            }
            parts.push(block);
        }
    }
    if parts.is_empty() {
        Ok(NO_RESULTS.to_string())
    } else {
        Ok(parts.join("\n\n"))
    }
}

/// 等待取消信号；发送端全部被丢弃不算取消（与 `chat::client` 同语义）。
async fn wait_cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        match cancel.changed().await {
            Ok(()) => {}
            Err(_) => std::future::pending::<()>().await,
        }
    }
}

#[cfg(test)]
pub(super) fn reset_circuit_breaker() {
    EXA_ANON_BLOCKED.store(false, Ordering::Release);
}

#[cfg(test)]
pub(super) fn anonymous_blocked() -> bool {
    EXA_ANON_BLOCKED.load(Ordering::Acquire)
}
