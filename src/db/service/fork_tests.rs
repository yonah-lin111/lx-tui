//! `/fork` 端到端：节点选择 → DB 事务 → 新会话装载与输入框回填。

use super::tests::trusted_service;
use super::*;
use crate::app::effects::Effect;
use crate::db::chat::{ChatPart, ModelSnapshot, TurnCommitted, TurnOutcome, Usage};
use crate::layout::PaneId;

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "example".into(),
        model: "example-chat".into(),
        variant: Some("high".into()),
    }
}

fn seed_turn(
    database: &Database,
    cwd: &std::path::Path,
    existing: Option<&str>,
    text: &str,
) -> TurnCommitted {
    let committed = database
        .begin_turn(cwd, existing, text, &model())
        .expect("begin turn");
    database
        .finish(
            cwd,
            &committed.assistant_message_id,
            &[ChatPart {
                position: 0,
                kind: "text".into(),
                content: format!("re: {text}"),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
            &TurnOutcome::Complete {
                stop_reason: Some("end_turn".into()),
            },
            Usage::default(),
        )
        .expect("finish turn");
    committed
}

fn focused_pane(state: &AppState) -> PaneId {
    state.active_tab().layout.focus()
}

fn load_session(service: &mut Persistence, state: &mut AppState, session_id: &str) {
    let pane = focused_pane(state);
    state.effects.push(Effect::LoadChatSession {
        pane,
        session_id: session_id.to_string(),
    });
    service.pump(state);
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.chat.session_id.clone())
            .as_deref(),
        Some(session_id)
    );
}

fn select_fork_entry(state: &mut AppState, pane: PaneId, index: usize) {
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(crate::app::state::SlashPanel::Fork(panel)) = target.lx.slash.as_mut()
    {
        panel.window.active = index;
    }
}

#[test]
fn fork_selected_message_creates_and_loads_new_session() {
    let (mut service, mut state, cwd, _) = trusted_service("fork-selected");
    let database = service.db.as_ref().expect("db");
    let first = seed_turn(database, &cwd, None, "first question");
    let second = seed_turn(database, &cwd, Some(&first.session_id), "second question");

    load_session(&mut service, &mut state, &first.session_id);
    let pane = focused_pane(&state);
    assert!(crate::app::update::open_fork_panel(&mut state, pane));
    // 面板顺序：Full session、second question、first question。
    select_fork_entry(&mut state, pane, 1);
    assert!(crate::app::update::apply_slash_enter(&mut state, pane));
    service.pump(&mut state);

    let target = state.pane_anywhere(pane).expect("pane");
    let new_session = target.lx.chat.session_id.clone().expect("fork bound");
    assert_ne!(new_session, first.session_id);
    assert_eq!(target.lx.input, "second question");
    assert_eq!(target.lx.chat.messages.len(), 2);
    assert_eq!(target.lx.chat.messages[0].role, "user");
    assert_eq!(target.lx.chat.messages[0].text(), "first question");
    assert_eq!(
        target.lx.chat.session_title.as_deref(),
        Some("first question (fork #1)")
    );
    assert_eq!(
        target.lx.toast.as_ref().map(|toast| toast.message.as_str()),
        Some(text::LX_FORKED)
    );
    assert!(!second.user_message_id.is_empty());
}

#[test]
fn fork_full_session_keeps_input_empty() {
    let (mut service, mut state, cwd, _) = trusted_service("fork-full");
    let database = service.db.as_ref().expect("db");
    let first = seed_turn(database, &cwd, None, "q1");
    seed_turn(database, &cwd, Some(&first.session_id), "q2");
    load_session(&mut service, &mut state, &first.session_id);
    let pane = focused_pane(&state);
    assert!(crate::app::update::open_fork_panel(&mut state, pane));
    select_fork_entry(&mut state, pane, 0);
    assert!(crate::app::update::apply_slash_enter(&mut state, pane));
    service.pump(&mut state);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 4);
    assert!(target.lx.input.is_empty());
}

#[test]
fn fork_failure_keeps_source_session_bound() {
    let (mut service, mut state, cwd, _) = trusted_service("fork-failure");
    let first = {
        let database = service.db.as_ref().expect("db");
        seed_turn(database, &cwd, None, "q1")
    };
    load_session(&mut service, &mut state, &first.session_id);
    let pane = focused_pane(&state);
    assert!(crate::app::update::open_fork_panel(&mut state, pane));
    assert!(crate::app::update::apply_slash_enter(&mut state, pane));
    // 面板确认后、效应执行前删掉源会话，模拟 DB 回执失败。
    service
        .db
        .as_ref()
        .expect("db")
        .delete_chat_session(&cwd, &first.session_id)
        .expect("delete source");
    service.pump(&mut state);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.session_id.as_deref(),
        Some(first.session_id.as_str()),
        "失败不得切走窗格"
    );
    assert_eq!(
        target.lx.toast.as_ref().map(|toast| toast.message.as_str()),
        Some(text::LX_FORK_FAILED)
    );
}

#[test]
fn fork_effect_is_routed_by_chat_service() {
    let (mut service, mut state, cwd, _) = trusted_service("fork-routing");
    let database = service.db.as_ref().expect("db");
    let first = seed_turn(database, &cwd, None, "q1");
    let pane = focused_pane(&state);
    state.effects.push(Effect::ForkChatSession {
        pane,
        workspace: cwd.clone(),
        session_id: first.session_id.clone(),
        before: Some(first.user_message_id.clone()),
    });
    service.pump(&mut state);
    assert!(matches!(
        state.pane_anywhere(pane).unwrap().lx.toast.as_ref(),
        Some(toast) if toast.message == text::LX_FORKED
    ));
}
