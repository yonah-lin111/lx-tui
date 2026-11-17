//! 规则文件与提权的决策优先级（design §18.4）。

use super::super::sandbox_decision::override_for;
use super::*;
use crate::chat::exec_policy::parser::PolicyParser;
use crate::chat::exec_policy::policy::Policy;
use crate::chat::sandbox::{
    SandboxApprovalDecision, SandboxApprovalKind, SandboxOverride, SandboxPlan,
};
use crate::config::sandbox::{EscalationPolicy, SandboxSettings};

struct PolicyExecutor {
    plan: SandboxPlan,
}

impl ToolExecutor for PolicyExecutor {
    async fn execute(&self, _cwd: &Path, _call: &ReadyToolCall) -> ToolExecutionResult {
        ToolFailure::new(ToolErrorCode::Io, "unused").into_result(Vec::new())
    }

    fn sandbox_plan(&self) -> Option<&SandboxPlan> {
        Some(&self.plan)
    }
}

fn policy(contents: &str) -> Policy {
    let mut parser = PolicyParser::new();
    parser.parse("test.rules", contents).expect("parse rules");
    Policy::from_rules(parser.build())
}

fn plan(escalation: EscalationPolicy) -> SandboxPlan {
    SandboxPlan::for_turn(
        &SandboxSettings {
            escalation,
            ..SandboxSettings::default()
        },
        &std::env::temp_dir(),
    )
}

fn exec_call(command: &str, extra: &str) -> ReadyToolCall {
    ReadyToolCall {
        call_id: "call-rules".into(),
        name: "exec_command".into(),
        arguments_json: format!(
            r#"{{"cmd":{},"login":false,"yield_time_ms":1000{extra}}}"#,
            serde_json::to_string(command).expect("command json")
        ),
        ordinal: 0,
    }
}

async fn decide(
    host: &mut RecordingHost,
    plan: SandboxPlan,
    policy: &Policy,
    call: &ReadyToolCall,
) -> Result<SandboxOverride, ToolExecutionResult> {
    let executor = PolicyExecutor { plan };
    override_for(host, &executor, call, Path::new("/"), policy).await
}

fn failure_code(result: ToolExecutionResult) -> ToolErrorCode {
    match result {
        ToolExecutionResult::Failed { code, .. } => code,
        other => panic!("expected failure, got {other:?}"),
    }
}

#[tokio::test]
async fn forbidden_rule_blocks_with_the_rule_justification() {
    let rules = policy(
        r#"prefix_rule(pattern = ["rm"], decision = "forbidden", justification = "no deletes")"#,
    );
    let mut host = RecordingHost::new();
    let result = decide(
        &mut host,
        plan(EscalationPolicy::Ask),
        &rules,
        &exec_call("rm -rf /tmp/x", ""),
    )
    .await;
    let Err(result) = result else {
        panic!("expected denial");
    };
    assert_eq!(failure_code(result), ToolErrorCode::SandboxDenied);
    assert!(
        host.trace.lock().expect("trace").is_empty(),
        "Forbidden 不得询问用户"
    );
}

