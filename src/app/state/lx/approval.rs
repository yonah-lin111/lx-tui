//! 沙箱审批命令面板的挂起状态（FIFO）。
//!
//! 边界：纯状态；按键路由与点击确认在 `app::update::chat::approval`，
//! 回执与 IO 在 `event::chat`。

use crate::chat::sandbox::{SandboxApprovalDecision, SandboxApprovalKind, SandboxApprovalRequest};
use crate::ui::text;

use super::RequestKey;
use super::slash::PanelWindow;

/// 审批面板中的单条可执行选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalOption {
    pub label: &'static str,
    pub preview: String,
    pub decision: SandboxApprovalDecision,
}

/// 一条待处理的沙箱审批。
#[derive(Debug, Clone)]
pub struct ApprovalState {
    /// 归属键。
    pub key: RequestKey,
    /// 工具调用 id。
    pub call_id: String,
    /// 展示内容。
    pub request: SandboxApprovalRequest,
    /// 高亮与滚动窗口（与 `/model` 共用同一套面板窗口状态）。
    pub window: PanelWindow,
}

impl ApprovalState {
    /// 构造一条审批（默认高亮第一项）。
    pub fn new(key: RequestKey, call_id: String, request: SandboxApprovalRequest) -> Self {
        Self {
            key,
            call_id,
            request,
            window: PanelWindow::new(0),
        }
    }

    /// 当前审批的可选裁决项列表（顺序固定）。
    pub fn options(&self) -> Vec<ApprovalOption> {
        let mut out = Vec::with_capacity(3);
        match self.request.kind {
            SandboxApprovalKind::Escalation => {
                out.push(ApprovalOption {
                    label: text::LX_APPROVAL_OPT_ONCE,
                    preview: String::new(),
                    decision: SandboxApprovalDecision::Once,
                });
                if self.request.can_approve_for_session() {
                    let prefix = self
                        .request
                        .prefix_rule
                        .as_deref()
                        .map(|tokens| tokens.join(" "))
                        .unwrap_or_default();
                    out.push(ApprovalOption {
                        label: text::LX_APPROVAL_OPT_SESSION,
                        preview: prefix,
                        decision: SandboxApprovalDecision::ForSession,
                    });
                }
                out.push(ApprovalOption {
                    label: text::LX_APPROVAL_OPT_DENY,
                    preview: String::new(),
                    decision: SandboxApprovalDecision::Denied,
                });
            }
            SandboxApprovalKind::Retry => {
                out.push(ApprovalOption {
                    label: text::LX_APPROVAL_OPT_RETRY_ONCE,
                    preview: String::new(),
                    decision: SandboxApprovalDecision::Once,
                });
                out.push(ApprovalOption {
                    label: text::LX_APPROVAL_OPT_RETRY_SESSION,
                    preview: text::LX_APPROVAL_OPT_SESSION_PREVIEW.to_string(),
                    decision: SandboxApprovalDecision::ForSession,
                });
                out.push(ApprovalOption {
                    label: text::LX_APPROVAL_OPT_KEEP_BLOCKED,
                    preview: String::new(),
                    decision: SandboxApprovalDecision::Denied,
                });
            }
            SandboxApprovalKind::Rule => {
                out.push(ApprovalOption {
                    label: text::LX_APPROVAL_OPT_ONCE,
                    preview: String::new(),
                    decision: SandboxApprovalDecision::Once,
                });
                if self.request.can_approve_for_session() {
                    let prefix = self
                        .request
                        .prefix_rule
                        .as_deref()
                        .map(|tokens| tokens.join(" "))
                        .unwrap_or_default();
                    out.push(ApprovalOption {
                        label: text::LX_APPROVAL_OPT_ALWAYS_ALLOW,
                        preview: prefix,
                        decision: SandboxApprovalDecision::ForSession,
                    });
                }
                out.push(ApprovalOption {
                    label: text::LX_APPROVAL_OPT_DENY,
                    preview: String::new(),
                    decision: SandboxApprovalDecision::Denied,
                });
            }
        }
        out
    }

    /// 键盘上下循环移动高亮。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = self.options().len();
        if len == 0 {
            return false;
        }
        self.window.move_active(delta, len);
        true
    }

    /// 鼠标悬停设置高亮；越界或未变化返回 false。
    pub fn set_active(&mut self, index: usize) -> bool {
        let len = self.options().len();
        if index >= len || self.window.active == index {
            return false;
        }
        self.window.set_active(index);
        true
    }

    /// 当前高亮选项对应的裁决。
    pub fn active_decision(&self) -> SandboxApprovalDecision {
        self.options()
            .get(self.window.active)
            .map_or(SandboxApprovalDecision::Denied, |opt| opt.decision)
    }

    /// 指定索引对应的裁决。
    pub fn decision_at(&self, index: usize) -> Option<SandboxApprovalDecision> {
        self.options().get(index).map(|opt| opt.decision)
    }
}
