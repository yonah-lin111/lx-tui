//! 沙箱审批的事件层接线：请求入窗格 FIFO、裁决回执、取消唤醒。
//!
//! 与 `ask` 同构：请求不改 DB，回执通道的唯一持有者是 [`ChatStreams`]；
//! UI 裁决只推 `Effect::AnswerApproval`，由效应泵比对 `call_id` 后唤醒 loop。

use tokio::sync::oneshot;

use crate::app::state::{AppState, ApprovalState, RequestKey};
use crate::chat::agent_loop::HostError;
use crate::chat::sandbox::{SandboxApprovalDecision, SandboxApprovalRequest, command_preview};

use super::{ChatStreams, PendingApproval, active_assistant_id};

/// 已有挂起审批时的稳定失败码。
pub const APPROVAL_BUSY_CODE: &str = "another sandbox approval is already pending";

const fn decision_str(decision: SandboxApprovalDecision) -> &'static str {
    match decision {
        SandboxApprovalDecision::Once => "once",
        SandboxApprovalDecision::ForSession => "for_session",
        SandboxApprovalDecision::Denied => "denied",
    }
}

fn format_subagent_source(handle: &super::subagent::SubagentHandle) -> String {
    let role = handle.role.as_str();
    match handle
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(nickname) => format!("{nickname} [{role}]"),
        None => role.to_string(),
    }
}

/// 挂起一次审批；返回是否写入了 UI 状态。
pub(crate) fn request(
    state: &mut AppState,
    streams: &mut ChatStreams,
    key: RequestKey,
    call_id: String,
    mut request: SandboxApprovalRequest,
    reply: oneshot::Sender<Result<SandboxApprovalDecision, HostError>>,
) -> bool {
    let main_live = active_assistant_id(state, &key).is_some()
        && !streams
            .cancels
            .get(&key)
            .is_some_and(|cancel| *cancel.borrow());
    let subagent_meta = (!main_live)
        .then(|| {
            let pane_session = state
                .pane_anywhere(key.pane)
                .and_then(|pane| pane.lx.chat.session_id.as_deref());
            streams
                .subagents
                .get(&key.session)
                .filter(|handle| {
                    handle.parent.pane == key.pane
                        && handle.parent.generation == key.generation
                        && handle.status == super::subagent::STATUS_RUNNING
                        && !*handle.cancel.borrow()
                        && pane_session == Some(handle.root_session.as_str())
                })
                .map(|handle| {
                    (
                        format_subagent_source(handle),
                        handle.agent_id.clone(),
                        handle.root_session.clone(),
                    )
                })
        })
        .flatten();
    if !main_live && subagent_meta.is_none() {
        tracing::debug!(
            target: "lx::agent::approval",
            pane = key.pane.raw(),
            session = %key.session,
            generation = key.generation,
            call_id = %call_id,
            kind = request.kind.as_str(),
            "sandbox approval dropped (inactive target)"
        );
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    }
    if request.source.is_none() {
        request.source = subagent_meta.as_ref().map(|(source, _, _)| source.clone());
    }
    let Some(pane) = state.pane_mut_anywhere(key.pane) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    if streams.approvals.contains_key(&key) {
        tracing::warn!(
            target: "lx::agent::approval",
            pane = key.pane.raw(),
            session = %key.session,
            call_id = %call_id,
            kind = request.kind.as_str(),
            source = request.source.as_deref().unwrap_or("main"),
            "sandbox approval rejected (another approval pending)"
        );
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
    let queue_len = pane.lx.approvals.len();
    let kind = request.kind;
    let source_label = request.source.clone().unwrap_or_else(|| "main".to_string());
    let has_prefix_rule = request.prefix_rule.is_some();
    let has_additional = request.additional.is_some();
    let preview = command_preview(&request.command);
    let agent_id = subagent_meta
        .as_ref()
        .map(|(_, id, _)| id.as_str())
        .unwrap_or("-");
    let parent_session = subagent_meta
        .as_ref()
        .map(|(_, _, root)| root.as_str())
        .unwrap_or("-");
    tracing::info!(
        target: "lx::agent::approval",
        pane = key.pane.raw(),
        session = %key.session,
        parent_session,
        agent_id,
        call_id = %call_id,
        kind = kind.as_str(),
        source = %source_label,
        has_prefix_rule,
        has_additional,
        queue_len,
        command = %preview,
        "sandbox approval requested"
    );
    streams.approvals.insert(
        key,
        PendingApproval {
            call_id,
            request,
            reply,
            requested_at: std::time::Instant::now(),
        },
    );
    true
}

struct AnswerTarget<'a> {
    key: &'a RequestKey,
    call_id: &'a str,
    decision: SandboxApprovalDecision,
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
    #[cfg(not(test))]
    let store = Some(crate::chat::exec_policy::global());
    #[cfg(test)]
    let store: Option<&crate::chat::exec_policy::PolicyStore> = None;
    answer_with_store(
        state,
        streams,
        persistence,
        AnswerTarget {
            key,
            call_id,
            decision,
        },
        store,
    )
}

