//! 子 agent 领域层测试：角色解析、工具面名单、控制工具参数与报告信封。

use serde_json::json;

use super::*;

mod instructions;
mod model_switch;

/// 是否含 CJK 字符（模型可见文本必须全英文）。
pub(super) fn has_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(ch as u32,
            0x3000..=0x303F
            | 0x3040..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF)
    })
}

#[test]
fn role_parse_roundtrips_and_rejects_unknown() {
    for role in AgentRole::ALL {
        assert_eq!(AgentRole::parse(role.as_str()), Some(role));
        assert!(!has_cjk(role.system_prompt()), "{}", role.as_str());
    }
    assert_eq!(AgentRole::parse(""), None);
    assert_eq!(AgentRole::parse("Explorer"), None);
    assert_eq!(AgentRole::parse("1bad"), None);
    assert_eq!(AgentRole::parse("bad-name"), None);
    let planner = AgentRole::parse("planner").expect("custom role");
    assert_eq!(planner.as_str(), "planner");
    assert_eq!(planner.base(), BaseRole::Explorer);
    assert!(!planner.is_builtin());
    assert!(!has_cjk(planner.system_prompt()));
    let reviewer =
        AgentRole::from_name_and_base("reviewer", Some(BaseRole::Executor)).expect("reviewer");
    assert_eq!(reviewer.as_str(), "reviewer");
    assert_eq!(reviewer.base(), BaseRole::Executor);
    assert!(!reviewer.is_builtin());
}

#[test]
fn tool_surfaces_are_exact() {
    assert_eq!(
        ToolSurface::Main.names(),
        &[
            "read",
            "search",
            "web_search",
            "edit",
            "write",
            "exec_command",
            "write_stdin",
            "update_plan",
            "ask",
            "curr_time",
            "spawn_agent",
            "send_input",
            "interrupt_agent",
            "resume_agent",
            "wait_agent",
            "close_agent",
            "list_agents",
            "cancel_agent_input",
        ]
    );
    assert_eq!(
        ToolSurface::Explorer.names(),
        &["read", "search", "web_search"]
    );
    assert_eq!(
        ToolSurface::Executor.names(),
        &[
            "read",
            "search",
            "web_search",
            "edit",
            "write",
            "exec_command",
            "write_stdin",
        ]
    );
    assert_eq!(
        ToolSurface::from_role(AgentRole::Explorer),
        ToolSurface::Explorer
    );
    assert_eq!(
        ToolSurface::from_role(AgentRole::Executor),
        ToolSurface::Executor
    );
    assert_eq!(AgentRole::Explorer.tools(), ToolSurface::Explorer.names());
}

/// exec 工具面 = 主工具面去掉交互式 `ask`，顺序与其保持一致。
#[test]
fn exec_main_surface_drops_only_ask() {
    let names = ToolSurface::ExecMain.names();
    assert_eq!(names.len(), 17);
    assert!(!names.contains(&"ask"));
    let expected: Vec<&str> = ToolSurface::Main
        .names()
        .iter()
        .copied()
        .filter(|name| *name != "ask")
        .collect();
    assert_eq!(names, expected.as_slice());
    assert!(!EXEC_ASK_UNSUPPORTED_CODE.is_empty());
    assert!(!has_cjk(EXEC_ASK_UNSUPPORTED_CODE));
    assert!(
        EXEC_ASK_UNSUPPORTED_CODE.is_ascii(),
        "{}",
        EXEC_ASK_UNSUPPORTED_CODE
    );
}

