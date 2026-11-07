//! AI Web 流管理：按行读取 `ego-browser nodejs` 的输出，转成普通网络事件。
//!
//! 子进程只存在于事件层；AI Web 会话是纯内存的（`ai-web-*`），因此这里既不写
//! checkpoint，也不覆盖本地工具与进程运行时。

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;

use super::*;
use crate::app::state::{AiWebSite, HistoryEntry};
use crate::chat::ai_web::{
    AiWebHistoryMessage, AiWebStreamEvent, DEEPSEEK_SCRIPT, parse_script_line,
    resolve_ego_browser_bin,
};
use crate::chat::events::{PartDelta, PartKind};

/// AI Web 后台任务回投主事件循环的事件。
#[derive(Debug)]
pub enum AiWebEvent {
    /// 首轮或多轮对话后绑定云端真实会话 ID 与标题。
    SessionBound {
        pane: PaneId,
        from_session_id: String,
        session_id: String,
        title: Option<String>,
    },
    /// `/resume` 列表加载完成。
    SessionsListed {
        pane: PaneId,
        entries: Vec<HistoryEntry>,
    },
    /// `/resume` 选中某条云端会话并加载完整消息。
    SessionLoaded {
        pane: PaneId,
        session_id: String,
        title: Option<String>,
        messages: Vec<AiWebHistoryMessage>,
    },
    /// 后台会话动作失败提示。
    ActionFailed { pane: PaneId, message: String },
}

/// 处理 `AppEvent::AiWeb` 回投事件。
pub fn handle_event(state: &mut AppState, event: AiWebEvent) -> bool {
    match event {
        AiWebEvent::SessionBound {
            pane,
            from_session_id,
            session_id,
            title,
        } => crate::app::update::on_ai_web_session_bound(
            state,
            pane,
            &from_session_id,
            session_id,
            title,
        ),
        AiWebEvent::SessionsListed { pane, entries } => {
            crate::app::update::on_history_listed(state, pane, entries)
        }
        AiWebEvent::SessionLoaded {
            pane,
            session_id,
            title,
            messages,
        } => crate::app::update::on_ai_web_session_loaded(state, pane, session_id, title, messages),
        AiWebEvent::ActionFailed { pane, message } => {
            crate::app::update::show_lx_toast(
                state,
                pane,
                Toast::new(ToastKind::Danger, message, Some(pane), Instant::now()),
            );
            true
        }
    }
}

/// 该效应是否属于 AI Web（流启动、重置，或 AI Web 窗格下的会话列表/加载/改名/删除）。
pub fn is_ai_web_effect(state: &AppState, effect: &Effect) -> bool {
    match effect {
        Effect::StartAiWebStream { .. } | Effect::ResetAiWebSession { .. } => true,
        Effect::ListChatSessions { pane } => state
            .pane_anywhere(*pane)
            .is_some_and(|p| p.lx.entry == crate::app::state::LxEntry::AiWeb),
        Effect::LoadChatSession { session_id, .. }
        | Effect::RenameChatSession { session_id, .. }
        | Effect::DeleteChatSession { session_id, .. } => is_ai_web_session(session_id),
        _ => false,
    }
}

/// 执行一条 AI Web 效应；返回是否产生了立即可见的状态变化。
pub fn apply_effect(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    effect: Effect,
) -> bool {
    match effect {
        Effect::StartAiWebStream {
            key,
            site,
            prompt,
            edit_user_index,
        } => start_stream(state, streams, sender, key, site, prompt, edit_user_index),
        Effect::ResetAiWebSession { site } => {
            reset_session(site);
            true
        }
        Effect::ListChatSessions { pane } => {
            list_sessions(sender.clone(), pane);
            true
        }
        Effect::LoadChatSession { pane, session_id } => {
            load_session(sender.clone(), pane, session_id);
            true
        }
        Effect::RenameChatSession {
            session_id, title, ..
        } => {
            rename_session(session_id.clone(), title.clone());
            let panes = state.all_pane_ids();
            let mut changed = false;
            for pane in panes {
                changed |= crate::app::update::on_session_renamed(state, pane, &session_id, &title);
            }
            changed
        }
        Effect::DeleteChatSession { session_id, .. } => {
            delete_session(session_id.clone());
            let panes = state.all_pane_ids();
            let mut changed = false;
            for pane in panes {
                changed |= crate::app::update::on_session_deleted(state, pane, &session_id);
                changed |= crate::app::update::on_session_removed(state, pane, &session_id);
            }
            changed
        }
        _ => false,
    }
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
    edit_user_index: Option<usize>,
) -> bool {
    if active_assistant_id(state, &key).is_none() || streams.cancels.contains_key(&key) {
        return false;
    }
    let first_turn = edit_user_index.is_none()
        && state
            .pane_anywhere(key.pane)
            .is_some_and(|pane| pane.lx.chat.messages.len() <= 2);
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_tx);
    let sender = sender.clone();
    tokio::spawn(async move {
        run_chat(
            sender,
            key,
            site,
            prompt,
            first_turn,
            edit_user_index,
            &mut cancel_rx,
        )
        .await;
    });
    true
}

