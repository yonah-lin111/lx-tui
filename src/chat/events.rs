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
///
/// 一次用户轮次可能包含**多次模型响应**（工具化后：响应 1 要求调用工具 → 本地执行 →
/// 响应 2 给出最终回答）。因此 `ResponseFinished`（一次响应结束）与 `Finished`
/// （整轮结束）是不同层的终态，解码器不得用前者结束用户轮次。
#[derive(Debug, Clone, PartialEq)]
pub enum ChatDelta {
    /// 追加一个内容块。
    Part(PartDelta),
    /// 供应商返回的用量；只报实际值，未知保持 None。
    Usage(Usage),
    /// 关闭当前开放（或指定 item）的思考块：协议明确的结束事件。
    ///
    /// `item_id` 为供应商侧思考块标识；协议无原生标识时为 None，此时由状态层关闭
    /// 当前开放的思考块（本组只有当前开放块，见 `LxChat::close_open_reasoning`）。
    /// 对未出现过的 item 不得创建空思考块；重复结束事件幂等。
    ReasoningEnded { item_id: Option<String> },
    /// 工具参数开始接收（此时不可执行）；`ordinal` 为供应商响应内顺序。
    ///
    /// `call_id` 是跨帧聚合与去重的**稳定键**：必须来自供应商原生 ID；
    /// 供应商无原生 ID（如 Google functionCall）时由传输层用
    /// `(generation, model step, candidate index, part index)` 合成，禁止使用当前时间戳，
    /// 否则跨帧无法聚合、重放无法识别。同一响应内同 `call_id` 重复出现即为同一调用。
    ToolInputStarted {
        call_id: String,
        name: String,
        ordinal: usize,
    },
    /// 工具参数增量（按 `call_id` 聚合）；累计超过 [`MAX_TOOL_ARGUMENTS_BYTES`] 即失败。
    ToolInputDelta { call_id: String, chunk: String },
    /// 工具参数接收完成（完整 JSON 字符串）；此时才允许进入持久化与调度。
    ToolInputEnded {
        call_id: String,
        arguments_json: String,
    },
    /// 本次**模型响应**结束（不等于用户轮次结束）。
    ResponseFinished { stop: ModelStepStop },
    /// 结束原因（整轮终态）。
    Finished { reason: StopReason },
}

/// 单次模型响应的结束原因（一次用户轮次可含多次响应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStepStop {
    /// 正常给出最终回答（无工具调用，整轮可结束）。
    Final,
    /// 本次响应要求执行工具，需要继续下一次模型响应。
    ToolCalls,
    /// 本次响应失败。
    Failed,
    /// 用户取消。
    Cancelled,
}

impl ChatDelta {
    /// 是否属于应**即时回投**给状态层的增量（整轮终态 `Finished` 单独回投）。
    pub fn is_incremental(&self) -> bool {
        !matches!(self, Self::Finished { .. })
    }

    /// 该增量携带的字符串负载字节数（正文/工具参数；用于 checkpoint 与上限估算）。
    pub fn payload_bytes(&self) -> usize {
        match self {
            Self::Part(part) => part.text.len(),
            Self::ToolInputDelta { chunk, .. } => chunk.len(),
            Self::ToolInputEnded { arguments_json, .. } => arguments_json.len(),
            _ => 0,
        }
    }
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

/// 提交给模型的图片载荷（多协议共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelImage {
    /// 图片 MIME 类型（如 `image/png`、`image/jpeg`）。
    pub mime: String,
    /// 经过缩放与规范化后的图片二进制。
    pub bytes: std::sync::Arc<Vec<u8>>,
}

/// 结构化模型历史项：工具化路径的请求表示（`turns` 只承载旧纯文本行为）。
///
/// `ChatTurn { role, text }` 无法表达 tool call / tool result 的成对结构；
/// 工具结果**不得**拼接进一段 assistant 文本充数。裁剪时调用与结果必须成对保留。
#[derive(Debug, Clone, PartialEq)]
pub enum ModelItem {
    /// 用户输入文本。
    UserText(String),
    /// 用户输入多模态内容（正文 + 图片）。
    UserContent {
        text: String,
        images: Vec<ModelImage>,
    },
    /// assistant 可见正文（reasoning 不回传）。
    AssistantText(String),
    /// assistant 发起的一次工具调用；`input` 已解析为 JSON 值。
    AssistantToolCall {
        call_id: String,
        name: String,
        input: serde_json::Value,
    },
    /// 对应 `call_id` 的工具结果；`is_error` 标记失败结果。
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

/// 一次请求的用途：决定系统提示与是否声明工具。
///
/// `/compact` 的摘要请求**不是**普通聊天轮次（不追加 user/assistant 消息、不携带工具、
/// 也不允许工具调用）；用显式用途区分，避免调用方靠“有没有 tools”来猜。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RequestPurpose {
    /// 普通对话轮次（四工具 + 稳定系统提示）。
    #[default]
    Chat,
    /// 上下文压缩的摘要请求（专用系统提示、无工具）。
    CompactSummary,
    /// 子 agent 轮次：工具面由角色决定（explorer 只读）。
    Subagent(crate::chat::agent::AgentRole),
}

