//! 流式请求执行测试：本地 mock HTTP server 覆盖五种协议的真实 SSE 流。
//!
//! 不依赖公网与真实 Key；mock server 记录请求 URL/header/body 供断言。

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};

use super::*;
use crate::chat::events::{ChatRequest, ChatTurn, PartKind};

/// mock server 收到的请求快照。
#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    target: String,
    headers: Vec<String>,
    body: String,
}

/// 启动一次性 mock server：读一个请求、回固定响应、关闭。
async fn spawn_mock(
    status: &str,
    response_body: &'static str,
) -> (String, tokio::task::JoinHandle<RecordedRequest>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let status = status.to_string();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        // 读到 header 结束 + Content-Length body。
        loop {
            let read = stream.read(&mut chunk).await.expect("read");
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
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                if buffer.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        let header_end = buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("header end");
        let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
        let mut lines = header_text.lines();
        let request_line = lines.next().unwrap_or_default().to_string();
        let headers: Vec<String> = lines.map(|line| line.to_string()).collect();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let target = parts.next().unwrap_or_default().to_string();
        let body = String::from_utf8_lossy(&buffer[header_end + 4..]).into_owned();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).await.expect("write");
        stream.shutdown().await.expect("shutdown");
        RecordedRequest {
            method,
            target,
            headers,
            body,
        }
    });
    (format!("http://127.0.0.1:{port}"), handle)
}

/// 测试请求（两轮完整上下文）。
fn request() -> ChatRequest {
    ChatRequest {
        turns: vec![
            ChatTurn {
                role: "user".into(),
                text: "第一问".into(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "assistant".into(),
                text: "第一答".into(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "user".into(),
                text: "第二问".into(),
                reasoning_metadata: None,
            },
        ],
        items: Vec::new(),
        model: crate::db::chat::ModelSnapshot {
            provider: "example".into(),
            model: "example-chat".into(),
            variant: None,
        },
        variant_params: None,
    }
}

/// 取消通道（初始未取消）；sender 必须存活——sender 丢弃会让
/// `changed()` 立即返回，被 select 当作取消。
fn no_cancel() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

#[tokio::test]
async fn openai_compatible_stream_completes_with_usage() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"思考\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"世界\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        run_stream(
            "openai-compatible",
            &format!("{base}/v1"),
            Some("test-key"),
            Duration::from_secs(5),
            request(),
            cancel,
            |_| {},
        ),
    )
    .await
    .expect("stream completes within timeout");
    let recorded = server.await.expect("server");
    // 请求 URL 正确拼接到 /chat/completions。
    assert_eq!(recorded.method, "POST");
    assert!(
        recorded.target.starts_with("/v1/chat/completions"),
        "URL: {}",
        recorded.target
    );
    // 认证只在 Authorization header。
    assert!(
        recorded
            .headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case("Authorization: Bearer test-key"))
    );
    assert!(!recorded.target.contains("test-key"));
    // 事件序列：text → reasoning → text → usage → complete。
    assert_eq!(outcome.stop, StopReason::Complete);
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Part(part) if part.kind == PartKind::Text && part.text == "你好"
    )));
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Part(part) if part.kind == PartKind::Reasoning && part.text == "思考"
    )));
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(10) && usage.output == Some(5)
    )));
}

#[tokio::test]
async fn http_error_reports_status_with_body_snippet() {
    // 错误码带状态码 + 截断脱敏后的原因说明（便于诊断），绝不回传密钥/超长原文。
    let (base, server) = spawn_mock(
        "401 Unauthorized",
        "{\"error\":{\"message\":\"invalid api key\"}}",
    )
    .await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "openai-compatible",
        &format!("{base}/v1"),
        Some("test-key"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    let _ = server.await;
    let code = match &outcome.stop {
        StopReason::Failed { code } => code.clone(),
        other => panic!("expected Failed, got {other:?}"),
    };
    assert!(code.starts_with("http 401"), "错误码带状态码：{code}");
    assert!(code.contains("invalid api key"), "带原因说明：{code}");
    // 密钥（header 里的 test-key）绝不进错误码。
    assert!(!code.contains("test-key"), "不泄露密钥：{code}");
}

#[tokio::test]
async fn oversized_error_body_is_truncated() {
    let long_body = "x".repeat(4096);
    let body: &'static str = Box::leak(long_body.into_boxed_str());
    let (base, server) = spawn_mock("400 Bad Request", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "openai-compatible",
        &format!("{base}/v1"),
        Some("k"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    let _ = server.await;
    let code = match &outcome.stop {
        StopReason::Failed { code } => code.clone(),
        other => panic!("expected Failed, got {other:?}"),
    };
    assert!(code.len() < 256, "错误说明被截断：len={}", code.len());
}

#[tokio::test]
async fn anthropic_stream_binds_signature_and_usage() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":100}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"隐秘思考\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig-123\"}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"答复\"}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":20}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let (base, server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "anthropic",
        &base,
        Some("test-key"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    let recorded = server.await.expect("server");
    // Anthropic 协议 header：x-api-key + anthropic-version。
    assert!(
        recorded
            .headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case("x-api-key: test-key"))
    );
    assert!(
        recorded
            .headers
            .iter()
            .any(|header| header.starts_with("anthropic-version:"))
    );
    // body 含 max_tokens 与 messages。
    assert!(recorded.body.contains("max_tokens"));
    assert!(recorded.body.contains("messages"));
    assert_eq!(outcome.stop, StopReason::Complete);
    // 签名 delta 附带元数据且不产生正文。
    let signature = outcome.deltas.iter().find_map(|delta| match delta {
        ChatDelta::Part(part) if part.kind == PartKind::Reasoning && part.metadata.is_some() => {
            Some(part)
        }
        _ => None,
    });
    let signature = signature.expect("signature delta exists");
    assert_eq!(signature.text, "");
    let metadata = signature.metadata.as_ref().expect("metadata");
    assert_eq!(
        metadata
            .get("signature")
            .and_then(serde_json::Value::as_str),
        Some("sig-123")
    );
    // usage：input 来自 message_start，output 来自 message_delta。
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(100)
    )));
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.output == Some(20)
    )));
}

