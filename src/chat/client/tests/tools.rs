//! 工具化路径的客户端测试：工具事件回流、半截参数、重试闸门、能力类 4xx。
//!
//! 复用父模块的 mock server 与请求构造；全部离线。

use super::*;

// ------------------------------------------------------------------ 工具化路径

/// 启动一个“先回 N 次 503、再回给定流”的 mock server，回包按序数变化。
///
/// 用于验证“已产生工具输入后绝不重试”：第一次 200 但流中断，随后若发生重试
/// 计数器会继续增长（测试据此断言请求只发了一次）。
async fn spawn_flaky_after_stream(
    failures: usize,
    success_body: &'static str,
    truncated_body: &'static str,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let served = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = served.clone();
    tokio::spawn(async move {
        for _ in 0..=failures {
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
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            let payload = if attempt < failures {
                // 前几次：200 但流在工具参数中途截断（模拟网络 EOF）。
                truncated_body
            } else {
                success_body
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (format!("http://127.0.0.1:{port}"), served)
}

/// 工具调用完整事件经 client 回流：Started → Delta → Ended → ResponseFinished{ToolCalls}。
#[tokio::test]
async fn client_streams_tool_events_for_openai_chat() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"read\",\"arguments\":\"\"}}]} ,\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        run_stream(
            "openai-compatible",
            &format!("{base}/v1"),
            Some("k"),
            Duration::from_secs(5),
            request(),
            cancel,
            |_| {},
        ),
    )
    .await
    .expect("stream completes within timeout");
    let _ = server.await;
    let trace: Vec<String> = outcome
        .deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::ToolInputStarted {
                call_id,
                name,
                ordinal,
            } => Some(format!("started:{call_id}:{name}:{ordinal}")),
            ChatDelta::ToolInputDelta { call_id, chunk } => {
                Some(format!("delta:{call_id}:{chunk}"))
            }
            ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            } => Some(format!("ended:{call_id}:{arguments_json}")),
            ChatDelta::ResponseFinished { stop } => Some(format!("response:{stop:?}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        trace,
        vec![
            "started:call_a:read:0".to_string(),
            "delta:call_a:{\"path\":\"a.rs\"}".to_string(),
            "ended:call_a:{\"path\":\"a.rs\"}".to_string(),
            "response:ToolCalls".to_string(),
        ]
    );
    // 工具阶段不是整轮终态：恰好一个 Finished 且为 Complete（`[DONE]` + EOF）。
    assert_eq!(outcome.stop, StopReason::Complete);
    assert_eq!(
        outcome
            .deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Finished { .. }))
            .count(),
        1
    );
}

/// 工具参数中途 EOF（半截 JSON）：明确截断错误，且**不产生**可执行调用。
#[tokio::test]
async fn truncated_tool_arguments_fail_without_executable_call() {
    let body = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n\n";
    let (base, server) = spawn_mock("200 OK", body).await;
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
    assert_eq!(
        code,
        crate::chat::transport::tools::TOOL_INPUT_TRUNCATED_CODE
    );
    assert!(
        !outcome
            .deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::ToolInputEnded { .. })),
        "半截参数不得产生可执行调用"
    );
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

/// 已产生工具输入后即使遇到可重试错误也不重试（防止重复文件副作用）。
#[tokio::test]
async fn response_with_tool_input_is_never_retried() {
    // 第一次 200 但流在工具参数处截断，第二次回完整响应；重试规则必须拦下第二次请求。
    let truncated = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"write\",\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n\n";
    let success = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, served) = spawn_flaky_after_stream(1, success, truncated).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        run_stream(
            "openai-compatible",
            &format!("{base}/v1"),
            Some("k"),
            Duration::from_secs(5),
            request(),
            cancel,
            |_| {},
        ),
    )
    .await
    .expect("no infinite retry");
    assert_eq!(
        served.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "已产生工具输入的响应绝不重试"
    );
    assert!(matches!(outcome.stop, StopReason::Failed { .. }));
}