impl RequestPurpose {
    /// 工具面：None = 不声明工具（摘要请求）。
    pub fn tools_surface(self) -> Option<crate::chat::agent::ToolSurface> {
        match self {
            Self::Chat => Some(crate::chat::agent::ToolSurface::Main),
            Self::Subagent(role) => Some(crate::chat::agent::ToolSurface::from_role(role)),
            Self::CompactSummary => None,
        }
    }

    /// 该用途的请求是否声明工具（等价于 `tools_surface().is_some()`）。
    pub fn includes_tools(self) -> bool {
        self.tools_surface().is_some()
    }
}

/// 一次流式请求的输入快照。
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// 已完成的上下文轮次（含当前用户消息，原顺序）。
    pub turns: Vec<ChatTurn>,
    /// 结构化模型历史（工具化路径使用；与 `turns` 并存）。
    ///
    /// 空时各 transport 保持旧的纯文本编码结果（等价性由测试保证）。
    pub items: Vec<ModelItem>,
    /// 用户可见的系统级错误码（已发生的失败原因）。
    pub model: crate::db::chat::ModelSnapshot,
    /// 思考等级参数对象（来自模型 variants）；None 时不发送伪参数。
    pub variant_params: Option<serde_json::Value>,
    /// 请求用途（默认普通对话）。
    pub purpose: RequestPurpose,
    /// 覆写的系统提示；None 时各 transport 使用 [`crate::chat::prompt::system_prompt`]。
    ///
    /// 摘要请求用它携带压缩专用提示，并且**不再**叠加 turn 环境快照（摘要不需要）。
    pub system_prompt: Option<String>,
}

impl ChatRequest {
    /// 本次请求的工具面（摘要请求为 None）。
    pub fn tools_surface(&self) -> Option<crate::chat::agent::ToolSurface> {
        self.purpose.tools_surface()
    }

    /// 是否声明工具（摘要请求为 false）。
    pub fn includes_tools(&self) -> bool {
        self.tools_surface().is_some()
    }

    /// 本次请求要发送的系统提示文本：请求覆写优先，否则用稳定提示。
    pub fn resolve_system_prompt(&self) -> String {
        match &self.system_prompt {
            Some(prompt) => prompt.clone(),
            None => crate::chat::prompt::system_prompt(),
        }
    }
}

/// 单帧字节上限（64 KiB）：超限按协议错误失败。
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// 单轮累计正文字节上限（2 MiB）：超限失败并保留此前 partial。
pub const MAX_TOTAL_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// 单次工具参数累计字节上限（1 MiB）：超限按协议/工具失败处理，
/// 不进入持久化、不执行任何文件 IO；错误语义由解码器与状态层负责。
pub const MAX_TOOL_ARGUMENTS_BYTES: usize = 1024 * 1024;

/// 工具/步骤 UI 摘要单字段最大字符数：摘要只装路径、行范围与计数，
/// 不装完整文件内容；超长在生成摘要时截断（由状态层负责）。
pub const MAX_SUMMARY_CHARS: usize = 256;

/// 工具参数累计超限时的稳定错误码（写入工具步骤的 `error`，不是用户可见文案）。
pub const TOOL_ARGUMENTS_TOO_LARGE_CODE: &str = "tool arguments too large";

/// 工具参数未闭合/非法 JSON 时的稳定错误码。
pub const TOOL_ARGUMENTS_INVALID_CODE: &str = "tool arguments invalid";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::chat::Usage;

    /// `Finished` 是整轮终态，不进即时回投的增量批；其余（含 `ResponseFinished`）都进。
    #[test]
    fn finished_is_not_incremental_but_response_finished_is() {
        assert!(
            !ChatDelta::Finished {
                reason: StopReason::Complete
            }
            .is_incremental()
        );
        assert!(
            ChatDelta::ResponseFinished {
                stop: ModelStepStop::ToolCalls
            }
            .is_incremental()
        );
        assert!(ChatDelta::ReasoningEnded { item_id: None }.is_incremental());
        assert!(
            ChatDelta::ToolInputStarted {
                call_id: "c".into(),
                name: "read".into(),
                ordinal: 0,
            }
            .is_incremental()
        );
    }

    /// 负载字节只统计正文与工具参数字符串（checkpoint 阈值与上限估算口径）。
    #[test]
    fn payload_bytes_counts_text_and_tool_arguments() {
        assert_eq!(
            ChatDelta::Part(PartDelta {
                kind: PartKind::Text,
                text: "abc".into(),
                metadata: None,
            })
            .payload_bytes(),
            3
        );
        assert_eq!(
            ChatDelta::ToolInputDelta {
                call_id: "c".into(),
                chunk: "12".into(),
            }
            .payload_bytes(),
            2
        );
        assert_eq!(
            ChatDelta::ToolInputEnded {
                call_id: "c".into(),
                arguments_json: "{}".into(),
            }
            .payload_bytes(),
            2
        );
        assert_eq!(ChatDelta::Usage(Usage::default()).payload_bytes(), 0);
    }

    /// 上限常量与 DB 侧参数上限一致（同一契约，不允许两处漂移）。
    #[test]
    fn tool_argument_limit_matches_db_limit() {
        assert_eq!(
            MAX_TOOL_ARGUMENTS_BYTES,
            crate::db::chat::MAX_TOOL_ARGUMENT_BYTES
        );
    }
}