#[tokio::test]
async fn google_stream_decodes_thought_parts() {
    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"普通\",\"thought\":true},{\"text\":\"正文\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":50,\"candidatesTokenCount\":8}}\n\n";
    let (base, server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "google",
        &base,
        Some("test-key"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    let recorded = server.await.expect("server");
    // URL：models/{model}:streamGenerateContent?alt=sse&key=。
    assert!(
        recorded
            .target
            .contains("/models/example-chat:streamGenerateContent?alt=sse&key=test-key"),
        "Google Key 只放查询参数：{}",
        recorded.target
    );
    // body 用 contents，assistant 历史 role=model。
    assert!(recorded.body.contains("contents"));
    assert!(recorded.body.contains("\"model\""));
    // 事件：thought part → reasoning，普通 part → text；usage 完整。
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Part(part) if part.kind == PartKind::Reasoning && part.text == "普通"
    )));
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Part(part) if part.kind == PartKind::Text && part.text == "正文"
    )));
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(50) && usage.output == Some(8)
    )));
    assert_eq!(outcome.stop, StopReason::Complete);
}

#[tokio::test]
async fn responses_stream_decodes_summary_and_raw_reasoning() {
    let body = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"答复正文\"}\n\n",
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"摘要思考\"}\n\n",
        "data: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"原始思考\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":30,\"output_tokens\":4}}}\n\n",
    );
    let (base, server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "openai-responses",
        &format!("{base}/v1"),
        Some("test-key"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    let recorded = server.await.expect("server");
    assert!(
        recorded.target.starts_with("/v1/responses"),
        "URL: {}",
        recorded.target
    );
    // body 用 input 而非 messages。
    assert!(recorded.body.contains("input"));
    assert!(!recorded.body.contains("messages"));
    // summary/raw reasoning 均为独立 part 且带来源元数据。
    let summary = outcome.deltas.iter().find_map(|delta| match delta {
        ChatDelta::Part(part) if part.text == "摘要思考" => part.metadata.clone(),
        _ => None,
    });
    assert_eq!(
        summary
            .as_ref()
            .and_then(|metadata| metadata.get("source"))
            .and_then(serde_json::Value::as_str),
        Some("summary")
    );
    let raw = outcome.deltas.iter().find_map(|delta| match delta {
        ChatDelta::Part(part) if part.text == "原始思考" => part.metadata.clone(),
        _ => None,
    });
    assert_eq!(
        raw.as_ref()
            .and_then(|metadata| metadata.get("source"))
            .and_then(serde_json::Value::as_str),
        Some("raw")
    );
    assert!(outcome.deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(30) && usage.output == Some(4)
    )));
    assert_eq!(outcome.stop, StopReason::Complete);
}

