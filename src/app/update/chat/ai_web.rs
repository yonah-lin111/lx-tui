//! AI Web 入口：纯内存会话提交、状态行同步与 `/new` 网页会话重置。
//!
//! 网页端对话由事件层驱动 `ego-browser` 完成；本模块只更新展示状态并入队效应，
//! 不读写 SQLite，也不注入 `lx` 系统提示词或本地工具。

use crate::app::effects::Effect;
use crate::app::state::{
    AppState, AssistantStepView, ChatMessageView, ChatPartView, InputHintLevel, Lx, RequestKey,
    RequestState, SlashPanel,
};
use crate::chat::events::PartKind;
use crate::layout::PaneId;
use crate::ui::text;

/// AI Web 内存会话的 ID 前缀（不写库，仅供流事件归属判断）。
pub const AI_WEB_SESSION_PREFIX: &str = "ai-web-";

/// 映射自云端真实会话的 ID 前缀（仍以 `ai-web-` 开头）。
pub const AI_WEB_REMOTE_SESSION_PREFIX: &str = "ai-web-ds-";

/// 会话 ID 是否属于 AI Web 内存会话。
pub fn is_ai_web_session(session_id: &str) -> bool {
    session_id.starts_with(AI_WEB_SESSION_PREFIX)
}

/// 提取云端真实会话 ID（若已关联云端会话）。
pub fn ai_web_remote_session_id(session_id: &str) -> Option<&str> {
    session_id
        .strip_prefix(AI_WEB_REMOTE_SESSION_PREFIX)
        .filter(|id| !id.is_empty())
}

/// 状态行展示站点名与流状态，隐藏 API 供应商与上下文占用。
pub fn apply_ai_web_status(lx: &mut Lx) {
    lx.chat.status.model_name = Some(text::ai_web_site_label(lx.ai_web_site).to_string());
    lx.chat.status.provider_name = None;
    lx.chat.status.variant = None;
    lx.chat.status.context_limit = None;
    lx.chat.status.input_tokens = None;
    lx.chat.status.streaming = lx.chat.request.is_busy();
}

