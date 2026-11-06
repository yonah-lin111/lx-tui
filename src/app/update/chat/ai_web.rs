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

/// 会话 ID 是否属于 AI Web 内存会话。
pub fn is_ai_web_session(session_id: &str) -> bool {
    session_id.starts_with(AI_WEB_SESSION_PREFIX)
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
/// 只接受单行 `/new`（含别名 `/clear`）；其它斜杠命令按不支持提示，未知命令按未知提示。
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
                super::session::new_session(state, pane);
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
    state.prompt_history.record(&draft);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let site = target.lx.ai_web_site;
    target.lx.clear();
    target.lx.chat.stop_armed_at = None;
    target.lx.chat.main_stopped = false;
    let session_id = match target.lx.chat.session_id.as_deref() {
        Some(session) if is_ai_web_session(session) => session.to_string(),
        _ => {
            let id = format!("{AI_WEB_SESSION_PREFIX}{}-{generation}", pane.raw());
            target.lx.chat.session_id = Some(id.clone());
            id
        }
    };
    let base_seq = target.lx.chat.messages.last().map_or(0, |m| m.sequence);
    let turn_index = target
        .lx
        .chat
        .messages
        .last()
        .map_or(0, |m| m.turn_index.saturating_add(1));
    let user_id = format!("{session_id}-u-{generation}");
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
                content: draft.clone(),
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
        attachments: Vec::new(),
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
        prompt: draft,
    });
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
}
