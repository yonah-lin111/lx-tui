//! 子 agent 沙箱继承、主会话 `/permission` 切档后动态替换及审批路由回归测试。

use super::tests::*;
use super::*;
use crate::chat::sandbox::{SandboxApprovalDecision, SandboxApprovalKind, SandboxApprovalRequest};
use crate::config::sandbox::SandboxMode;

#[tokio::test]
async fn subagent_inherits_and_replaces_parent_sandbox_mode_across_turns() {
    let (_fixture, mut state, key, persistence, mut streams, parent_session) = setup_parent();
    // 首轮派发前主窗格切为 Read-only。
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .sandbox_mode = Some(SandboxMode::ReadOnly);

    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn-sb"),
        spawn_op(AgentRole::Executor, "Sandbox test", "turn 1"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);
    let db = persistence.database().expect("db");
    assert_eq!(
        db.session_sandbox_mode(&state.workspaces[0].cwd.clone().unwrap(), &child_session)
            .ok()
            .flatten()
            .as_deref(),
        Some("read-only"),
        "spawn 时子会话记录应继承主会话当前的 read-only 模式"
    );

    // 结束第 1 轮（保持 resumed idle 状态），然后切换主会话权限到 Full。
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
    );
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .sandbox_mode = Some(SandboxMode::DangerFullAccess);

    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-resume-sb"),
        SubagentOp::Resume {
            id: agent_id.clone(),
        },
    )
    .await
    .expect("resume");

    // 通过 send_input 触发第 2 轮：子会话必须立即替换为 danger-full-access，并在上下文注入 <system-update>。
    let sent = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-send-sb"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "turn 2".into(),
            }],
            interrupt: false,
            trigger_turn: true,
        },
    )
    .await
    .expect("send input");
    assert!(matches!(sent, SubagentOutcome::InputAccepted { .. }));
    assert_eq!(
        db.session_sandbox_mode(&state.workspaces[0].cwd.clone().unwrap(), &child_session)
            .ok()
            .flatten()
            .as_deref(),
        Some("danger-full-access"),
        "主会话切换权限后，子 agent 下一轮必须替换为最新权限"
    );

    // 顺带验证子 agent 的 `effective_sandbox_for_session` 读取主会话覆盖。
    let (resolved, _, _) = effective_sandbox_for_session(
        &state,
        Some(db),
        &state.workspaces[0].cwd.clone().unwrap(),
        key.pane,
        &parent_session,
    );
    assert_eq!(resolved.mode, SandboxMode::DangerFullAccess);
}

#[tokio::test]
async fn running_subagent_approval_routes_to_parent_pane_with_source_label() {
    let (_fixture, mut state, key, persistence, mut streams, parent_session) = setup_parent();
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn-approve"),
        spawn_op(AgentRole::Executor, "Approval test", "run cmd"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);
    let child_key = RequestKey {
        pane: key.pane,
        session: child_session.clone(),
        generation: key.generation,
    };
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let accepted = crate::event::chat::approval::request(
        &mut state,
        &mut streams,
        child_key.clone(),
        "call-sub-1".into(),
        SandboxApprovalRequest {
            kind: SandboxApprovalKind::Escalation,
            command: "git push".into(),
            cwd: "/ws".into(),
            justification: "push branch".into(),
            prefix_rule: Some(vec!["git".into(), "push".into()]),
            additional: None,
            denial_excerpt: None,
            source: None,
        },
        reply_tx,
    );
    assert!(
        accepted,
        "运行中的子 agent 审批不得被 active_assistant_id 误取消"
    );
    let front = state
        .pane_anywhere(key.pane)
        .and_then(|p| p.lx.approvals.front())
        .expect("approval queued");
    let source = front.request.source.as_deref().expect("subagent source");
    assert!(
        source.contains("[executor]"),
        "子 agent 来源应明确包含角色标注: {source}"
    );

    // 模拟真实事件循环：先通过 UI 确认，再依次经过 persistence.pump -> pump_chat_streams，
    // 验证 AnswerApproval 绝不会被 persistence.pump 吞掉而卡死。
    assert!(crate::app::update::apply_approval_input(
        &mut state,
        key.pane,
        crate::app::actions::LxInput::Enter,
    ));
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .approvals
            .is_empty()
    );
    let mut persistence = persistence;
    persistence.pump(&mut state);
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::AnswerApproval { .. })),
        "persistence.pump 不得吞掉 AnswerApproval"
    );
    let (ev_tx, _ev_rx) = mpsc::unbounded_channel();
    crate::event::chat::pump_chat_streams(&mut state, &mut streams, &ev_tx, &persistence);
    assert_eq!(
        reply_rx.await.expect("reply").expect("decision"),
        SandboxApprovalDecision::Once
    );
    let _ = parent_session;
}

#[tokio::test]
async fn subagent_approval_is_rejected_if_pane_switched_sessions() {
    let (_fixture, mut state, key, _persistence, mut streams, _parent_session) = setup_parent();
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &_persistence,
        &key,
        Some("c-spawn-switched"),
        spawn_op(AgentRole::Executor, "Switched session", "run cmd"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);
    let child_key = RequestKey {
        pane: key.pane,
        session: child_session,
        generation: key.generation,
    };
    // 窗格切走新建会话后，旧会话残留的子 agent 审批不得劫持新会话窗格。
    assert!(crate::app::update::new_session(&mut state, key.pane));
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let accepted = crate::event::chat::approval::request(
        &mut state,
        &mut streams,
        child_key,
        "call-sub-stale".into(),
        SandboxApprovalRequest {
            kind: SandboxApprovalKind::Escalation,
            command: "pwd".into(),
            cwd: "/ws".into(),
            justification: "stale".into(),
            prefix_rule: None,
            additional: None,
            denial_excerpt: None,
            source: None,
        },
        reply_tx,
    );
    assert!(!accepted);
    assert!(matches!(
        reply_rx.await.expect("reply"),
        Err(HostError::Cancelled)
    ));
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .approvals
            .is_empty()
    );
}
