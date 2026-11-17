//! 一次执行的沙箱计划：模式、权限档案、网络与平台能力。
//!
//! 计划在 turn 起点冻结；`/permission` 只影响下一个 turn。平台层只读计划，
//! 不访问全局状态。完整文件系统档案见 [`permissions`]。

use std::path::PathBuf;

use crate::config::sandbox::SandboxSettings;
use crate::config::sandbox::{EscalationPolicy, SandboxMode};
use crate::platform::sandbox::{self, SandboxSpec, SandboxSupport};

pub use permissions::{
    AdditionalPermissions, FileSystemAccessMode, FileSystemEntry, FileSystemPath, FileSystemPolicy,
    FileSystemRequest, PermissionError,
};

pub mod permissions;

/// 本次命令的沙箱处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSandbox {
    /// 用平台后端包装。
    Wrap,
    /// 不套进程沙箱（danger-full-access 或无进程沙箱的平台）。
    None,
    /// 受限档但平台后端缺失：fail-closed。
    Unavailable,
}

/// 审批种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxApprovalKind {
    /// 模型显式申请提权。
    Escalation,
    /// 沙箱拒绝后的重试申请（A 批 §8.5）。
    Retry,
    /// 规则文件命中的 `prompt`（C 批 §18.3）。
    Rule,
}

impl SandboxApprovalKind {
    /// 稳定标识。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Escalation => "escalation",
            Self::Retry => "retry",
            Self::Rule => "rule",
        }
    }
}

/// 一次沙箱审批请求（UI 只读展示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxApprovalRequest {
    /// 审批种类。
    pub kind: SandboxApprovalKind,
    /// 命令原文（已截断）。
    pub command: String,
    /// 实际工作目录。
    pub cwd: String,
    /// 模型给出的理由或规则 justification（不回过模型）。
    pub justification: String,
    /// 可申请的会话前缀规则。
    pub prefix_rule: Option<Vec<String>>,
    /// 附加权限（仅 `with_additional_permissions`）。
    pub additional: Option<AdditionalPermissions>,
    /// 被沙箱拒绝时的输出摘录（仅 on-failure）。
    pub denial_excerpt: Option<String>,
    /// 来源标签（子 agent 名称等）。
    pub source: Option<String>,
}

impl SandboxApprovalRequest {
    /// 是否允许「批准一次」以外的会话级批准。
    pub fn can_approve_for_session(&self) -> bool {
        match self.kind {
            SandboxApprovalKind::Escalation => self.prefix_rule.is_some(),
            SandboxApprovalKind::Retry | SandboxApprovalKind::Rule => true,
        }
    }
}

/// 用户对审批的裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxApprovalDecision {
    /// 仅本次命令。
    Once,
    /// 本会话内同类命令（前缀/精确命令）自动放行。
    ForSession,
    /// 拒绝。
    Denied,
}

/// 单次命令的提权覆盖。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SandboxOverride {
    /// 按会话模式执行。
    #[default]
    None,
    /// 追加本次命令的网络与文件系统权限（`with_additional_permissions`）。
    Additional {
        /// 是否允许网络（None = 不改变）。
        network: Option<bool>,
        /// 文件系统请求。
        file_system: Option<FileSystemRequest>,
    },
    /// 完全绕过沙箱（已审批的 `require_escalated` 或会话规则）。
    Unsandboxed,
    /// 经用户显式审批通过后的提权覆盖（供执行层在结果文本中回显批准摘要）。
    Approved {
        /// 实际生效的沙箱覆盖。
        effect: Box<SandboxOverride>,
        /// 审批种类（`Escalation` / `Rule`）。
        kind: SandboxApprovalKind,
        /// 本次是否记入会话级规则。
        for_session: bool,
        /// 已批准的会话前缀（若有）。
        prefix: Option<Vec<String>>,
    },
}

impl SandboxOverride {
    /// 该覆盖是否将命令转为完全不套进程沙箱执行。
    pub fn is_unsandboxed(&self) -> bool {
        match self {
            Self::Unsandboxed => true,
            Self::Approved { effect, .. } => effect.is_unsandboxed(),
            Self::None | Self::Additional { .. } => false,
        }
    }

    /// 构造带批准摘要的 [`SandboxRetry`]（仅在用户显式批准过该调用时返回 `Some`）。
    pub fn approval_summary(&self, command: &str) -> Option<SandboxRetry> {
        let Self::Approved {
            effect,
            kind,
            for_session,
            prefix,
        } = self
        else {
            return None;
        };
        let additional = matches!(effect.as_ref(), Self::Additional { .. });
        let prefix_range = prefix
            .as_deref()
            .and_then(|tokens| find_prefix_range(command, tokens));
        Some(SandboxRetry {
            approved: true,
            for_session: *for_session,
            kind: *kind,
            additional,
            prefix_range,
        })
    }
}

