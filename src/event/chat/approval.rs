//! 沙箱审批的事件层接线：请求入窗格 FIFO、裁决回执、取消唤醒。
//!
//! 与 `ask` 同构：请求不改 DB，回执通道的唯一持有者是 [`ChatStreams`]；
//! UI 裁决只推 `Effect::AnswerApproval`，由效应泵比对 `call_id` 后唤醒 loop。

use tokio::sync::oneshot;

use crate::app::state::{AppState, ApprovalState, RequestKey};
use crate::chat::agent_loop::HostError;
use crate::chat::sandbox::{SandboxApprovalDecision, SandboxApprovalRequest};

use super::{ChatStreams, PendingApproval, active_assistant_id};

/// 已有挂起审批时的稳定失败码。
pub const APPROVAL_BUSY_CODE: &str = "another sandbox approval is already pending";

/// 挂起一次审批；返回是否写入了 UI 状态。
pub(crate) fn request(
    state: &mut AppState,
    streams: &mut ChatStreams,
    key: RequestKey,
    call_id: String,
    request: SandboxApprovalRequest,
    reply: oneshot::Sender<Result<SandboxApprovalDecision, HostError>>,
) -> bool {
    if active_assistant_id(state, &key).is_none()
        || streams
            .cancels
            .get(&key)
            .is_some_and(|cancel| *cancel.borrow())
    {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    }
    let Some(pane) = state.pane_mut_anywhere(key.pane) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    if streams.approvals.contains_key(&key) {
        let _ = reply.send(Err(HostError::Failed {
            code: APPROVAL_BUSY_CODE.to_string(),
        }));
        return false;
    }
    pane.lx.approvals.push_back(ApprovalState::new(
        key.clone(),
        call_id.clone(),
        request.clone(),
    ));
    streams.approvals.insert(
        key,
        PendingApproval {
            call_id,
            request,
            reply,
        },
    );
    true
}

/// 提交一次裁决：`call_id` 与当前挂起项不一致时丢弃（防串台）。
pub(crate) fn answer(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &crate::db::service::Persistence,
    key: &RequestKey,
    call_id: &str,
    decision: SandboxApprovalDecision,
) -> bool {
    let Some(pending) = streams.approvals.get(key) else {
        return false;
    };
    if pending.call_id != call_id {
        return false;
    }
    let Some(pending) = streams.approvals.remove(key) else {
        return false;
    };
    let for_session = decision == SandboxApprovalDecision::ForSession;
    let kind = pending.request.kind;
    let command = pending.request.command.clone();
    let prefix = pending.request.prefix_rule.clone();
    let _ = pending.reply.send(Ok(decision));
    let Some(pane) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    let before = pane.lx.approvals.len();
    pane.lx
        .approvals
        .retain(|approval| approval.call_id != call_id);
    let mut changed = pane.lx.approvals.len() != before;
    let mut new_rule = None;
    if for_session {
        match kind {
            crate::chat::sandbox::SandboxApprovalKind::Retry => {
                if let Some(tokens) = shlex::split(&command)
                    && !pane.lx.sandbox_exact_rules.contains(&tokens)
                {
                    pane.lx.sandbox_exact_rules.push(tokens.clone());
                    new_rule = Some(crate::db::sandbox::SessionSandboxRule {
                        kind: crate::db::sandbox::SandboxRuleKind::Exact,
                        tokens,
                    });
                    changed = true;
                }
            }
            crate::chat::sandbox::SandboxApprovalKind::Rule => {}
            crate::chat::sandbox::SandboxApprovalKind::Escalation => {
                if let Some(prefix) = prefix
                    && !pane.lx.sandbox_prefix_rules.contains(&prefix)
                {
                    pane.lx.sandbox_prefix_rules.push(prefix.clone());
                    new_rule = Some(crate::db::sandbox::SessionSandboxRule {
                        kind: crate::db::sandbox::SandboxRuleKind::Prefix,
                        tokens: prefix,
                    });
                    changed = true;
                }
            }
        }
    }
    if let Some(rule) = new_rule
        && let Some(session_id) = pane.lx.chat.session_id.clone()
        && let Some(workspace) = state.chat_session_key_for_pane(key.pane)
        && let Some(database) = persistence.database()
        && let Err(error) = database.insert_sandbox_rule(&workspace, &session_id, &rule)
    {
        tracing::warn!(%error, "failed to persist sandbox approval rule");
    }
    changed
}

/// 取走挂起审批并回 `Cancelled`，同时清空该 key 的展示项。
pub(crate) fn cancel(state: &mut AppState, streams: &mut ChatStreams, key: &RequestKey) -> bool {
    let pending = streams.approvals.remove(key);
    let had_pending = pending.is_some();
    if let Some(pending) = pending {
        let _ = pending.reply.send(Err(HostError::Cancelled));
    }
    let Some(pane) = state.pane_mut_anywhere(key.pane) else {
        return had_pending;
    };
    let before = pane.lx.approvals.len();
    pane.lx.approvals.retain(|approval| &approval.key != key);
    before != pane.lx.approvals.len() || had_pending
}
