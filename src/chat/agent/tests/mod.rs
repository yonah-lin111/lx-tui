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
            task: "find the compact entry".into(),
            items: vec![SubagentInputItem::Text {
                text: "find the compact entry".into(),
            }],
            fork_context: false,
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
            task: "review it".into(),
            items: vec![SubagentInputItem::Text {
                text: "review it".into(),
            }],
            fork_context: true,
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
            role: AgentRole::Executor,
            title: "fix it".into(),
            model: Some("openai/gpt-4.1-mini".into()),
            reasoning_effort: Some("high".into()),
            fork_context: true,
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
                    title: "compact entry".into(),
                    role: AgentRole::Explorer,
                    status: "completed".into(),
                    report: Some("found it".into()),
                },
                AgentStatusView {
                    agent_id: "sub-2".into(),
                    nickname: None,
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
        }))
    );
    assert_eq!(
        parse_control("resume_agent", r#"{"id":"sub_1"}"#),
        Some(Ok(SubagentOp::Resume { id: "sub_1".into() }))
    );
    assert_eq!(
        parse_control("list_agents", "{}"),
        Some(Ok(SubagentOp::List))
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