#[tokio::test]
async fn allow_rule_skips_the_sandbox_only_without_metacharacters() {
    let rules = policy(r#"prefix_rule(pattern = ["echo"], decision = "allow")"#);
    let mut host = RecordingHost::new();
    let allowed = decide(
        &mut host,
        plan(EscalationPolicy::Ask),
        &rules,
        &exec_call("echo hi", ""),
    )
    .await
    .expect("allowed");
    assert_eq!(allowed, SandboxOverride::Unsandboxed);

    let composite = decide(
        &mut host,
        plan(EscalationPolicy::Ask),
        &rules,
        &exec_call("echo hi && curl http://x", ""),
    )
    .await
    .expect("default sandbox");
    assert_eq!(composite, SandboxOverride::None, "含元字符时 allow 不生效");
}

#[tokio::test]
async fn prompt_rule_uses_the_rule_approval_kind() {
    let rules =
        policy(r#"prefix_rule(pattern = ["make"], decision = "prompt", justification = "build")"#);
    let mut host = RecordingHost::approving(SandboxApprovalDecision::Once);
    let result = decide(
        &mut host,
        plan(EscalationPolicy::Ask),
        &rules,
        &exec_call("make all", ""),
    )
    .await
    .expect("approved");
    assert_eq!(
        result,
        SandboxOverride::Approved {
            effect: Box::new(SandboxOverride::Unsandboxed),
            kind: SandboxApprovalKind::Rule,
            for_session: false,
            prefix: None,
        }
    );
    assert_eq!(
        host.trace.lock().expect("trace").as_slice(),
        &[HostTrace::Approval(SandboxApprovalKind::Rule)]
    );

    let mut host = RecordingHost::approving(SandboxApprovalDecision::Denied);
    let denied = decide(
        &mut host,
        plan(EscalationPolicy::Ask),
        &rules,
        &exec_call("make all", ""),
    )
    .await;
    let Err(denied) = denied else {
        panic!("expected denial");
    };
    assert_eq!(failure_code(denied), ToolErrorCode::SandboxDenied);
}

#[tokio::test]
async fn prompt_rule_is_denied_when_escalation_is_never() {
    let rules = policy(r#"prefix_rule(pattern = ["make"], decision = "prompt")"#);
    let mut host = RecordingHost::approving(SandboxApprovalDecision::Once);
    let result = decide(
        &mut host,
        plan(EscalationPolicy::Never),
        &rules,
        &exec_call("make all", ""),
    )
    .await;
    let Err(result) = result else {
        panic!("expected denial");
    };
    assert_eq!(failure_code(result), ToolErrorCode::SandboxDenied);
    assert!(host.trace.lock().expect("trace").is_empty());
}

#[tokio::test]
async fn explicit_escalation_still_approves_without_rules() {
    let rules = Policy::empty();
    let mut host = RecordingHost::approving(SandboxApprovalDecision::Once);
    let call = exec_call(
        "cargo publish",
        r#","sandbox_permissions":"require_escalated","justification":"publish""#,
    );
    let result = decide(&mut host, plan(EscalationPolicy::Ask), &rules, &call)
        .await
        .expect("approved");
    assert_eq!(
        result,
        SandboxOverride::Approved {
            effect: Box::new(SandboxOverride::Unsandboxed),
            kind: SandboxApprovalKind::Escalation,
            for_session: false,
            prefix: None,
        }
    );
    assert_eq!(
        host.trace.lock().expect("trace").as_slice(),
        &[HostTrace::Approval(SandboxApprovalKind::Escalation)]
    );
}

#[tokio::test]
async fn session_exact_rule_runs_before_asking() {
    let rules = Policy::empty();
    let mut host = RecordingHost::new();
    let plan = plan(EscalationPolicy::Ask)
        .with_exact_rules(vec![vec!["touch".into(), "/tmp/x".into()]], Vec::new());
    let result = decide(&mut host, plan, &rules, &exec_call("touch /tmp/x", ""))
        .await
        .expect("session rule");
    assert_eq!(result, SandboxOverride::Unsandboxed);
    assert!(host.trace.lock().expect("trace").is_empty());
}

#[tokio::test]
async fn default_command_keeps_the_sandbox() {
    let rules = policy(r#"prefix_rule(pattern = ["git", "status"], decision = "allow")"#);
    let mut host = RecordingHost::new();
    let result = decide(
        &mut host,
        plan(EscalationPolicy::Ask),
        &rules,
        &exec_call("cargo build", ""),
    )
    .await
    .expect("default");
    assert_eq!(result, SandboxOverride::None);
    assert!(host.trace.lock().expect("trace").is_empty());
}

#[tokio::test]
async fn session_exact_rule_beats_rule_prompt() {
    let rules = policy(r#"prefix_rule(pattern = ["make"], decision = "prompt")"#);
    let mut host = RecordingHost::new();
    let plan = plan(EscalationPolicy::Ask)
        .with_exact_rules(vec![vec!["make".into(), "all".into()]], Vec::new());
    let result = decide(&mut host, plan, &rules, &exec_call("make all", ""))
        .await
        .expect("session rule");
    assert_eq!(result, SandboxOverride::Unsandboxed);
    assert!(
        host.trace.lock().expect("trace").is_empty(),
        "规则 prompt 不得覆盖会话精确规则"
    );
}

#[tokio::test]
async fn model_request_beats_rule_prompt() {
    let rules = policy(r#"prefix_rule(pattern = ["make"], decision = "prompt")"#);
    let mut host = RecordingHost::approving(SandboxApprovalDecision::Once);
    let call = exec_call(
        "make all",
        r#","sandbox_permissions":"require_escalated","justification":"build""#,
    );
    let result = decide(&mut host, plan(EscalationPolicy::Ask), &rules, &call)
        .await
        .expect("approved");
    assert_eq!(
        result,
        SandboxOverride::Approved {
            effect: Box::new(SandboxOverride::Unsandboxed),
            kind: SandboxApprovalKind::Escalation,
            for_session: false,
            prefix: None,
        }
    );
    assert_eq!(
        host.trace.lock().expect("trace").as_slice(),
        &[HostTrace::Approval(SandboxApprovalKind::Escalation)],
        "模型请求优先于规则 prompt"
    );
}

#[tokio::test]
async fn forbidden_rule_beats_session_exact_rule() {
    let rules = policy(r#"prefix_rule(pattern = ["rm"], decision = "forbidden")"#);
    let mut host = RecordingHost::new();
    let plan = plan(EscalationPolicy::Ask).with_exact_rules(
        vec![vec!["rm".into(), "-rf".into(), "/".into()]],
        Vec::new(),
    );
    let result = decide(&mut host, plan, &rules, &exec_call("rm -rf /", "")).await;
    let Err(result) = result else {
        panic!("expected denial");
    };
    assert_eq!(failure_code(result), ToolErrorCode::SandboxDenied);
}

#[tokio::test]
async fn deny_only_additional_permissions_skip_approval() {
    let rules = Policy::empty();
    let mut host = RecordingHost::new();
    let call = exec_call(
        "printf x > /tmp/deny-probe",
        r#","sandbox_permissions":"with_additional_permissions","justification":"shrink","additional_permissions":{"file_system":{"deny":["/tmp/deny-probe"]}}"#,
    );
    let result = decide(&mut host, plan(EscalationPolicy::Ask), &rules, &call)
        .await
        .expect("deny-only request");
    assert_eq!(
        result,
        SandboxOverride::Additional {
            network: None,
            file_system: Some(crate::chat::sandbox::FileSystemRequest {
                deny: vec!["/tmp/deny-probe".into()],
                ..crate::chat::sandbox::FileSystemRequest::default()
            }),
        }
    );
    assert!(
        host.trace.lock().expect("trace").is_empty(),
        "deny-only 不得弹审批"
    );
}

#[tokio::test]
async fn allow_rule_survives_escalation_never() {
    let rules = policy(r#"prefix_rule(pattern = ["echo"], decision = "allow")"#);
    let mut host = RecordingHost::new();
    let result = decide(
        &mut host,
        plan(EscalationPolicy::Never),
        &rules,
        &exec_call("echo hi", ""),
    )
    .await
    .expect("allow");
    assert_eq!(result, SandboxOverride::Unsandboxed);
}

#[tokio::test]
async fn maybe_retry_skips_already_unsandboxed_or_session_approved_commands() {
    use super::super::sandbox_decision::maybe_retry_on_denial;
    use crate::chat::process::{ProcessResult, ProcessResultKind, ProcessState};
    use crate::db::chat::ToolCallTicket;

    let mut host = RecordingHost::approving(SandboxApprovalDecision::Once);
    let executor = PolicyExecutor {
        plan: plan(EscalationPolicy::Ask)
            .with_exact_rules(vec![vec!["ls".into(), "/root".into()]], Vec::new()),
    };
    let call = exec_call("ls /root", "");
    let ticket = ToolCallTicket {
        session_id: "s".into(),
        message_id: "m".into(),
        call_id: "c".into(),
        position: 0,
        started_at_ms: 0,
    };
    let denied_output = ToolExecutionResult::Process(Box::new(ProcessResult {
        kind: ProcessResultKind::ExecCommand,
        session_id: 1,
        command: "ls /root".into(),
        cwd: "/".into(),
        state: ProcessState::Exited { code: Some(1) },
        exit_code: Some(1),
        output: "Permission denied".into(),
        output_start: 0,
        output_end: 17,
        dropped_bytes: 0,
        truncated: false,
        wrote_input: false,
        input: None,
        terminate: false,
        tty: false,
        chunk_id: "000001".into(),
        wall_time_ms: 5,
        sandbox_retry: None,
    }));

    // 已按 Unsandboxed 跑过的命令不得再次弹 retry 审批。
    let out1 = maybe_retry_on_denial(
        &mut host,
        &executor,
        &call,
        &ticket,
        Path::new("/"),
        Some(&SandboxOverride::Unsandboxed),
        denied_output.clone(),
    )
    .await;
    assert!(matches!(out1, ToolExecutionResult::Process(_)));
    assert!(
        host.trace.lock().expect("trace").is_empty(),
        "Unsandboxed 失败不应再问 retry"
    );

    // 命中会话精确批准规则（裸跑）的命令同样不得再次弹 retry 审批。
    let out2 = maybe_retry_on_denial(
        &mut host,
        &executor,
        &call,
        &ticket,
        Path::new("/"),
        Some(&SandboxOverride::None),
        denied_output,
    )
    .await;
    assert!(matches!(out2, ToolExecutionResult::Process(_)));
    assert!(
        host.trace.lock().expect("trace").is_empty(),
        "会话已放行命令不应再问 retry"
    );
}

#[tokio::test]
async fn session_prefix_rule_skips_rule_prompt_while_forbidden_still_blocks() {
    let rules = policy(
        r#"
prefix_rule(pattern = ["make"], decision = "prompt", justification = "confirm build")
prefix_rule(pattern = ["make", "danger"], decision = "forbidden", justification = "blocked target")
"#,
    );
    let mut host = RecordingHost::new();
    let approved_plan =
        plan(EscalationPolicy::Ask).with_exact_rules(Vec::new(), vec![vec!["make".into()]]);
    let ok = decide(
        &mut host,
        approved_plan.clone(),
        &rules,
        &exec_call("make all", ""),
    )
    .await
    .expect("prefix rule bypasses prompt");
    assert_eq!(ok, SandboxOverride::Unsandboxed);
    assert!(
        host.trace.lock().expect("trace").is_empty(),
        "已记住的会话前缀规则不得再次触发 Rule 弹窗"
    );

    let blocked = decide(
        &mut host,
        approved_plan,
        &rules,
        &exec_call("make danger", ""),
    )
    .await;
    let Err(blocked) = blocked else {
        panic!("forbidden must beat session prefix rule");
    };
    assert_eq!(failure_code(blocked), ToolErrorCode::SandboxDenied);
}

#[tokio::test]
async fn maybe_retry_skips_write_stdin_even_with_permission_denied() {
    use super::super::sandbox_decision::maybe_retry_on_denial;
    use crate::chat::process::{ProcessResult, ProcessResultKind, ProcessState};
    use crate::db::chat::ToolCallTicket;

    let mut host = RecordingHost::approving(SandboxApprovalDecision::Once);
    let mut test_plan = plan(EscalationPolicy::Ask);
    test_plan.support = crate::platform::sandbox::SandboxSupport::Seatbelt;
    let executor = PolicyExecutor { plan: test_plan };
    let stdin_call = ReadyToolCall {
        call_id: "call-stdin".into(),
        name: "write_stdin".into(),
        arguments_json: r#"{"session_id":1,"chars":"x\n"}"#.into(),
        ordinal: 0,
    };
    let ticket = ToolCallTicket {
        session_id: "s".into(),
        message_id: "m".into(),
        call_id: "c".into(),
        position: 0,
        started_at_ms: 0,
    };
    let process_result = ToolExecutionResult::Process(Box::new(ProcessResult {
        kind: ProcessResultKind::WriteStdin,
        session_id: 1,
        command: "cat".into(),
        cwd: "/".into(),
        state: ProcessState::Exited { code: Some(1) },
        exit_code: Some(1),
        output: "Permission denied".into(),
        output_start: 0,
        output_end: 17,
        dropped_bytes: 0,
        truncated: false,
        wrote_input: true,
        input: Some("x\\n".into()),
        terminate: false,
        tty: false,
        chunk_id: "000001".into(),
        wall_time_ms: 5,
        sandbox_retry: None,
    }));

    let out = maybe_retry_on_denial(
        &mut host,
        &executor,
        &stdin_call,
        &ticket,
        Path::new("/"),
        Some(&SandboxOverride::None),
        process_result,
    )
    .await;
    assert!(matches!(out, ToolExecutionResult::Process(_)));
    assert!(
        host.trace.lock().expect("trace").is_empty(),
        "write_stdin 不得触发 on-failure 重试审批"
    );
}

#[tokio::test]
async fn file_tools_only_skips_unsandboxed_escalation_approval() {
    let rules = Policy::empty();
    let mut host = RecordingHost::approving(SandboxApprovalDecision::Denied);
    let mut win_plan = plan(EscalationPolicy::Ask);
    win_plan.support = crate::platform::sandbox::SandboxSupport::FileToolsOnly;
    let call = exec_call(
        "cargo test",
        r#","sandbox_permissions":"require_escalated","justification":"run test""#,
    );
    let result = decide(&mut host, win_plan, &rules, &call)
        .await
        .expect("file-tools-only skips require_escalated prompt");
    assert_eq!(result, SandboxOverride::None);
    assert!(host.trace.lock().expect("trace").is_empty());
}

#[tokio::test]
async fn approved_for_session_produces_summary_on_override() {
    let rules = policy(r#"prefix_rule(pattern = ["cargo", "publish"], decision = "prompt")"#);
    let mut host = RecordingHost::approving(SandboxApprovalDecision::ForSession);
    let result = decide(
        &mut host,
        plan(EscalationPolicy::Ask),
        &rules,
        &exec_call("cargo publish --dry-run", ""),
    )
    .await
    .expect("approved for session");
    let summary = result
        .approval_summary("cargo publish --dry-run")
        .expect("summary");
    assert!(summary.for_session);
    assert_eq!(summary.kind, SandboxApprovalKind::Rule);
    assert_eq!(summary.prefix_range, Some((0, 13)));
}