/// 零增量 + 503 仍然重试（保留既有临时故障自愈能力）。
#[tokio::test]
async fn retry_still_happens_without_any_delta() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, served) = spawn_flaky_mock(1, body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = run_stream(
        "openai-compatible",
        &base,
        Some("k"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    assert_eq!(outcome.stop, StopReason::Complete);
    assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// 带 tools 的 400：归类为“该端点不支持工具调用”，且不做静默降级重试。
#[tokio::test]
async fn tools_rejected_with_400_reports_capability_code_once() {
    let (base, server) = spawn_mock(
        "400 Bad Request",
        "{\"error\":{\"message\":\"tools is not supported\"}}",
    )
    .await;
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
    assert!(
        code.starts_with(crate::chat::transport::tools::TOOLS_UNSUPPORTED_CODE),
        "能力类 400 归类为不支持工具：{code}"
    );
    assert!(code.contains("http 400"), "{code}");
    // 不泄露密钥。
    assert!(!code.contains("k\n"), "{code}");
}

/// 请求体确实带 tools 与 lx 系统提示（真实网络路径回归）。
#[tokio::test]
async fn request_body_carries_tools_and_system_prompt() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let _ = run_stream(
        "openai-compatible",
        &format!("{base}/v1"),
        Some("k"),
        Duration::from_secs(5),
        request(),
        cancel,
        |_| {},
    )
    .await;
    let recorded = server.await.expect("server");
    let sent: serde_json::Value = serde_json::from_str(&recorded.body).expect("json body");
    assert_eq!(
        sent["messages"][0]["content"],
        crate::chat::prompt::LX_SYSTEM_PROMPT
    );
    let names: Vec<&str> = sent["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(names, vec!["read", "search", "edit", "write"]);
}

/// 取消发生在工具参数中途：保留已收增量、只产出一个终态、绝不产生可执行调用。
#[tokio::test]
async fn cancellation_during_tool_input_yields_single_terminal() {
    // 只发一帧工具分片就结束（模拟取消后服务端断开）。
    let body = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"write\",\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n\n";
    let (base, _server) = spawn_mock("200 OK", body).await;
    let (cancel_sender, cancel) = watch::channel(false);
    cancel_sender.send(true).expect("cancel");
    let _ = cancel_sender;
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
    assert!(
        !outcome
            .deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::ToolInputEnded { .. })),
        "取消后不得产生可执行调用"
    );
}

/// Anthropic 工具流经 client 回流：tool_use block → input_json_delta → block_stop →
/// message_delta(tool_use) → message_stop，终态恰好一个。
#[tokio::test]
async fn client_streams_tool_events_for_anthropic() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":9}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_a\",\"name\":\"read\",\"input\":{}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":4}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let (base, _server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        run_stream(
            "anthropic",
            &base,
            Some("k"),
            Duration::from_secs(5),
            request(),
            cancel,
            |_| {},
        ),
    )
    .await
    .expect("stream completes within timeout");
    assert_eq!(outcome.stop, StopReason::Complete);
    assert!(outcome.deltas.contains(&ChatDelta::ToolInputStarted {
        call_id: "toolu_a".into(),
        name: "read".into(),
        ordinal: 0,
    }));
    assert!(outcome.deltas.contains(&ChatDelta::ToolInputEnded {
        call_id: "toolu_a".into(),
        arguments_json: "{\"path\":\"a.rs\"}".into(),
    }));
    assert!(outcome.deltas.contains(&ChatDelta::ResponseFinished {
        stop: crate::chat::events::ModelStepStop::ToolCalls
    }));
    assert_eq!(
        outcome
            .deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Finished { .. }))
            .count(),
        1
    );
}

