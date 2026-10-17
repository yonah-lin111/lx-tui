//! SSE 帧解析器测试：UTF-8 分块、CRLF、多行 data、心跳、超限与半帧。

use super::*;

fn frame(data: &str) -> SseFrame {
    SseFrame {
        event: None,
        data: data.to_string(),
    }
}

#[test]
fn simple_frames_with_lf() {
    let mut parser = SseParser::new();
    parser
        .feed(b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\n", 1024)
        .expect("feed ok");
    assert_eq!(
        parser.take_frames(),
        vec![frame("{\"a\":1}"), frame("{\"b\":2}")]
    );
}

#[test]
fn crlf_line_endings() {
    let mut parser = SseParser::new();
    parser
        .feed(b"data: {\"a\":1}\r\n\r\ndata: {\"b\":2}\r\n\r\n", 1024)
        .expect("feed ok");
    assert_eq!(
        parser.take_frames(),
        vec![frame("{\"a\":1}"), frame("{\"b\":2}")]
    );
}

#[test]
fn multi_line_data_joined_by_newline() {
    let mut parser = SseParser::new();
    parser
        .feed(b"data: line1\ndata: line2\n\n", 1024)
        .expect("feed ok");
    assert_eq!(parser.take_frames(), vec![frame("line1\nline2")]);
}

#[test]
fn event_field_captured() {
    let mut parser = SseParser::new();
    parser
        .feed(b"event: message_start\ndata: {\"x\":1}\n\n", 1024)
        .expect("feed ok");
    let frames = parser.take_frames();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].event.as_deref(), Some("message_start"));
    assert_eq!(frames[0].data, "{\"x\":1}");
}

#[test]
fn comment_lines_and_empty_heartbeats_are_ignored() {
    let mut parser = SseParser::new();
    parser
        .feed(b": ping\n\n: keep-alive\ndata: {\"ok\":1}\n\n\n\n", 1024)
        .expect("feed ok");
    assert_eq!(parser.take_frames(), vec![frame("{\"ok\":1}")]);
}

#[test]
fn utf8_split_across_chunks_is_lossless() {
    let mut parser = SseParser::new();
    // 中文 UTF-8 多字节按任意字节位置切分。
    let payload = "data: 你好世界\n\n";
    let bytes = payload.as_bytes();
    for chunk in bytes.chunks(2) {
        parser.feed(chunk, 1024).expect("feed ok");
    }
    assert_eq!(parser.take_frames(), vec![frame("你好世界")]);
}

#[test]
fn frame_split_across_chunks() {
    let mut parser = SseParser::new();
    parser.feed(b"data: {\"par", 1024).expect("feed ok");
    assert!(parser.take_frames().is_empty());
    parser.feed(b"tial\":true}\n", 1024).expect("feed ok");
    assert!(parser.take_frames().is_empty());
    parser.feed(b"\n", 1024).expect("feed ok");
    assert_eq!(parser.take_frames(), vec![frame("{\"partial\":true}")]);
}

#[test]
fn oversized_frame_is_rejected() {
    let mut parser = SseParser::new();
    let big = vec![b'a'; 200];
    let error = parser.feed(&big, 100);
    assert!(error.is_err(), "帧超限拒绝");
}

#[test]
fn flush_emits_unterminated_partial_frame() {
    let mut parser = SseParser::new();
    parser.feed(b"data: half", 1024).expect("feed ok");
    assert!(parser.take_frames().is_empty());
    // EOF 无终止空行：半帧仍吐出（协议层判定完成性）。
    assert_eq!(parser.flush(), Some(frame("half")));
    assert_eq!(parser.flush(), None, "再次 flush 无残留");
}

#[test]
fn flush_no_data_returns_none() {
    let mut parser = SseParser::new();
    assert_eq!(parser.flush(), None);
    parser.feed(b": comment\n", 1024).expect("feed ok");
    assert_eq!(parser.flush(), None, "纯注释行不产出帧");
}

#[test]
fn data_without_space_prefix() {
    let mut parser = SseParser::new();
    // 规范允许 `data:value`（无空格）与 `data: value`。
    parser.feed(b"data:nospace\n\n", 1024).expect("feed ok");
    assert_eq!(parser.take_frames(), vec![frame("nospace")]);
}

#[test]
fn cr_only_line_endings() {
    let mut parser = SseParser::new();
    parser.feed(b"data: {\"a\":1}\r\r", 1024).expect("feed ok");
    assert_eq!(parser.take_frames(), vec![frame("{\"a\":1}")]);
}

#[test]
fn multiple_frames_mixed_endings() {
    let mut parser = SseParser::new();
    parser
        .feed(
            b"event: a\ndata: 1\n\ndata: 2\r\n\r\nevent: c\ndata: 3\r\n\r\n",
            1024,
        )
        .expect("feed ok");
    let frames = parser.take_frames();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].event.as_deref(), Some("a"));
    assert_eq!(frames[0].data, "1");
    assert_eq!(frames[1].data, "2");
    assert_eq!(frames[2].event.as_deref(), Some("c"));
    assert_eq!(frames[2].data, "3");
}