/// `/new`：让网页端开启新对话；纯后台执行，失败不影响本地已清空的会话。
pub fn reset_session(_site: AiWebSite) {
    spawn_action(ScriptParams {
        action: "new",
        ..Default::default()
    });
}

/// `/resume`：异步拉取网页端历史会话列表。
fn list_sessions(sender: mpsc::UnboundedSender<AppEvent>, pane: PaneId) {
    tokio::spawn(async move {
        match run_query(ScriptParams {
            action: "list_sessions",
            ..Default::default()
        })
        .await
        {
            Ok(events) => {
                for event in events {
                    match event {
                        AiWebStreamEvent::Sessions(items) => {
                            let entries = items
                                .into_iter()
                                .map(|item| HistoryEntry {
                                    session_id: format!(
                                        "{}{}",
                                        crate::app::update::AI_WEB_REMOTE_SESSION_PREFIX,
                                        item.id
                                    ),
                                    title: item.title,
                                    status: "completed".into(),
                                    updated_at_ms: item.updated_at_ms,
                                })
                                .collect();
                            let _ = sender.send(AppEvent::AiWeb(AiWebEvent::SessionsListed {
                                pane,
                                entries,
                            }));
                            return;
                        }
                        AiWebStreamEvent::LoginRequired { .. } => {
                            let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed {
                                pane,
                                message: crate::ui::text::LX_AI_WEB_LOGIN_REQUIRED.into(),
                            }));
                            return;
                        }
                        AiWebStreamEvent::Error(message) => {
                            let _ = sender
                                .send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
                            return;
                        }
                        _ => {}
                    }
                }
            }
            Err(message) => {
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
            }
        }
    });
}

/// `/resume` 选中某条云端会话：切换网页标签并拉取完整历史消息。
fn load_session(sender: mpsc::UnboundedSender<AppEvent>, pane: PaneId, session_id: String) {
    let Some(remote_id) =
        crate::app::update::ai_web_remote_session_id(&session_id).map(str::to_string)
    else {
        return;
    };
    tokio::spawn(async move {
        match run_query(ScriptParams {
            action: "load_session",
            session_id: Some(&remote_id),
            ..Default::default()
        })
        .await
        {
            Ok(events) => {
                for event in events {
                    match event {
                        AiWebStreamEvent::SessionLoaded {
                            id,
                            title,
                            messages,
                        } => {
                            let _ = sender.send(AppEvent::AiWeb(AiWebEvent::SessionLoaded {
                                pane,
                                session_id: format!(
                                    "{}{id}",
                                    crate::app::update::AI_WEB_REMOTE_SESSION_PREFIX
                                ),
                                title,
                                messages,
                            }));
                            return;
                        }
                        AiWebStreamEvent::LoginRequired { .. } => {
                            let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed {
                                pane,
                                message: crate::ui::text::LX_AI_WEB_LOGIN_REQUIRED.into(),
                            }));
                            return;
                        }
                        AiWebStreamEvent::Error(message) => {
                            let _ = sender
                                .send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
                            return;
                        }
                        _ => {}
                    }
                }
            }
            Err(message) => {
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
            }
        }
    });
}

/// `/rename`：同步重命名网页端会话标题。
fn rename_session(session_id: String, title: String) {
    let remote_id = crate::app::update::ai_web_remote_session_id(&session_id).map(str::to_string);
    tokio::spawn(async move {
        let _ = run_query(ScriptParams {
            action: "rename_session",
            session_id: remote_id.as_deref(),
            title: Some(&title),
            ..Default::default()
        })
        .await;
    });
}

/// 删除云端会话。
fn delete_session(session_id: String) {
    let Some(remote_id) =
        crate::app::update::ai_web_remote_session_id(&session_id).map(str::to_string)
    else {
        return;
    };
    tokio::spawn(async move {
        let _ = run_query(ScriptParams {
            action: "delete_session",
            session_id: Some(&remote_id),
            ..Default::default()
        })
        .await;
    });
}

