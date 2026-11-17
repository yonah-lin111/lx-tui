//! 沙箱审批 strip 的按键路由：只做状态迁移并登记效应，不碰 IO。
//!
//! 键位：
//! - 显式申请：`y` 批准一次、`a` 本会话批准（有 `prefix_rule` 时）、`n` 拒绝；
//! - on-failure 重试：`r` 重试一次、`a` 本会话同命令自动重试、`n` 保持拒绝；
//! - `Esc` 一律拒绝；`Enter` 被吞掉（等待期间不提交输入）。

use crate::app::actions::LxInput;
use crate::app::effects::Effect;
use crate::app::state::{AppState, RequestKey};
use crate::chat::sandbox::{SandboxApprovalDecision, SandboxApprovalKind};
use crate::tiling::PaneId;

/// 审批 strip 接管按键；返回是否产生状态变化。
pub fn apply_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(front) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.approvals.front())
    else {
        return false;
    };
    let kind = front.request.kind;
    let decision = match command {
        LxInput::Escape => Some(SandboxApprovalDecision::Denied),
        LxInput::Char('n') => Some(SandboxApprovalDecision::Denied),
        LxInput::Char('y')
            if matches!(
                kind,
                SandboxApprovalKind::Escalation | SandboxApprovalKind::Rule
            ) =>
        {
            Some(SandboxApprovalDecision::Once)
        }
        LxInput::Char('r') if kind == SandboxApprovalKind::Retry => {
            Some(SandboxApprovalDecision::Once)
        }
        LxInput::Char('a') if front.request.can_approve_for_session() => {
            Some(SandboxApprovalDecision::ForSession)
        }
        // 等待审批期间 Enter 不得提交输入。
        LxInput::Enter => return true,
        _ => return false,
    };
    let Some(decision) = decision else {
        return false;
    };
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
