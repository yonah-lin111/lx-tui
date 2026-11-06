//! AI Web 流管理：按行读取 `ego-browser nodejs` 的输出，转成普通网络事件。
//!
//! 子进程只存在于事件层；AI Web 会话是纯内存的（`ai-web-*`），因此这里既不写
//! checkpoint，也不覆盖本地工具与进程运行时。

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;

use super::*;
use crate::app::state::AiWebSite;
use crate::chat::ai_web::{
    AiWebStreamEvent, DEEPSEEK_SCRIPT, parse_script_line, resolve_ego_browser_bin,
};
use crate::chat::events::{PartDelta, PartKind};

/// 该效应属于 AI Web 流（启动或重置网页会话）。
pub fn is_ai_web_effect(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::StartAiWebStream { .. } | Effect::ResetAiWebSession { .. }
    )
}

/// 会话标识是否为 AI Web 的内存会话。
pub fn is_ai_web_session(session_id: &str) -> bool {
    session_id.starts_with("ai-web-")
}

/// AI Web 失败终态在触发它的 lx 窗格内弹一条红色 Toast（登录 / 驱动不可用等）。
pub fn notify_failure(state: &mut AppState, key: &RequestKey, reason: &StopReason) {
    if !is_ai_web_session(&key.session) {
        return;
    }
    let StopReason::Failed { code } = reason else {
        return;
    };
    crate::app::update::show_lx_toast(
        state,
        key.pane,
        Toast::new(
            ToastKind::Danger,
            code.clone(),
            Some(key.pane),
            Instant::now(),
        ),
    );
}

/// 启动一轮 AI Web 流；key 已失效或已有在途流时不重复启动。
pub fn start_stream(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    site: AiWebSite,
    prompt: String,
) -> bool {
    if active_assistant_id(state, &key).is_none() || streams.cancels.contains_key(&key) {
        return false;
    }
    let first_turn = state
        .pane_anywhere(key.pane)
        .is_some_and(|pane| pane.lx.chat.messages.len() <= 2);
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_tx);
    let sender = sender.clone();
    tokio::spawn(async move {
        run_chat(sender, key, site, prompt, first_turn, &mut cancel_rx).await;
    });
    true
}

/// `/new`：让网页端开启新对话；纯后台执行，失败不影响本地已清空的会话。
pub fn reset_session(_site: AiWebSite) {
    spawn_action("new", None);
}

/// 一轮网页对话：拉起子进程、喂脚本、按行回投增量与终态。
async fn run_chat(
    sender: mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    site: AiWebSite,
    prompt: String,
    first_turn: bool,
    cancel: &mut watch::Receiver<bool>,
) {
    tracing::debug!(?site, session = %key.session, "ai web stream started");
    let mut child = match Command::new(resolve_ego_browser_bin())
        .arg("nodejs")
        .env("LX_AI_WEB_ACTION", "chat")
        .env("LX_AI_WEB_PROMPT", &prompt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(?site, %error, "ai web: failed to start ego-browser");
            finished(
                &sender,
                key,
                failed(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE),
            );
            return;
        }
    };

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    let (stream_url, listener_task) = match spawn_stream_listener(line_tx.clone()).await {
        Some((url, handle)) => (Some(url), Some(handle)),
        None => (None, None),
    };

    let Some(mut stdin) = child.stdin.take() else {
        if let Some(handle) = listener_task {
            handle.abort();
        }
        finished(
            &sender,
            key,
            failed(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE),
        );
        return;
    };
    let source = script_source("chat", Some(&prompt), first_turn, stream_url.as_deref());
    if stdin.write_all(source.as_bytes()).await.is_err() || stdin.shutdown().await.is_err() {
        if let Some(handle) = listener_task {
            handle.abort();
        }
        finished(
            &sender,
            key,
            failed(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE),
        );
        return;
    }
    drop(stdin);

    // 同时采集 stdout 与 stderr，并在二者均 EOF（子进程已退出）时关闭实时 HTTP 监听。
    let stdout_task = child.stdout.take().map(|stdout| {
        let tx = line_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
    });
    let stderr_task = child.stderr.take().map(|stderr| {
        let tx = line_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
    });
    drop(line_tx);
    tokio::spawn(async move {
        if let Some(t) = stdout_task {
            let _ = t.await;
        }
        if let Some(t) = stderr_task {
            let _ = t.await;
        }
        if let Some(handle) = listener_task {
            handle.abort();
        }
    });

    let mut received = false;
    let mut last_diag: Option<String> = None;
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_ok() && !*cancel.borrow() {
                    continue;
                }
                let _ = child.kill().await;
                spawn_action("stop", None);
                finished(&sender, key, StopReason::Cancelled);
                return;
            }
            line = line_rx.recv() => {
                match line {
                    Some(line) => match parse_script_line(&line) {
                        Some(AiWebStreamEvent::Thinking(delta)) => {
                            received = true;
                            send_delta(&sender, &key, PartKind::Reasoning, delta);
                        }
                        Some(AiWebStreamEvent::Text(delta)) => {
                            received = true;
                            send_delta(&sender, &key, PartKind::Text, delta);
                        }
                        Some(AiWebStreamEvent::LoginRequired { .. }) => {
                            finished(
                                &sender,
                                key,
                                failed(crate::ui::text::LX_AI_WEB_LOGIN_REQUIRED),
                            );
                            return;
                        }
                        Some(AiWebStreamEvent::Error(message)) => {
                            finished(&sender, key, failed(&message));
                            return;
                        }
                        Some(AiWebStreamEvent::Done) => {
                            finished(&sender, key, StopReason::Complete);
                            return;
                        }
                        None => {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() && !trimmed.starts_with("[ego-browser:") {
                                tracing::debug!(target: "lx::ai_web", %trimmed, "ego-browser output");
                                last_diag = Some(trimmed.to_string());
                            }
                        }
                    },
                    None => {
                        let reason = if received {
                            StopReason::Complete
                        } else {
                            let msg = last_diag
                                .as_deref()
                                .unwrap_or(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE);
                            failed(msg)
                        };
                        finished(&sender, key, reason);
                        return;
                    }
                }
            }
        }
    }
}

