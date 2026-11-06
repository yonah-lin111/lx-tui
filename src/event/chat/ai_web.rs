//! AI Web 流管理：按行读取 `ego-browser nodejs` 的输出，转成普通网络事件。
//!
//! 子进程只存在于事件层；AI Web 会话是纯内存的（`ai-web-*`），因此这里既不写
//! checkpoint，也不覆盖本地工具与进程运行时。

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_tx);
    let sender = sender.clone();
    tokio::spawn(async move {
        run_chat(sender, key, site, prompt, &mut cancel_rx).await;
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

    let Some(mut stdin) = child.stdin.take() else {
        finished(
            &sender,
            key,
            failed(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE),
        );
        return;
    };
    if stdin
        .write_all(script_source("chat", Some(&prompt)).as_bytes())
        .await
        .is_err()
        || stdin.shutdown().await.is_err()
    {
        finished(
            &sender,
            key,
            failed(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE),
        );
        return;
    }
    drop(stdin);

    // stderr 只用于诊断；必须持续读取，否则管道写满会卡住子进程。
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "lx::ai_web", %line, "ego-browser stderr");
            }
        });
    }

    let Some(stdout) = child.stdout.take() else {
        finished(
            &sender,
            key,
            failed(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE),
        );
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut received = false;
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
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => match parse_script_line(&line) {
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
                        None => {}
                    },
                    Ok(None) => {
                        let reason = if received {
                            StopReason::Complete
                        } else {
                            failed(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE)
                        };
                        finished(&sender, key, reason);
                        return;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "ai web: failed to read ego-browser output");
                        finished(
                            &sender,
                            key,
                            failed(crate::ui::text::LX_AI_WEB_EGO_UNAVAILABLE),
                        );
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
            let source = script_source(action, prompt.as_deref());
            if stdin.write_all(source.as_bytes()).await.is_ok() {
                let _ = stdin.shutdown().await;
            }
        }
        let _ = child.wait().await;
    });
}

/// 组装交给 `ego-browser nodejs` 的脚本。
///
/// `ego-browser` 会剥离调用方的大部分环境变量，所以把动作与提示词显式写进脚本
/// 前置赋值，脚本本身仍按 `LX_AI_WEB_*` 的约定读取。
fn script_source(action: &str, prompt: Option<&str>) -> String {
    let mut source = String::from("process.env.LX_AI_WEB_ACTION = ");
    source.push_str(&json_string(action));
    source.push_str(";\n");
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
