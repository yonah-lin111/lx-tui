//! 沙箱审批命令面板的按键与鼠标路由：只做状态迁移并登记效应，不碰 IO。
//!
//! 交互对齐 `/model` / `/rewind`：
//! - `↑`/`↓` 循环高亮选项，`Enter`/`Tab`/鼠标点击确认当前选中裁决；
//! - `Esc` 一律等价拒绝；
//! - 同步保留 `y`/`r`/`a`/`n` 单键快捷裁决；审批打开期间拦截普通字符输入。

use crate::app::actions::LxInput;
use crate::app::effects::Effect;
use crate::app::state::{AppState, RequestKey};
use crate::chat::sandbox::{SandboxApprovalDecision, SandboxApprovalKind};
use crate::tiling::PaneId;

/// 审批命令面板接管按键；返回是否产生状态变化。
pub fn apply_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(front) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.approvals.front())
        .cloned()
    else {
        return false;
    };
    match command {
        LxInput::Up => state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.approvals.front_mut())
            .is_some_and(|item| item.move_active(-1)),
        LxInput::Down => state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.approvals.front_mut())
            .is_some_and(|item| item.move_active(1)),
        LxInput::Enter | LxInput::Tab => commit_decision(state, pane, front.active_decision()),
        LxInput::Escape | LxInput::Char('n') => {
            commit_decision(state, pane, SandboxApprovalDecision::Denied)
        }
        LxInput::Char('y')
            if matches!(
                front.request.kind,
                SandboxApprovalKind::Escalation | SandboxApprovalKind::Rule
            ) =>
        {
            commit_decision(state, pane, SandboxApprovalDecision::Once)
        }
        LxInput::Char('r') if front.request.kind == SandboxApprovalKind::Retry => {
            commit_decision(state, pane, SandboxApprovalDecision::Once)
        }
        LxInput::Char('a') if front.request.can_approve_for_session() => {
            commit_decision(state, pane, SandboxApprovalDecision::ForSession)
        }
        _ => false,
    }
}

/// 鼠标悬停审批命令面板条目：只移动高亮、不改变窗口滚动。
pub fn hover_approval(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.approvals.front_mut())
        .is_some_and(|item| item.set_active(index))
}

/// 鼠标点击审批命令面板条目：直接采纳该条目对应裁决。
pub fn select_approval(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let Some(decision) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.approvals.front())
        .and_then(|front| front.decision_at(index))
    else {
        return false;
    };
    commit_decision(state, pane, decision)
}

fn commit_decision(state: &mut AppState, pane: PaneId, decision: SandboxApprovalDecision) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(front) = target.lx.approvals.pop_front() else {
        return false;
    };
    let (RequestKey { .. }, call_id) = (front.key.clone(), front.call_id.clone());
    state.effects.push(Effect::AnswerApproval {
        key: front.key,
        call_id,
        decision,
    });
    true
}