#[cfg(test)]
pub(crate) fn answer_in_store(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &crate::db::service::Persistence,
    target: (&RequestKey, &str, SandboxApprovalDecision),
    store: &crate::chat::exec_policy::PolicyStore,
) -> bool {
    let (key, call_id, decision) = target;
    answer_with_store(
        state,
        streams,
        persistence,
        AnswerTarget {
            key,
            call_id,
            decision,
        },
        Some(store),
    )
}

fn append_exec_policy_rule(
    store: Option<&crate::chat::exec_policy::PolicyStore>,
    kind: crate::chat::sandbox::SandboxApprovalKind,
    prefix: &[String],
    justification: Option<&str>,
) {
    let Some(store) = store else {
        return;
    };
    let result = match kind {
        crate::chat::sandbox::SandboxApprovalKind::Rule => {
            store.append_allow_rule(prefix, justification)
        }
        crate::chat::sandbox::SandboxApprovalKind::Escalation => {
            store.append_model_allow_rule(prefix, justification)
        }
        crate::chat::sandbox::SandboxApprovalKind::Retry => return,
    };
    if let Err(error) = result {
        tracing::warn!(%error, "failed to append exec policy rule");
    }
}

fn answer_with_store(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &crate::db::service::Persistence,
    target: AnswerTarget<'_>,
    store: Option<&crate::chat::exec_policy::PolicyStore>,
) -> bool {
    let AnswerTarget {
        key,
        call_id,
        decision,
    } = target;
    let Some(pending) = streams.approvals.get(key) else {
        tracing::debug!(
            target: "lx::agent::approval",
            pane = key.pane.raw(),
            session = %key.session,
            call_id = %call_id,
            decision = decision_str(decision),
            "sandbox approval answer ignored (missing pending)"
        );
        return false;
    };
    if pending.call_id != call_id {
        tracing::debug!(
            target: "lx::agent::approval",
            pane = key.pane.raw(),
            session = %key.session,
            call_id = %call_id,
            pending_call_id = %pending.call_id,
            "sandbox approval answer ignored (call_id mismatch)"
        );
        return false;
    }
    let Some(pending) = streams.approvals.remove(key) else {
        return false;
    };
    let wait_ms = pending.requested_at.elapsed().as_millis() as u64;
    let for_session = decision == SandboxApprovalDecision::ForSession;
    let kind = pending.request.kind;
    let command = pending.request.command.clone();
    let prefix = pending.request.prefix_rule.clone();
    let justification = pending.request.justification.clone();
    let source_label = pending
        .request
        .source
        .clone()
        .unwrap_or_else(|| "main".to_string());
    let (agent_id, rule_session) = match streams.subagents.get(&key.session) {
        Some(handle) => (
            Some(handle.agent_id.clone()),
            Some(handle.root_session.clone()),
        ),
        None => (None, None),
    };
    let stream_workspace = streams.db_workspaces.get(key).cloned();
    tracing::info!(
        target: "lx::agent::approval",
        pane = key.pane.raw(),
        session = %key.session,
        parent_session = rule_session.as_deref().unwrap_or("-"),
        agent_id = agent_id.as_deref().unwrap_or("-"),
        call_id = %call_id,
        kind = kind.as_str(),
        source = %source_label,
        decision = decision_str(decision),
        wait_ms,
        command = %command_preview(&command),
        "sandbox approval answered"
    );
    let _ = pending.reply.send(Ok(decision));
    let Some(pane) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    let target_session = rule_session.or_else(|| pane.lx.chat.session_id.clone());
    let same_main_session =
        target_session.as_deref() == pane.lx.chat.session_id.as_deref() && target_session.is_some();
    let before = pane.lx.approvals.len();
    pane.lx
        .approvals
        .retain(|approval| !(&approval.key == key && approval.call_id == call_id));
    let mut changed = pane.lx.approvals.len() != before;
    let mut new_rule = None;
    if for_session {
        match kind {
            crate::chat::sandbox::SandboxApprovalKind::Retry => {
                if let Some(tokens) = shlex::split(&command) {
                    if same_main_session && !pane.lx.sandbox_exact_rules.contains(&tokens) {
                        pane.lx.sandbox_exact_rules.push(tokens.clone());
                        changed = true;
                    }
                    new_rule = Some(crate::db::sandbox::SessionSandboxRule {
                        kind: crate::db::sandbox::SandboxRuleKind::Exact,
                        tokens,
                    });
                }
            }
            crate::chat::sandbox::SandboxApprovalKind::Rule
            | crate::chat::sandbox::SandboxApprovalKind::Escalation => {
                if let Some(prefix) = prefix {
                    // 规则文件是跨会话通道；同步记录到会话前缀规则与 DB，保证本会话及 resume 后立即放行不再重复弹窗。
                    append_exec_policy_rule(
                        store,
                        kind,
                        &prefix,
                        Some(justification.as_str()).filter(|text| !text.is_empty()),
                    );
                    if same_main_session && !pane.lx.sandbox_prefix_rules.contains(&prefix) {
                        pane.lx.sandbox_prefix_rules.push(prefix.clone());
                        changed = true;
                    }
                    new_rule = Some(crate::db::sandbox::SessionSandboxRule {
                        kind: crate::db::sandbox::SandboxRuleKind::Prefix,
                        tokens: prefix,
                    });
                }
            }
        }
    }
    if let Some(rule) = new_rule
        && let Some(session_id) = target_session
        && let Some(workspace) =
            stream_workspace.or_else(|| state.chat_session_key_for_pane(key.pane))
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
        let wait_ms = pending.requested_at.elapsed().as_millis() as u64;
        tracing::info!(
            target: "lx::agent::approval",
            pane = key.pane.raw(),
            session = %key.session,
            call_id = %pending.call_id,
            kind = pending.request.kind.as_str(),
            source = pending.request.source.as_deref().unwrap_or("main"),
            wait_ms,
            "sandbox approval cancelled"
        );
        let _ = pending.reply.send(Err(HostError::Cancelled));
    }
    let Some(pane) = state.pane_mut_anywhere(key.pane) else {
        return had_pending;
    };
    let before = pane.lx.approvals.len();
    pane.lx.approvals.retain(|approval| &approval.key != key);
    before != pane.lx.approvals.len() || had_pending
}

