//! 统一聊天数据事件：协议适配器的唯一输出形态。
//!
//! 事件只携数据：网络模块不直接修改 `AppState`；`request_key` 在发起时固定，
//! 事件层按 `(pane, session, generation)` 过滤过期结果。适配器必须恰好发出
//! 一个终态（`Completed` 与 `Failed`/`Cancelled` 互斥）。

use crate::db::chat::Usage;

/// 内容块种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// 正文。
    Text,
    /// 思考/推理内容。
    Reasoning,
}

impl PartKind {
    /// 存储稳定值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Reasoning => "reasoning",
        }
    }
}

/// 一个增量内容块（part 顺序按收到顺序严格递增）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartDelta {
    pub kind: PartKind,
    pub text: String,
    /// 供应商专属元数据（如 Anthropic signature）；与该 part 绑定。
    pub metadata: Option<serde_json::Value>,
}

/// 流式增量事件。
#[derive(Debug, Clone, PartialEq)]
pub enum ChatDelta {
    /// 追加一个内容块。
    Part(PartDelta),
    /// 供应商返回的用量；只报实际值，未知保持 None。
    Usage(Usage),
    /// 结束原因。
    Finished { reason: StopReason },
}

/// 结束原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// 正常完成。
    Complete,
    /// 用户取消。
    Cancelled,
    /// 连接/协议/HTTP 错误（截断后的稳定错误码，不含密钥与原始响应）。
    Failed { code: String },
}

impl StopReason {
    /// DB 稳定 stop reason 字符串。
    pub fn as_str(&self) -> &str {
        match self {
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Failed { code } => code,
        }
    }
}

/// 一条完整聊天消息（上下文组装与请求编码共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTurn {
    /// `user` / `assistant`。
    pub role: String,
    pub text: String,
    /// 供应商思考元数据（跨轮回传用）；跨 provider 不携带。
    pub reasoning_metadata: Option<String>,
}

/// 一次流式请求的输入快照。
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// 已完成的上下文轮次（含当前用户消息，原顺序）。
    pub turns: Vec<ChatTurn>,
    /// 用户可见的系统级错误码（已发生的失败原因）。
    pub model: crate::db::chat::ModelSnapshot,
    /// 思考等级参数对象（来自模型 variants）；None 时不发送伪参数。
    pub variant_params: Option<serde_json::Value>,
}

/// 单帧字节上限（64 KiB）：超限按协议错误失败。
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// 单轮累计正文字节上限（2 MiB）：超限失败并保留此前 partial。
pub const MAX_TOTAL_TEXT_BYTES: usize = 2 * 1024 * 1024;
