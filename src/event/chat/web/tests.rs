//! 本地流监听器的回环协议与鉴权边界测试。

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::spawn_stream_listener;

/// 向监听器发送一次 HTTP POST，返回完整响应文本。
async fn post(url: &str, path: &str, body: &str) -> std::io::Result<String> {
    let authority = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split_once('/').map(|(authority, _)| authority))
        .expect("http url with port");
    let mut stream = tokio::net::TcpStream::connect(authority).await?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response)
}

fn token_path(url: &str) -> String {
    let rest = url.strip_prefix("http://").expect("http url");
    let (_, path) = rest.split_once('/').expect("token path");
    format!("/{path}")
}

#[tokio::test]
async fn stream_listener_rejects_unknown_token_and_accepts_protocol_frames() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let url = spawn_stream_listener(tx).await.expect("listener url").0;
    let path = token_path(&url);

    let response = post(
        &url,
        "/wrong-token",
        "{\"event\":\"text\",\"delta\":\"injected\"}",
    )
    .await
    .expect("404 response");
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");
    assert!(rx.try_recv().is_err(), "未知 token 不得注入事件");

    let response = post(
        &url,
        &path,
        "{\"event\":\"text\",\"delta\":\"hello\"}\n{\"event\":\"done\"}",
    )
    .await
    .expect("200 response");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(
        rx.recv().await.as_deref(),
        Some("{\"event\":\"text\",\"delta\":\"hello\"}")
    );
    assert_eq!(rx.recv().await.as_deref(), Some("{\"event\":\"done\"}"));
}

#[tokio::test]
async fn stream_listener_closes_oversized_body_without_emitting() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let url = spawn_stream_listener(tx).await.expect("listener url").0;
    let path = token_path(&url);
    let response = post(&url, &path, &"x".repeat(1024 * 1024 + 1))
        .await
        .unwrap_or_default();
    // 超限连接被直接关闭，不会回 200，也不产生事件。
    assert!(
        response.is_empty() || !response.starts_with("HTTP/1.1 200"),
        "{response}"
    );
    assert!(rx.try_recv().is_err());
}
