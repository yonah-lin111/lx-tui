//! AI Web 流管理：按行读取 `ego-browser nodejs` 的输出，转成普通网络事件。
//!
//! 子进程只存在于事件层；AI Web 会话是纯内存的（`ai-web-*`），因此这里既不写
//! checkpoint，也不覆盖本地工具与进程运行时。

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::*;
pub use crate::app::state::is_ai_web_session;
use crate::app::state::{AiWebSite, HistoryEntry, ai_web_remote_session};
use crate::chat::events::{PartDelta, PartKind};
use crate::chat::web::{
    AiWebHistoryMessage, AiWebStreamEvent, ScriptParams, parse_script_line,
    resolve_ego_browser_bin, script_source, should_record_diag_line,
};

/// AI Web 后台任务回投主事件循环的事件。
#[derive(Debug)]
pub enum AiWebEvent {
    /// 网页端生成的图片已落盘到本地会话图片目录。
    ImageGenerated {
        key: RequestKey,
        image: crate::chat::web::AiWebGeneratedImage,
    },
    /// 网页端生成的原图已下载到目标目录。
    OriginalImageDownloaded {
        pane: PaneId,
        session_id: String,
        attachment_id: String,
        path: String,
    },
    /// 网页端生成的原图下载失败。
    OriginalImageDownloadFailed {
        pane: PaneId,
        session_id: String,
        attachment_id: String,
        message: String,
    },
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
        AiWebEvent::ImageGenerated { key, image } => {
            crate::app::update::on_ai_web_image_generated(state, &key, image)
        }
        AiWebEvent::OriginalImageDownloaded {
            pane,
            session_id,
            attachment_id,
            path,
        } => crate::app::update::on_ai_web_original_image_downloaded(
            state,
            pane,
            &session_id,
            &attachment_id,
            path,
        ),
        AiWebEvent::OriginalImageDownloadFailed {
            pane,
            session_id,
            attachment_id,
            message,
        } => crate::app::update::on_ai_web_original_image_download_failed(
            state,
            pane,
            &session_id,
            &attachment_id,
            message,
        ),
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
            if let Some(target) = state.pane_mut_anywhere(pane) {
                if let Some(crate::app::state::SlashPanel::History(panel)) =
                    target.lx.slash.as_mut()
                {
                    panel.loading = false;
                }
                target.lx.chat.loading_session = false;
            }
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
        Effect::StartAiWebStream { .. }
        | Effect::ResetAiWebSession { .. }
        | Effect::RevealInFileManager { .. }
        | Effect::DownloadAiWebOriginalImage { .. } => true,
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
            model,
            thinking,
            prompt,
            files,
            keep_files,
            edit_user_index,
        } => start_stream(
            state,
            streams,
            sender,
            key,
            site,
            model,
            thinking,
            prompt,
            files,
            keep_files,
            edit_user_index,
        ),
        Effect::ResetAiWebSession { site } => {
            reset_session(site);
            true
        }
        Effect::RevealInFileManager { path } => {
            tokio::task::spawn_blocking(move || {
                if let Err(error) = crate::platform::reveal_in_file_manager(&path) {
                    tracing::warn!(%error, path = %path.display(), "revealing file in file manager failed");
                }
            });
            true
        }
        Effect::DownloadAiWebOriginalImage {
            pane,
            session_id,
            attachment_id,
            site,
            original_url,
            fallback_url,
            file_stem,
        } => {
            download_original_image(
                sender.clone(),
                pane,
                session_id,
                attachment_id,
                site,
                original_url,
                fallback_url,
                file_stem,
            );
            true
        }
        Effect::ListChatSessions { pane } => {
            let site = state
                .pane_anywhere(pane)
                .map(|p| p.lx.ai_web_site)
                .unwrap_or_default();
            list_sessions(sender.clone(), pane, site);
            true
        }
        Effect::LoadChatSession { pane, session_id } => {
            load_session(sender.clone(), pane, session_id);
            true
        }
        Effect::RenameChatSession {
            session_id, title, ..
        } => {
            let owner = state
                .all_pane_ids()
                .into_iter()
                .find(|pane| {
                    state.pane_anywhere(*pane).is_some_and(|p| {
                        p.lx.chat.session_id.as_deref() == Some(session_id.as_str())
                    })
                })
                .unwrap_or_else(|| state.active_tab().layout.focus());
            rename_session(sender.clone(), owner, session_id.clone(), title.clone());
            let panes = state.all_pane_ids();
            let mut changed = false;
            for pane in panes {
                changed |= crate::app::update::on_session_renamed(state, pane, &session_id, &title);
            }
            changed
        }
        Effect::DeleteChatSession { session_id, .. } => {
            let owner = state
                .all_pane_ids()
                .into_iter()
                .find(|pane| {
                    state.pane_anywhere(*pane).is_some_and(|p| {
                        p.lx.chat.session_id.as_deref() == Some(session_id.as_str())
                    })
                })
                .unwrap_or_else(|| state.active_tab().layout.focus());
            delete_session(sender.clone(), owner, session_id.clone());
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
// AI Web 流启动需同时接收状态表、流注册表、通道、请求键、站点、模型、思考等级、提示词与附件上下文，保持直接传参避免一次性包装结构体。
#[allow(clippy::too_many_arguments)]
pub fn start_stream(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    site: AiWebSite,
    model: String,
    thinking: String,
    prompt: String,
    files: Vec<String>,
    keep_files: Vec<String>,
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
            model,
            thinking,
            prompt,
            files,
            keep_files,
            first_turn,
            edit_user_index,
            &mut cancel_rx,
        )
        .await;
    });
    true
}

