//! `exec_command` 的沙箱裁决：规则文件、会话规则与模型提权的优先级。
//!
//! 本模块只做决策与审批往返，不执行命令；执行入口在 `executor.rs`。

use std::path::Path;

use tokio::sync::oneshot;

use super::{AgentHost, AgentHostRequest, HostError, ReadyToolCall, ToolExecutor};
use crate::chat::tools::{ToolErrorCode, ToolExecutionResult, ToolFailure};
use crate::db::chat::ToolCallTicket;

/// 解析本次 `exec_command` 的执行裁决并请求审批。
///
/// 决策优先级（design §18.4）：规则 `Forbidden` → 规则 `Allow` → 会话精确规则
/// → 模型显式提权 → 规则 `Prompt` → 默认沙箱（失败后走 on-failure）。
pub(super) async fn override_for<H, E>(
    host: &mut H,
    executor: &E,
    call: &ReadyToolCall,
    cwd: &Path,
    policy: &crate::chat::exec_policy::policy::Policy,
) -> Result<crate::chat::sandbox::SandboxOverride, ToolExecutionResult>
where
    H: AgentHost,
    E: ToolExecutor,
{
    use crate::chat::exec_policy::RuleDecision;
    use crate::chat::sandbox::{
        SandboxApprovalDecision, SandboxApprovalKind, SandboxApprovalRequest, SandboxOverride,
    };
    use crate::config::sandbox::{EscalationPolicy, SandboxMode};

    if call.name != "exec_command" {
        return Ok(SandboxOverride::None);
    }
    let request = match crate::chat::tools::parse_exec_request(&call.arguments_json) {
        Ok(request) => request,
        Err(failure) => return Err(failure.into_result(Vec::new())),
    };
    let Some(plan) = executor.sandbox_plan() else {
        return Ok(SandboxOverride::None);
    };

    // 1–2. 规则文件：Forbidden 直接拒绝；安全分段全部命中 Allow 才跳过沙箱。
    let prompt_evaluation = match crate::chat::exec_policy::decide(policy, &request.args.cmd) {
        RuleDecision::Allow => return Ok(SandboxOverride::Unsandboxed),
        RuleDecision::Forbidden(evaluation) => {
            let reason = evaluation
                .justification
                .unwrap_or_else(|| "command is blocked by an exec policy rule".into());
            return Err(ToolFailure::new(
                ToolErrorCode::SandboxDenied,
                format!("blocked by exec policy: {reason}"),
            )
            .into_result(Vec::new()));
        }
        RuleDecision::Prompt(evaluation) => Some(evaluation),
        RuleDecision::None => None,
    };

    // 3. 会话精确命令规则：执行器会识别并裸跑，这里不再询问。
    if plan.command_is_approved(&request.args.cmd) {
        return Ok(SandboxOverride::None);
    }

    // 4. 模型显式提权。
    if let Some(escalation) = request.escalation {
        if plan.mode == SandboxMode::DangerFullAccess {
            return Ok(SandboxOverride::None);
        }
        // 仅收缩权限的 deny 不需要审批，也不受 escalation=never 影响。
        if escalation.kind == crate::chat::tools::EscalationKind::Additional
            && escalation
                .additional
                .as_ref()
                .is_some_and(crate::chat::sandbox::AdditionalPermissions::denies_only)
        {
            let additional = escalation.additional.unwrap_or_default();
            return Ok(SandboxOverride::Additional {
                network: additional.network,
                file_system: additional.file_system,
            });
        }
        if plan.command_sandbox() == crate::chat::sandbox::CommandSandbox::Unavailable {
            return Err(ToolFailure::new(
                ToolErrorCode::SandboxUnavailable,
                "process sandbox is unavailable on this platform; install the backend or switch to danger-full-access",
            )
            .into_result(Vec::new()));
        }
        if plan.escalation == EscalationPolicy::Never {
            return Err(ToolFailure::new(
                ToolErrorCode::SandboxDenied,
                "sandbox escalation is disabled by settings",
            )
            .into_result(Vec::new()));
        }
        let approval = SandboxApprovalRequest {
            kind: SandboxApprovalKind::Escalation,
            command: request.args.cmd.clone(),
            cwd: cwd.display().to_string(),
            justification: escalation.justification.clone(),
            prefix_rule: escalation.prefix_rule.clone(),
            additional: escalation.additional.clone(),
            denial_excerpt: None,
            source: None,
        };
        let decision = approve(host, call, approval).await?;
        if decision == SandboxApprovalDecision::Denied {
            return Err(ToolFailure::new(
                ToolErrorCode::SandboxDenied,
                "the user denied sandbox escalation.",
            )
            .into_result(Vec::new()));
        }
        return Ok(match escalation.kind {
            crate::chat::tools::EscalationKind::Unsandboxed => SandboxOverride::Unsandboxed,
            crate::chat::tools::EscalationKind::Additional => {
                let additional = escalation.additional.unwrap_or_default();
                SandboxOverride::Additional {
                    network: additional.network,
                    file_system: additional.file_system,
                }
            }
        });
    }

    // 5. 规则文件 `prompt`。
    if let Some(evaluation) = prompt_evaluation {
        if plan.escalation == EscalationPolicy::Never {
            return Err(ToolFailure::new(
                ToolErrorCode::SandboxDenied,
                "sandbox escalation is disabled by settings",
            )
            .into_result(Vec::new()));
        }
        // 危险模式本来就不套沙箱：规则 Prompt 无额外意义。
        if plan.mode == SandboxMode::DangerFullAccess {
            return Ok(SandboxOverride::None);
        }
        let approval = SandboxApprovalRequest {
            kind: SandboxApprovalKind::Rule,
            command: request.args.cmd.clone(),
            cwd: cwd.display().to_string(),
            justification: evaluation.justification.unwrap_or_default(),
            prefix_rule: Some(evaluation.matched_prefix),
            additional: None,
            denial_excerpt: None,
            source: None,
        };
        let decision = approve(host, call, approval).await?;
        if decision == SandboxApprovalDecision::Denied {
            return Err(ToolFailure::new(
                ToolErrorCode::SandboxDenied,
                "the user denied the exec policy prompt.",
            )
            .into_result(Vec::new()));
        }
        return Ok(SandboxOverride::Unsandboxed);
    }

    // 6. 默认沙箱执行。
    Ok(SandboxOverride::None)
}

