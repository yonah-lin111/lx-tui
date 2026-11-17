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
