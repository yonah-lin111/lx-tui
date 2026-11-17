//! 沙箱审批端到端回归测试：真实 `AgentTools` + `Persistence::pump` + `pump_chat_streams`。
//!
//! 覆盖 `sess-18e8ca48d78ff6d8-0` 对应故障：用户确认提权后，事件循环先跑
//! `persistence.pump` 再跑 `pump_chat_streams`，审批回执必须正确唤醒 agent loop 并
//! 推进工具至完成，不得卡死在 `running`。

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc, watch};

use super::*;
use crate::app::actions::LxInput;
use crate::chat::agent_loop::{AgentTools, TurnExecution};
use crate::chat::events::RequestPurpose;
use crate::chat::process::{ProcessManager, ProcessOwner};
use crate::chat::sandbox::SandboxPlan;
use crate::config::sandbox::SandboxSettings;

#[tokio::test]
async fn escalated_exec_command_completes_after_persistence_and_chat_pump() {
    let fixture = Fixture::new("approval-e2e");
    let (mut state, key, mut persistence, _session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    streams.set_visible_panes(&[(key.pane, Rect::new(0, 0, 80, 24))]);
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());

    let args = serde_json::json!({
        "cmd": "echo approval-e2e-ok",
        "justification": "run test outside workspace restriction",
        "sandbox_permissions": "require_escalated",
    })
    .to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("call-esc-1", "exec_command", &args, 0)], None),
            final_response("all done", None),
        ],
        StopReason::Complete,
    );
    transport.attach(sender.clone(), key.clone());

    let processes = Arc::new(ProcessManager::new(
        ProcessOwner {
            workspace: fixture.workspace().to_path_buf(),
            chat_session_id: key.session.clone(),
            runtime_epoch: "epoch-test".into(),
        },
        None,
    ));
    let executor = AgentTools::new(
        Arc::clone(&processes),
        Arc::new(Mutex::new(TurnExecution::default())),
    )
    .with_sandbox(SandboxPlan::for_turn(
        &SandboxSettings::default(),
        fixture.workspace(),
    ));

    let prepared = prepare_turn(
        ChatRequest {
            turns: Vec::new(),
            items: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
            variant_params: None,
            purpose: RequestPurpose::Chat,
            system_prompt: None,
            tools: std::sync::Arc::new(crate::chat::tools::turn_tools(
                crate::chat::agent::ToolSurface::Main,
                &crate::mcp::McpToolSnapshot::default(),
            )),
        },
        fixture.workspace().to_path_buf(),
    );

    let (_cancel_tx, cancel) = watch::channel(false);
    let mut driver = Box::pin(host::drive_with(
        prepared.context,
        host::EventHost {
            key: key.clone(),
            sender: sender.clone(),
        },
        executor,
        transport.stream_one(),
        cancel,
    ));

    let mut approval_answered = false;
    let summary = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                summary = &mut driver => break summary,
                Some(event) = receiver.recv() => {
                    match event {
                        AppEvent::AgentHost(host_event) => {
                            let is_begin = matches!(&host_event, AgentHostEvent::BeginTool { .. });
                            let is_approval = matches!(&host_event, AgentHostEvent::ApproveSandbox { .. });
                            handle_agent_host_event(&mut state, &mut streams, &persistence, host_event);
                            if is_begin {
                                flush_pending_tool_begins(&mut state, &mut streams, &persistence);
                            }
                            if is_approval {
                                assert!(
                                    !state.pane_anywhere(key.pane).expect("pane").lx.approvals.is_empty(),
                                    "审批命令面板必须已入队"
                                );
                                // 模拟用户敲 Enter 授权，并按生产主循环同序先后运行 persistence.pump -> pump_chat_streams。
                                crate::app::update::apply_lx_input(&mut state, LxInput::Enter);
                                persistence.pump(&mut state);
                                pump_chat_streams(&mut state, &mut streams, &sender, &persistence);
                                approval_answered = true;
                            }
                        }
                        AppEvent::ChatNet(net_event) => {
                            handle_net_event(&mut state, &mut streams, &persistence, net_event);
                        }
                        _ => {}
                    }
                }
            }
        }
    })
    .await
    .expect("approved exec_command must not stall in running");

    assert!(approval_answered, "必须触发并完成审批握手");
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    let output = transport
        .requests()
        .last()
        .map(|req| format!("{:?}", req.items))
        .unwrap_or_default();
    assert!(
        output.contains("approval-e2e-ok"),
        "审批后命令输出必须真实回填给下一步模型: {output}"
    );
    processes.shutdown().await;
}