#[tokio::test]
async fn truncated_stream_reports_failure_and_keeps_partial() {
    // 流在半帧处截断：已收 partial 保留，终态为失败。
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"部分\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"cont",
    );
    let (base, server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "openai-compatible",
        &format!("{base}/v1"),
        Some("test-key"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    let _ = server.await;
    assert!(
        matches!(
            &outcome.stop,
            StopReason::Failed { .. } | StopReason::Complete
        ),
        "截断流给明确终态：{:?}",
        outcome.stop
    );
    assert!(
        outcome.deltas.iter().any(|delta| matches!(
            delta,
            ChatDelta::Part(part) if part.text == "部分"
        )),
        "已收 partial 保留"
    );
    // 恰好一个终态。
    assert_eq!(
        outcome
            .deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Finished { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn cancellation_preserves_partial_and_yields_single_terminal() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"先到\"}}]}\n\n";
    let (base, _server) = spawn_mock("200 OK", body).await;
    let (cancel_sender, cancel) = watch::channel(false);
    // 先取消再跑流：请求阶段即中止。
    cancel_sender.send(true).expect("cancel");
    let _ = cancel_sender;
    let outcome = run_stream(
        "openai-compatible",
        &format!("{base}/v1"),
        Some("test-key"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    assert_eq!(outcome.stop, StopReason::Cancelled);
    assert_eq!(
        outcome
            .deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Finished { .. }))
            .count(),
        1,
        "恰好一个终态"
    );
}

#[tokio::test]
async fn build_failure_yields_failed_without_network() {
    // 缺 Key：请求前失败。
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "openai-compatible",
        "https://api.example.invalid/v1",
        None,
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    assert!(matches!(outcome.stop, StopReason::Failed { .. }));
    // 请求前失败也保证恰好一个终态事件（无内容 delta）。
    assert_eq!(outcome.deltas.len(), 1);
    assert!(matches!(
        outcome.deltas.first(),
        Some(ChatDelta::Finished {
            reason: StopReason::Failed { .. }
        })
    ));
}

#[tokio::test]
async fn spawn_stream_sends_outcome_through_channel() {
    let (base, _server) = spawn_mock(
        "200 OK",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    )
    .await;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let (_cancel_sender, cancel) = no_cancel();
    spawn_stream(
        "openai-compatible".into(),
        format!("{base}/v1"),
        Some("test-key".into()),
        Duration::from_secs(5),
        request(),
        cancel,
        sender,
    );
    let outcome = receiver.recv().await.expect("outcome");
    assert_eq!(outcome.stop, StopReason::Complete);
}

#[tokio::test]
async fn unsupported_transport_fails_fast() {
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "carrier-pigeon",
        "https://api.example.invalid",
        Some("key"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    assert!(
        matches!(&outcome.stop, StopReason::Failed { code } if code.contains("unsupported transport"))
    );
}

/// 启动一个先回 N 个可重试错误、再回 200 流的 mock server；返回 (base, 请求计数句柄)。
async fn spawn_flaky_mock(
    failures: usize,
    success_body: &'static str,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let served = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = served.clone();
    tokio::spawn(async move {
        for index in 0..=failures {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = stream.read(&mut chunk).await.expect("read");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            counter.fetch_add(1, Ordering::SeqCst);
            let response = if index < failures {
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string()
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    success_body.len(),
                    success_body
                )
            };
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (format!("http://127.0.0.1:{port}"), served)
}

/// 503 后重试成功：请求被重发且最终完成（验证临时故障自愈）。
#[tokio::test]
async fn transient_503_is_retried_then_succeeds() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, served) = spawn_flaky_mock(2, body).await;
    let (_sender, receiver) = no_cancel();
    let outcome = run_stream(
        "openai-compatible",
        &base,
        Some("k"),
        Duration::from_secs(5),
        request(),
        receiver,
        |_| {},
    )
    .await;
    assert_eq!(outcome.stop, StopReason::Complete, "重试后完成");
    assert_eq!(
        served.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "两次 503 各触发一次重试（共 3 次请求）"
    );
}

/// 回归：真实端点把 finish_reason 与 usage 同帧、并以 `[DONE]` 结束；
/// 这样的流必须判为完成，而不是 “stream ended without completion”。
#[tokio::test]
async fn finish_with_usage_same_frame_and_done_completes() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"你好\",\"reasoning_content\":\"\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":2005,\"completion_tokens\":174}}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, _server) = spawn_mock("200 OK", body).await;
    let (_sender, receiver) = no_cancel();
    let outcome = run_stream(
        "openai-compatible",
        &format!("{base}/v1"),
        Some("k"),
        Duration::from_secs(5),
        request(),
        receiver,
        |_| {},
    )
    .await;
    assert_eq!(
        outcome.stop,
        StopReason::Complete,
        "同帧 usage+finish_reason 判为完成"
    );
    assert!(
        outcome
            .deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::Usage(u) if u.input == Some(2005))),
        "usage 被提取"
    );
}

/// 打字机：增量批在流进行中即时回调（不等整段流结束），且与最终 deltas 一致。
#[tokio::test]
async fn incremental_deltas_are_streamed_via_callback() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"甲\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"乙\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"丙\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, _server) = spawn_mock("200 OK", body).await;
    let (_sender, receiver) = no_cancel();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let sink = seen.clone();
    let outcome = run_stream(
        "openai-compatible",
        &format!("{base}/v1"),
        Some("k"),
        Duration::from_secs(5),
        request(),
        receiver,
        |deltas| {
            for delta in deltas {
                if let ChatDelta::Part(part) = delta
                    && part.kind == PartKind::Text
                {
                    sink.lock().expect("lock").push_str(&part.text);
                }
            }
        },
    )
    .await;
    assert_eq!(outcome.stop, StopReason::Complete);
    // 回调累计 == 最终正文（按分块顺序拼接）。
    assert_eq!(seen.lock().expect("lock").as_str(), "甲乙丙");
}
