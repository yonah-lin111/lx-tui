use super::*;
use rmcp::model::CallToolResult;

fn result(json: serde_json::Value) -> CallToolResult {
    serde_json::from_value(json).expect("call tool result")
}

#[test]
fn text_blocks_are_joined() {
    let result = result(serde_json::json!({
        "content": [{"type": "text", "text": "hello"}, {"type": "text", "text": "world"}]
    }));
    match convert_result(&result, "sess-test") {
        McpCallOutcome::Success { text, .. } => assert_eq!(text, "hello\nworld"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn structured_content_is_kept_first() {
    let result = result(serde_json::json!({
        "content": [{"type": "text", "text": "raw"}],
        "structuredContent": {"answer": 42}
    }));
    match convert_result(&result, "sess-test") {
        McpCallOutcome::Success { text, .. } => {
            assert!(text.starts_with("{\"answer\":42}"));
            assert!(text.contains("raw"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn is_error_becomes_tool_failure_with_fallback_text() {
    let result = result(serde_json::json!({"content": [], "isError": true}));
    assert_eq!(
        convert_result(&result, "sess-test"),
        McpCallOutcome::Failure {
            kind: McpFailureKind::ToolError,
            text: "MCP tool returned an error".into()
        }
    );
}

#[test]
fn audio_gets_a_placeholder() {
    let result = result(serde_json::json!({
        "content": [{"type": "audio", "data": "aGVsbG8=", "mimeType": "audio/wav"}]
    }));
    match convert_result(&result, "sess-test") {
        McpCallOutcome::Success { text, .. } => assert!(text.contains("audio/wav")),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn truncation_marks_the_result() {
    let (text, truncated) = truncate_text(&"a".repeat(100), 10);
    assert!(truncated);
    assert!(text.contains("truncated at 10 bytes"));
}