#[tokio::test]
async fn rule_for_session_records_prefix_in_pane_db_and_policy_store() {
    use crate::chat::sandbox::{
        SandboxApprovalDecision, SandboxApprovalKind, SandboxApprovalRequest,
    };
    use crate::db::sandbox::{SandboxRuleKind, SessionSandboxRule};

    let fixture = Fixture::new("approval-rule-for-session");
    let (mut state, key, persistence, session_id) = streaming_state(&fixture);
    let rules_dir = fixture.workspace().join("rules");
    std::fs::create_dir_all(&rules_dir).expect("rules dir");
    std::fs::write(
        rules_dir.join("prompt.rules"),
        "prefix_rule(pattern = [\"make\", \"test\"], decision = \"prompt\", justification = \"confirm make test\")\n",
    )
    .expect("write prompt rules");
    let store = crate::chat::exec_policy::PolicyStore::new(rules_dir.clone());

    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let accepted = crate::event::chat::approval::request(
        &mut state,
        &mut streams,
        key.clone(),
        "call-rule-1".into(),
        SandboxApprovalRequest {
            kind: SandboxApprovalKind::Rule,
            command: "make test --all".into(),
            cwd: fixture.workspace().display().to_string(),
            justification: "confirm make test".into(),
            prefix_rule: Some(vec!["make".into(), "test".into()]),
            additional: None,
            denial_excerpt: None,
            source: None,
        },
        reply_tx,
    );
    assert!(accepted);

    assert!(crate::event::chat::approval::answer_in_store(
        &mut state,
        &mut streams,
        &persistence,
        (&key, "call-rule-1", SandboxApprovalDecision::ForSession),
        &store,
    ));
    assert_eq!(
        reply_rx.await.expect("reply").expect("decision"),
        SandboxApprovalDecision::ForSession
    );

    let pane = state.pane_anywhere(key.pane).expect("pane");
    assert_eq!(
        pane.lx.sandbox_prefix_rules,
        vec![vec!["make".to_string(), "test".to_string()]]
    );
    let db_rules = persistence
        .database()
        .expect("db")
        .sandbox_rules(fixture.workspace(), &session_id)
        .expect("db rules");
    assert_eq!(
        db_rules,
        vec![SessionSandboxRule {
            kind: SandboxRuleKind::Prefix,
            tokens: vec!["make".into(), "test".into()],
        }]
    );
    let default_rules =
        std::fs::read_to_string(rules_dir.join("default.rules")).expect("default.rules");
    assert!(
        default_rules.contains("prefix_rule(pattern = [\"make\", \"test\"], decision = \"allow\""),
        "default.rules 必须追加 allow 前缀规则: {default_rules}"
    );
    let plan = SandboxPlan::for_turn(&SandboxSettings::default(), fixture.workspace())
        .with_exact_rules(Vec::new(), pane.lx.sandbox_prefix_rules.clone());
    assert!(
        plan.command_is_approved("make test --all"),
        "会话前缀规则写入后，后续匹配命令必须直接命中放行"
    );
}