/// 冻结后的沙箱计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPlan {
    /// 生效模式。
    pub mode: SandboxMode,
    /// 完整文件系统档案（基准 + 本次命令追加）。
    pub policy: FileSystemPolicy,
    /// 可写根（由 `policy` 派生）。
    pub writable_roots: Vec<PathBuf>,
    /// 写根内的只读保护路径（`.git` 等 + 显式 `read` carveout）。
    pub protected: Vec<PathBuf>,
    /// 显式拒绝的字面路径。
    pub deny_paths: Vec<PathBuf>,
    /// 显式拒绝的 glob。
    pub deny_globs: Vec<String>,
    /// 是否允许网络。
    pub network_access: bool,
    /// 提权总开关。
    pub escalation: EscalationPolicy,
    /// 本会话已批准、可直接裸跑的精确命令（token 序列）。
    pub approved_exact: Vec<Vec<String>>,
    /// 本会话已批准的前缀规则。
    pub approved_prefix: Vec<Vec<String>>,
    /// 平台能力。
    pub support: SandboxSupport,
}

impl SandboxPlan {
    /// 按全局配置与工作区构建计划。
    pub fn for_turn(settings: &SandboxSettings, cwd: &std::path::Path) -> Self {
        let mut plan = Self {
            mode: settings.mode,
            policy: FileSystemPolicy::baseline(settings.mode, cwd, &settings.writable_roots),
            writable_roots: Vec::new(),
            protected: Vec::new(),
            deny_paths: Vec::new(),
            deny_globs: Vec::new(),
            network_access: settings.network_access,
            escalation: settings.escalation,
            approved_exact: Vec::new(),
            approved_prefix: Vec::new(),
            support: sandbox::support(),
        };
        plan.refresh_derived();
        if settings.mode == SandboxMode::DangerFullAccess {
            plan.network_access = true;
        }
        plan
    }

    /// 不套沙箱的计划（用户 `!` 命令、危险模式）。
    pub fn unsandboxed() -> Self {
        let mut plan = Self {
            mode: SandboxMode::DangerFullAccess,
            policy: FileSystemPolicy::empty(),
            writable_roots: Vec::new(),
            protected: Vec::new(),
            deny_paths: Vec::new(),
            deny_globs: Vec::new(),
            network_access: true,
            escalation: EscalationPolicy::Never,
            approved_exact: Vec::new(),
            approved_prefix: Vec::new(),
            support: sandbox::support(),
        };
        plan.refresh_derived();
        plan
    }

    /// 本次命令是否真的套进程沙箱。
    pub fn process_sandbox(&self) -> bool {
        self.mode.process_sandboxed() && self.support.supports_process_sandbox()
    }

    /// 应用本次命令的提权覆盖。
    pub fn with_override(&self, overridden: &SandboxOverride) -> Self {
        match overridden {
            SandboxOverride::None => self.clone(),
            SandboxOverride::Unsandboxed => {
                let mut plan = self.clone();
                plan.mode = SandboxMode::DangerFullAccess;
                plan.policy = FileSystemPolicy::empty();
                plan.network_access = true;
                plan.refresh_derived();
                plan
            }
            SandboxOverride::Additional {
                network,
                file_system,
            } => {
                let mut plan = self.clone();
                if let Some(request) = file_system {
                    match self.policy.with_request(request) {
                        Ok(policy) => plan.policy = policy,
                        Err(error) => {
                            tracing::warn!(%error, "invalid additional permissions; keeping the base policy");
                        }
                    }
                }
                plan.network_access = plan.network_access || network.unwrap_or(false);
                plan.refresh_derived();
                plan
            }
            SandboxOverride::Approved { effect, .. } => self.with_override(effect),
        }
    }

    /// 注入本会话已批准的精确命令与前缀规则。
    pub fn with_exact_rules(mut self, exact: Vec<Vec<String>>, prefix: Vec<Vec<String>>) -> Self {
        self.approved_exact = exact;
        self.approved_prefix = prefix;
        self
    }

    /// 从数据库会话规则记录注入精确命令与前缀规则。
    pub fn with_rules(mut self, rules: &[crate::db::sandbox::SessionSandboxRule]) -> Self {
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        for rule in rules {
            match rule.kind {
                crate::db::sandbox::SandboxRuleKind::Exact => exact.push(rule.tokens.clone()),
                crate::db::sandbox::SandboxRuleKind::Prefix => prefix.push(rule.tokens.clone()),
            }
        }
        self.approved_exact = exact;
        self.approved_prefix = prefix;
        self
    }

