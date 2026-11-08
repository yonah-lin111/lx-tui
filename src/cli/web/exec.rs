//! `lx-tui web` 子命令执行器：驱动 `ego-browser nodejs` 并格式化人类可读或 `--json` 输出。

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::app::state::{AiWebSite, DeepseekThinkingMode, GeminiThinkingLevel, GeminiWebModel};
use crate::chat::prepare::{AiWebPrepareError, prepare_ai_web_message};
use crate::chat::web::{
    AiWebStreamEvent, ScriptParams, parse_script_line, resolve_ego_browser_bin, script_source,
    should_record_diag_line,
};
use crate::cli::EXIT_OK;
use crate::cli::output::Output;
use crate::event::chat::web::{run_query, spawn_stream_listener};
use crate::ui::text;

use super::{WebCli, WebCommand, site_key};

pub fn run(cli: WebCli) -> Result<i32, Box<dyn std::error::Error>> {
    let out = Output::new(cli.global.json, cli.global.color);
    let cwd = cli
        .global
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    if let WebCommand::Models { site } = cli.command {
        exec_models(&out, site)?;
        return Ok(EXIT_OK);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to build tokio runtime: {e}"))?;

    rt.block_on(async move {
        match cli.command {
            WebCommand::Models { .. } => Ok(()),
            WebCommand::New { site } => exec_simple_action(&out, site, "new").await,
            WebCommand::Stop { site } => exec_simple_action(&out, site, "stop").await,
            WebCommand::List { site, limit } => exec_list(&out, site, limit).await,
            WebCommand::Show {
                site,
                session_id,
                remote_session_id,
                show_thinking,
            } => exec_show(&out, site, &session_id, &remote_session_id, show_thinking).await,
            WebCommand::Rename {
                site,
                session_id,
                remote_session_id,
                title,
            } => exec_rename(&out, site, &session_id, &remote_session_id, &title).await,
            WebCommand::Delete {
                site,
                session_id,
                remote_session_id,
            } => exec_delete(&out, site, &session_id, &remote_session_id).await,
            WebCommand::Chat {
                site,
                model,
                thinking,
                prompt,
                session_id,
                remote_session_id,
                first_turn,
                edit_user_index,
                files,
                keep_files,
            } => {
                exec_chat(
                    &out,
                    &cwd,
                    ChatRequest {
                        site,
                        model,
                        thinking,
                        prompt,
                        session_id,
                        remote_session_id,
                        first_turn,
                        edit_user_index,
                        files,
                        keep_files,
                    },
                )
                .await
            }
        }
    })?;

    Ok(EXIT_OK)
}

fn exec_models(out: &Output, site_filter: Option<AiWebSite>) -> Result<(), String> {
    let sites: Vec<AiWebSite> = match site_filter {
        Some(s) => vec![s],
        None => AiWebSite::ALL.to_vec(),
    };
    let mut entries = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();

    for site in sites {
        match site {
            AiWebSite::Deepseek => {
                let variants: Vec<&str> =
                    DeepseekThinkingMode::ALL.iter().map(|m| m.id()).collect();
                entries.push(json!({
                    "site": "deepseek",
                    "label": text::AI_WEB_SITE_DEEPSEEK,
                    "taskSpace": "lx-tui-ai-web-deepseek",
                    "sessionPrefix": site.remote_session_prefix(),
                    "defaultModel": text::AI_WEB_SITE_DEEPSEEK,
                    "defaultThinking": DeepseekThinkingMode::default().id(),
                    "models": [text::AI_WEB_SITE_DEEPSEEK],
                    "thinkingVariants": variants,
                }));
                rows.push(vec![
                    "deepseek".into(),
                    text::AI_WEB_SITE_DEEPSEEK.into(),
                    variants.join(", "),
                    format!(
                        "{} ({})",
                        text::AI_WEB_SITE_DEEPSEEK,
                        DeepseekThinkingMode::default().id()
                    ),
                    site.remote_session_prefix().into(),
                    "lx-tui-ai-web-deepseek".into(),
                ]);
            }
            AiWebSite::Gemini => {
                let models: Vec<&str> = GeminiWebModel::ALL.iter().map(|m| m.id()).collect();
                let variants: Vec<&str> = GeminiThinkingLevel::ALL.iter().map(|l| l.id()).collect();
                entries.push(json!({
                    "site": "gemini",
                    "label": text::AI_WEB_SITE_GEMINI,
                    "taskSpace": "lx-tui-ai-web-gemini",
                    "sessionPrefix": site.remote_session_prefix(),
                    "defaultModel": GeminiWebModel::default().id(),
                    "defaultThinking": GeminiThinkingLevel::default().id(),
                    "models": models,
                    "thinkingVariants": variants,
                }));
                for model in GeminiWebModel::ALL {
                    let default_mark = if model == GeminiWebModel::default() {
                        format!(
                            "{} ({})",
                            GeminiWebModel::default().id(),
                            GeminiThinkingLevel::default().id()
                        )
                    } else {
                        "-".into()
                    };
                    rows.push(vec![
                        "gemini".into(),
                        model.id().into(),
                        variants.join(", "),
                        default_mark,
                        site.remote_session_prefix().into(),
                        "lx-tui-ai-web-gemini".into(),
                    ]);
                }
            }
        }
    }

    let payload = json!({ "sites": entries });
    out.emit(&payload, |o, w| {
        o.print_table(
            w,
            &[
                "SITE",
                "MODEL",
                "THINKING",
                "DEFAULT",
                "SESSION_PREFIX",
                "TASK_SPACE",
            ],
            &rows,
        )
    })
}

async fn exec_simple_action(out: &Output, site: AiWebSite, action: &str) -> Result<(), String> {
    let events = run_query(
        site,
        ScriptParams {
            action,
            ..Default::default()
        },
    )
    .await?;
    check_action_events(site, &events)?;
    let payload = json!({
        "site": site_key(site),
        "action": action,
        "ok": true,
    });
    out.emit(&payload, |o, w| {
        writeln!(
            w,
            "{} {} ({})",
            o.green("✓"),
            action,
            text::ai_web_site_label(site)
        )
    })
}

async fn exec_list(out: &Output, site: AiWebSite, limit: Option<usize>) -> Result<(), String> {
    let events = run_query(
        site,
        ScriptParams {
            action: "list_sessions",
            ..Default::default()
        },
    )
    .await?;
    check_action_events(site, &events)?;
    let Some(items) = events.into_iter().find_map(|ev| match ev {
        AiWebStreamEvent::Sessions(list) => Some(list),
        _ => None,
    }) else {
        return Err(text::LX_AI_WEB_EGO_UNAVAILABLE.to_string());
    };
    let sliced: Vec<_> = match limit {
        Some(n) => items.into_iter().take(n).collect(),
        None => items,
    };
    let sessions_json: Vec<_> = sliced
        .iter()
        .map(|item| {
            json!({
                "sessionId": site.format_remote_session_id(&item.id),
                "remoteSessionId": item.id,
                "title": item.title,
                "updatedAtMs": item.updated_at_ms,
            })
        })
        .collect();
    let rows: Vec<Vec<String>> = sliced
        .iter()
        .map(|item| {
            vec![
                site.format_remote_session_id(&item.id),
                item.id.clone(),
                item.title.clone(),
            ]
        })
        .collect();
    let payload = json!({
        "site": site_key(site),
        "count": sessions_json.len(),
        "sessions": sessions_json,
    });
    out.emit(&payload, |o, w| {
        if rows.is_empty() {
            writeln!(
                w,
                "No cloud sessions found on {}.",
                text::ai_web_site_label(site)
            )
        } else {
            o.print_table(w, &["SESSION_ID", "REMOTE_ID", "TITLE"], &rows)
        }
    })
}

async fn exec_show(
    out: &Output,
    site: AiWebSite,
    session_id: &str,
    remote_session_id: &str,
    show_thinking: bool,
) -> Result<(), String> {
    let events = run_query(
        site,
        ScriptParams {
            action: "load_session",
            session_id: Some(remote_session_id),
            ..Default::default()
        },
    )
    .await?;
    check_action_events(site, &events)?;
    let Some((id, title, messages)) = events.into_iter().find_map(|ev| match ev {
        AiWebStreamEvent::SessionLoaded {
            id,
            title,
            messages,
        } => Some((id, title, messages)),
        _ => None,
    }) else {
        return Err(text::LX_AI_WEB_EGO_UNAVAILABLE.to_string());
    };
    let full_id = site.format_remote_session_id(&id);
    let _ = session_id;
    let payload = json!({
        "site": site_key(site),
        "sessionId": full_id,
        "remoteSessionId": id,
        "title": title,
        "messageCount": messages.len(),
        "messages": messages,
    });
    out.emit(&payload, |o, w| {
        writeln!(
            w,
            "{}: {} ({})",
            o.bold("Session"),
            full_id,
            text::ai_web_site_label(site)
        )?;
        if let Some(t) = &title {
            writeln!(w, "{}: {}", o.bold("Title"), t)?;
        }
        writeln!(w, "{}: {}", o.bold("Messages"), messages.len())?;
        for (idx, msg) in messages.iter().enumerate() {
            writeln!(w, "\n[{}] {}", idx, o.cyan(&msg.role))?;
            if !msg.files.is_empty() {
                writeln!(w, "  files: {}", msg.files.join(", "))?;
            }
            if show_thinking && !msg.thinking.trim().is_empty() {
                writeln!(w, "  thinking: {}", o.muted(msg.thinking.trim()))?;
            }
            writeln!(w, "  {}", msg.text)?;
        }
        Ok(())
    })
}

async fn exec_rename(
    out: &Output,
    site: AiWebSite,
    session_id: &str,
    remote_session_id: &str,
    title: &str,
) -> Result<(), String> {
    let events = run_query(
        site,
        ScriptParams {
            action: "rename_session",
            session_id: Some(remote_session_id),
            title: Some(title),
            ..Default::default()
        },
    )
    .await?;
    check_action_events(site, &events)?;
    let payload = json!({
        "site": site_key(site),
        "sessionId": session_id,
        "remoteSessionId": remote_session_id,
        "title": title,
        "ok": true,
    });
    out.emit(&payload, |o, w| {
        writeln!(
            w,
            "{} Renamed {} to \"{}\"",
            o.green("✓"),
            session_id,
            title
        )
    })
}

async fn exec_delete(
    out: &Output,
    site: AiWebSite,
    session_id: &str,
    remote_session_id: &str,
) -> Result<(), String> {
    let events = run_query(
        site,
        ScriptParams {
            action: "delete_session",
            session_id: Some(remote_session_id),
            ..Default::default()
        },
    )
    .await?;
    check_action_events(site, &events)?;
    let payload = json!({
        "site": site_key(site),
        "sessionId": session_id,
        "remoteSessionId": remote_session_id,
        "deleted": true,
    });
    out.emit(&payload, |o, w| {
        writeln!(w, "{} Deleted {}", o.green("✓"), session_id)
    })
}

fn check_action_events(site: AiWebSite, events: &[AiWebStreamEvent]) -> Result<(), String> {
    for ev in events {
        match ev {
            AiWebStreamEvent::LoginRequired { .. } => {
                return Err(text::lx_ai_web_login_required_for(site).to_string());
            }
            AiWebStreamEvent::Error(msg) => {
                return Err(msg.clone());
            }
            _ => {}
        }
    }
    Ok(())
}

struct ChatRequest {
    site: AiWebSite,
    model: String,
    thinking: String,
    prompt: String,
    session_id: Option<String>,
    remote_session_id: Option<String>,
    first_turn: bool,
    edit_user_index: Option<usize>,
    files: Vec<PathBuf>,
    keep_files: Vec<String>,
}

async fn exec_chat(out: &Output, cwd: &Path, req: ChatRequest) -> Result<(), String> {
    let mut prepared = prepare_ai_web_message(cwd, "cli-user", &req.prompt, &req.keep_files)
        .map_err(|err| match err {
            AiWebPrepareError::FolderUnsupported => text::LX_AI_WEB_FOLDER_UNSUPPORTED.to_string(),
            AiWebPrepareError::LocalReferenceUnsupported => {
                text::LX_AI_WEB_LOCAL_REF_UNSUPPORTED.to_string()
            }
            AiWebPrepareError::FileNotFound => text::LX_AI_WEB_FILE_NOT_FOUND.to_string(),
        })?;

    for raw_path in &req.files {
        let resolved = if raw_path.is_absolute() {
            raw_path.clone()
        } else {
            cwd.join(raw_path)
        };
        if !resolved.exists() {
            return Err(format!(
                "{}: {}",
                text::LX_AI_WEB_FILE_NOT_FOUND,
                raw_path.display()
            ));
        }
        if resolved.is_dir() {
            return Err(text::LX_AI_WEB_FOLDER_UNSUPPORTED.to_string());
        }
        let abs = std::fs::canonicalize(&resolved).unwrap_or(resolved);
        let abs_str = abs.to_string_lossy().to_string();
        if !prepared.files.contains(&abs_str) {
            prepared.files.push(abs_str);
        }
    }

    for keep in &req.keep_files {
        if !prepared.keep_files.contains(keep) {
            prepared.keep_files.push(keep.clone());
        }
    }

    if prepared.web_prompt.is_empty() && prepared.files.is_empty() && prepared.keep_files.is_empty()
    {
        return Err("message cannot be empty".to_string());
    }

    let first_turn =
        req.first_turn || (req.remote_session_id.is_none() && req.edit_user_index.is_none());

    let mut child = Command::new(resolve_ego_browser_bin())
        .arg("nodejs")
        .env("LX_AI_WEB_ACTION", "chat")
        .env("LX_AI_WEB_PROMPT", &prepared.web_prompt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| text::LX_AI_WEB_EGO_UNAVAILABLE.to_string())?;

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    let (stream_url, listener_task) = match spawn_stream_listener(line_tx.clone()).await {
        Some((url, handle)) => (Some(url), Some(handle)),
        None => (None, None),
    };

    let Some(mut stdin) = child.stdin.take() else {
        if let Some(handle) = listener_task {
            handle.abort();
        }
        return Err(text::LX_AI_WEB_EGO_UNAVAILABLE.to_string());
    };

    let source = script_source(
        req.site,
        ScriptParams {
            action: "chat",
            model: Some(&req.model),
            thinking: Some(&req.thinking),
            prompt: Some(&prepared.web_prompt),
            files: &prepared.files,
            keep_files: &prepared.keep_files,
            first_turn,
            stream_url: stream_url.as_deref(),
            session_id: req.remote_session_id.as_deref(),
            edit_user_index: req.edit_user_index,
            ..Default::default()
        },
    );

    if stdin.write_all(source.as_bytes()).await.is_err() || stdin.shutdown().await.is_err() {
        if let Some(handle) = listener_task {
            handle.abort();
        }
        return Err(text::LX_AI_WEB_EGO_UNAVAILABLE.to_string());
    }
    drop(stdin);

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

    let mut thinking_text = String::new();
    let mut answer_text = String::new();
    let mut bound_remote_id = req.remote_session_id.clone();
    let mut bound_title: Option<String> = None;
    let mut last_diag: Option<String> = None;

    while let Some(line) = line_rx.recv().await {
        match parse_script_line(&line) {
            Some(AiWebStreamEvent::Thinking(delta)) => {
                thinking_text.push_str(&delta);
            }
            Some(AiWebStreamEvent::Text(delta)) => {
                answer_text.push_str(&delta);
            }
            Some(AiWebStreamEvent::Session { id, title }) => {
                bound_remote_id = Some(id);
                if title.is_some() {
                    bound_title = title;
                }
            }
            Some(AiWebStreamEvent::LoginRequired { .. }) => {
                return Err(text::lx_ai_web_login_required_for(req.site).to_string());
            }
            Some(AiWebStreamEvent::Error(message)) => {
                return Err(message);
            }
            Some(AiWebStreamEvent::Done) => {
                let final_session_id = bound_remote_id
                    .as_deref()
                    .map(|id| req.site.format_remote_session_id(id))
                    .or(req.session_id.clone());
                let payload = json!({
                    "site": site_key(req.site),
                    "model": req.model,
                    "thinking": req.thinking,
                    "sessionId": final_session_id,
                    "remoteSessionId": bound_remote_id,
                    "title": bound_title,
                    "thinkingText": thinking_text,
                    "text": answer_text,
                    "files": prepared.files,
                    "keepFiles": prepared.keep_files,
                    "editIndex": req.edit_user_index,
                });
                return out.emit(&payload, |o, w| {
                    if !thinking_text.trim().is_empty() {
                        writeln!(w, "{}", o.muted(thinking_text.trim()))?;
                    }
                    writeln!(w, "{answer_text}")?;
                    if let Some(sid) = &final_session_id {
                        let title_suffix = bound_title
                            .as_deref()
                            .map(|t| format!(" ({t})"))
                            .unwrap_or_default();
                        writeln!(
                            w,
                            "{}",
                            o.muted(&format!(
                                "[session: {sid}{title_suffix} · {} · {}]",
                                req.model, req.thinking
                            ))
                        )?;
                    }
                    Ok(())
                });
            }
            Some(AiWebStreamEvent::Sessions(_) | AiWebStreamEvent::SessionLoaded { .. }) => {}
            None => {
                let trimmed = line.trim();
                if should_record_diag_line(trimmed, last_diag.as_deref()) {
                    last_diag = Some(trimmed.to_string());
                }
            }
        }
    }

    Err(last_diag.unwrap_or_else(|| text::LX_AI_WEB_EGO_UNAVAILABLE.to_string()))
}
