use super::*;
use std::path::PathBuf;

fn temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("lx-payload-{label}-{}-{nanos}", std::process::id()))
}

fn request<'a>(body: &'a str, headers: &'a [(String, String)]) -> PayloadRequest<'a> {
    PayloadRequest {
        method: "POST",
        url: "https://api.example.com/v1/messages?key=secret",
        headers,
        body,
        tools_requested: true,
    }
}

fn meta() -> PayloadMeta {
    PayloadMeta {
        kind: "chat",
        pane: Some(1),
        session_id: Some("ses-1".into()),
        turn_index: Some(3),
        generation: Some(7),
        ..PayloadMeta::default()
    }
}

#[test]
fn disabled_capture_writes_nothing() {
    let dir = temp_dir("disabled");
    std::fs::create_dir_all(&dir).expect("create dir");
    let capture = PayloadCapture::start(
        false,
        Some(dir.clone()),
        "req-00000001",
        0,
        meta(),
        request("{}", &[]),
    );
    capture.finish(None);
    assert!(std::fs::read_dir(&dir).expect("read").next().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn enabled_capture_writes_redacted_jsonl() {
    let dir = temp_dir("enabled");
    std::fs::create_dir_all(&dir).expect("create dir");
    let headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Authorization".to_string(), "Bearer secret".to_string()),
        ("x-api-key".to_string(), "secret".to_string()),
    ];
    let body = r#"{"model":"m","messages":[{"content":[{"type":"base64","data":"QUJD"}]}]}"#;
    let mut capture = PayloadCapture::start(
        true,
        Some(dir.clone()),
        "req-00000002",
        1,
        meta(),
        request(body, &headers),
    );
    capture.response_status(200);
    capture.record_frame(Some("content_block_delta"), r#"{"delta":"hi"}"#);
    capture.finish(None);

    let path = payload_path(&dir);
    let text = std::fs::read_to_string(&path).expect("payload file");
    let value: Value = serde_json::from_str(text.trim()).expect("valid json");
    assert_eq!(value["request_id"], "req-00000002");
    assert_eq!(value["attempt"], 1);
    assert_eq!(value["response"]["status"], 200);
    assert_eq!(value["truncated"], false);
    assert_eq!(
        value["request"]["url"],
        "https://api.example.com/v1/messages"
    );
    assert_eq!(value["request"]["headers"]["authorization"], "[redacted]");
    assert_eq!(value["request"]["headers"]["x-api-key"], "[redacted]");
    assert_eq!(
        value["request"]["headers"]["content-type"],
        "application/json"
    );
    assert!(!std::fs::read_to_string(&path).unwrap().contains("secret"));
    assert!(text.contains("[image omitted: 4 bytes]"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn oversized_request_body_is_truncated() {
    let dir = temp_dir("body-cap");
    std::fs::create_dir_all(&dir).expect("create dir");
    let body = "x".repeat(MAX_REQUEST_BODY_BYTES + 1);
    let capture = PayloadCapture::start(
        true,
        Some(dir.clone()),
        "req-00000003",
        0,
        meta(),
        request(&body, &[]),
    );
    capture.finish(Some("failed"));
    let text = std::fs::read_to_string(payload_path(&dir)).expect("payload file");
    let value: Value = serde_json::from_str(text.trim()).expect("valid json");
    assert_eq!(value["truncated"], true);
    assert_eq!(value["error"], "failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn oversized_sse_line_is_truncated() {
    let dir = temp_dir("sse-line");
    std::fs::create_dir_all(&dir).expect("create dir");
    let mut capture = PayloadCapture::start(
        true,
        Some(dir.clone()),
        "req-00000004",
        0,
        meta(),
        request("{}", &[]),
    );
    capture.record_frame(Some("delta"), &"x".repeat(MAX_SSE_LINE_BYTES + 1));
    capture.finish(None);
    let text = std::fs::read_to_string(payload_path(&dir)).expect("payload file");
    let value: Value = serde_json::from_str(text.trim()).expect("valid json");
    assert_eq!(value["truncated"], true);
    let data = value["response"]["sse"][0]["data"].as_str().expect("data");
    assert!(data.ends_with("…[truncated]"));
    let _ = std::fs::remove_dir_all(&dir);
}
