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

#[test]
fn persist_sandbox_mode_syncs_all_bound_panes_with_and_without_switch_commit() {
    let (mut service, mut state, cwd, _) = trusted_service("persist-sandbox-multipane");
    let pane_a = state.active_tab().layout.focus();
    let pane_b = state
        .active_tab_mut()
        .split_pane(pane_a, ratatui::layout::Direction::Vertical)
        .expect("split");
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

    for pane in [pane_a, pane_b] {
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.chat.session_id = Some(turn.session_id.clone());
            target.lx.sandbox_exact_rules = vec![vec!["echo".into(), "ok".into()]];
            target.lx.sandbox_prefix_rules = vec![vec!["cargo".into(), "test".into()]];
        }
    }

    // 1. 无 switch_commit 路径（例如空会话切档）：兄弟窗格仍须同步模式并清空旧规则。
    state.effects.push(Effect::PersistSandboxMode {
        workspace: cwd.clone(),
        session_id: turn.session_id.clone(),
        mode: Some(SandboxMode::ReadOnly),
        switch_commit: None,
    });
    assert!(service.pump(&mut state));
    for pane in [pane_a, pane_b] {
        let lx = &state.pane_anywhere(pane).expect("pane").lx;
        assert_eq!(lx.sandbox_mode, Some(SandboxMode::ReadOnly));
        assert!(lx.sandbox_exact_rules.is_empty());
        assert!(lx.sandbox_prefix_rules.is_empty());
    }

    // 2. 带 switch_commit 路径：写入切换记录并同步两个窗格的模式、规则与切换条目。
    if let Some(target) = state.pane_mut_anywhere(pane_b) {
        target.lx.sandbox_exact_rules = vec![vec!["ls".into()]];
    }
    state.effects.push(Effect::PersistSandboxMode {
        workspace: cwd.clone(),
        session_id: turn.session_id.clone(),
        mode: Some(SandboxMode::DangerFullAccess),
        switch_commit: Some(PermissionSwitchCommit {
            session_id: turn.session_id.clone(),
            from_mode: "read-only".into(),
            to_mode: "danger-full-access".into(),
            injected_prompt: "Switched to Full".into(),
        }),
    });
    assert!(service.pump(&mut state));
    for pane in [pane_a, pane_b] {
        let lx = &state.pane_anywhere(pane).expect("pane").lx;
        assert_eq!(lx.sandbox_mode, Some(SandboxMode::DangerFullAccess));
        assert!(lx.sandbox_exact_rules.is_empty());
        assert!(lx.sandbox_prefix_rules.is_empty());
        assert_eq!(lx.permission_switch.switches.len(), 1);
        assert_eq!(
            lx.permission_switch.switches[0].to_mode,
            "danger-full-access"
        );
    }
}

#[test]
fn persist_sandbox_mode_failure_reverts_optimistic_state_and_shows_warning_toast() {
    let (mut service, mut state, cwd, db_path) = trusted_service("persist-sandbox-fail");
    let pane = state.active_tab().layout.focus();
    let model = ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let session_id = {
        let db = service.db.as_ref().expect("db");
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
        db.set_session_sandbox_mode(&cwd, &turn.session_id, Some("read-only"))
            .expect("set initial mode");
        db.insert_sandbox_rule(
            &cwd,
            &turn.session_id,
            &crate::db::sandbox::SessionSandboxRule {
                kind: crate::db::sandbox::SandboxRuleKind::Exact,
                tokens: vec!["pwd".into()],
            },
        )
        .expect("insert rule");
        turn.session_id
    };

    // 注入触发器使 UPDATE agent_session.sandbox_mode 失败，但保留只读查询能力。
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open raw conn");
        conn.execute_batch(
            "CREATE TRIGGER block_sandbox_update
             BEFORE UPDATE OF sandbox_mode ON agent_session
             BEGIN
                 SELECT RAISE(ABORT, 'simulated write failure');
             END;",
        )
        .expect("trigger");
    }

    // 模拟前端乐观切档后的窗格状态。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.session_id = Some(session_id.clone());
        target.lx.sandbox_mode = Some(SandboxMode::DangerFullAccess);
        target.lx.sandbox_exact_rules.clear();
        target
            .lx
            .permission_switch
            .switches
            .push(crate::app::state::PermissionSwitchNoticeView {
                id: "optimistic".into(),
                sequence: 99,
                from_mode: "read-only".into(),
                to_mode: "danger-full-access".into(),
                injected_prompt: "Switched to Full".into(),
                created_at_ms: 0,
            });
    }

    state.effects.push(Effect::PersistSandboxMode {
        workspace: cwd.clone(),
        session_id: session_id.clone(),
        mode: Some(SandboxMode::DangerFullAccess),
        switch_commit: Some(PermissionSwitchCommit {
            session_id: session_id.clone(),
            from_mode: "read-only".into(),
            to_mode: "danger-full-access".into(),
            injected_prompt: "Switched to Full".into(),
        }),
    });

    assert!(service.pump(&mut state));
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(
        lx.sandbox_mode,
        Some(SandboxMode::ReadOnly),
        "DB 写失败时必须回滚到数据库真实模式"
    );
    assert_eq!(
        lx.sandbox_exact_rules,
        vec![vec!["pwd".to_string()]],
        "DB 写失败时必须恢复数据库中的会话规则"
    );
    assert!(
        lx.permission_switch.switches.is_empty(),
        "DB 写失败时必须清掉乐观写入的切换卡片"
    );
    let toast = lx.toast.as_ref().expect("warning toast");
    assert_eq!(toast.kind, crate::app::toast::ToastKind::Warning);
    assert!(toast.message.contains("Failed to save permission mode"));
}