#[test]
fn handle_rewind_finished_syncs_and_resets_sandbox_state() {
    use crate::app::state::{ApprovalState, RewindAction};
    use crate::config::sandbox::SandboxMode;
    use crate::db::chat::{ModelSnapshot, PermissionSwitchCommit, TurnOutcome, Usage};
    use crate::db::sandbox::{SandboxRuleKind, SessionSandboxRule};

    let fixture = Fixture::new("rewind-sandbox-sync");
    let database = fixture.database();
    let persistence = Persistence::with_database(database);
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(fixture.workspace().to_path_buf());
    let pane = state.active_tab().layout.focus();
    let model = ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };

    let db = persistence.database().expect("db");
    let t1 = db
        .begin_turn(fixture.workspace(), None, "turn 1", &model)
        .expect("t1");
    db.finish(
        fixture.workspace(),
        &t1.assistant_message_id,
        &[],
        &TurnOutcome::Complete {
            stop_reason: Some("stop".into()),
        },
        Usage::default(),
    )
    .expect("f1");
    db.set_session_sandbox_mode(fixture.workspace(), &t1.session_id, Some("read-only"))
        .expect("set mode");
    db.insert_sandbox_rule(
        fixture.workspace(),
        &t1.session_id,
        &SessionSandboxRule {
            kind: SandboxRuleKind::Exact,
            tokens: vec!["pwd".into()],
        },
    )
    .expect("rule");
    db.record_permission_switch(
        fixture.workspace(),
        &PermissionSwitchCommit {
            session_id: t1.session_id.clone(),
            from_mode: "read-only".into(),
            to_mode: "danger-full-access".into(),
            injected_prompt: "Switched to Full".into(),
        },
    )
    .expect("switch");

    // 1. 存活撤销：DB 中模式与规则已同步（含存活规则与开关），handle_rewind_finished 必须把状态同步回窗格。
    db.set_session_sandbox_mode(fixture.workspace(), &t1.session_id, Some("read-only"))
        .expect("rollback mode");
    db.clear_sandbox_rules(&t1.session_id).expect("clear rules");
    db.insert_sandbox_rule(
        fixture.workspace(),
        &t1.session_id,
        &SessionSandboxRule {
            kind: SandboxRuleKind::Prefix,
            tokens: vec!["git".into(), "status".into()],
        },
    )
    .expect("surviving rule");
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.session_id = Some(t1.session_id.clone());
        target.lx.sandbox_mode = Some(SandboxMode::DangerFullAccess);
        target.lx.sandbox_exact_rules = vec![vec!["stale".into()]];
        target.lx.sandbox_prefix_rules = vec![vec!["stale-prefix".into()]];
    }
    assert!(crate::event::chat::handle_rewind_finished(
        &mut state,
        &persistence,
        pane,
        &t1.session_id,
        fixture.workspace(),
        "op-surviving",
        crate::event::chat::RewindOutcome::Succeeded {
            action: RewindAction::DeleteOnly,
            restored_files: 0,
            already_restored: false,
            user_text: "turn 1".into(),
            deleted: true,
            session_deleted: false,
        },
    ));
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.sandbox_mode, Some(SandboxMode::ReadOnly));
    assert!(lx.sandbox_exact_rules.is_empty());
    assert_eq!(
        lx.sandbox_prefix_rules,
        vec![vec!["git".to_string(), "status".to_string()]]
    );
    assert_eq!(lx.permission_switch.switches.len(), 1);

    // 2. 删空会话：session_deleted = true 时必须清空全部会话沙箱状态与挂起审批。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.sandbox_mode = Some(SandboxMode::DangerFullAccess);
        target.lx.sandbox_exact_rules = vec![vec!["echo".into()]];
        target.lx.sandbox_prefix_rules = vec![vec!["cargo".into()]];
        target.lx.approvals.push_back(ApprovalState::new(
            RequestKey {
                pane,
                session: t1.session_id.clone(),
                generation: 1,
            },
            "call-stale".into(),
            crate::chat::sandbox::SandboxApprovalRequest {
                kind: crate::chat::sandbox::SandboxApprovalKind::Escalation,
                command: "echo".into(),
                cwd: "/ws".into(),
                justification: "test".into(),
                prefix_rule: None,
                additional: None,
                denial_excerpt: None,
                source: None,
            },
        ));
    }
    assert!(crate::event::chat::handle_rewind_finished(
        &mut state,
        &persistence,
        pane,
        &t1.session_id,
        fixture.workspace(),
        "op-deleted",
        crate::event::chat::RewindOutcome::Succeeded {
            action: RewindAction::DeleteOnly,
            restored_files: 0,
            already_restored: false,
            user_text: "turn 1".into(),
            deleted: true,
            session_deleted: true,
        },
    ));
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.sandbox_mode, None);
    assert!(lx.sandbox_exact_rules.is_empty());
    assert!(lx.sandbox_prefix_rules.is_empty());
    assert!(lx.approvals.is_empty());
    assert!(lx.permission_switch.switches.is_empty());
}
