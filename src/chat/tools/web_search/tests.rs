//! `web_search` 测试：参数校验、Exa JSON/SSE 解析、Tavily 格式化、双引擎回退、
//! 匿名熔断、批量部分成功与取消中断。
//!
//! 所有网络用例都跑在本地回环 mock server 上：零外网依赖，毫秒级完成。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

use super::client::{self, NO_RESULTS, WebSearchEndpoints};
use super::*;
use crate::chat::tools::types::{ToolErrorCode, ToolExecutionResult};

/// 匿名熔断是进程级状态：所有网络用例串行执行，避免互相改写。
static NET_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// mock server 收到的原始请求（按到达顺序）。
type Requests = Arc<Mutex<Vec<String>>>;

// ---------------------------------------------------------------------------
// 参数校验
// ---------------------------------------------------------------------------

#[test]
fn single_query_uses_defaults() {
    let args = parse_arguments(r#"{"query":"  ratatui 0.29 release notes  "}"#).expect("args");
    assert_eq!(args.queries, vec!["ratatui 0.29 release notes"]);
    assert_eq!(args.num_results, DEFAULT_NUM_RESULTS);
    assert_eq!(args.search_type, SearchDepth::Auto);
}

#[test]
fn batch_merges_query_and_queries_and_dedupes() {
    let args =
        parse_arguments(r#"{"query":"a","queries":["b","a","c"],"num_results":3,"type":"deep"}"#)
            .expect("args");
    assert_eq!(args.queries, vec!["a", "b", "c"]);
    assert_eq!(args.num_results, 3);
    assert_eq!(args.search_type, SearchDepth::Deep);
}

#[test]
fn camel_case_num_results_is_accepted() {
    let args = parse_arguments(r#"{"query":"a","numResults":5}"#).expect("args");
    assert_eq!(args.num_results, 5);
    let conflict = parse_arguments(r#"{"query":"a","num_results":5,"numResults":6}"#)
        .expect_err("conflict must be rejected");
    assert_eq!(conflict.code, ToolErrorCode::InvalidArguments);
}

#[test]
fn invalid_queries_are_rejected() {
    for (payload, needle) in [
        (r#"{}"#, "requires `query`"),
        (r#"{"queries":[]}"#, "requires `query`"),
        (r#"{"query":"   "}"#, "must not be empty"),
        (r#"{"query":42}"#, "must be a string"),
        (r#"{"queries":"a"}"#, "array of strings"),
        (r#"{"queries":[1]}"#, "array of strings"),
        (r#"{"query":"a","queries":["b","c","d","e"]}"#, "at most 4"),
    ] {
        let failure = parse_arguments(payload).expect_err(payload);
        assert_eq!(failure.code, ToolErrorCode::InvalidArguments, "{payload}");
        assert!(
            failure.message.contains(needle),
            "{payload}: {}",
            failure.message
        );
    }

    let long = "x".repeat(MAX_QUERY_CHARS + 1);
    let failure = parse_arguments(&format!(r#"{{"query":"{long}"}}"#)).expect_err("long");
    assert!(
        failure.message.contains("500 characters"),
        "{}",
        failure.message
    );

    let failure = parse_arguments(r#"{"query":"line\nbreak"}"#).expect_err("control");
    assert!(
        failure.message.contains("control characters"),
        "{}",
        failure.message
    );
}

#[test]
fn unknown_fields_and_bad_options_are_rejected() {
    let failure = parse_arguments(r#"{"query":"a","limit":3}"#).expect_err("unknown field");
    assert!(
        failure.message.contains("unknown argument 'limit'"),
        "{}",
        failure.message
    );

    for payload in [
        r#"{"query":"a","num_results":0}"#,
        r#"{"query":"a","num_results":11}"#,
    ] {
        let failure = parse_arguments(payload).expect_err(payload);
        assert!(
            failure.message.contains("between 1 and 10"),
            "{payload}: {}",
            failure.message
        );
    }

    let failure = parse_arguments(r#"{"query":"a","type":"wide"}"#).expect_err("type");
    assert!(
        failure.message.contains("auto, fast, deep"),
        "{}",
        failure.message
    );
}

#[test]
fn non_object_arguments_are_rejected() {
    for payload in ["", "   ", "[]", "null"] {
        let failure = parse_arguments(payload).expect_err(payload);
        assert_eq!(failure.code, ToolErrorCode::InvalidArguments);
    }
}

// ---------------------------------------------------------------------------
// 响应解析（纯函数）
// ---------------------------------------------------------------------------

#[test]
fn exa_json_and_sse_bodies_are_parsed() {
    let json = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[
        {"type":"text","text":"Result body\nhttps://example.com"}
    ]}}"#;
    assert_eq!(
        client::parse_exa_body(json).expect("json"),
        "Result body\nhttps://example.com"
    );

    let sse = format!(
        "event: message\ndata: {}\n\ndata: [DONE]\n",
        json.replace('\n', "")
    );
    assert_eq!(
        client::parse_exa_body(&sse).expect("sse"),
        "Result body\nhttps://example.com"
    );

    // 空 200 与空 content 都落到统一无结果文案。
    assert_eq!(client::parse_exa_body("  ").expect("empty"), NO_RESULTS);
    assert_eq!(
        client::parse_exa_body(r#"{"result":{"content":[]}}"#).expect("empty content"),
        NO_RESULTS
    );

    // JSON-RPC error 上抛，未解析的正文视为协议错误。
    assert_eq!(
        client::parse_exa_body(r#"{"error":{"message":"bad key"}}"#),
        Err("bad key".to_string())
    );
    assert!(client::parse_exa_body("not json at all").is_err());
}

#[test]
fn tavily_body_lists_answer_and_numbered_results() {
    let body = r#"{
        "answer": "Short answer",
        "results": [
            {"title": "Rust", "url": "https://rust-lang.org", "content": "Systems language"},
            {"title": "Docs", "url": "https://doc.rust-lang.org", "content": "Reference"}
        ]
    }"#;
    assert_eq!(
        client::parse_tavily_body(body).expect("tavily"),
        "Short answer\n\n1. Rust\nhttps://rust-lang.org\nSystems language\n\n\
         2. Docs\nhttps://doc.rust-lang.org\nReference"
    );
    assert_eq!(
        client::parse_tavily_body(r#"{"results":[]}"#).expect("empty"),
        NO_RESULTS
    );
    assert_eq!(
        client::parse_tavily_body(r#"{"detail":{"error":"invalid api key"}}"#),
        Err("invalid api key".to_string())
    );
    assert!(client::parse_tavily_body("nope").is_err());
}

#[test]
fn outcomes_format_single_and_batch_text() {
    let ok = QuerySearchOutcome {
        query: "a".into(),
        provider: Some(WebSearchProvider::Exa),
        output: "body a".into(),
        error: None,
    };
    assert_eq!(format_outcomes(std::slice::from_ref(&ok)), "body a");

    let failed = QuerySearchOutcome {
        query: "b".into(),
        provider: None,
        output: String::new(),
        error: Some("boom".into()),
    };
    let text = format_outcomes(&[ok, failed]);
    assert_eq!(
        text,
        "### 1. \"a\"\nbody a\n\n### 2. \"b\"\n[Search failed: boom]"
    );
}

// ---------------------------------------------------------------------------
// 网络链路（本地回环 mock server）
// ---------------------------------------------------------------------------

fn exa_body(text: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "content": [{ "type": "text", "text": text }] }
    })
    .to_string()
}

fn tavily_body(prefix: &str) -> String {
    serde_json::json!({
        "answer": format!("{prefix} answer"),
        "results": [{
            "title": format!("{prefix} title"),
            "url": "https://example.com",
            "content": format!("{prefix} content"),
        }]
    })
    .to_string()
}

fn endpoints(exa_url: &str, tavily_url: &str) -> WebSearchEndpoints {
    WebSearchEndpoints {
        exa_url: exa_url.to_string(),
        tavily_url: tavily_url.to_string(),
    }
}

/// 启动回环 mock server：每个连接读完整请求，交给 handler 决定响应。
async fn spawn_server<F>(handler: F) -> (String, Requests)
where
    F: Fn(&str) -> (u16, String) + Send + Sync + 'static,
{
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let requests: Requests = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(handler);
    let seen = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let handler = Arc::clone(&handler);
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let request = read_request(&mut stream).await;
                seen.lock().expect("requests").push(request.clone());
                let (status, body) = handler(&request);
                let response = format!(
                    "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    reason(status),
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://127.0.0.1:{port}"), requests)
}

/// 启动慢响应 server：先睡 `delay` 再回 200，用于验证取消。
async fn spawn_slow_server(delay: Duration) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let delay = delay;
            tokio::spawn(async move {
                let _ = read_request(&mut stream).await;
                tokio::time::sleep(delay).await;
                let body = exa_body("late result");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.unwrap_or(0);
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
            let content_length = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if buffer.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

fn request_count(requests: &Requests) -> usize {
    requests.lock().expect("requests").len()
}

#[tokio::test]
async fn exa_success_returns_the_single_query_text() {
    let _guard = NET_LOCK.lock().await;
    let (exa_url, requests) = spawn_server(|_| (200, exa_body("Exa result body"))).await;
    let tavily = spawn_server(|_| (500, String::new())).await.0;
    let (_tx, cancel) = watch::channel(false);

    let result = client::execute_with_endpoints(
        r#"{"query":"ratatui 0.29","num_results":3,"type":"fast"}"#,
        "exa-key",
        "tavily-key",
        &endpoints(&exa_url, &tavily),
        cancel,
    )
    .await;

    let ToolExecutionResult::WebSearch(result) = result else {
        panic!("expected web search result");
    };
    assert_eq!(result.provider, WebSearchProvider::Exa);
    assert_eq!(result.text, "Exa result body");
    assert_eq!(result.queries, vec!["ratatui 0.29"]);
    assert_eq!(result.num_results, 3);
    assert_eq!(result.search_type, SearchDepth::Fast);
    assert_eq!(request_count(&requests), 1);
    let request = requests.lock().expect("requests")[0].clone();
    assert!(
        request.contains("exaApiKey=exa-key"),
        "key must be in the query string: {request}"
    );
    assert!(request.contains("web_search_exa"));
    assert!(request.contains("\"numResults\":3"));
}

#[tokio::test]
async fn exa_sse_response_is_parsed() {
    let _guard = NET_LOCK.lock().await;
    let (exa_url, _) = spawn_server(|_| {
        let body = format!("event: message\ndata: {}\n\n", exa_body("SSE result"));
        (200, body)
    })
    .await;
    let tavily = spawn_server(|_| (500, String::new())).await.0;
    let (_tx, cancel) = watch::channel(false);

    let result = client::execute_with_endpoints(
        r#"{"query":"sse"}"#,
        "exa-key",
        "",
        &endpoints(&exa_url, &tavily),
        cancel,
    )
    .await;

    let ToolExecutionResult::WebSearch(result) = result else {
        panic!("expected web search result");
    };
    assert_eq!(result.text, "SSE result");
}

#[tokio::test]
async fn exa_failure_falls_back_to_tavily() {
    let _guard = NET_LOCK.lock().await;
    let (exa_url, exa_requests) = spawn_server(|_| (500, String::new())).await;
    let (tavily_url, tavily_requests) = spawn_server(|_| (200, tavily_body("fallback"))).await;
    let (_tx, cancel) = watch::channel(false);

    let result = client::execute_with_endpoints(
        r#"{"query":"fallback","type":"deep"}"#,
        "exa-key",
        "tavily-key",
        &endpoints(&exa_url, &tavily_url),
        cancel,
    )
    .await;

    let ToolExecutionResult::WebSearch(result) = result else {
        panic!("expected web search result");
    };
    assert_eq!(result.provider, WebSearchProvider::Tavily);
    assert_eq!(
        result.text,
        "fallback answer\n\n1. fallback title\nhttps://example.com\nfallback content"
    );
    assert_eq!(request_count(&exa_requests), 1);
    assert_eq!(request_count(&tavily_requests), 1);
    let tavily_request = tavily_requests.lock().expect("requests")[0].clone();
    assert!(
        tavily_request
            .to_lowercase()
            .contains("authorization: bearer tavily-key"),
        "{tavily_request}"
    );
    assert!(tavily_request.contains("\"search_depth\":\"advanced\""));
}

#[tokio::test]
async fn anonymous_exa_401_trips_the_breaker() {
    let _guard = NET_LOCK.lock().await;
    client::reset_anonymous_state();
    let (exa_url, exa_requests) = spawn_server(|_| (401, String::new())).await;
    let (tavily_url, tavily_requests) = spawn_server(|_| (200, tavily_body("anonymous"))).await;
    let targets = endpoints(&exa_url, &tavily_url);

    for _ in 0..2 {
        let (_tx, cancel) = watch::channel(false);
        let result = client::execute_with_endpoints(
            r#"{"query":"anonymous"}"#,
            "",
            "tavily-key",
            &targets,
            cancel,
        )
        .await;
        let ToolExecutionResult::WebSearch(result) = result else {
            panic!("anonymous fallback must still succeed");
        };
        assert_eq!(result.provider, WebSearchProvider::Tavily);
    }

    // 第二次调用不再浪费一次必然 401 的匿名 Exa 请求。
    assert_eq!(request_count(&exa_requests), 1);
    assert_eq!(request_count(&tavily_requests), 2);
    assert!(client::anonymous_blocked());

    // 配置了非空 Key 即解除熔断：下一次重新尝试带 Key 的 Exa（失败后再匿名兜底）。
    let _ = client::execute_with_endpoints(
        r#"{"query":"with key"}"#,
        "exa-key",
        "tavily-key",
        &targets,
        watch::channel(false).1,
    )
    .await;
    assert_eq!(request_count(&exa_requests), 3);
    client::reset_anonymous_state();
}

#[tokio::test]
async fn batch_keeps_partial_success() {
    let _guard = NET_LOCK.lock().await;
    let (exa_url, _) = spawn_server(|request| {
        if request.contains("fail-me") {
            (500, String::new())
        } else {
            (200, exa_body("good result"))
        }
    })
    .await;
    let tavily_url = spawn_server(|_| (500, String::new())).await.0;
    let (_tx, cancel) = watch::channel(false);

    let result = client::execute_with_endpoints(
        r#"{"queries":["good one","fail-me"]}"#,
        "exa-key",
        "tavily-key",
        &endpoints(&exa_url, &tavily_url),
        cancel,
    )
    .await;

    let ToolExecutionResult::WebSearch(result) = result else {
        panic!("partial success must not fail the whole call");
    };
    assert_eq!(result.provider, WebSearchProvider::Exa);
    assert_eq!(result.outcomes.len(), 2);
    assert!(result.outcomes[0].error.is_none());
    assert!(result.outcomes[1].error.is_some());
    assert!(result.text.starts_with("### 1. \"good one\"\ngood result"));
    assert!(
        result
            .text
            .contains("### 2. \"fail-me\"\n[Search failed: Exa returned HTTP 500")
    );
}

#[tokio::test]
async fn all_queries_failing_returns_an_error() {
    let _guard = NET_LOCK.lock().await;
    let exa_url = spawn_server(|_| (500, String::new())).await.0;
    let tavily_url = spawn_server(|_| (500, String::new())).await.0;
    let (_tx, cancel) = watch::channel(false);

    let result = client::execute_with_endpoints(
        r#"{"queries":["a","b"]}"#,
        "exa-key",
        "tavily-key",
        &endpoints(&exa_url, &tavily_url),
        cancel,
    )
    .await;

    assert_eq!(result.error_code(), Some(ToolErrorCode::Io));
    assert!(crate::chat::tools::format_for_model(&result).contains("failed across all providers"));
}

#[tokio::test]
async fn cancel_signal_interrupts_the_batch() {
    let _guard = NET_LOCK.lock().await;
    let exa_url = spawn_slow_server(Duration::from_millis(400)).await;
    let tavily_url = spawn_slow_server(Duration::from_millis(400)).await;
    let (sender, cancel) = watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = sender.send(true);
    });

    let result = client::execute_with_endpoints(
        r#"{"queries":["slow one","slow two"]}"#,
        "exa-key",
        "tavily-key",
        &endpoints(&exa_url, &tavily_url),
        cancel,
    )
    .await;

    assert_eq!(result.error_code(), Some(ToolErrorCode::Cancelled));
    assert_eq!(
        crate::chat::tools::format_for_model(&result),
        "Tool failed [cancelled]: cancelled\nno files were changed\n"
    );
}

#[tokio::test]
async fn invalid_arguments_never_touch_the_network() {
    let _guard = NET_LOCK.lock().await;
    let (exa_url, exa_requests) = spawn_server(|_| (200, exa_body("never"))).await;
    let (tavily_url, _) = spawn_server(|_| (200, tavily_body("never"))).await;

    let result = client::execute_with_endpoints(
        r#"{"query":""}"#,
        "exa-key",
        "tavily-key",
        &endpoints(&exa_url, &tavily_url),
        watch::channel(false).1,
    )
    .await;

    assert_eq!(result.error_code(), Some(ToolErrorCode::InvalidArguments));
    assert_eq!(request_count(&exa_requests), 0);
}

/// 结构体字段在 `to_result_json` 前后保持稳定（另一侧 UI 依赖这些键名）。
#[test]
fn provider_roundtrips_through_serde() {
    for provider in [WebSearchProvider::Exa, WebSearchProvider::Tavily] {
        let text = format!("\"{}\"", provider.as_str());
        let parsed: WebSearchProvider = serde_json::from_str(&text).expect("provider");
        assert_eq!(parsed, provider);
        assert_eq!(WebSearchProvider::parse(provider.as_str()), Some(provider));
        assert_eq!(serde_json::to_string(&provider).expect("serialize"), text);
    }
    for depth in [SearchDepth::Auto, SearchDepth::Fast, SearchDepth::Deep] {
        assert_eq!(SearchDepth::parse(depth.as_str()), Some(depth));
    }
}
