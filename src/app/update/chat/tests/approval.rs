//! 沙箱审批命令面板的键位路由：y/a/n、r/a/n、Ctrl+C、Esc、Enter 与草稿不变量。

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
fn arrows_navigate_and_enter_confirms_selected_option() {
    let (mut state, pane, _) = pending(
        SandboxApprovalKind::Escalation,
        Some(vec!["cargo".into(), "publish".into()]),
    );
    crate::app::update::apply_lx_input(&mut state, LxInput::Down);
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.approvals.front())
            .map(|item| item.window.active),
        Some(1)
    );
    assert!(pressed(&mut state, LxInput::Enter));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::ForSession));
}

#[test]
fn escape_denies_and_mouse_click_commits_selected_option() {
    let (mut state, mut pane, _) = pending(SandboxApprovalKind::Escalation, None);
    let draft = state.pane_anywhere(pane).expect("pane").lx.input.clone();
    assert!(crate::app::update::hover_lx_approval(&mut state, pane, 1));
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        draft,
        "草稿不变"
    );
    assert!(crate::app::update::select_lx_approval(&mut state, pane, 1));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::Denied));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .approvals
            .is_empty()
    );

    let (mut state2, pane2, _) = pending(SandboxApprovalKind::Escalation, None);
    pane = pane2;
    assert!(pressed(&mut state2, LxInput::Escape));
    assert_eq!(decision(&state2), Some(SandboxApprovalDecision::Denied));
    assert!(
        state2
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .approvals
            .is_empty()
    );
}

#[test]
fn unrelated_keys_do_not_edit_the_draft_while_pending() {
    let (mut state, pane, _) = pending(SandboxApprovalKind::Escalation, None);
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('x'));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.approvals.len(), 1);
    assert_eq!(target.lx.input, "draft");
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

#[test]
fn ctrl_c_denies_approval_and_stops_matching_main_stream() {
    let (mut state, pane, key) = pending(SandboxApprovalKind::Escalation, None);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: "a1".into(),
        };
    }
    assert!(pressed(&mut state, LxInput::Clear));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::Denied));
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CancelChatStream { key: cancelled } if cancelled == &key)),
        "主轮在途审批被 Ctrl+C 拒绝时应同步取消对应流"
    );
    assert_eq!(state.pane_anywhere(pane).expect("pane").lx.input, "draft");

    // 子 agent 审批（不同 RequestKey）：只拒绝审批，不得误停主轮流。
    let mut state_sub = ready_state("draft");
    let pane_sub = focus_pane(&state_sub);
    let main_key = key_for(pane_sub);
    let sub_key = RequestKey {
        pane: pane_sub,
        session: "sess-subagent".into(),
        generation: 2,
    };
    if let Some(target) = state_sub.pane_mut_anywhere(pane_sub) {
        target.lx.chat.request = RequestState::Streaming {
            key: main_key,
            assistant_id: "a1".into(),
        };
        target.lx.approvals.push_back(ApprovalState::new(
            sub_key,
            "call-sub".into(),
            approval(SandboxApprovalKind::Escalation, None),
        ));
    }
    assert!(pressed(&mut state_sub, LxInput::Clear));
    assert_eq!(decision(&state_sub), Some(SandboxApprovalDecision::Denied));
    assert!(
        !state_sub
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CancelChatStream { .. })),
        "子 agent 审批被 Ctrl+C 拒绝时不得取消主轮"
    );
}

#[test]
fn approval_takes_keyboard_priority_over_pending_ask() {
    use crate::app::state::AskState;
    use crate::chat::tools::ask::{AskOption, AskQuestion};

    let (mut state, pane, key) = pending(SandboxApprovalKind::Escalation, None);
    let question = AskQuestion {
        id: "q1".into(),
        header: "Choice".into(),
        question: "Pick one".into(),
        multi_select: false,
        options: vec![AskOption {
            label: "Opt A".into(),
            description: String::new(),
            preview: None,
        }],
    };
    state.pane_mut_anywhere(pane).expect("pane").lx.ask =
        Some(AskState::new(key, "call-ask".into(), vec![question]));

    assert!(pressed(&mut state, LxInput::Enter));
    assert_eq!(decision(&state), Some(SandboxApprovalDecision::Once));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.approvals.is_empty());
    assert!(target.lx.ask.is_some(), "审批未消耗 ask 面板");
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::AnswerAsk { .. })),
        "审批按键不得透传提交 ask"
    );
}

#[test]
fn pending_approval_marks_lx_agent_state_as_blocked() {
    let (mut state, pane, _) = pending(SandboxApprovalKind::Escalation, None);
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.agent_state(),
        crate::detect::AgentState::Blocked
    );
    assert!(pressed(&mut state, LxInput::Char('y')));
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.agent_state(),
        crate::detect::AgentState::Idle
    );
}
