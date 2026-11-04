//! `/fork` 应用层测试：节点顺序、过滤、忙拦截与效应边界。

use super::*;
use crate::app::state::{
    AppState, AssistantStepView, ChatMessageView, ChatPartView, ForkPanel, SlashPanel,
};
use crate::app::update::chat::{apply_slash_enter, open_fork_panel};

fn user_message(id: &str, text: &str) -> ChatMessageView {
    ChatMessageView {
        id: id.into(),
        sequence: 0,
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: text.into(),
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
    }
}

fn fork_panel(state: &AppState, pane: PaneId) -> &ForkPanel {
    match state.pane_anywhere(pane).and_then(|p| p.lx.slash.as_ref()) {
        Some(SlashPanel::Fork(panel)) => panel,
        other => panic!("fork panel missing: {other:?}"),
    }
}

#[test]
fn fork_panel_lists_full_session_then_user_messages_newest_first() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.session_id = Some("sess".into());
        target.lx.chat.messages = vec![user_message("u1", "first"), user_message("u2", "second")];
        target.lx.chat.set_message_time("u1", 1_000);
        target.lx.chat.set_message_time("u2", 2_000);
    }
    assert!(open_fork_panel(&mut state, pane));
    let panel = fork_panel(&state, pane);
    assert!(panel.entries[0].is_full_session());
    assert_eq!(panel.entries[1].message_id.as_deref(), Some("u2"));
    assert_eq!(panel.entries[2].message_id.as_deref(), Some("u1"));
    assert_eq!(panel.entries[1].label, "second");
    assert_eq!(panel.entries[1].created_at_ms, Some(2_000));
    assert!(panel.now_ms > 0);
}

#[test]
fn fork_panel_filter_keeps_full_session() {
    let mut panel = ForkPanel {
        entries: vec![
            crate::app::state::ForkEntry {
                message_id: None,
                label: String::new(),
                created_at_ms: None,
            },
            crate::app::state::ForkEntry {
                message_id: Some("u2".into()),
                label: "deploy the release".into(),
                created_at_ms: Some(2_000),
            },
            crate::app::state::ForkEntry {
                message_id: Some("u1".into()),
                label: "write tests".into(),
                created_at_ms: Some(1_000),
            },
        ],
        window: Default::default(),
        filter: String::new(),
        filtered: vec![0, 1, 2],
        now_ms: 2_000 + 5 * 60_000,
    };
    panel.refilter("deploy");
    assert_eq!(panel.filtered, vec![0, 1], "Full session 恒在首位");
    assert_eq!(
        panel.active_entry().and_then(|e| e.message_id.clone()),
        None
    );
    assert_eq!(panel.entries[1].time_label(panel.now_ms), "5m ago");
}

#[test]
fn fork_busy_blocks_panel_and_shows_hint() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.session_id = Some("sess".into());
        target.lx.chat.messages = vec![user_message("u1", "first")];
        target.lx.chat.request = RequestState::Committing {
            generation: 1,
            draft: "in flight".into(),
        };
    }
    assert!(open_fork_panel(&mut state, pane));
    assert!(
        state.pane_anywhere(pane).unwrap().lx.slash.is_none(),
        "忙时不得打开面板"
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.toast.as_ref())
            .map(|toast| toast.message.as_str()),
        Some(text::LX_FORK_BUSY)
    );
}

#[test]
fn fork_without_session_or_messages_blocks() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_fork_panel(&mut state, pane));
    assert!(state.pane_anywhere(pane).unwrap().lx.slash.is_none());
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.toast.as_ref())
            .map(|toast| toast.message.as_str()),
        Some(text::LX_FORK_NOTHING)
    );

    state.pane_mut_anywhere(pane).unwrap().lx.toast = None;
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.session_id = Some("sess".into());
    target.lx.chat.messages = vec![user_message("a1", "assistant only")];
    target.lx.chat.messages[0].role = "assistant".into();
    assert!(open_fork_panel(&mut state, pane));
    assert!(state.pane_anywhere(pane).unwrap().lx.slash.is_none());
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.toast.as_ref())
            .map(|toast| toast.message.as_str()),
        Some(text::LX_FORK_NOTHING)
    );
}

#[test]
fn fork_enter_pushes_effect_with_boundary_and_clears_input() {
    let mut state = ready_state("/fork");
    let pane = focus_pane(&state);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.session_id = Some("sess".into());
        target.lx.chat.messages = vec![user_message("u1", "first"), user_message("u2", "second")];
    }
    assert!(open_fork_panel(&mut state, pane));
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Fork(panel)) = target.lx.slash.as_mut()
    {
        panel.window.active = 1;
    }
    assert!(apply_slash_enter(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.input.is_empty());
    assert!(target.lx.slash.is_none());
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::ForkChatSession {
            before: Some(id),
            session_id,
            ..
        } if id == "u2" && session_id == "sess"
    )));
}

#[test]
fn fork_full_session_pushes_none_boundary() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.session_id = Some("sess".into());
        target.lx.chat.messages = vec![user_message("u1", "first")];
    }
    assert!(open_fork_panel(&mut state, pane));
    assert!(apply_slash_enter(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::ForkChatSession { before: None, .. }
    )));
}
