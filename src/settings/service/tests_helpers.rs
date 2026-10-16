//! 设置服务测试共用 HTTP 辅助：原始请求、状态码与响应体解析。

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::MAX_BODY_BYTES;

/// 发送原始 HTTP 请求并读回完整响应文本。
pub(super) async fn request(port: u16, raw: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    stream.write_all(raw.as_bytes()).await.expect("write");
    let mut out = String::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_string(&mut out))
        .await
        .expect("no timeout")
        .expect("read");
    out
}

/// 解析响应状态码。
pub(super) fn status(response: &str) -> u16 {
    response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

/// 响应体（跳过 header）。
pub(super) fn body(response: &str) -> &str {
    response.split("\r\n\r\n").nth(1).unwrap_or_default()
}

/// 发送原始请求但容忍写失败（服务端可能提前拒绝并关闭连接）。
///
/// 用于超大请求体等场景：服务端回 413 后立刻关闭，客户端 write 可能报 broken pipe。
pub(super) async fn request_lossy(port: u16, raw: &str) -> String {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)).await else {
        return String::new();
    };
    let _ = stream.write_all(raw.as_bytes()).await;
    let _ = stream.shutdown().await;
    let mut out = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(3), stream.read_to_string(&mut out)).await;
    out
}

/// 便捷 GET。
pub(super) fn get(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
}

/// 便捷带 JSON body 的写请求。
pub(super) fn write(method: &str, path: &str, body: &str, origin: Option<&str>) -> String {
    let origin = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n{origin}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// 上限常量引用（保持与父模块一致）。
const _: usize = MAX_BODY_BYTES;
