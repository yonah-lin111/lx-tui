//! `ego-browser nodejs` 子进程启动、带协议行查询与本地回环流监听器。

use std::process::Stdio;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::app::state::AiWebSite;
use crate::chat::web::{
    AiWebStreamEvent, ScriptParams, parse_script_line, resolve_ego_browser_bin, script_source,
    should_record_diag_line,
};

/// 后台执行 `new` / `stop` 这类一次性无返回值脚本动作（带 30 秒超时保护）。
pub(super) fn spawn_action(site: AiWebSite, params: ScriptParams<'static>) {
    let source = script_source(site, params);
    tokio::spawn(async move {
        let Ok(mut child) = Command::new(resolve_ego_browser_bin())
            .arg("nodejs")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        else {
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let write_result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                stdin.write_all(source.as_bytes()).await?;
                stdin.shutdown().await
            })
            .await;
            if !matches!(write_result, Ok(Ok(()))) {
                return;
            }
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), child.wait()).await;
    });
}

/// 执行带返回协议行的脚本动作（`list_sessions` / `load_session` / `rename_session` / `delete_session` / `download_image`）。
pub(crate) async fn run_query(
    site: AiWebSite,
    params: ScriptParams<'_>,
) -> Result<Vec<AiWebStreamEvent>, String> {
    let mut child = Command::new(resolve_ego_browser_bin())
        .arg("nodejs")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.to_string())?;
    let action = params.action.to_string();
    let source = script_source(site, params);
    if let Some(mut stdin) = child.stdin.take() {
        let write_result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            stdin.write_all(source.as_bytes()).await?;
            stdin.shutdown().await
        })
        .await;
        if !matches!(write_result, Ok(Ok(()))) {
            return Err(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.to_string());
        }
    }
    // `load_session` 内部先等标签页（25s）再等历史渲染（10s），预算必须高于两者之和。
    // Grok 冷启动还要叠加登录检测，非 chat 动作统一给 60s，避免 UI 等待被 Rust 先杀掉。
    let budget =
        if action == "load_session" || action == "download_image" || site == AiWebSite::Grok {
            60
        } else {
            30
        };
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(budget),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.to_string())?
    .map_err(|_| crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.to_string())?;
    let mut events = Vec::new();
    let mut last_diag: Option<String> = None;
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        for line in text.lines() {
            if let Some(ev) = parse_script_line(line) {
                events.push(ev);
            } else {
                let trimmed = line.trim();
                if should_record_diag_line(trimmed, last_diag.as_deref()) {
                    last_diag = Some(trimmed.to_string());
                }
            }
        }
    }
    if events.is_empty() {
        return Err(
            last_diag.unwrap_or_else(|| crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.to_string())
        );
    }
    Ok(events)
}

/// 启动本地回环 HTTP 接收器：绕过 `ego-browser nodejs` 对 stdio 的进程级缓冲，
/// 让脚本通过 `fetch(STREAM_URL, { method: 'POST' })` 实时推送每一个流分片。
///
/// URL 携带一次性 token，仅接受该路径；header/body 有上限，连接读写有超时，
/// 避免同机任意进程向当前回复注入文本或耗尽内存。
pub(crate) async fn spawn_stream_listener(
    line_tx: mpsc::UnboundedSender<String>,
) -> Option<(String, tokio::task::JoinHandle<()>)> {
    const HEADER_LIMIT: usize = 16 * 1024;
    const BODY_LIMIT: usize = 1024 * 1024;
    const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    let listener = TcpListener::bind("127.0.0.1:0").await.ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = new_stream_token();
    let expected_path = format!("/{token}");
    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let tx = line_tx.clone();
            let expected_path = expected_path.clone();
            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(2048);
                let mut chunk = [0u8; 1024];
                let header_end = loop {
                    let read =
                        match tokio::time::timeout(READ_TIMEOUT, socket.read(&mut chunk)).await {
                            Ok(Ok(read)) => read,
                            _ => return,
                        };
                    if read == 0 {
                        break None;
                    }
                    buf.extend_from_slice(&chunk[..read]);
                    if buf.len() > HEADER_LIMIT {
                        return;
                    }
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(pos + 4);
                    }
                };
                let Some(body_start) = header_end else {
                    return;
                };
                let header_str = String::from_utf8_lossy(&buf[..body_start]);
                let request_path = header_str
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or_default();
                if request_path != expected_path {
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    return;
                }
                let content_len = header_str.lines().find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                });
                if content_len.is_some_and(|len| len > BODY_LIMIT) {
                    return;
                }
                let target_len = body_start.saturating_add(content_len.unwrap_or(0));
                while buf.len() < target_len {
                    let read =
                        match tokio::time::timeout(READ_TIMEOUT, socket.read(&mut chunk)).await {
                            Ok(Ok(read)) => read,
                            _ => break,
                        };
                    if read == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..read]);
                    if buf.len() > body_start.saturating_add(BODY_LIMIT) {
                        break;
                    }
                }
                let end = target_len.min(buf.len());
                let body = String::from_utf8_lossy(&buf[body_start..end]);
                for line in body.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        let _ = tx.send(trimmed.to_string());
                    }
                }
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            });
        }
    });
    Some((format!("http://127.0.0.1:{port}/{token}"), handle))
}

/// 生成一次性流监听 token（进程内单调 + 时间 + pid 的 SHA-256 前缀）。
fn new_stream_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let mut hasher = Sha256::new();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    hasher.update(nanos.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(SEQUENCE.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