/// 一轮网页对话：拉起子进程、喂脚本、按行回投增量与终态。
async fn run_chat(
    sender: mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    site: AiWebSite,
    prompt: String,
    first_turn: bool,
    edit_user_index: Option<usize>,
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
    let source = script_source(ScriptParams {
        action: "chat",
        prompt: Some(&prompt),
        first_turn,
        stream_url: stream_url.as_deref(),
        edit_user_index,
        ..Default::default()
    });
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
    let mut bound_session: Option<(String, Option<String>)> = None;
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_ok() && !*cancel.borrow() {
                    continue;
                }
                let _ = child.kill().await;
                spawn_action(ScriptParams {
                    action: "stop",
                    ..Default::default()
                });
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
                        Some(AiWebStreamEvent::Session { id, title }) => {
                            bound_session = Some((id, title));
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
                            let pane = key.pane;
                            let from_session = key.session.clone();
                            finished(&sender, key, StopReason::Complete);
                            if let Some((id, title)) = bound_session {
                                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::SessionBound {
                                    pane,
                                    from_session_id: from_session,
                                    session_id: format!(
                                        "{}{id}",
                                        crate::app::update::AI_WEB_REMOTE_SESSION_PREFIX
                                    ),
                                    title,
                                }));
                            }
                            return;
                        }
                        Some(AiWebStreamEvent::Sessions(_) | AiWebStreamEvent::SessionLoaded { .. }) => {}
                        None => {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() && !trimmed.starts_with("[ego-browser:") {
                                tracing::debug!(target: "lx::ai_web", %trimmed, "ego-browser output");
                                last_diag = Some(trimmed.to_string());
                            }
                        }
                    },
                    None => {
                        let pane = key.pane;
                        let from_session = key.session.clone();
                        let reason = if received {
                            StopReason::Complete
                        } else {
                            let msg = last_diag
                                .as_deref()
                                .unwrap_or(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE);
                            failed(msg)
                        };
                        finished(&sender, key, reason);
                        if received
                            && let Some((id, title)) = bound_session
                        {
                            let _ = sender.send(AppEvent::AiWeb(AiWebEvent::SessionBound {
                                pane,
                                from_session_id: from_session,
                                session_id: format!(
                                    "{}{id}",
                                    crate::app::update::AI_WEB_REMOTE_SESSION_PREFIX
                                ),
                                title,
                            }));
                        }
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

#[derive(Default)]
struct ScriptParams<'a> {
    action: &'a str,
    prompt: Option<&'a str>,
    first_turn: bool,
    stream_url: Option<&'a str>,
    session_id: Option<&'a str>,
    title: Option<&'a str>,
    edit_user_index: Option<usize>,
}

/// 后台执行 `new` / `stop` 这类一次性无返回值脚本动作。
fn spawn_action(params: ScriptParams<'static>) {
    let source = script_source(params);
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
        if let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(source.as_bytes()).await.is_ok()
        {
            let _ = stdin.shutdown().await;
        }
        let _ = child.wait().await;
    });
}

/// 执行带返回协议行的脚本动作（`list_sessions` / `load_session` / `rename_session` / `delete_session`）。
async fn run_query(params: ScriptParams<'_>) -> Result<Vec<AiWebStreamEvent>, String> {
    let mut child = Command::new(resolve_ego_browser_bin())
        .arg("nodejs")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.to_string())?;
    let source = script_source(params);
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(source.as_bytes())
            .await
            .map_err(|_| crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.to_string())?;
        let _ = stdin.shutdown().await;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|_| crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.to_string())?;
    let mut events = Vec::new();
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        for line in text.lines() {
            if let Some(ev) = parse_script_line(line) {
                events.push(ev);
            }
        }
    }
    Ok(events)
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
                    };
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
fn script_source(params: ScriptParams<'_>) -> String {
    let mut source = String::from("process.env.LX_AI_WEB_ACTION = ");
    source.push_str(&json_string(params.action));
    source.push_str(";\n");
    if params.first_turn {
        source.push_str("process.env.LX_AI_WEB_FIRST_TURN = \"1\";\n");
    }
    if let Some(url) = params.stream_url {
        source.push_str("process.env.LX_AI_WEB_STREAM_URL = ");
        source.push_str(&json_string(url));
        source.push_str(";\n");
    }
    if let Some(session_id) = params.session_id {
        source.push_str("process.env.LX_AI_WEB_SESSION_ID = ");
        source.push_str(&json_string(session_id));
        source.push_str(";\n");
    }
    if let Some(title) = params.title {
        source.push_str("process.env.LX_AI_WEB_TITLE = ");
        source.push_str(&json_string(title));
        source.push_str(";\n");
    }
    if let Some(idx) = params.edit_user_index {
        source.push_str(&format!("process.env.LX_AI_WEB_EDIT_INDEX = \"{idx}\";\n"));
    }
    if let Some(prompt) = params.prompt {
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
