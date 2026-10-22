//! 续轮 guard：必须在 BeginChatTurn 事务之前取得；未完成撤销阻止发送。

use super::*;

/// 构造一个已绑定会话、刚按下 Enter 的窗格（BeginChatTurn 效应尚未执行）。
fn sending_state(fixture: &Fixture, session: &str) -> (AppState, crate::layout::PaneId, u64) {
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(fixture.workspace().to_path_buf());
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.entry = crate::app::state::LxEntry::Native;
        target.lx.input = "next".into();
        target.lx.chat.session_id = Some(session.to_string());
    }
    assert!(update::try_send(&mut state, pane), "发送受理");
    let generation = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        other => panic!("expected Committing, got {other:?}"),
    };
    (state, pane, generation)
}

#[test]
fn guard_is_taken_before_begin_turn() {
    let fixture = Fixture::new("guard-before-begin");
    let database = fixture.database();
    let committed = database
        .begin_turn(
            fixture.workspace(),
            None,
            "first",
            &crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    let persistence = Persistence::with_database(database);
    let (mut state, pane, generation) = sending_state(&fixture, &committed.session_id);
    let mut streams = crate::event::chat::ChatStreams::default();
    let _ = crate::event::chat::guard_begin_turns(&mut state, &mut streams, &persistence);
    assert!(
        streams.pending_guards.contains_key(&(pane, generation)),
        "guard 必须在 DB 事务之前取得"
    );
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BeginChatTurn { .. })),
        "取得 guard 不影响正常发送"
    );
}

#[test]
fn busy_session_is_refused_before_begin_turn() {
    let fixture = Fixture::new("guard-busy");
    let database = fixture.database();
    let committed = database
        .begin_turn(
            fixture.workspace(),
            None,
            "first",
            &crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    let persistence = Persistence::with_database(database);
    let (mut state, pane, _) = sending_state(&fixture, &committed.session_id);
    let mut streams = crate::event::chat::ChatStreams::default();
    let held = crate::platform::try_lock_session(&committed.session_id)
        .expect("lock")
        .expect("free");
    assert!(crate::event::chat::guard_begin_turns(
        &mut state,
        &mut streams,
        &persistence
    ));
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BeginChatTurn { .. })),
        "拿不到 guard 时不能提交 DB 轮次"
    );
    assert!(matches!(
        state.pane_anywhere(pane).expect("pane").lx.chat.request,
        RequestState::Idle
    ));
    assert!(state.toast.is_some(), "必须给用户可见原因");
    drop(held);
}

#[test]
fn unfinished_rewind_blocks_sending() {
    let fixture = Fixture::new("guard-unfinished");
    let database = fixture.database();
    let committed = database
        .begin_turn(
            fixture.workspace(),
            None,
            "first",
            &crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("turn");
    database
        .finish(
            fixture.workspace(),
            &committed.assistant_message_id,
            &[],
            &crate::db::chat::TurnOutcome::Interrupted,
            crate::db::chat::Usage::default(),
        )
        .expect("finish");
    let persistence = Persistence::with_database(database);
    persistence
        .begin_rewind_operation(
            fixture.workspace(),
            "op-block",
            &committed.session_id,
            &committed.user_message_id,
            committed.revision,
            "revert_only",
            "",
        )
        .expect("operation");
    persistence
        .set_rewind_operation_phase("op-block", "restoring", None)
        .expect("phase");
    let (mut state, pane, _) = sending_state(&fixture, &committed.session_id);
    let mut streams = crate::event::chat::ChatStreams::default();
    assert!(crate::event::chat::guard_begin_turns(
        &mut state,
        &mut streams,
        &persistence
    ));
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BeginChatTurn { .. })),
        "未完成撤销必须先处理"
    );
    assert!(matches!(
        state.pane_anywhere(pane).expect("pane").lx.chat.request,
        RequestState::Idle
    ));
    assert!(state.toast.is_some());
}
