//! 子 agent 领域层测试：角色解析、工具面名单、控制工具参数与报告信封。

use serde_json::json;

use super::*;

/// 是否含 CJK 字符（模型可见文本必须全英文）。
fn has_cjk(text: &str) -> bool {
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
    assert_eq!(AgentRole::parse("planner"), None);
}

#[test]
fn tool_surfaces_are_exact() {
    assert_eq!(
        ToolSurface::Main.names(),
        &[
            "read",
            "search",
            "edit",
            "write",
            "exec_command",
            "write_stdin",
            "update_plan",
            "ask",
            "spawn_agent",
            "wait_agent",
            "close_agent",
        ]
    );
    assert_eq!(ToolSurface::Explorer.names(), &["read", "search"]);
    assert_eq!(
        ToolSurface::Executor.names(),
        &[
            "read",
            "search",
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
fn parse_control_rejects_bad_arguments() {
    // 缺字段。
    assert!(parse_control("spawn_agent", r#"{"role":"explorer"}"#).is_some_and(|r| r.is_err()));
    assert!(parse_control("wait_agent", r#"{"wait_for":"some"}"#).is_some_and(|r| r.is_err()));
    // 多余字段。
    assert!(
        parse_control("spawn_agent", r#"{"role":"explorer","task":"x","extra":1}"#)
            .is_some_and(|r| r.is_err())
    );
    // 非法 role。
    assert!(
        parse_control("spawn_agent", r#"{"role":"planner","task":"x"}"#)
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
            role: AgentRole::Executor,
            title: "fix it".into(),
        },
        SubagentOutcome::Waited {
            timed_out: true,
            agents: vec![
                AgentStatusView {
                    agent_id: "sub-1".into(),
                    title: "compact entry".into(),
                    role: AgentRole::Explorer,
                    status: "completed".into(),
                    report: Some("found it".into()),
                },
                AgentStatusView {
                    agent_id: "sub-2".into(),
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
    ];
    for outcome in cases {
        let json = outcome.to_json();
        assert_eq!(SubagentOutcome::from_json(&json), Some(outcome));
    }
    assert_eq!(SubagentOutcome::from_json(&json!({"tool": "read"})), None);
}

#[test]
fn report_envelope_escapes_and_truncates() {
    let report = SubagentReportView {
        agent_id: "sub-<1>".into(),
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
    assert!(envelope.contains("role=\"explorer\""));
    assert!(envelope.contains("files_changed=\"2\""));
    assert!(envelope.contains("duration_ms=\"14200\""));
    assert!(envelope.contains("a &lt; b &amp; c\nline2"));
    assert!(envelope.ends_with("</subagent_report>"));

    // 控制字符被剥离，正文超限截断并显式标注。
    let report = SubagentReportView {
        agent_id: "sub-1".into(),
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
    let prompt = compose_subagent_prompt(AgentRole::Explorer, Some("<env/>"));
    assert!(prompt.starts_with("You are an explorer"));
    assert!(prompt.ends_with("<env/>\n"));
    assert_eq!(
        compose_subagent_prompt(AgentRole::Executor, None),
        format!("{}\n\n\n", EXECUTOR_PROMPT)
    );
}
