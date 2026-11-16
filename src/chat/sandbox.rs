//! 一次执行的沙箱计划：模式、可写根、保护路径、网络与平台能力。
//!
//! 计划在 turn 起点冻结；`/permission` 只影响下一个 turn。平台层只读计划，
//! 不访问全局状态。

use std::path::PathBuf;

use crate::config::sandbox::SandboxSettings;
use crate::config::sandbox::{EscalationPolicy, SandboxMode};
use crate::platform::sandbox::{self, SandboxSupport};

/// 单次命令的提权覆盖。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SandboxOverride {
    /// 按会话模式执行。
    #[default]
    None,
    /// 追加本次命令的网络与可写根（`with_additional_permissions`）。
    Additional {
        /// 是否允许网络。
        network: bool,
        /// 追加的可写根（绝对路径）。
        write_roots: Vec<PathBuf>,
    },
    /// 完全绕过沙箱（已审批的 `require_escalated` 或会话精确规则）。
    Unsandboxed,
}

/// 冻结后的沙箱计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPlan {
    /// 生效模式。
    pub mode: SandboxMode,
    /// 可写根（已规范化）。
    pub writable_roots: Vec<PathBuf>,
    /// 保护路径（只读）。
    pub protected: Vec<PathBuf>,
    /// 是否允许网络。
    pub network_access: bool,
    /// 提权总开关。
    pub escalation: EscalationPolicy,
    /// 平台能力。
    pub support: SandboxSupport,
}

impl SandboxPlan {
    /// 按全局配置与工作区构建计划。
    pub fn for_turn(settings: &SandboxSettings, cwd: &std::path::Path) -> Self {
        let support = sandbox::support();
        let mut plan = Self {
            mode: settings.mode,
            writable_roots: Vec::new(),
            protected: Vec::new(),
            network_access: settings.network_access,
            escalation: settings.escalation,
            support,
        };
        if settings.mode == SandboxMode::WorkspaceWrite {
            plan.writable_roots = sandbox::roots::writable_roots(cwd, &settings.writable_roots);
            plan.protected = sandbox::roots::protected_paths(&plan.writable_roots);
        }
        if settings.mode == SandboxMode::DangerFullAccess {
            plan.network_access = true;
        }
        plan
    }

    /// 不套沙箱的计划（用户 `!` 命令、危险模式）。
    pub fn unsandboxed() -> Self {
        Self {
            mode: SandboxMode::DangerFullAccess,
            writable_roots: Vec::new(),
            protected: Vec::new(),
            network_access: true,
            escalation: EscalationPolicy::Never,
            support: sandbox::support(),
        }
    }

    /// 本次命令是否真的套进程沙箱。
    pub fn process_sandbox(&self) -> bool {
        self.mode.process_sandboxed() && self.support.supports_process_sandbox()
    }

    /// 应用本次命令的提权覆盖。
    pub fn with_override(&self, overridden: &SandboxOverride) -> Self {
        match overridden {
            SandboxOverride::None => self.clone(),
            SandboxOverride::Unsandboxed => Self {
                mode: SandboxMode::DangerFullAccess,
                writable_roots: Vec::new(),
                protected: Vec::new(),
                network_access: true,
                escalation: self.escalation,
                support: self.support,
            },
            SandboxOverride::Additional {
                network,
                write_roots,
            } => {
                let mut plan = self.clone();
                for root in write_roots {
                    if !plan.writable_roots.contains(root) {
                        plan.writable_roots.push(root.clone());
                    }
                }
                plan.protected = sandbox::roots::protected_paths(&plan.writable_roots);
                plan.network_access = plan.network_access || *network;
                plan
            }
        }
    }

    /// 文件工具使用的只读策略视图。
    pub fn policy_view(&self) -> SandboxPolicyView {
        SandboxPolicyView {
            mode: self.mode,
            writable_roots: self.writable_roots.clone(),
            protected: self.protected.clone(),
        }
    }
}

/// 文件工具闸门读取的策略快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPolicyView {
    /// 生效模式。
    pub mode: SandboxMode,
    /// 可写根。
    pub writable_roots: Vec<PathBuf>,
    /// 保护路径。
    pub protected: Vec<PathBuf>,
}

#[cfg(test)]
mod tests;