/// AI Web 发送：直接构造内存态 user + streaming assistant 消息并入队网页流。
///
/// 支持单行 `/new`（`/clear`）、`/resume`（`/session`）、`/rewind`（`/undo`）；
/// `/rename`（`/title`）已在上游 `try_submit_rename_command` 处理。
pub fn try_send_ai_web(
    state: &mut AppState,
    pane: PaneId,
    draft: String,
    busy: bool,
    generation: u64,
) -> bool {
    if draft.is_empty() || busy {
        return false;
    }
    if !draft.contains('\n') && draft.trim_start().starts_with('/') {
        let trimmed = draft.trim();
        let name = trimmed.trim_start_matches('/');
        if !name.is_empty() && !name.contains(char::is_whitespace) && !name.contains('/') {
            if name.eq_ignore_ascii_case("new") || name.eq_ignore_ascii_case("clear") {
                super::slash::clear_lx_input(state, pane);
                super::session::new_session(state, pane);
                return true;
            }
            if name.eq_ignore_ascii_case("resume") || name.eq_ignore_ascii_case("session") {
                super::slash::clear_lx_input(state, pane);
                super::slash::open_history(state, pane);
                return true;
            }
            if name.eq_ignore_ascii_case("rewind") || name.eq_ignore_ascii_case("undo") {
                super::slash::clear_lx_input(state, pane);
                rewind_ai_web(state, pane);
                return true;
            }
            if !SlashPanel::commands_matching(&format!("/{name}")).is_empty() {
                super::session::show_lx_hint(
                    state,
                    pane,
                    InputHintLevel::Warning,
                    text::LX_AI_WEB_ONLY_NEW_CMD,
                );
                return true;
            }
            super::session::show_lx_hint(
                state,
                pane,
                InputHintLevel::Warning,
                text::chat::unknown_command(trimmed),
            );
            return true;
        }
    }
    let reference_root = state
        .pane_anywhere(pane)
        .and_then(|p| p.cwd.clone())
        .or_else(|| state.workspace_cwd_for_pane(pane))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let session_preview = state
        .pane_anywhere(pane)
        .and_then(|t| t.lx.chat.session_id.as_deref())
        .filter(|s| is_ai_web_session(s))
        .map(str::to_string)
        .unwrap_or_else(|| format!("{AI_WEB_SESSION_PREFIX}{}-{generation}", pane.raw()));
    let user_id = format!("{session_preview}-u-{generation}");
    let prepared = match crate::chat::prepare_ai_web_message(&reference_root, &user_id, &draft) {
        Ok(prepared) => prepared,
        Err(err) => {
            let msg = match err {
                crate::chat::AiWebPrepareError::FolderUnsupported => {
                    text::LX_AI_WEB_FOLDER_UNSUPPORTED
                }
                crate::chat::AiWebPrepareError::FileNotFound => text::LX_AI_WEB_FILE_NOT_FOUND,
                crate::chat::AiWebPrepareError::LocalReferenceUnsupported => {
                    text::LX_AI_WEB_LOCAL_REF_UNSUPPORTED
                }
            };
            super::session::show_lx_hint(state, pane, InputHintLevel::Warning, msg);
            return true;
        }
    };
    if prepared.web_prompt.is_empty() && prepared.files.is_empty() {
        return false;
    }
    state.prompt_history.record(&draft);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let site = target.lx.ai_web_site;
    let edit_user_index = target.lx.ai_web_edit_user_index.take();
    target.lx.clear();
    target.lx.clear_rewind();
    target.lx.chat.loading_session = false;
    target.lx.chat.generation_counter = generation;
    target.lx.chat.stop_armed_at = None;
    target.lx.chat.main_stopped = false;
    let session_id = match target.lx.chat.session_id.as_deref() {
        Some(session) if is_ai_web_session(session) => session.to_string(),
        _ => {
            target.lx.chat.session_id = Some(session_preview.clone());
            session_preview
        }
    };
    let base_seq = target.lx.chat.messages.last().map_or(0, |m| m.sequence);
    let turn_index = target
        .lx
        .chat
        .messages
        .last()
        .map_or(0, |m| m.turn_index.saturating_add(1));
    let assistant_id = format!("{session_id}-a-{generation}");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(0)
        });
    target.lx.chat.messages.push(ChatMessageView {
        sequence: base_seq + 1,
        id: user_id.clone(),
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: draft,
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: prepared.attachments,
    });
    target.lx.chat.messages.push(ChatMessageView {
        sequence: base_seq + 2,
        id: assistant_id.clone(),
        kind: "chat".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index,
        shell: None,
        steps: Vec::new(),
        model_label: Some(text::ai_web_site_label(site).to_string()),
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    target.lx.chat.set_message_time(&user_id, now_ms);
    target.lx.chat.set_message_time(&assistant_id, now_ms);
    target.lx.chat.follow_bottom = true;
    target.lx.chat.reset_step_usage();
    let key = RequestKey {
        pane,
        session: session_id,
        generation,
    };
    target.lx.chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id,
    };
    state.effects.push(Effect::StartAiWebStream {
        key,
        site,
        prompt: prepared.web_prompt,
        files: prepared.files,
        edit_user_index,
    });
    super::refresh_lx_status(state, pane);
    true
}

/// AI Web `/rewind`（`/undo`）：一键撤回最新一轮问答并将用户原文回填输入框，
/// 标记下一轮发送在网页端原地编辑该条用户消息以对齐云端分支。
pub fn rewind_ai_web(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.request.is_busy() {
        super::session::show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_REWIND_FAILED);
        return true;
    }
    let Some(last_user_idx) = target
        .lx
        .chat
        .messages
        .iter()
        .rposition(|message| message.role == "user" && !message.is_user_shell())
    else {
        super::session::show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_REWIND_FAILED);
        return true;
    };
    let user_text = target.lx.chat.messages[last_user_idx].text();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.messages.truncate(last_user_idx);
    let remaining_users = target
        .lx
        .chat
        .messages
        .iter()
        .filter(|message| message.role == "user" && !message.is_user_shell())
        .count();
    target.lx.ai_web_edit_user_index = Some(remaining_users);
    target.lx.clear_rewind();
    target.lx.slash = None;
    target.lx.replace_input(&user_text);
    target.lx.chat.follow_bottom = true;
    super::refresh_lx_status(state, pane);
    true
}

