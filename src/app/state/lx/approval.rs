//! 沙箱审批 strip 的挂起状态（FIFO）。
//!
//! 边界：纯状态；按键路由在 `app::update::chat::approval`，回执与 IO 在 `event::chat`。

use crate::chat::sandbox::SandboxApprovalRequest;

use super::RequestKey;

/// 一条待处理的沙箱审批。
#[derive(Debug, Clone)]
pub struct ApprovalState {
    /// 归属键。
    pub key: RequestKey,
    /// 工具调用 id。
    pub call_id: String,
    /// 展示内容。
    pub request: SandboxApprovalRequest,
}

impl ApprovalState {
    /// 构造一条审批。
    pub fn new(key: RequestKey, call_id: String, request: SandboxApprovalRequest) -> Self {
        Self {
            key,
            call_id,
            request,
        }
    }
}