    /// 命令 token 是否已在会话内获准裸跑（整条序列全等或前缀命中）。
    ///
    /// 前缀规则仅在命令不含任何 shell 元字符时生效，防止 `"cargo publish && evil"` 绕过沙箱；
    /// 精确命令规则仍匹配完整的 token 序列。
    pub fn command_is_approved(&self, command: &str) -> bool {
        if self.approved_exact.is_empty() && self.approved_prefix.is_empty() {
            return false;
        }
        let Some(tokens) = shlex::split(command) else {
            return false;
        };
        if self
            .approved_exact
            .iter()
            .any(|rule| rule.as_slice() == tokens.as_slice())
        {
            return true;
        }
        if self.approved_prefix.is_empty() {
            return false;
        }
        let Some(segments) = crate::chat::exec_policy::split_shell_segments(command) else {
            return false;
        };
        segments.iter().all(|segment| {
            self.approved_prefix
                .iter()
                .any(|rule| segment.len() >= rule.len() && segment.starts_with(rule))
        })
    }

    /// 本次命令的沙箱处置。
    pub fn command_sandbox(&self) -> CommandSandbox {
        if !self.mode.process_sandboxed() || self.support == SandboxSupport::FileToolsOnly {
            return CommandSandbox::None;
        }
        if self.support.supports_process_sandbox() {
            CommandSandbox::Wrap
        } else {
            CommandSandbox::Unavailable
        }
    }

    /// 平台层包装规格（调用方仅在 `CommandSandbox::Wrap` 时使用）。
    pub fn spec<'a>(&'a self, cwd: &'a std::path::Path) -> SandboxSpec<'a> {
        SandboxSpec {
            cwd,
            writable_roots: &self.writable_roots,
            read_only_paths: &self.protected,
            deny_paths: &self.deny_paths,
            deny_globs: &self.deny_globs,
            glob_scan_max_depth: self.policy.glob_scan_max_depth,
            network_access: self.network_access,
        }
    }

    /// 文件工具使用的只读策略视图。
    pub fn policy_view(&self) -> SandboxPolicyView {
        SandboxPolicyView {
            mode: self.mode,
            policy: self.policy.clone(),
        }
    }

    fn refresh_derived(&mut self) {
        self.writable_roots = self.policy.writable_roots();
        self.protected = self.policy.protected_paths();
        self.deny_paths = self.policy.deny_paths();
        self.deny_globs = self.policy.deny_globs();
    }
}

/// 文件工具闸门读取的策略快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPolicyView {
    /// 生效模式。
    pub mode: SandboxMode,
    /// 完整文件系统档案。
    pub policy: FileSystemPolicy,
}

pub mod denial;

/// 一次沙箱重试或显式审批的标记（保持 `Copy` 以兼容 `ProcessResult::to_json`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxRetry {
    /// 是否由用户批准。
    pub approved: bool,
    /// 会话级规则是否已写入。
    pub for_session: bool,
    /// 审批种类（重试 / 显式提权 / 规则询问）。
    pub kind: SandboxApprovalKind,
    /// 是否为 `with_additional_permissions`。
    pub additional: bool,
    /// 已批准的会话前缀在 `ProcessResult.command` 中的字节区间。
    pub prefix_range: Option<(u16, u16)>,
}

fn find_prefix_range(command: &str, prefix: &[String]) -> Option<(u16, u16)> {
    if prefix.is_empty() {
        return None;
    }
    let joined = prefix.join(" ");
    if let Some(start) = command.find(&joined) {
        let end = start.saturating_add(joined.len());
        if command.is_char_boundary(start)
            && command.is_char_boundary(end)
            && let (Ok(start), Ok(end)) = (u16::try_from(start), u16::try_from(end))
        {
            return Some((start, end));
        }
    }
    let mut spans: Vec<(usize, usize, &str)> = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, ch) in command.char_indices() {
        if ch.is_whitespace() {
            if let Some(begin) = start.take() {
                spans.push((begin, idx, &command[begin..idx]));
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(begin) = start {
        spans.push((begin, command.len(), &command[begin..]));
    }
    spans.windows(prefix.len()).find_map(|window| {
        if window
            .iter()
            .zip(prefix.iter())
            .all(|((_, _, word), expected)| *word == expected.as_str())
        {
            let first_start = u16::try_from(window.first()?.0).ok()?;
            let last_end = u16::try_from(window.last()?.1).ok()?;
            Some((first_start, last_end))
        } else {
            None
        }
    })
}

/// 把可能包含换行与多余空白的命令折叠为单行截断摘要，供审计日志安全输出。
pub fn command_preview(command: &str) -> String {
    const MAX_LOG_CMD_CHARS: usize = 120;
    let collapsed = command.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(collapsed.len().min(MAX_LOG_CMD_CHARS));
    for (idx, ch) in collapsed.chars().enumerate() {
        if idx >= MAX_LOG_CMD_CHARS {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests;
