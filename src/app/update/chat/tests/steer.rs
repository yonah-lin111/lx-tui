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
fn escape_recalls_pending_steers_sequentially_in_lifo_without_merging() {
    let (mut state, pane, _) = streaming_state("s");
    {
        let target = state.pane_mut_anywhere(pane).unwrap();
        target.lx.chat.pending_steers.push(pending("first"));
        target.lx.chat.pending_steers.push(pending("second"));
        target.lx.chat.pending_steers.push(pending("third"));
    }

    // 第 1 次 Esc：回退最新一条 "third"，直接覆盖回显输入框，不触发停流待命。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    {
        let target = state.pane_anywhere(pane).unwrap();
        assert_eq!(target.lx.chat.pending_steers.len(), 2);
        assert_eq!(target.lx.input, "third");
        assert!(target.lx.chat.stop_armed_at.is_none());
        assert!(!target.lx.chat.main_stopped);
    }

    // 第 2 次 Esc：回退 "second"，直接替换为 "second"，绝不与 "third" 合并。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    {
        let target = state.pane_anywhere(pane).unwrap();
        assert_eq!(target.lx.chat.pending_steers.len(), 1);
        assert_eq!(target.lx.input, "second");
        assert!(target.lx.chat.stop_armed_at.is_none());
    }

    // 第 3 次 Esc：回退 "first"，队列排空，输入框回显 "first"。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    {
        let target = state.pane_anywhere(pane).unwrap();
        assert!(target.lx.chat.pending_steers.is_empty());
        assert_eq!(target.lx.input, "first");
        assert!(target.lx.chat.stop_armed_at.is_none());
    }

    // 第 4 次 Esc（队列已空）：进入原有的双击 Esc 停止待命。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .stop_armed_at
            .is_some()
    );

    // 第 5 次 Esc：真正停止当前流。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(state.pane_anywhere(pane).unwrap().lx.chat.main_stopped);
    // 撤回尚未落库的 steer 不应污染持久化 prompt_history。
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::RecordPromptHistory { .. }
        )),
        "未发送落库的撤回草稿不得派发 RecordPromptHistory"
    );
}

#[test]
fn backspace_on_empty_input_does_not_recall_pending_steer() {
    let (mut state, pane, _) = streaming_state("s");
    let target = state.pane_mut_anywhere(pane).unwrap();
    target.lx.chat.pending_steers.push(pending("keep"));
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Backspace);
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.pending_steers.len(), 1);
    assert!(target.lx.input.is_empty());
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
fn cancelled_finish_restores_latest_pending_without_merging() {
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
    let target = state.pane_mut_anywhere(pane).unwrap();
    assert!(target.lx.chat.pending_steers.is_empty());
    assert_eq!(target.lx.input, "second");
    assert!(matches!(target.lx.chat.request, RequestState::Idle));
    // 前序条目已压入编辑器 undo 栈，不丢失且不合并。
    target.lx.undo();
    assert_eq!(target.lx.input, "first");
    target.lx.undo();
    assert_eq!(target.lx.input, "draft");
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