/// 请求一次沙箱审批；取消/事件层失败映射为稳定工具失败。
async fn approve<H: AgentHost>(
    host: &mut H,
    call: &ReadyToolCall,
    approval: crate::chat::sandbox::SandboxApprovalRequest,
) -> Result<crate::chat::sandbox::SandboxApprovalDecision, ToolExecutionResult> {
    let (reply, receiver) = oneshot::channel();
    host.send(AgentHostRequest::ApproveSandbox {
        call_id: call.call_id.clone(),
        request: approval,
        reply,
    });
    match receiver.await {
        Ok(Ok(decision)) => Ok(decision),
        Ok(Err(error)) => Err(approval_failure(error)),
        Err(_) => Err(approval_failure(HostError::Cancelled)),
    }
}

fn approval_failure(error: HostError) -> ToolExecutionResult {
    match error {
        HostError::Cancelled => {
            ToolFailure::new(ToolErrorCode::Cancelled, "cancelled").into_result(Vec::new())
        }
        HostError::Failed { code } => {
            ToolFailure::new(ToolErrorCode::SandboxDenied, code).into_result(Vec::new())
        }
    }
}

/// on-failure：识别沙箱拒绝并请求一次重试（design §8.5）。
pub(super) async fn maybe_retry_on_denial<H, E>(
    host: &mut H,
    executor: &E,
    call: &ReadyToolCall,
    ticket: &ToolCallTicket,
    cwd: &Path,
    result: ToolExecutionResult,
) -> ToolExecutionResult
where
    H: AgentHost,
    E: ToolExecutor,
{
    let Some(plan) = executor.sandbox_plan() else {
        return result;
    };
    if plan.escalation != crate::config::sandbox::EscalationPolicy::Ask || !plan.process_sandbox() {
        return result;
    }
    let crate::chat::tools::ToolExecutionResult::Process(process) = &result else {
        return result;
    };
    if !crate::chat::sandbox::denial::is_likely_sandbox_denied(process, true)
        || process.sandbox_retry.is_some()
    {
        return result;
    }
    let excerpt: String = process.output.chars().take(400).collect();
    let request = crate::chat::sandbox::SandboxApprovalRequest {
        kind: crate::chat::sandbox::SandboxApprovalKind::Retry,
        command: process.command.clone(),
        cwd: process.cwd.clone(),
        justification: String::new(),
        prefix_rule: None,
        additional: None,
        denial_excerpt: Some(excerpt),
        source: None,
    };
    match approve(host, call, request).await {
        Ok(crate::chat::sandbox::SandboxApprovalDecision::Denied) => result,
        Ok(decision) => {
            let for_session = decision == crate::chat::sandbox::SandboxApprovalDecision::ForSession;
            let retried = executor
                .execute_tracked_sandboxed(
                    cwd,
                    call,
                    ticket,
                    crate::chat::sandbox::SandboxOverride::Unsandboxed,
                )
                .await;
            if let crate::chat::tools::ToolExecutionResult::Process(mut retried) = retried {
                retried.sandbox_retry = Some(crate::chat::sandbox::SandboxRetry {
                    approved: true,
                    for_session,
                });
                crate::chat::tools::ToolExecutionResult::Process(retried)
            } else {
                result
            }
        }
        Err(failure) => failure,
    }
}