/// Responses 工具流经 client 回流：output_item.added → arguments delta → done → completed。
#[tokio::test]
async fn client_streams_tool_events_for_responses() {
    let body = concat!(
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"item-1\",\"type\":\"function_call\",\"call_id\":\"call_a\",\"name\":\"read\",\"arguments\":\"\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}\n\n",
        "data: {\"type\":\"response.function_call_arguments.done\",\"output_index\":0,\"arguments\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":9,\"output_tokens\":4}}}\n\n",
    );
    let (base, _server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        run_stream(
            "openai-responses",
            &format!("{base}/v1"),
            Some("k"),
            Duration::from_secs(5),
            request(),
            cancel,
            |_| {},
        ),
    )
    .await
    .expect("stream completes within timeout");
    assert_eq!(outcome.stop, StopReason::Complete);
    assert!(outcome.deltas.contains(&ChatDelta::ToolInputEnded {
        call_id: "call_a".into(),
        arguments_json: "{\"path\":\"a.rs\"}".into(),
    }));
    assert!(outcome.deltas.contains(&ChatDelta::ResponseFinished {
        stop: crate::chat::events::ModelStepStop::ToolCalls
    }));
    // usage 必须早于终态（`Finished` 之后不再有增量）。
    let usage_position = outcome
        .deltas
        .iter()
        .position(|delta| matches!(delta, ChatDelta::Usage(_)))
        .expect("usage present");
    let finished_position = outcome
        .deltas
        .iter()
        .position(|delta| matches!(delta, ChatDelta::Finished { .. }))
        .expect("finished present");
    assert!(usage_position < finished_position);
    assert_eq!(
        outcome
            .deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Finished { .. }))
            .count(),
        1
    );
}

/// Google 工具流经 client 回流：functionCall → Started + Ended + ResponseFinished{ToolCalls}，
/// 且重复的累计快照只产生一组事件。
#[tokio::test]
async fn client_streams_tool_events_for_google_and_dedupes() {
    let body = concat!(
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"read\",\"args\":{\"path\":\"a.rs\"}}}],\"role\":\"model\"},\"index\":0}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"read\",\"args\":{\"path\":\"a.rs\"}}}],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":9,\"candidatesTokenCount\":4}}\n\n",
    );
    let (base, _server) = spawn_mock("200 OK", body).await;
    let (_cancel_sender, cancel) = no_cancel();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        run_stream(
            "google",
            &base,
            Some("k"),
            Duration::from_secs(5),
            request(),
            cancel,
            |_| {},
        ),
    )
    .await
    .expect("stream completes within timeout");
    assert_eq!(outcome.stop, StopReason::Complete);
    let started: Vec<&ChatDelta> = outcome
        .deltas
        .iter()
        .filter(|delta| matches!(delta, ChatDelta::ToolInputStarted { .. }))
        .collect();
    assert_eq!(started.len(), 1, "累计快照去重后只执行一次");
    assert!(outcome.deltas.contains(&ChatDelta::ResponseFinished {
        stop: crate::chat::events::ModelStepStop::ToolCalls
    }));
}

/// **G8 回归（真实网络路径）**：同一轮的第 1、2 次模型采样各跑一次 `run_stream`，
/// 同一个 `read` 调用必须拿到**不同**的 call ID。
///
/// 旧实现里两次采样都合成 `gemini-0-0-read`，DB 的
/// `UNIQUE(session_id, call_id)` 让第 2 步 `begin_tool_call` 返回
/// `ToolCallConflict`，read → edit → read 这类多步工具链整轮失败。
#[tokio::test]
async fn google_call_ids_differ_between_two_model_responses() {
    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"read\",\"args\":{\"path\":\"a.rs\"}}}],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}]}\n\n";
    let mut ids = Vec::new();
    for _ in 0..2 {
        let (base, _server) = spawn_mock("200 OK", body).await;
        let (_cancel_sender, cancel) = no_cancel();
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            run_stream(
                "google",
                &base,
                Some("k"),
                Duration::from_secs(5),
                request(),
                cancel,
                |_| {},
            ),
        )
        .await
        .expect("stream completes within timeout");
        let started: Vec<String> = outcome
            .deltas
            .iter()
            .filter_map(|delta| match delta {
                ChatDelta::ToolInputStarted { call_id, .. } => Some(call_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(started.len(), 1, "每次响应一个 functionCall");
        ids.extend(started);
    }
    assert_ne!(
        ids[0], ids[1],
        "两次模型采样不得复用同一 call ID（否则撞 UNIQUE(session_id, call_id)）"
    );
    for id in &ids {
        assert!(!id.trim().is_empty(), "合成 ID 不得为空或全空白：{id:?}");
    }
}