/// `/new`：让网页端开启新对话；纯后台执行，失败不影响本地已清空的会话。
pub fn reset_session(site: AiWebSite) {
    spawn_action(
        site,
        ScriptParams {
            action: "new",
            ..Default::default()
        },
    );
}

/// `/resume`：异步拉取网页端历史会话列表。
fn list_sessions(sender: mpsc::UnboundedSender<AppEvent>, pane: PaneId, site: AiWebSite) {
    tokio::spawn(async move {
        match run_query(
            site,
            ScriptParams {
                action: "list_sessions",
                ..Default::default()
            },
        )
        .await
        {
            Ok(events) => {
                for event in events {
                    match event {
                        AiWebStreamEvent::Sessions(items) => {
                            let entries = items
                                .into_iter()
                                .map(|item| HistoryEntry {
                                    session_id: site.format_remote_session_id(&item.id),
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
                                message: crate::ui::text::lx_ai_web_login_required_for(site).into(),
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
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed {
                    pane,
                    message: crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.into(),
                }));
            }
            Err(message) => {
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
            }
        }
    });
}

/// `/resume` 选中某条云端会话：切换网页标签并拉取完整历史消息。
fn load_session(sender: mpsc::UnboundedSender<AppEvent>, pane: PaneId, session_id: String) {
    let Some((site, remote_id)) =
        ai_web_remote_session(&session_id).map(|(s, id)| (s, id.to_string()))
    else {
        let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed {
            pane,
            message: crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.into(),
        }));
        return;
    };
    tokio::spawn(async move {
        match run_query(
            site,
            ScriptParams {
                action: "load_session",
                session_id: Some(&remote_id),
                ..Default::default()
            },
        )
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
                                session_id: site.format_remote_session_id(&id),
                                title,
                                messages,
                            }));
                            return;
                        }
                        AiWebStreamEvent::LoginRequired { .. } => {
                            let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed {
                                pane,
                                message: crate::ui::text::lx_ai_web_login_required_for(site).into(),
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
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed {
                    pane,
                    message: crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE.into(),
                }));
            }
            Err(message) => {
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
            }
        }
    });
}

/// `/rename`：同步重命名网页端会话标题；失败时回投提示，避免本地标题与云端分叉。
fn rename_session(
    sender: mpsc::UnboundedSender<AppEvent>,
    pane: PaneId,
    session_id: String,
    title: String,
) {
    let Some((site, remote_id)) =
        ai_web_remote_session(&session_id).map(|(s, id)| (s, id.to_string()))
    else {
        let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed {
            pane,
            message: "session is not linked to a cloud conversation".into(),
        }));
        return;
    };
    tokio::spawn(async move {
        match run_query(
            site,
            ScriptParams {
                action: "rename_session",
                session_id: Some(&remote_id),
                title: Some(&title),
                ..Default::default()
            },
        )
        .await
        {
            Ok(events) => {
                if let Some(message) = action_error(&events, site) {
                    let _ =
                        sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
                }
            }
            Err(message) => {
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
            }
        }
    });
}

