//! 流中 steer 的 app 状态测试：提交、撤回、终态提升/退回与会话切换丢弃。

use super::*;
use crate::app::state::lx::chat::PendingSteer;
use crate::db::chat::ModelSnapshot;

fn pending(text: &str) -> PendingSteer {
    PendingSteer {
        text: text.into(),
        attachments: Vec::new(),
        model_images: Vec::new(),
        model: ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: Some("high".into()),
        },
    }
}

fn streaming_state(session: &str) -> (AppState, PaneId, RequestKey) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    let key = RequestKey {
        pane,
        session: session.into(),
        generation: 7,
    };
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.session_id = Some(session.into());
    target.lx.chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id: "a".into(),
    };
    (state, pane, key)
}

#[test]
fn enter_while_streaming_queues_pending_and_clears_input() {
    let (mut state, pane, _) = streaming_state("s");
    state.pane_mut_anywhere(pane).unwrap().lx.input = "调整一下".into();
    assert!(try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.pending_steers.len(), 1);
    assert_eq!(target.lx.chat.pending_steers[0].text, "调整一下");
    assert!(target.lx.input.is_empty());
    assert!(matches!(
        target.lx.chat.request,
        RequestState::Streaming { .. }
    ));
}

#[test]
fn enter_while_committing_does_not_queue_steer() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.request = RequestState::Committing {
            generation: 1,
            draft: "old".into(),
        };
    }
    assert!(!try_send(&mut state, pane));
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .pending_steers
            .is_empty()
    );
}

#[test]
fn slash_draft_is_not_steered() {
    let (mut state, pane, _) = streaming_state("s");
    state.pane_mut_anywhere(pane).unwrap().lx.input = "/compact".into();
    assert!(try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.chat.pending_steers.is_empty());
    assert_eq!(target.lx.input, "/compact");
}

#[test]
fn recall_prepends_pending_to_existing_draft() {
    let (mut state, pane, _) = streaming_state("s");
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.pending_steers.push(pending("first"));
    target.lx.chat.pending_steers.push(pending("second"));
    target.lx.input = "draft".into();
    assert!(super::super::steer::recall(&mut state, pane, 1));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.pending_steers.len(), 1);
    assert_eq!(target.lx.input, "second\n\ndraft");
}

#[test]
fn recall_last_on_empty_input() {
    let (mut state, pane, _) = streaming_state("s");
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.pending_steers.push(pending("first"));
    target.lx.chat.pending_steers.push(pending("second"));
    assert!(super::super::steer::recall_last(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.pending_steers.len(), 1);
    assert_eq!(target.lx.input, "second");
}

#[test]
fn complete_finish_promotes_pending_to_next_turn() {
    let (mut state, pane, key) = streaming_state("s");
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.pending_steers.push(pending("next"));
    target.lx.chat.request = RequestState::Finishing {
        key: key.clone(),
        reason: StopReason::Complete,
    };
    assert!(on_finish_ack(&mut state, &key, true));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(matches!(
        target.lx.chat.request,
        RequestState::Committing { .. }
    ));
    assert_eq!(target.lx.chat.promoting_steers, 1);
    assert_eq!(target.lx.chat.pending_steers.len(), 1);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::BeginChatTurn { draft, .. } if draft == "next"
    )));
}

#[test]
fn cancelled_finish_restores_pending_to_input() {
    let (mut state, pane, key) = streaming_state("s");
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.pending_steers.push(pending("first"));
    target.lx.chat.pending_steers.push(pending("second"));
    target.lx.input = "draft".into();
    target.lx.chat.request = RequestState::Finishing {
        key: key.clone(),
        reason: StopReason::Cancelled,
    };
    assert!(on_finish_ack(&mut state, &key, true));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.chat.pending_steers.is_empty());
    assert_eq!(target.lx.input, "first\n\nsecond\n\ndraft");
    assert!(matches!(target.lx.chat.request, RequestState::Idle));
}

#[test]
fn promotion_commit_consumes_pending_without_clearing_new_draft() {
    let (mut state, pane, key) = streaming_state("s");
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.pending_steers.push(pending("next"));
    target.lx.chat.request = RequestState::Finishing {
        key: key.clone(),
        reason: StopReason::Complete,
    };
    assert!(on_finish_ack(&mut state, &key, true));
    let generation = match state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => generation,
        ref other => panic!("expected committing, got {other:?}"),
    };
    state.pane_mut_anywhere(pane).unwrap().lx.input = "fresh draft".into();
    assert!(on_turn_committed(
        &mut state,
        pane,
        generation,
        &committed("s")
    ));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.chat.pending_steers.is_empty());
    assert_eq!(target.lx.chat.promoting_steers, 0);
    assert_eq!(target.lx.input, "fresh draft");
}

#[test]
fn empty_input_backspace_recalls_last_pending() {
    let (mut state, pane, _) = streaming_state("s");
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.pending_steers.push(pending("recall me"));
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Backspace);
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.chat.pending_steers.is_empty());
    assert_eq!(target.lx.input, "recall me");
}

#[test]
fn backspace_with_draft_still_deletes_text() {
    let (mut state, pane, _) = streaming_state("s");
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.pending_steers.push(pending("keep"));
    target.lx.input = "abc".into();
    target.lx.cursor = 3;
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Backspace);
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.pending_steers.len(), 1);
    assert_eq!(target.lx.input, "ab");
}

#[test]
fn session_change_discards_pending() {
    let (mut state, pane, _) = streaming_state("s");
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .chat
        .pending_steers
        .push(pending("x"));
    assert_eq!(
        super::super::steer::discard_for_session_change(&mut state, pane),
        1
    );
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .pending_steers
            .is_empty()
    );
}
