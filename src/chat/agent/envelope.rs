//! Agent 间结构化消息信封（仅在组装模型请求时包裹，保持 DB 与 UI 文本干净）。

/// 消息投递类型（对齐 Codex `InterAgentMessageType`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterAgentMessageType {
    /// `send_input(trigger_turn = false)`：仅入队补充上下文。
    Message,
    /// `send_input(trigger_turn = true)`：唤醒或分配新子任务。
    NewTask,
}

impl InterAgentMessageType {
    /// 稳定协议标签。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Message => "MESSAGE",
            Self::NewTask => "NEW_TASK",
        }
    }

    /// 由 `trigger_turn` 布尔值映射投递类型。
    pub const fn from_trigger_turn(trigger_turn: bool) -> Self {
        if trigger_turn {
            Self::NewTask
        } else {
            Self::Message
        }
    }
}

/// 将发送给子 agent 的后续跟进文本包裹为带归属路径的结构化信封。
///
/// 幂等保护：若 `payload` 已经以 `"Message Type: "` 开头，原样返回不重复包裹。
pub fn format_inter_agent_envelope(
    message_type: InterAgentMessageType,
    recipient_path: &str,
    sender_path: &str,
    payload: &str,
) -> String {
    let recipient = if recipient_path.trim().is_empty() {
        "/root"
    } else {
        recipient_path.trim()
    };
    let sender = if sender_path.trim().is_empty() {
        "/root"
    } else {
        sender_path.trim()
    };
    let trimmed = payload.trim();
    if trimmed.starts_with("Message Type: ")
        && trimmed.contains(&format!("\nSender: {sender}\nPayload:\n"))
    {
        return payload.to_string();
    }
    format!(
        "Message Type: {}\nTask name: {}\nSender: {}\nPayload:\n{}",
        message_type.as_str(),
        recipient,
        sender,
        trimmed
    )
}