/// 下载 AI Web 生成图片的原图到用户配置目录（默认 `~/Desktop`）。
#[allow(clippy::too_many_arguments)]
fn download_original_image(
    sender: mpsc::UnboundedSender<AppEvent>,
    pane: PaneId,
    session_id: String,
    attachment_id: String,
    site: AiWebSite,
    original_url: String,
    fallback_url: String,
    file_stem: String,
) {
    tokio::spawn(async move {
        let download_dir = match crate::config::load_settings()
            .unwrap_or_default()
            .resolved_image_download_dir()
        {
            Ok(dir) => dir.to_string_lossy().into_owned(),
            Err(error) => {
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::OriginalImageDownloadFailed {
                    pane,
                    session_id,
                    attachment_id,
                    message: error.to_string(),
                }));
                return;
            }
        };
        match run_query(
            site,
            ScriptParams {
                action: "download_image",
                download_url: Some(&original_url),
                fallback_url: Some(&fallback_url),
                download_dir: Some(&download_dir),
                download_stem: Some(&file_stem),
                ..Default::default()
            },
        )
        .await
        {
            Ok(events) => {
                if let Some(message) = action_error(&events, site) {
                    let _ = sender.send(AppEvent::AiWeb(AiWebEvent::OriginalImageDownloadFailed {
                        pane,
                        session_id,
                        attachment_id,
                        message,
                    }));
                    return;
                }
                let saved = events.into_iter().find_map(|ev| match ev {
                    AiWebStreamEvent::GeneratedImage { path, .. } if !path.is_empty() => Some(path),
                    _ => None,
                });
                match saved {
                    Some(path) => {
                        let _ = sender.send(AppEvent::AiWeb(AiWebEvent::OriginalImageDownloaded {
                            pane,
                            session_id,
                            attachment_id,
                            path,
                        }));
                    }
                    None => {
                        let _ =
                            sender.send(AppEvent::AiWeb(AiWebEvent::OriginalImageDownloadFailed {
                                pane,
                                session_id,
                                attachment_id,
                                message: "Failed to download original image".into(),
                            }));
                    }
                }
            }
            Err(message) => {
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::OriginalImageDownloadFailed {
                    pane,
                    session_id,
                    attachment_id,
                    message,
                }));
            }
        }
    });
}

/// 删除云端会话；失败时回投提示。
fn delete_session(sender: mpsc::UnboundedSender<AppEvent>, pane: PaneId, session_id: String) {
    let Some((site, remote_id)) =
        ai_web_remote_session(&session_id).map(|(s, id)| (s, id.to_string()))
    else {
        let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed {
            pane,
            message: "session is not linked to a cloud conversation".into(),
        }));
        return;
    };
    tokio::spawn(async move {
        match run_query(
            site,
            ScriptParams {
                action: "delete_session",
                session_id: Some(&remote_id),
                ..Default::default()
            },
        )
        .await
        {
            Ok(events) => {
                if let Some(message) = action_error(&events, site) {
                    let _ =
                        sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
                } else {
                    crate::platform::remove_session_images(&session_id);
                }
            }
            Err(message) => {
                let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ActionFailed { pane, message }));
            }
        }
    });
}

/// 从动作脚本事件里提取登录/执行失败提示。
fn action_error(events: &[AiWebStreamEvent], site: AiWebSite) -> Option<String> {
    events.iter().find_map(|event| match event {
        AiWebStreamEvent::LoginRequired { .. } => {
            Some(crate::ui::text::lx_ai_web_login_required_for(site).to_string())
        }
        AiWebStreamEvent::Error(message) => Some(message.clone()),
        _ => None,
    })
}