/// 网页端首轮生成后绑定云端真实会话 ID 与自动标题。
pub fn on_ai_web_session_bound(
    state: &mut AppState,
    pane: PaneId,
    from_session_id: &str,
    session_id: String,
    title: Option<String>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let current = target.lx.chat.session_id.as_deref();
    if current != Some(from_session_id) && current != Some(session_id.as_str()) {
        return false;
    }
    let mut changed = target.lx.chat.session_id.as_deref() != Some(session_id.as_str());
    target.lx.chat.session_id = Some(session_id);
    if target.lx.chat.session_title.is_none()
        && let Some(title) = title
    {
        changed |= target.lx.chat.set_session_title(Some(title));
    }
    changed
}

/// `/resume` 选中云端会话后重建窗格消息视图。
pub fn on_ai_web_session_loaded(
    state: &mut AppState,
    pane: PaneId,
    session_id: String,
    title: Option<String>,
    messages: Vec<crate::chat::ai_web::AiWebHistoryMessage>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let site_label = text::ai_web_site_label(target.lx.ai_web_site).to_string();
    target.lx.chat.reset_session_view();
    target.lx.clear_agents();
    target.lx.compact.reset();
    target.lx.model_switch.reset();
    target.lx.clear_rewind();
    target.lx.ai_web_edit_user_index = None;
    target.lx.chat.session_id = Some(session_id.clone());
    target.lx.chat.set_session_title(title);

    let mut views = Vec::with_capacity(messages.len());
    let mut turn_index: i64 = 0;
    for (idx, item) in messages.into_iter().enumerate() {
        let sequence = i64::try_from(idx + 1).unwrap_or(1);
        let msg_id = format!("{session_id}-m-{}", idx + 1);
        if item.role == "user" {
            if idx > 0 {
                turn_index = turn_index.saturating_add(1);
            }
            let attachments = item
                .files
                .into_iter()
                .enumerate()
                .map(|(att_idx, name)| {
                    let kind = if crate::chat::is_image_filename(&name) {
                        crate::db::chat::MessageAttachmentKind::Image
                    } else {
                        crate::db::chat::MessageAttachmentKind::File
                    };
                    crate::db::chat::MessageAttachmentMeta {
                        id: format!("{msg_id}-att-{att_idx}"),
                        position: i64::try_from(att_idx).unwrap_or(0),
                        kind,
                        label: name,
                        marker_start: None,
                        marker_end: None,
                        source_path: None,
                        mime: None,
                        width: None,
                        height: None,
                    }
                })
                .collect();
            views.push(ChatMessageView {
                sequence,
                id: msg_id.clone(),
                kind: "chat".into(),
                role: "user".into(),
                status: "complete".into(),
                turn_index,
                shell: None,
                steps: vec![AssistantStepView::Part {
                    position: 0,
                    part: ChatPartView {
                        kind: PartKind::Text,
                        content: item.text,
                        provider_metadata_json: None,
                        started_at_ms: None,
                        ended_at_ms: None,
                        completed_at_ms: None,
                    },
                }],
                model_label: None,
                error_code: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                duration_ms: None,
                reports: Vec::new(),
                attachments,
            });
        } else {
            let mut steps = Vec::new();
            let completed_ms = item.created_at_ms.max(1);
            if !item.thinking.is_empty() {
                steps.push(AssistantStepView::Part {
                    position: 0,
                    part: ChatPartView {
                        kind: PartKind::Reasoning,
                        content: item.thinking,
                        provider_metadata_json: None,
                        started_at_ms: item.thinking_ms.map(|_| 0),
                        ended_at_ms: item.thinking_ms,
                        completed_at_ms: Some(completed_ms),
                    },
                });
            }
            if !item.text.is_empty() || steps.is_empty() {
                let position = i64::try_from(steps.len()).unwrap_or(0);
                steps.push(AssistantStepView::Part {
                    position,
                    part: ChatPartView {
                        kind: PartKind::Text,
                        content: item.text,
                        provider_metadata_json: None,
                        started_at_ms: None,
                        ended_at_ms: None,
                        completed_at_ms: Some(completed_ms),
                    },
                });
            }
            views.push(ChatMessageView {
                sequence,
                id: msg_id.clone(),
                kind: "chat".into(),
                role: "assistant".into(),
                status: "complete".into(),
                turn_index,
                shell: None,
                steps,
                model_label: Some(site_label.clone()),
                error_code: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                duration_ms: None,
                reports: Vec::new(),
                attachments: Vec::new(),
            });
        }
        if item.created_at_ms > 0 {
            target.lx.chat.set_message_time(&msg_id, item.created_at_ms);
        }
    }
    target.lx.chat.messages = views;
    target.lx.chat.follow_bottom = true;
    crate::app::update::clear_lx_selection_for(state, pane);
    super::refresh_lx_status(state, pane);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AiWebSite, LxEntry};
    use crate::chat::events::{ChatDelta, PartDelta, StopReason};
    use crate::db::chat::Usage;

    fn ai_web_state() -> (AppState, PaneId) {
        let mut state = AppState::demo();
        let pane = state.active_tab().layout.focus();
        crate::app::update::set_lx_entry(&mut state, pane, LxEntry::AiWeb);
        (state, pane)
    }

    fn draft(state: &mut AppState, pane: PaneId, text: &str) {
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.input = text.into();
        }
    }

    fn finish(state: &mut AppState, pane: PaneId) {
        let key = match &state
            .pane_anywhere(pane)
            .map(|target| &target.lx.chat.request)
        {
            Some(RequestState::Streaming { key, .. }) => key.clone(),
            _ => panic!("expected streaming request"),
        };
        let delta = ChatDelta::Part(PartDelta {
            kind: PartKind::Text,
            text: "reply".into(),
            metadata: None,
        });
        assert!(super::super::on_stream_delta(state, &key, &[delta]));
        assert!(super::super::on_stream_finished(
            state,
            &key,
            &StopReason::Complete,
            Usage::default(),
            Usage::default(),
        ));
    }

    #[test]
    fn send_creates_memory_messages_and_starts_web_stream() {
        let (mut state, pane) = ai_web_state();
        draft(&mut state, pane, "hello");
        assert!(super::super::try_send(&mut state, pane));
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.chat.messages.len(), 2);
        assert_eq!(target.lx.chat.messages[0].role, "user");
        assert_eq!(target.lx.chat.messages[1].role, "assistant");
        assert_eq!(target.lx.chat.messages[1].status, "streaming");
        assert!(matches!(
            target.lx.chat.request,
            RequestState::Streaming { .. }
        ));
        assert_eq!(
            target.lx.chat.status.model_name.as_deref(),
            Some("DeepSeek")
        );
        assert!(
            state
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::StartAiWebStream { .. }))
        );
    }

    #[test]
    fn finished_web_stream_settles_in_memory_without_db_write() {
        let (mut state, pane) = ai_web_state();
        draft(&mut state, pane, "hello");
        assert!(super::super::try_send(&mut state, pane));
        finish(&mut state, pane);
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.chat.request, RequestState::Idle);
        let assistant = target.lx.chat.messages.last().expect("assistant");
        assert_eq!(assistant.status, "complete");
        assert_eq!(assistant.text(), "reply");
        assert!(
            !state
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::FinishChatTurn { .. }))
        );
    }

    #[test]
    fn new_command_resets_messages_and_web_session() {
        let (mut state, pane) = ai_web_state();
        draft(&mut state, pane, "hello");
        assert!(super::super::try_send(&mut state, pane));
        finish(&mut state, pane);
        draft(&mut state, pane, "/new");
        assert!(super::super::try_send(&mut state, pane));
        let target = state.pane_anywhere(pane).expect("pane");
        assert!(target.lx.chat.messages.is_empty());
        assert!(state.effects.iter().any(|effect| matches!(
            effect,
            Effect::ResetAiWebSession {
                site: AiWebSite::Deepseek
            }
        )));
    }

    #[test]
    fn model_command_is_rejected_in_ai_web() {
        let (mut state, pane) = ai_web_state();
        draft(&mut state, pane, "/model");
        assert!(super::super::try_send(&mut state, pane));
        let target = state.pane_anywhere(pane).expect("pane");
        assert!(target.lx.chat.messages.is_empty());
        assert_eq!(
            target.lx.toast.as_ref().map(|toast| toast.message.as_str()),
            Some(text::LX_AI_WEB_ONLY_NEW_CMD)
        );
    }

    #[test]
    fn multi_turn_send_advances_generation_and_keeps_qa_order() {
        let (mut state, pane) = ai_web_state();
        draft(&mut state, pane, "你是谁");
        assert!(super::super::try_send(&mut state, pane));
        finish(&mut state, pane);

        draft(&mut state, pane, "最近有什么新闻");
        assert!(super::super::try_send(&mut state, pane));
        let key2 = match &state
            .pane_anywhere(pane)
            .map(|target| &target.lx.chat.request)
        {
            Some(RequestState::Streaming { key, .. }) => key.clone(),
            _ => panic!("expected streaming request on turn 2"),
        };
        assert_eq!(key2.generation, 2);
        let delta2 = ChatDelta::Part(PartDelta {
            kind: PartKind::Text,
            text: "second reply".into(),
            metadata: None,
        });
        assert!(super::super::on_stream_delta(&mut state, &key2, &[delta2]));
        assert!(super::super::on_stream_finished(
            &mut state,
            &key2,
            &StopReason::Complete,
            Usage::default(),
            Usage::default(),
        ));

        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.chat.messages.len(), 4);
        assert_eq!(target.lx.chat.messages[0].text(), "你是谁");
        assert_eq!(target.lx.chat.messages[1].text(), "reply");
        assert_eq!(target.lx.chat.messages[2].text(), "最近有什么新闻");
        assert_eq!(target.lx.chat.messages[3].text(), "second reply");
    }

    #[test]
    fn rewind_pops_latest_turn_restores_input_and_marks_edit_index() {
        let (mut state, pane) = ai_web_state();
        draft(&mut state, pane, "第一问");
        assert!(super::super::try_send(&mut state, pane));
        finish(&mut state, pane);

        draft(&mut state, pane, "第二问");
        assert!(super::super::try_send(&mut state, pane));
        finish(&mut state, pane);
        state.effects.clear();

        draft(&mut state, pane, "/rewind");
        assert!(super::super::try_send(&mut state, pane));
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.chat.messages.len(), 2);
        assert_eq!(target.lx.input, "第二问");
        assert_eq!(target.lx.ai_web_edit_user_index, Some(1));

        draft(&mut state, pane, "修改后的第二问");
        assert!(super::super::try_send(&mut state, pane));
        assert!(state.effects.iter().any(|effect| matches!(
            effect,
            Effect::StartAiWebStream {
                prompt,
                edit_user_index: Some(1),
                ..
            } if prompt == "修改后的第二问"
        )));
    }

    #[test]
    fn rename_and_session_load_work_in_ai_web() {
        let (mut state, pane) = ai_web_state();
        on_ai_web_session_loaded(
            &mut state,
            pane,
            "ai-web-ds-uuid-1".into(),
            Some("Old Title".into()),
            vec![
                crate::chat::ai_web::AiWebHistoryMessage {
                    role: "user".into(),
                    thinking: String::new(),
                    thinking_ms: None,
                    text: "hello".into(),
                    files: vec!["photo.png".into(), "report.pdf".into()],
                    created_at_ms: 1000,
                },
                crate::chat::ai_web::AiWebHistoryMessage {
                    role: "assistant".into(),
                    thinking: "let me think".into(),
                    thinking_ms: Some(1500),
                    text: "world".into(),
                    files: Vec::new(),
                    created_at_ms: 2000,
                },
            ],
        );
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.chat.messages.len(), 2);
        assert_eq!(target.lx.chat.messages[0].attachments.len(), 2);
        assert_eq!(
            target.lx.chat.messages[0].attachments[0].kind,
            crate::db::chat::MessageAttachmentKind::Image
        );
        assert_eq!(
            target.lx.chat.messages[0].attachments[1].kind,
            crate::db::chat::MessageAttachmentKind::File
        );
        assert_eq!(target.lx.chat.session_title.as_deref(), Some("Old Title"));
        assert!(!target.lx.chat.reasoning_expanded(1, 0));

        draft(&mut state, pane, "/rename New Cloud Title");
        assert!(super::super::try_send(&mut state, pane));
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(
            target.lx.chat.session_title.as_deref(),
            Some("New Cloud Title")
        );
        assert!(state.effects.iter().any(|effect| matches!(
            effect,
            Effect::RenameChatSession {
                session_id,
                title,
                ..
            } if session_id == "ai-web-ds-uuid-1" && title == "New Cloud Title"
        )));
    }

    #[test]
    fn file_and_image_upload_and_rewind_attachment_removal_work_in_ai_web() {
        let dir =
            std::env::temp_dir().join(format!("lx-ai-web-upload-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let img_path = dir.join("diagram.png");
        let doc_path = dir.join("notes.md");
        // Write 1x1 PNG header and markdown text
        std::fs::write(
            &img_path,
            [
                0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
                b'D', b'R',
            ],
        )
        .expect("write png");
        std::fs::write(&doc_path, "# Notes\nhello\n").expect("write md");

        let (mut state, pane) = ai_web_state();
        let raw_draft = format!(
            "请对比这两份材料 @[refer-image]({}) @[refer-file]({})",
            img_path.display(),
            doc_path.display()
        );
        draft(&mut state, pane, &raw_draft);
        assert!(super::super::try_send(&mut state, pane));

        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.chat.messages[0].text(), raw_draft);
        assert_eq!(target.lx.chat.messages[0].attachments.len(), 2);
        assert!(state.effects.iter().any(|effect| matches!(
            effect,
            Effect::StartAiWebStream {
                prompt,
                files,
                edit_user_index: None,
                ..
            } if prompt == "请对比这两份材料" && files.len() == 2
        )));

        finish(&mut state, pane);
        state.effects.clear();

        // Trigger /rewind: restores full draft with image and file tags
        draft(&mut state, pane, "/rewind");
        assert!(super::super::try_send(&mut state, pane));
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.input, raw_draft);
        assert_eq!(target.lx.ai_web_edit_user_index, Some(0));

        // User removes the image and keeps only the markdown file
        let edited_draft = format!("只看这份文档 @[refer-file]({})", doc_path.display());
        draft(&mut state, pane, &edited_draft);
        assert!(super::super::try_send(&mut state, pane));
        assert!(state.effects.iter().any(|effect| matches!(
            effect,
            Effect::StartAiWebStream {
                prompt,
                files,
                edit_user_index: Some(0),
                ..
            } if prompt == "只看这份文档" && files.len() == 1
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_reference_is_rejected_in_ai_web_and_keeps_draft() {
        let dir =
            std::env::temp_dir().join(format!("lx-ai-web-folder-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let (mut state, pane) = ai_web_state();
        let raw_draft = format!("看看目录 @[refer-folder]({})", dir.display());
        draft(&mut state, pane, &raw_draft);
        assert!(super::super::try_send(&mut state, pane));
        let target = state.pane_anywhere(pane).expect("pane");
        assert!(target.lx.chat.messages.is_empty());
        assert_eq!(target.lx.input, raw_draft, "被拦截时保留草稿");
        assert_eq!(
            target.lx.toast.as_ref().map(|t| t.message.as_str()),
            Some(text::LX_AI_WEB_FOLDER_UNSUPPORTED)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ai_web_mention_panel_cycles_only_files_and_ref_dir() {
        let (mut state, pane) = ai_web_state();
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target
                .lx
                .set_mention_root(Some(std::path::PathBuf::from("/tmp")));
            target.lx.insert_char('@');
            assert_eq!(
                target.lx.mention_tab(),
                crate::app::prompt::MentionTab::Files
            );
            assert!(target.lx.mention_switch_tab(1));
            assert_eq!(
                target.lx.mention_tab(),
                crate::app::prompt::MentionTab::RefDir
            );
            assert!(target.lx.mention_switch_tab(1));
            assert_eq!(
                target.lx.mention_tab(),
                crate::app::prompt::MentionTab::Files,
                "AI Web 下右切直接从 RefDir 回到 Files，跳过 Skills 和 MCP"
            );
            assert!(
                !target
                    .lx
                    .mention_set_tab(crate::app::prompt::MentionTab::Skills)
            );
            assert!(
                !target
                    .lx
                    .mention_set_tab(crate::app::prompt::MentionTab::Mcp)
            );
        }
    }
}