#[test]
fn parse_control_accepts_valid_arguments() {
    assert_eq!(
        parse_control(
            "spawn_agent",
            r#"{"role":"explorer","title":"compact entry","task":"find the compact entry"}"#
        ),
        Some(Ok(SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "compact entry".into(),
            task_name: None,
            task: "find the compact entry".into(),
            items: vec![SubagentInputItem::Text {
                text: "find the compact entry".into(),
            }],
            fork_context: false,
            fork_turns: None,
            max_tokens: None,
            model: None,
            reasoning_effort: None,
        }))
    );
    assert_eq!(
        parse_control("interrupt_agent", r#"{"target":"sub-1"}"#),
        Some(Ok(SubagentOp::Interrupt {
            target: "sub-1".into(),
        }))
    );
    assert_eq!(
        parse_control("wait_agent", r#"{"targets":["sub-1","sub-2"]}"#),
        Some(Ok(SubagentOp::Wait {
            targets: vec!["sub-1".into(), "sub-2".into()],
            mode: WaitMode::All,
            timeout_ms: DEFAULT_WAIT_TIMEOUT_MS,
        }))
    );
    assert_eq!(
        parse_control("close_agent", r#"{"target":"sub-1"}"#),
        Some(Ok(SubagentOp::Close {
            target: "sub-1".into(),
        }))
    );
    // 非控制工具一律 None（主工具照旧走原有执行层）。
    assert_eq!(parse_control("read", r#"{"path":"a"}"#), None);
}

#[test]
fn parse_wait_supports_all_and_any_modes() {
    assert_eq!(
        parse_control("wait_agent", "{}"),
        Some(Ok(SubagentOp::Wait {
            targets: Vec::new(),
            mode: WaitMode::All,
            timeout_ms: DEFAULT_WAIT_TIMEOUT_MS,
        }))
    );
    assert_eq!(
        parse_control("wait_agent", r#"{"wait_for":"any"}"#),
        Some(Ok(SubagentOp::Wait {
            targets: Vec::new(),
            mode: WaitMode::Any,
            timeout_ms: DEFAULT_WAIT_TIMEOUT_MS,
        }))
    );
    assert_eq!(
        parse_control("wait_agent", r#"{"targets":["sub-1"],"wait_for":"any"}"#),
        Some(Ok(SubagentOp::Wait {
            targets: vec!["sub-1".into()],
            mode: WaitMode::Any,
            timeout_ms: DEFAULT_WAIT_TIMEOUT_MS,
        }))
    );
}

#[test]
fn parse_control_accepts_custom_role_and_spawn_overrides() {
    assert_eq!(
        parse_control(
            "spawn_agent",
            r#"{"role":"reviewer","title":"review","task":"review it","fork_context":true,"model":"openai/gpt-4.1-mini","reasoning_effort":"high"}"#
        ),
        Some(Ok(SubagentOp::Spawn {
            role: AgentRole::from_name_and_base("reviewer", None).expect("reviewer"),
            title: "review".into(),
            task_name: None,
            task: "review it".into(),
            items: vec![SubagentInputItem::Text {
                text: "review it".into(),
            }],
            fork_context: true,
            fork_turns: None,
            max_tokens: None,
            model: Some("openai/gpt-4.1-mini".into()),
            reasoning_effort: Some("high".into()),
        }))
    );
}

#[test]
fn parse_control_rejects_bad_arguments() {
    // 缺字段。
    assert!(parse_control("spawn_agent", r#"{"role":"explorer"}"#).is_some_and(|r| r.is_err()));
    assert!(parse_control("wait_agent", r#"{"wait_for":"some"}"#).is_some_and(|r| r.is_err()));
    // 多余字段。
    assert!(
        parse_control("spawn_agent", r#"{"role":"explorer","task":"x","extra":1}"#)
            .is_some_and(|r| r.is_err())
    );
    // 非法 role 语法。
    assert!(
        parse_control("spawn_agent", r#"{"role":"Bad-Role","task":"x"}"#)
            .is_some_and(|r| r.is_err())
    );
    // 非对象。
    assert!(parse_control("spawn_agent", "[]").is_some_and(|r| r.is_err()));
    // 超长 task。
    let task = "x".repeat(MAX_TASK_CHARS + 1);
    let arguments = json!({ "role": "executor", "task": task }).to_string();
    assert!(parse_control("spawn_agent", &arguments).is_some_and(|r| r.is_err()));
    // targets 超上限。
    let targets: Vec<String> = (0..=MAX_WAIT_TARGETS)
        .map(|index| format!("a{index}"))
        .collect();
    let arguments = json!({ "targets": targets }).to_string();
    assert!(parse_control("wait_agent", &arguments).is_some_and(|r| r.is_err()));
    // timeout 钳制：0、负数与超大值。
    assert_eq!(
        parse_control("wait_agent", r#"{"targets":["a"],"timeout_ms":0}"#),
        Some(Ok(SubagentOp::Wait {
            targets: vec!["a".into()],
            mode: WaitMode::All,
            timeout_ms: 1,
        }))
    );
    assert_eq!(
        parse_control("wait_agent", r#"{"targets":["a"],"timeout_ms":-5}"#),
        Some(Ok(SubagentOp::Wait {
            targets: vec!["a".into()],
            mode: WaitMode::All,
            timeout_ms: 1,
        }))
    );
    let oversized = format!(
        r#"{{"targets":["a"],"timeout_ms":{}}}"#,
        MAX_WAIT_TIMEOUT_MS + 1
    );
    assert_eq!(
        parse_control("wait_agent", &oversized),
        Some(Ok(SubagentOp::Wait {
            targets: vec!["a".into()],
            mode: WaitMode::All,
            timeout_ms: MAX_WAIT_TIMEOUT_MS,
        }))
    );
}

#[test]
fn outcome_json_roundtrips() {
    let cases = [
        SubagentOutcome::Spawned {
            agent_id: "sub-1".into(),
            nickname: Some("Turing".into()),
            agent_path: Some("/root/turing".into()),
            role: AgentRole::Executor,
            title: "fix it".into(),
            model: Some("openai/gpt-4.1-mini".into()),
            reasoning_effort: Some("high".into()),
            fork_context: true,
            fork_turns: Some(2),
            max_tokens: Some(50_000),
        },
        SubagentOutcome::Interrupted {
            agent_id: "sub-1".into(),
            nickname: Some("Turing".into()),
            previous_status: "running".into(),
            queued_count: 2,
        },
        SubagentOutcome::Waited {
            timed_out: true,
            agents: vec![
                AgentStatusView {
                    agent_id: "sub-1".into(),
                    nickname: Some("Turing".into()),
                    agent_path: Some("/root/turing".into()),
                    title: "compact entry".into(),
                    role: AgentRole::Explorer,
                    status: "completed".into(),
                    report: Some("found it".into()),
                },
                AgentStatusView {
                    agent_id: "sub-2".into(),
                    nickname: None,
                    agent_path: None,
                    title: "fix tests".into(),
                    role: AgentRole::Executor,
                    status: "running".into(),
                    report: None,
                },
            ],
            all: false,
            mode: WaitMode::All,
        },
        SubagentOutcome::Closed {
            agent_id: "sub-3".into(),
            previous_status: "running".into(),
        },
        SubagentOutcome::InputAccepted {
            submission_id: "input-1".into(),
            agent_id: "sub-1".into(),
            status: "queued".into(),
            queued_count: 2,
            interrupt: true,
            trigger_turn: false,
        },
        SubagentOutcome::Resumed {
            agent_id: "sub-1".into(),
            status: "idle".into(),
            queued_count: 0,
        },
        SubagentOutcome::Listed {
            agents: vec![AgentListItemView {
                agent_id: "sub-1".into(),
                nickname: Some("Turing".into()),
                agent_path: Some("/root/turing".into()),
                role: AgentRole::Explorer,
                title: "inspect".into(),
                status: "idle".into(),
                queued_count: 1,
            }],
        },
        SubagentOutcome::InputCancelled {
            submission_id: "input-1".into(),
            agent_id: "sub-1".into(),
            status: "cancelled".into(),
            queued_count: 0,
        },
    ];
    for outcome in cases {
        let json = outcome.to_json();
        assert_eq!(SubagentOutcome::from_json(&json), Some(outcome));
    }
    assert_eq!(SubagentOutcome::from_json(&json!({"tool": "read"})), None);
}

#[test]
fn interrupted_outcome_roundtrips_with_and_without_nickname() {
    for nickname in [None, Some("Turing".to_string())] {
        let outcome = SubagentOutcome::Interrupted {
            agent_id: "sub-1".into(),
            nickname: nickname.clone(),
            previous_status: "running".into(),
            queued_count: 3,
        };
        let json = outcome.to_json();
        assert_eq!(json["tool"], "interrupt_agent");
        assert_eq!(
            json.get("nickname").and_then(serde_json::Value::as_str),
            nickname.as_deref()
        );
        assert_eq!(SubagentOutcome::from_json(&json), Some(outcome));
    }
}

#[test]
fn allocate_nickname_uses_defaults_custom_pool_and_round_suffixes() {
    assert_eq!(allocate_nickname(&[], &[]), DEFAULT_NICKNAME_CANDIDATES[0]);
    assert_eq!(
        allocate_nickname(&["turing".into()], &[]),
        DEFAULT_NICKNAME_CANDIDATES[1],
        "ASCII 大小写不敏感去重"
    );

    let custom = vec!["Ada".to_string(), "Grace".to_string()];
    assert_eq!(allocate_nickname(&[], &custom), "Ada");
    assert_eq!(
        allocate_nickname(&["  ada  ".into()], &custom),
        "Grace",
        "自定义候选池优先于默认池"
    );
    assert_eq!(
        allocate_nickname(&[], &["  Ada  ".into(), "   ".into()]),
        "Ada",
        "空白候选被裁剪或忽略"
    );

    let all_defaults: Vec<String> = DEFAULT_NICKNAME_CANDIDATES
        .iter()
        .map(|name| name.to_string())
        .collect();
    assert_eq!(
        allocate_nickname(&all_defaults, &[]),
        format!("{} #2", DEFAULT_NICKNAME_CANDIDATES[0])
    );
    let mut used = all_defaults;
    used.push(format!("{} #2", DEFAULT_NICKNAME_CANDIDATES[0]));
    assert_eq!(
        allocate_nickname(&used, &[]),
        format!("{} #2", DEFAULT_NICKNAME_CANDIDATES[1])
    );
}

#[test]
fn format_agent_identity_combines_nickname_and_role() {
    assert_eq!(
        format_agent_identity(Some("Turing"), "explorer"),
        "Turing [explorer]"
    );
    assert_eq!(
        format_agent_identity(Some("  Turing "), " reviewer "),
        "Turing [reviewer]"
    );
    assert_eq!(format_agent_identity(None, "executor"), "executor");
    assert_eq!(format_agent_identity(Some("   "), "executor"), "executor");
    assert_eq!(format_agent_identity(Some("Turing"), ""), "Turing");
    assert_eq!(format_agent_identity(None, ""), "");
}

#[test]
fn parse_control_handles_new_codex_tools_and_items_contract() {
    // spawn_agent with ordered items (text + local_image + data URL image)
    let spawn_items = parse_control(
        "spawn_agent",
        r#"{
            "role": "explorer",
            "title": "Inspect screenshot",
            "items": [
                {"type": "text", "text": "Part 1"},
                {"type": "local_image", "path": "shots/bug.png"},
                {"type": "text", "text": "Part 2"},
                {"type": "image", "image_url": "data:image/png;base64,iVBORw0KGgo="}
            ]
        }"#,
    );
    assert_eq!(
        spawn_items,
        Some(Ok(SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "Inspect screenshot".into(),
            task_name: None,
            task: "Part 1\nPart 2".into(),
            items: vec![
                SubagentInputItem::Text {
                    text: "Part 1".into()
                },
                SubagentInputItem::LocalImage {
                    path: "shots/bug.png".into()
                },
                SubagentInputItem::Text {
                    text: "Part 2".into()
                },
                SubagentInputItem::Image {
                    image_url: "data:image/png;base64,iVBORw0KGgo=".into()
                },
            ],
            fork_context: false,
            fork_turns: None,
            max_tokens: None,
            model: None,
            reasoning_effort: None,
        }))
    );

    // send_input with message or items and interrupt
    assert_eq!(
        parse_control(
            "send_input",
            r#"{"target":"sub_1","message":"continue","interrupt":true}"#
        ),
        Some(Ok(SubagentOp::SendInput {
            target: "sub_1".into(),
            items: vec![SubagentInputItem::Text {
                text: "continue".into()
            }],
            interrupt: true,
            trigger_turn: true,
        }))
    );
    assert_eq!(
        parse_control("resume_agent", r#"{"id":"sub_1"}"#),
        Some(Ok(SubagentOp::Resume { id: "sub_1".into() }))
    );
    assert_eq!(
        parse_control("list_agents", "{}"),
        Some(Ok(SubagentOp::List { path_prefix: None }))
    );
    assert_eq!(
        parse_control("cancel_agent_input", r#"{"submission_id":"input_1"}"#),
        Some(Ok(SubagentOp::CancelInput {
            submission_id: "input_1".into()
        }))
    );

    // Reject task+items or message+items together, empty items, unknown fields, remote URLs, non-image data URLs, unsupported types
    for (tool, bad) in [
        (
            "spawn_agent",
            r#"{"role":"explorer","title":"t","task":"a","items":[{"type":"text","text":"b"}]}"#,
        ),
        (
            "spawn_agent",
            r#"{"role":"explorer","title":"t","items":[]}"#,
        ),
        (
            "spawn_agent",
            r#"{"role":"explorer","title":"   ","task":"a"}"#,
        ),
        (
            "send_input",
            r#"{"target":"sub_1","message":"a","items":[{"type":"text","text":"b"}]}"#,
        ),
        ("send_input", r#"{"target":"sub_1"}"#),
        ("send_input", r#"{"target":"sub_1","items":[]}"#),
        (
            "send_input",
            r#"{"target":"sub_1","items":[{"type":"image","image_url":"https://example.com/a.png"}]}"#,
        ),
        (
            "send_input",
            r#"{"target":"sub_1","items":[{"type":"local_image","path":"http://example.com/a.png"}]}"#,
        ),
        (
            "send_input",
            r#"{"target":"sub_1","items":[{"type":"image","image_url":"data:text/plain;base64,aGVsbG8="}]}"#,
        ),
        (
            "send_input",
            r#"{"target":"sub_1","items":[{"type":"audio","path":"a.wav"}]}"#,
        ),
        (
            "send_input",
            r#"{"target":"sub_1","items":[{"type":"skill","name":"x"}]}"#,
        ),
        (
            "send_input",
            r#"{"target":"sub_1","items":[{"type":"text","text":"x","extra":1}]}"#,
        ),
        ("resume_agent", r#"{"id":""}"#),
        ("resume_agent", r#"{"id":"sub_1","extra":true}"#),
        ("list_agents", r#"{"filter":"running"}"#),
        ("cancel_agent_input", r#"{"submission_id":"  "}"#),
        (
            "cancel_agent_input",
            r#"{"submission_id":"in_1","force":true}"#,
        ),
    ] {
        assert!(
            parse_control(tool, bad).is_some_and(|r| r.is_err()),
            "{tool} must reject {bad}"
        );
    }

    // Reject > 1 MiB arguments JSON
    let huge = format!(
        r#"{{"target":"sub_1","message":"{}"}}"#,
        "a".repeat(crate::chat::events::MAX_TOOL_ARGUMENTS_BYTES + 10)
    );
    let err = parse_control("send_input", &huge)
        .expect("control")
        .expect_err("too large");
    assert_eq!(err.code, crate::chat::tools::ToolErrorCode::TooLarge);
}

#[test]
fn report_envelope_escapes_and_truncates() {
    let report = SubagentReportView {
        agent_id: "sub-<1>".into(),
        nickname: Some("Turing".into()),
        title: "test title".into(),
        role: AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 2,
        changed_paths: vec!["src/a.rs".into(), "src/b.rs".into()],
        duration_ms: Some(14_200),
        body: "a < b & c\nline2".into(),
        delivered_at_ms: 1,
    };
    let envelope = format_report_envelope(&report);
    assert!(envelope.starts_with("<subagent_report "));
    assert!(envelope.contains("agent_id=\"sub-&lt;1&gt;\""));
    assert!(envelope.contains("nickname=\"Turing\""));
    assert!(envelope.contains("role=\"explorer\""));
    assert!(envelope.contains("files_changed=\"2\""));
    assert!(envelope.contains("duration_ms=\"14200\""));
    assert!(envelope.contains("a &lt; b &amp; c\nline2"));
    assert!(envelope.ends_with("</subagent_report>"));

    // 控制字符被剥离，正文超限截断并显式标注。
    let report = SubagentReportView {
        agent_id: "sub-1".into(),
        nickname: None,
        title: "test title".into(),
        role: AgentRole::Executor,
        status: "failed".into(),
        files_changed: 0,
        changed_paths: Vec::new(),
        duration_ms: None,
        body: format!("\u{1b}[31m{}", "y".repeat(MAX_REPORT_BYTES + 100)),
        delivered_at_ms: 0,
    };
    let envelope = format_report_envelope(&report);
    assert!(!envelope.contains('\u{1b}'));
    assert!(envelope.contains("[truncated]"));
    assert!(envelope.contains("duration_ms=\"unknown\""));
}

#[test]
fn report_view_json_roundtrips_and_rejects_garbage() {
    let report = SubagentReportView {
        agent_id: "sub-1".into(),
        nickname: Some("Turing".into()),
        title: "test title".into(),
        role: AgentRole::Executor,
        status: "completed".into(),
        files_changed: 3,
        changed_paths: vec!["src/a.rs".into(), "src/b.rs".into()],
        duration_ms: Some(99),
        body: "done".into(),
        delivered_at_ms: 42,
    };
    let text = report.to_json();
    assert_eq!(SubagentReportView::from_json(&text), Some(report.clone()));
    assert_eq!(SubagentReportView::from_json("{"), None);
    assert_eq!(SubagentReportView::from_json("{}"), None);

    let batch = reports_to_json(&[report.clone(), report]);
    let decoded = reports_from_json(&batch);
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].agent_id, "sub-1");
    // 旧版单对象 JSON 仍可读。
    assert_eq!(reports_from_json(&text).len(), 1);
    assert!(reports_from_json("{").is_empty());
    // 旧报告缺少 `changed_paths` 时默认为空数组。
    let legacy = json!({
        "agent_id": "sub-1",
        "title": "legacy",
        "role": "explorer",
        "status": "completed",
        "files_changed": 0,
        "duration_ms": null,
        "body": "done",
        "delivered_at_ms": 0,
    });
    let legacy_report = SubagentReportView::from_json(&legacy.to_string()).expect("legacy report");
    assert!(legacy_report.changed_paths.is_empty());
}

#[test]
fn executor_coordinator_surface_and_depth_gating() {
    assert_eq!(
        ToolSurface::ExecutorCoordinator.names(),
        &[
            "read",
            "search",
            "web_search",
            "edit",
            "write",
            "exec_command",
            "write_stdin",
            "spawn_agent",
            "send_input",
            "interrupt_agent",
            "resume_agent",
            "wait_agent",
            "close_agent",
            "list_agents",
            "cancel_agent_input",
        ]
    );
    // Explorer 永远只读；Executor 在未达上限时获得控制工具。
    for depth in 0..=3 {
        assert_eq!(
            AgentRole::Explorer.tool_surface_at_depth(depth, 3),
            ToolSurface::Explorer
        );
    }
    assert_eq!(
        AgentRole::Executor.tool_surface_at_depth(1, 2),
        ToolSurface::ExecutorCoordinator
    );
    assert_eq!(
        AgentRole::Executor.tool_surface_at_depth(2, 2),
        ToolSurface::Executor
    );
    assert_eq!(
        ToolSurface::from_role_at_depth(AgentRole::Executor, 0, 1),
        ToolSurface::ExecutorCoordinator
    );
    assert_eq!(
        ToolSurface::from_role_at_depth(AgentRole::Executor, 1, 1),
        ToolSurface::Executor
    );
}

#[test]
fn fork_spec_normalizes_params() {
    assert_eq!(ForkSpec::from_params(false, None), ForkSpec::None);
    assert_eq!(ForkSpec::from_params(true, None), ForkSpec::AllCompleted);
    assert_eq!(ForkSpec::from_params(true, Some(0)), ForkSpec::None);
    assert_eq!(
        ForkSpec::from_params(false, Some(3)),
        ForkSpec::RecentTurns(3)
    );
    assert_eq!(
        ForkSpec::from_params(false, Some(MAX_FORK_TURNS + 1)),
        ForkSpec::RecentTurns(MAX_FORK_TURNS)
    );
    assert!(!ForkSpec::None.is_enabled());
    assert!(ForkSpec::AllCompleted.is_enabled());
    assert_eq!(ForkSpec::AllCompleted.fork_turns(), None);
    assert_eq!(ForkSpec::RecentTurns(7).fork_turns(), Some(7));
    assert!(!BUDGET_EXCEEDED_CODE.is_empty());
}

#[test]
fn agent_status_roundtrips_and_projects() {
    let all = [
        AgentStatus::Running,
        AgentStatus::Idle,
        AgentStatus::Blocked,
        AgentStatus::Completed,
        AgentStatus::Failed,
        AgentStatus::Cancelled,
        AgentStatus::Interrupted,
    ];
    for status in all {
        assert_eq!(AgentStatus::parse(status.as_str()), Some(status));
    }
    assert_eq!(AgentStatus::parse("pending"), Some(AgentStatus::Running));
    assert_eq!(AgentStatus::parse("complete"), Some(AgentStatus::Completed));
    assert_eq!(AgentStatus::parse("  idle "), Some(AgentStatus::Idle));
    assert_eq!(AgentStatus::parse("unknown"), None);
    assert!(AgentStatus::Running.is_running());
    assert!(!AgentStatus::Running.is_settled());
    assert!(AgentStatus::Idle.is_settled());
    assert!(AgentStatus::Failed.is_settled());

    assert_eq!(
        projected_agent_status(true, true, true, "failed"),
        AgentStatus::Running
    );
    assert_eq!(
        projected_agent_status(false, true, true, "failed"),
        AgentStatus::Blocked
    );
    assert_eq!(
        projected_agent_status(false, true, false, "failed"),
        AgentStatus::Idle
    );
    assert_eq!(
        projected_agent_status(false, false, false, "failed"),
        AgentStatus::Failed
    );
    assert_eq!(
        projected_agent_status(false, false, false, "not-a-status"),
        AgentStatus::Completed
    );
}

#[test]
fn path_segments_slugify_depth_and_allocation() {
    for valid in ["a", "agent_2", "0", "a_b_c"] {
        assert!(is_valid_path_segment(valid), "{valid}");
    }
    let too_long = "a".repeat(MAX_PATH_SEGMENT_LEN + 1);
    for invalid in [
        "",
        "root",
        "_lead",
        "UPPER",
        "a-b",
        "a b",
        too_long.as_str(),
    ] {
        assert!(!is_valid_path_segment(invalid), "{invalid}");
    }
    assert_eq!(slugify_segment("Turing"), "turing");
    assert_eq!(slugify_segment("Turing #2"), "turing_2");
    assert_eq!(slugify_segment("vonNeumann"), "vonneumann");
    assert_eq!(slugify_segment("!!!"), "agent");
    assert_eq!(slugify_segment("root"), "agent");

    assert_eq!(path_depth("/root"), 0);
    assert_eq!(path_depth("/root/"), 0);
    assert_eq!(path_depth("/root/turing"), 1);
    assert_eq!(path_depth("/root/turing/curie"), 2);
    assert_eq!(path_depth("turing"), 1);

    let existing = vec!["/root/turing".to_string(), "/root/turing_2".to_string()];
    assert_eq!(
        allocate_child_path("/root", "turing", &existing),
        "/root/turing_3"
    );
    assert_eq!(
        allocate_child_path("/root/turing", "curie", &existing),
        "/root/turing/curie"
    );
    assert_eq!(
        allocate_child_path("", "Ada Lovelace", &[]),
        "/root/ada_lovelace"
    );
    assert_eq!(
        allocate_child_path("/root/turing", "Bad-Name", &[]),
        "/root/turing/bad_name"
    );
}

#[test]
fn stored_role_and_path_encodes_and_decodes_legacy_formats() {
    let encoded = encode_stored_role_and_path(BaseRole::Executor, " /root/turing ");
    assert_eq!(encoded, "executor:/root/turing");
    assert_eq!(
        decode_stored_role_and_path(Some(&encoded)),
        (Some(BaseRole::Executor), Some("/root/turing".into()))
    );
    assert_eq!(
        decode_stored_role_and_path(Some("explorer")),
        (Some(BaseRole::Explorer), None)
    );
    assert_eq!(
        decode_stored_role_and_path(Some("/root/turing")),
        (None, Some("/root/turing".into()))
    );
    assert_eq!(decode_stored_role_and_path(Some("")), (None, None));
    assert_eq!(decode_stored_role_and_path(None), (None, None));
    assert_eq!(decode_stored_role_and_path(Some("garbage")), (None, None));
}

#[test]
fn agent_target_matching_supports_id_nickname_and_paths() {
    let id = "sub-1";
    let nickname = Some("Turing");
    let path = Some("/root/turing/curie");
    // id 与昵称大小写不敏感。
    assert!(matches_agent_target(id, nickname, path, "/root", "sub-1"));
    assert!(matches_agent_target(
        id,
        nickname,
        path,
        "/root",
        "  turing "
    ));
    // 绝对路径。
    assert!(matches_agent_target(
        id,
        nickname,
        path,
        "/root",
        "/root/turing/curie"
    ));
    // 相对调用者。
    assert!(matches_agent_target(
        id,
        nickname,
        path,
        "/root/turing",
        "curie"
    ));
    // 相对根回退。
    assert!(matches_agent_target(
        id,
        nickname,
        path,
        "/root",
        "turing/curie"
    ));
    assert!(!matches_agent_target(id, nickname, path, "/root", "curie"));
    assert!(!matches_agent_target(
        id,
        nickname,
        path,
        "/root",
        "/root/other"
    ));
    assert!(!matches_agent_target(id, nickname, path, "/root", "   "));
}

#[test]
fn path_prefix_matching_normalizes_absolute_and_relative() {
    let path = "/root/turing/curie";
    assert!(path_matches_prefix(path, "/root/turing"));
    assert!(path_matches_prefix(path, "/root/turing/"));
    assert!(path_matches_prefix(path, "turing"));
    assert!(path_matches_prefix(path, "TURING"));
    assert!(path_matches_prefix(path, path));
    assert!(path_matches_prefix(path, ""));
    assert!(!path_matches_prefix(path, "/root/other"));
    assert!(!path_matches_prefix(path, "turing/curie/deeper"));
}

#[test]
fn token_budget_reminds_at_thresholds_and_exhausts() {
    assert!(SubagentTokenBudget::new(0).is_none());
    assert!(SubagentTokenBudget::new(-1).is_none());
    let mut budget = SubagentTokenBudget::new(1000).expect("budget");
    assert_eq!(budget.limit_tokens(), 1000);
    assert_eq!(budget.used_tokens(), 0);
    assert_eq!(budget.remaining_tokens(), 1000);
    assert_eq!(budget.take_pending_reminder(), None);

    let exhausted = budget.record_step_usage(crate::db::chat::Usage {
        input: Some(700),
        output: Some(100),
        ..crate::db::chat::Usage::default()
    });
    assert!(!exhausted);
    assert_eq!(budget.used_tokens(), 800);
    assert_eq!(budget.remaining_tokens(), 200);
    let reminder = budget.take_pending_reminder().expect("25% reminder");
    assert!(reminder.contains("<rollout_budget>"));
    assert!(reminder.contains("200 tokens left"));
    assert!(reminder.contains("limit: 1000"));
    assert_eq!(budget.take_pending_reminder(), None);

    let exhausted = budget.record_step_usage(crate::db::chat::Usage {
        input: Some(-5),
        output: Some(100),
        ..crate::db::chat::Usage::default()
    });
    assert!(!exhausted);
    assert_eq!(budget.remaining_tokens(), 100);
    let reminder = budget.take_pending_reminder().expect("10% reminder");
    assert!(reminder.contains("100 tokens left"));
    assert_eq!(budget.take_pending_reminder(), None);

    assert!(budget.record_step_usage(crate::db::chat::Usage {
        input: Some(200),
        output: None,
        ..crate::db::chat::Usage::default()
    }));
    assert_eq!(budget.remaining_tokens(), 0);
    assert_eq!(budget.take_pending_reminder(), None);
}

#[test]
fn parse_spawn_accepts_task_name_fork_turns_and_max_tokens() {
    assert_eq!(
        parse_control(
            "spawn_agent",
            r#"{"role":"executor","title":"delegate","task":"do it","task_name":"curie","fork_turns":2,"max_tokens":5000}"#
        ),
        Some(Ok(SubagentOp::Spawn {
            role: AgentRole::Executor,
            title: "delegate".into(),
            task_name: Some("curie".into()),
            task: "do it".into(),
            items: vec![SubagentInputItem::Text {
                text: "do it".into()
            }],
            fork_context: true,
            fork_turns: Some(2),
            max_tokens: Some(5000),
            model: None,
            reasoning_effort: None,
        }))
    );
    // fork_turns=0 显式关闭分叉，即使同时传了 fork_context=true。
    assert_eq!(
        parse_control(
            "spawn_agent",
            r#"{"role":"executor","title":"t","task":"x","fork_context":true,"fork_turns":0}"#
        ),
        Some(Ok(SubagentOp::Spawn {
            role: AgentRole::Executor,
            title: "t".into(),
            task_name: None,
            task: "x".into(),
            items: vec![SubagentInputItem::Text { text: "x".into() }],
            fork_context: false,
            fork_turns: Some(0),
            max_tokens: None,
            model: None,
            reasoning_effort: None,
        }))
    );
    for bad in [
        r#"{"role":"executor","title":"t","task":"x","task_name":"Root"}"#,
        r#"{"role":"executor","title":"t","task":"x","task_name":"_bad"}"#,
        r#"{"role":"executor","title":"t","task":"x","task_name":"has space"}"#,
        r#"{"role":"executor","title":"t","task":"x","fork_turns":65}"#,
        r#"{"role":"executor","title":"t","task":"x","fork_turns":-1}"#,
        r#"{"role":"executor","title":"t","task":"x","max_tokens":999}"#,
        r#"{"role":"executor","title":"t","task":"x","max_tokens":0}"#,
    ] {
        assert!(
            parse_control("spawn_agent", bad).is_some_and(|result| result.is_err()),
            "must reject {bad}"
        );
    }
    // 边界值合法。
    let arguments = json!({
        "role": "executor",
        "title": "t",
        "task": "x",
        "fork_turns": MAX_FORK_TURNS,
        "max_tokens": crate::config::subagents::MIN_SUBAGENT_MAX_TOKENS,
    })
    .to_string();
    assert!(parse_control("spawn_agent", &arguments).is_some_and(|result| result.is_ok()));
}

#[test]
fn parse_send_input_supports_trigger_turn() {
    assert_eq!(
        parse_control(
            "send_input",
            r#"{"target":"curie","message":"queue it","trigger_turn":false}"#
        ),
        Some(Ok(SubagentOp::SendInput {
            target: "curie".into(),
            items: vec![SubagentInputItem::Text {
                text: "queue it".into()
            }],
            interrupt: false,
            trigger_turn: false,
        }))
    );
    assert!(
        parse_control(
            "send_input",
            r#"{"target":"curie","message":"x","interrupt":true,"trigger_turn":false}"#
        )
        .is_some_and(|result| result.is_err())
    );
    assert!(
        parse_control(
            "send_input",
            r#"{"target":"curie","message":"x","trigger_turn":"yes"}"#
        )
        .is_some_and(|result| result.is_err())
    );
}

#[test]
fn parse_list_supports_empty_and_path_prefix() {
    assert_eq!(
        parse_control("list_agents", ""),
        Some(Ok(SubagentOp::List { path_prefix: None }))
    );
    assert_eq!(
        parse_control("list_agents", "{}"),
        Some(Ok(SubagentOp::List { path_prefix: None }))
    );
    assert_eq!(
        parse_control("list_agents", r#"{"path_prefix":"turing"}"#),
        Some(Ok(SubagentOp::List {
            path_prefix: Some("turing".into()),
        }))
    );
    assert!(
        parse_control("list_agents", r#"{"path_prefix":"  "}"#)
            .is_some_and(|result| result.is_err())
    );
}

#[test]
fn outcome_json_is_backward_compatible_with_optional_agent_fields() {
    let legacy_spawn = json!({
        "tool": "spawn_agent",
        "agent_id": "sub-1",
        "role": "explorer",
        "title": "legacy",
    });
    assert_eq!(
        SubagentOutcome::from_json(&legacy_spawn),
        Some(SubagentOutcome::Spawned {
            agent_id: "sub-1".into(),
            nickname: None,
            agent_path: None,
            role: AgentRole::Explorer,
            title: "legacy".into(),
            model: None,
            reasoning_effort: None,
            fork_context: false,
            fork_turns: None,
            max_tokens: None,
        })
    );
    let legacy_input = json!({
        "tool": "send_input",
        "submission_id": "input-1",
        "agent_id": "sub-1",
        "status": "queued",
        "queued_count": 1,
        "interrupt": false,
    });
    let parsed = SubagentOutcome::from_json(&legacy_input).expect("legacy input");
    assert!(matches!(
        parsed,
        SubagentOutcome::InputAccepted {
            trigger_turn: true,
            ..
        }
    ));

    let legacy_status = json!({
        "tool": "wait_agent",
        "timed_out": false,
        "all": false,
        "wait_for": "all",
        "agents": [{
            "agent_id": "sub-1",
            "title": "legacy",
            "role": "executor",
            "status": "completed",
            "report": null,
        }],
    });
    let parsed = SubagentOutcome::from_json(&legacy_status).expect("legacy wait");
    let SubagentOutcome::Waited { agents, .. } = parsed else {
        panic!("expected wait outcome");
    };
    assert_eq!(agents[0].agent_path, None);
}

#[test]
fn report_envelope_includes_changed_paths_only_when_present() {
    let report = SubagentReportView {
        agent_id: "sub-1".into(),
        nickname: None,
        title: "fix".into(),
        role: AgentRole::Executor,
        status: "completed".into(),
        files_changed: 2,
        changed_paths: vec!["src/a.rs".into(), "src/b.rs".into()],
        duration_ms: Some(1),
        body: "done".into(),
        delivered_at_ms: 0,
    };
    let envelope = format_report_envelope(&report);
    assert!(envelope.contains("done\nChanged files: src/a.rs, src/b.rs\n</subagent_report>"));

    let empty = SubagentReportView {
        changed_paths: Vec::new(),
        ..report.clone()
    };
    let envelope = format_report_envelope(&empty);
    assert!(!envelope.contains("Changed files:"));
    assert!(envelope.contains(">\ndone\n</subagent_report>"));

    let truncated = SubagentReportView {
        changed_paths: (0..(MAX_REPORT_CHANGED_PATHS + 2))
            .map(|index| format!("src/f{index}.rs"))
            .collect(),
        ..report
    };
    let envelope = format_report_envelope(&truncated);
    assert!(envelope.contains("(+2 more)"));
}

#[test]
fn synthesize_partial_report_body_uses_text_or_progress() {
    let with_text = synthesize_partial_report_body(
        "  still analyzing  ",
        3,
        &["src/a.rs".into()],
        "interrupted",
    );
    assert_eq!(with_text, "[Partial report (interrupted)]\nstill analyzing");
    let without_text = synthesize_partial_report_body(
        "",
        4,
        &["src/a.rs".into(), "src/b.rs".into()],
        "budget_exceeded",
    );
    assert!(without_text.contains("[Partial report (budget_exceeded)]"));
    assert!(without_text.contains("Completed 4 tool step(s)"));
    assert!(without_text.contains("modified 2 file(s): src/a.rs, src/b.rs"));
    let no_files = synthesize_partial_report_body("   ", 0, &[], "failed");
    assert!(no_files.contains("no files modified"));
}

#[test]
fn prompts_have_no_cjk_and_are_composed() {
    for role in AgentRole::ALL {
        assert!(!has_cjk(role.system_prompt()));
    }
    let prompt = compose_subagent_prompt(
        AgentRole::Explorer,
        crate::chat::prompt::PromptParts {
            environment: Some("<env/>"),
            ..crate::chat::prompt::PromptParts::default()
        },
    );
    assert!(prompt.starts_with("You are an explorer"));
    assert!(prompt.ends_with("<env/>\n"));
    assert_eq!(
        compose_subagent_prompt(
            AgentRole::Executor,
            crate::chat::prompt::PromptParts::default(),
        ),
        format!("{}\n\n\n", EXECUTOR_PROMPT)
    );
}