/// 判断审批归属的主请求或子 agent 是否仍在当前会话中存活。
fn is_approval_key_live(state: &AppState, streams: &ChatStreams, key: &RequestKey) -> bool {
    let main_live = active_assistant_id(state, key).is_some()
        && !streams
            .cancels
            .get(key)
            .is_some_and(|cancel| *cancel.borrow());
    if main_live {
        return true;
    }
    let pane_session = state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.chat.session_id.as_deref());
    streams.subagents.get(&key.session).is_some_and(|handle| {
        handle.parent.pane == key.pane
            && handle.parent.generation == key.generation
            && handle.status == super::subagent::STATUS_RUNNING
            && !*handle.cancel.borrow()
            && pane_session == Some(handle.root_session.as_str())
    })
}

/// 防御 sweep：审批条目所属主请求与子 agent 都已不在途/不在当前会话时清除并唤醒。
pub(crate) fn sweep_orphans(state: &mut AppState, streams: &mut ChatStreams) -> bool {
    let mut orphaned: Vec<RequestKey> = Vec::new();
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_anywhere(pane) else {
            continue;
        };
        for approval in &target.lx.approvals {
            if !is_approval_key_live(state, streams, &approval.key)
                && !orphaned.contains(&approval.key)
            {
                orphaned.push(approval.key.clone());
            }
        }
    }
    for key in streams.approvals.keys() {
        if !is_approval_key_live(state, streams, key) && !orphaned.contains(key) {
            orphaned.push(key.clone());
        }
    }
    if !orphaned.is_empty() {
        tracing::info!(
            target: "lx::agent::approval",
            count = orphaned.len(),
            "sweeping orphan sandbox approvals"
        );
    }
    let mut changed = false;
    for key in orphaned {
        changed |= cancel(state, streams, &key);
    }
    changed
}