/// 一轮网页对话：拉起子进程、喂脚本、按行回投增量与终态。
/// 驱动 `ego-browser nodejs` 执行一轮网页对话并把行协议转成 `ChatNetEvent`。
// 异步对话任务需要完整持有通道、请求键、站点、模型、思考等级、提示词、附件与取消句柄，直接传参比额外定义生命周期包装结构更直观。
#[allow(clippy::too_many_arguments)]
async fn run_chat(
    sender: mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    site: AiWebSite,
    model: String,
    thinking: String,
    prompt: String,
    files: Vec<String>,
    keep_files: Vec<String>,
    first_turn: bool,
    edit_user_index: Option<usize>,
    cancel: &mut watch::Receiver<bool>,
) {
    tracing::debug!(?site, session = %key.session, "ai web stream started");
    let mut child = match Command::new(resolve_ego_browser_bin())
        .arg("nodejs")
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
    let remote_session_id = crate::app::update::ai_web_remote_session_id(&key.session);
    let source = script_source(
        site,
        ScriptParams {
            action: "chat",
            model: Some(&model),
            thinking: Some(&thinking),
            prompt: Some(&prompt),
            files: &files,
            keep_files: &keep_files,
            first_turn,
            stream_url: stream_url.as_deref(),
            session_id: remote_session_id,
            edit_user_index,
            ..Default::default()
        },
    );
    let write_result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        stdin.write_all(source.as_bytes()).await?;
        stdin.shutdown().await
    })
    .await;
    if !matches!(write_result, Ok(Ok(()))) {
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
    let mut listener_abort = listener_task
        .as_ref()
        .map(tokio::task::JoinHandle::abort_handle);
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

    let mut last_diag: Option<String> = None;
    let mut bound_session: Option<(String, Option<String>)> = None;
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_ok() && !*cancel.borrow() {
                    continue;
                }
                if let Some(abort) = listener_abort.take() {
                    abort.abort();
                }
                let _ = child.kill().await;
                spawn_action(
                    site,
                    ScriptParams {
                        action: "stop",
                        ..Default::default()
                    },
                );
                finished(&sender, key, StopReason::Cancelled);
                return;
            }
            line = line_rx.recv() => {
                match line {
                    Some(line) => match parse_script_line(&line) {
                        Some(AiWebStreamEvent::Thinking(delta)) => {
                            send_delta(&sender, &key, PartKind::Reasoning, delta);
                        }
                        Some(AiWebStreamEvent::Text(delta)) => {
                            send_delta(&sender, &key, PartKind::Text, delta);
                        }
                        Some(AiWebStreamEvent::GeneratedImage {
                            label,
                            path,
                            original_url,
                            preview_url,
                            file_stem,
                        }) => {
                            let _ = sender.send(AppEvent::AiWeb(AiWebEvent::ImageGenerated {
                                key: key.clone(),
                                image: crate::chat::web::AiWebGeneratedImage {
                                    label,
                                    path: (!path.is_empty()).then_some(path),
                                    original_url: (!original_url.is_empty())
                                        .then_some(original_url),
                                    preview_url: (!preview_url.is_empty()).then_some(preview_url),
                                    file_stem: (!file_stem.is_empty()).then_some(file_stem),
                                },
                            }));
                        }
                        Some(AiWebStreamEvent::Session { id, title }) => {
                            bound_session = Some((id, title));
                        }
                        Some(AiWebStreamEvent::LoginRequired { .. }) => {
                            finished(
                                &sender,
                                key,
                                failed(crate::ui::text::lx_ai_web_login_required_for(site)),
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
                                    session_id: site.format_remote_session_id(&id),
                                    title,
                                }));
                            }
                            return;
                        }
                        Some(AiWebStreamEvent::Sessions(_) | AiWebStreamEvent::SessionLoaded { .. }) => {}
                        None => {
                            let trimmed = line.trim();
                            if should_record_diag_line(trimmed, last_diag.as_deref()) {
                                tracing::debug!(target: "lx::ai_web", %trimmed, "ego-browser output");
                                last_diag = Some(trimmed.to_string());
                            }
                        }
                    },
                    None => {
                        let msg = last_diag
                            .as_deref()
                            .unwrap_or(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE);
                        finished(&sender, key, failed(msg));
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

mod runner;
use runner::spawn_action;
pub(crate) use runner::{run_query, spawn_stream_listener};

#[cfg(test)]
mod tests;
