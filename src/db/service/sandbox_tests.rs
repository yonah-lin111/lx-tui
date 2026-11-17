//! 沙箱审批与权限切换效应的持久化泵回归测试。

use super::*;
use crate::app::effects::Effect;
use crate::app::state::RequestKey;
use crate::chat::sandbox::SandboxApprovalDecision;
use crate::config::sandbox::SandboxMode;
use crate::db::chat::{ModelSnapshot, PermissionSwitchCommit, TurnOutcome, Usage};
use tests::trusted_service;

#[test]
fn persistence_pump_defers_answer_approval_to_chat_streams() {
    let (mut service, mut state, _cwd, _) = trusted_service("defer-approval");
    let pane = state.active_tab().layout.focus();
    let effect = Effect::AnswerApproval {
        key: RequestKey {
            pane,
            session: "sess-1".into(),
            generation: 1,
        },
        call_id: "call-1".into(),
        decision: SandboxApprovalDecision::Once,
    };
    state.effects.push(effect.clone());

    service.pump(&mut state);

    assert_eq!(
        state.effects,
        vec![effect],
        "AnswerApproval 必须留给事件层 pump_chat_streams，绝不能被持久化泵吞掉"
    );
}

#[test]
fn persistence_pump_commits_sandbox_mode_and_permission_switch() {
    let (mut service, mut state, cwd, _) = trusted_service("persist-sandbox-mode");
    let pane = state.active_tab().layout.focus();
    let db = service.db.as_ref().expect("db");
    let model = ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let turn = db.begin_turn(&cwd, None, "hello", &model).expect("turn");
    db.finish(
        &cwd,
        &turn.assistant_message_id,
        &[],
        &TurnOutcome::Complete {
            stop_reason: Some("stop".into()),
        },
        Usage::default(),
    )
    .expect("finish");
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.session_id = Some(turn.session_id.clone());
    }

    state.effects.push(Effect::PersistSandboxMode {
        workspace: cwd.clone(),
        session_id: turn.session_id.clone(),
        mode: Some(SandboxMode::ReadOnly),
        switch_commit: Some(PermissionSwitchCommit {
            session_id: turn.session_id.clone(),
            from_mode: "workspace-write".into(),
            to_mode: "read-only".into(),
            injected_prompt: "Switched to Read-only".into(),
        }),
    });

    assert!(service.pump(&mut state));
    assert!(state.effects.is_empty());

    let db = service.db.as_ref().expect("db");
    assert_eq!(
        db.session_sandbox_mode(&cwd, &turn.session_id)
            .expect("mode")
            .as_deref(),
        Some("read-only")
    );
    assert_eq!(
        db.permission_switches(&cwd, &turn.session_id)
            .expect("switches")
            .len(),
        1
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .permission_switch
            .switches
            .len(),
        1
    );
}
