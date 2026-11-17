//! 沙箱审批 strip 的键位路由：y/a/n、r/a/n、Esc、Enter 与草稿不变量。

use super::*;
use crate::app::actions::LxInput;
use crate::app::effects::Effect;
use crate::app::state::ApprovalState;
use crate::chat::sandbox::{SandboxApprovalDecision, SandboxApprovalKind, SandboxApprovalRequest};

fn key_for(pane: PaneId) -> RequestKey {
    RequestKey {
        pane,
        session: "sess-approval".into(),
        generation: 1,
    }
}

fn approval(kind: SandboxApprovalKind, prefix_rule: Option<Vec<String>>) -> SandboxApprovalRequest {
    SandboxApprovalRequest {
        kind,
        command: "touch ~/x".into(),
        cwd: "/ws".into(),
        justification: "needs home write".into(),
        prefix_rule,
        additional: None,
        denial_excerpt: (kind == SandboxApprovalKind::Retry)
            .then(|| "Operation not permitted".to_string()),
        source: None,
    }
}

fn pending(
    kind: SandboxApprovalKind,
    prefix_rule: Option<Vec<String>>,
) -> (AppState, PaneId, RequestKey) {
    let mut state = ready_state("draft");
    let pane = focus_pane(&state);
    let key = key_for(pane);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.approvals.push_back(ApprovalState::new(
        key.clone(),
        "call-1".into(),
        approval(kind, prefix_rule),
    ));
    (state, pane, key)
}

fn pressed(state: &mut AppState, command: LxInput) -> bool {
    let before = state.pane_anywhere(focus_pane(state)).and_then(|pane| {
        pane.lx
            .approvals
            .front()
            .map(|approval| approval.call_id.clone())
    });
    crate::app::update::apply_lx_input(state, command);
    let after = state.pane_anywhere(focus_pane(state)).and_then(|pane| {
        pane.lx
            .approvals
            .front()
            .map(|approval| approval.call_id.clone())
    });
    before != after
}

fn decision(state: &AppState) -> Option<SandboxApprovalDecision> {
    state.effects.iter().find_map(|effect| match effect {
        Effect::AnswerApproval { decision, .. } => Some(*decision),
        _ => None,
    })
}

#[test]
fn y_approves_once_and_clears_the_strip() {
    let (mut state, pane, _) = pending(SandboxApprovalKind::Escalation, None);
    assert!(pressed(&mut state, LxInput::Char('y')));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::Once));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .approvals
            .is_empty()
    );
}

#[test]
fn a_is_only_available_with_a_prefix_rule() {
    let (mut state, pane, _) = pending(SandboxApprovalKind::Escalation, None);
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('a'));
    assert!(decision(&state).is_none());
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.approvals.len(),
        1
    );

    let (mut state, _, _) = pending(
        SandboxApprovalKind::Escalation,
        Some(vec!["cargo".into(), "publish".into()]),
    );
    assert!(pressed(&mut state, LxInput::Char('a')));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::ForSession));
}

#[test]
fn retry_uses_r_and_always_can_approve_for_session() {
    let (mut state, _, _) = pending(SandboxApprovalKind::Retry, None);
    assert!(pressed(&mut state, LxInput::Char('r')));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::Once));

    let (mut state, _, _) = pending(SandboxApprovalKind::Retry, None);
    assert!(pressed(&mut state, LxInput::Char('a')));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::ForSession));
}

#[test]
fn escape_denies_and_enter_is_swallowed() {
    let (mut state, mut pane, _) = pending(SandboxApprovalKind::Escalation, None);
    let draft = state.pane_anywhere(pane).expect("pane").lx.input.clone();
    crate::app::update::apply_lx_input(&mut state, LxInput::Enter);
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.approvals.len(),
        1,
        "Enter 不得提交或关闭审批"
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        draft,
        "草稿不变"
    );
    pane = focus_pane(&state);
    assert!(pressed(&mut state, LxInput::Escape));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::Denied));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .approvals
            .is_empty()
    );
}

#[test]
fn unrelated_keys_still_edit_the_draft_while_pending() {
    let (mut state, pane, _) = pending(SandboxApprovalKind::Escalation, None);
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('x'));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.approvals.len(), 1);
    assert!(decision(&state).is_none());
}

#[test]
fn rule_prompt_approves_once_with_y() {
    let (mut state, pane, _) = pending(SandboxApprovalKind::Rule, Some(vec!["make".into()]));
    assert!(pressed(&mut state, LxInput::Char('y')));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::Once));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .approvals
            .is_empty()
    );
}
