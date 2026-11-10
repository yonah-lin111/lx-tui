use super::*;

#[test]
fn url_origin_drops_userinfo_query_and_fragment() {
    assert_eq!(
        url_origin("https://user:pass@host:8443/v1/messages?key=secret#x"),
        "https://host:8443"
    );
    assert_eq!(
        url_origin("https://api.example.com"),
        "https://api.example.com"
    );
}

#[test]
fn url_origin_and_path_keeps_path_only() {
    assert_eq!(
        url_origin_and_path("https://user:pass@host:8443/v1/messages?key=secret#x"),
        "https://host:8443/v1/messages"
    );
    assert_eq!(
        url_origin_and_path(
            "https://generativelanguage.googleapis.com/v1beta/models/x:streamGenerateContent?alt=sse&key=g-key"
        ),
        "https://generativelanguage.googleapis.com/v1beta/models/x:streamGenerateContent"
    );
    assert_eq!(url_origin_and_path("not a url"), "<invalid-url>");
}

#[test]
fn headers_are_allowlisted() {
    let headers = vec![
        ("Content-Type".into(), "application/json".into()),
        ("Authorization".into(), "Bearer secret".into()),
        ("x-api-key".into(), "secret".into()),
        ("Cookie".into(), "session=1".into()),
        ("Anthropic-Version".into(), "2023-06-01".into()),
    ];
    let redacted = redacted_headers(&headers);
    assert!(redacted.contains(&("content-type".into(), "application/json".into())));
    assert!(redacted.contains(&("anthropic-version".into(), "2023-06-01".into())));
    assert!(!redacted.iter().any(|(_, value)| value.contains("secret")));
    assert!(redacted.contains(&("authorization".into(), "[redacted]".into())));
}

#[test]
fn sanitize_text_removes_controls_and_truncates() {
    let text = format!("a\u{0}b\n\nc{}", "x".repeat(64));
    let sanitized = sanitize_text(&text, 16);
    assert!(!sanitized.contains('\u{0}'));
    assert!(sanitized.ends_with("…[truncated]"), "{sanitized}");
    assert!(sanitized.len() <= 16 + "…[truncated]".len());
}

#[test]
fn truncate_utf8_respects_boundaries() {
    let text = "日志";
    assert_eq!(truncate_utf8(text, 4), "日");
    assert_eq!(truncate_utf8(text, 6), "日志");
}

#[test]
fn inline_images_are_replaced() {
    let body = r#"{"messages":[{"content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]},{"image_url":{"url":"data:image/png;base64,QUJD"}}]}"#;
    let output = omit_inline_images(body, 1024 * 1024);
    assert!(output.contains("[image omitted: 4 bytes]"), "{output}");
    assert!(!output.contains("QUJD"), "{output}");
    assert!(!output.contains("data:image/png;base64"), "{output}");
}

#[test]
fn google_inline_data_images_are_replaced() {
    let body = r#"{"contents":[{"role":"user","parts":[{"inlineData":{"mimeType":"image/png","data":"QUJDREVG"}},{"inline_data":{"mime_type":"image/jpeg","data":"MTIzNA=="}}]}]}"#;
    let output = omit_inline_images(body, 1024 * 1024);
    assert!(output.contains("[image omitted: 8 bytes]"), "{output}");
    assert!(!output.contains("QUJDREVG"), "{output}");
    assert!(!output.contains("MTIzNA=="), "{output}");
}

#[test]
fn plain_text_preserves_multi_space_indentation() {
    let body = r#"{"model":"m","messages":[{"role":"user","content":"fn main() {\n    println!(\"hi\");\n}"}]}"#;
    let output = omit_inline_images(body, 1024 * 1024);
    assert!(output.contains("    println!"), "{output}");
    assert!(!output.contains("image omitted"));
}