/// 一次正文/思考增量回投。
fn send_delta(
    sender: &mpsc::UnboundedSender<AppEvent>,
    key: &RequestKey,
    kind: PartKind,
    text: String,
) {
    let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Delta {
        key: key.clone(),
        deltas: vec![ChatDelta::Part(PartDelta {
            kind,
            text,
            metadata: None,
        })],
    }));
}

/// 整轮终态回投。
fn finished(sender: &mpsc::UnboundedSender<AppEvent>, key: RequestKey, reason: StopReason) {
    let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
        key,
        reason,
        budget_exceeded: false,
        usage: Usage::default(),
        billing: Usage::default(),
    }));
}

fn failed(code: &str) -> StopReason {
    StopReason::Failed {
        code: code.to_string(),
    }
}

/// 后台执行 `new` / `stop` 这类一次性脚本动作。
fn spawn_action(action: &'static str, prompt: Option<String>) {
    tokio::spawn(async move {
        let mut command = Command::new(resolve_ego_browser_bin());
        command
            .arg("nodejs")
            .env("LX_AI_WEB_ACTION", action)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(prompt) = prompt.as_deref() {
            command.env("LX_AI_WEB_PROMPT", prompt);
        }
        let Ok(mut child) = command.spawn() else {
            tracing::warn!(action, "ai web: failed to start ego-browser");
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let source = script_source(action, prompt.as_deref(), false, None);
            if stdin.write_all(source.as_bytes()).await.is_ok() {
                let _ = stdin.shutdown().await;
            }
        }
        let _ = child.wait().await;
    });
}

/// 启动本地回环 HTTP 接收器：绕过 `ego-browser nodejs` 对 stdio 的进程级缓冲，
/// 让脚本通过 `fetch(STREAM_URL, { method: 'POST' })` 实时推送每一个流分片。
async fn spawn_stream_listener(
    line_tx: mpsc::UnboundedSender<String>,
) -> Option<(String, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await.ok()?;
    let port = listener.local_addr().ok()?.port();
    let handle = tokio::spawn(async move {
        let _keep_tx = line_tx.clone();
        while let Ok((mut socket, _)) = listener.accept().await {
            let tx = line_tx.clone();
            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(2048);
                let mut chunk = [0u8; 1024];
                let header_end = loop {
                    let Ok(n) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        break None;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(pos + 4);
                    }
                };
                if let Some(body_start) = header_end {
                    let header_str = String::from_utf8_lossy(&buf[..body_start]);
                    let content_len = header_str
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    let target_len = body_start.saturating_add(content_len);
                    while buf.len() < target_len {
                        let Ok(n) = socket.read(&mut chunk).await else {
                            break;
                        };
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let end = target_len.min(buf.len());
                    let body = String::from_utf8_lossy(&buf[body_start..end]);
                    for line in body.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            let _ = tx.send(trimmed.to_string());
                        }
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
    Some((format!("http://127.0.0.1:{port}"), handle))
}

/// 组装交给 `ego-browser nodejs` 的脚本。
///
/// `ego-browser` 会剥离调用方的大部分环境变量，所以把动作与提示词显式写进脚本
/// 前置赋值，脚本本身仍按 `LX_AI_WEB_*` 的约定读取。
fn script_source(
    action: &str,
    prompt: Option<&str>,
    first_turn: bool,
    stream_url: Option<&str>,
) -> String {
    let mut source = String::from("process.env.LX_AI_WEB_ACTION = ");
    source.push_str(&json_string(action));
    source.push_str(";\n");
    if first_turn {
        source.push_str("process.env.LX_AI_WEB_FIRST_TURN = \"1\";\n");
    }
    if let Some(url) = stream_url {
        source.push_str("process.env.LX_AI_WEB_STREAM_URL = ");
        source.push_str(&json_string(url));
        source.push_str(";\n");
    }
    if let Some(prompt) = prompt {
        source.push_str("process.env.LX_AI_WEB_PROMPT = ");
        source.push_str(&json_string(prompt));
        source.push_str(";\n");
    }
    source.push_str(DEEPSEEK_SCRIPT);
    source
}

/// JSON 字符串字面量；输入是任意用户文本，必须完整转义。
fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}
