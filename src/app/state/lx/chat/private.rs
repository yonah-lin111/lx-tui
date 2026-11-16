//! 子 agent 面板私聊追问的视觉标记：与 spawn 任务、steer 插入和 `/report` 请求区分。
//!
//! 标记只存内存；消息真相仍在 DB（`agent_subagent_input.position > 0` 且无
//! `parent_call_id`），渲染层只读集合。

use super::LxChat;
use crate::db::chat::ChatMessage;

impl LxChat {
    /// 标记一条用户消息为面板私聊追问。
    pub fn mark_private_user_message(&mut self, message_id: &str) {
        self.private_user_messages.insert(message_id.to_string());
    }

    /// 该消息是否为面板私聊追问。
    pub fn is_private_user_message(&self, message_id: &str) -> bool {
        self.private_user_messages.contains(message_id)
    }

    /// 按 DB 候选 id 重建私聊集合；报告请求（正文 marker）与 steer 不参与。
    pub fn rebuild_private_user_messages(
        &mut self,
        messages: &[ChatMessage],
        candidates: &[String],
    ) {
        self.private_user_messages.clear();
        for id in candidates {
            let Some(message) = messages.iter().find(|message| &message.external_id == id) else {
                continue;
            };
            if message.role != "user" || message.kind != "chat" || message.user_shell.is_some() {
                continue;
            }
            if self.is_steered_user_message(id) {
                continue;
            }
            let text = message
                .parts
                .iter()
                .filter(|part| part.kind == "text")
                .map(|part| part.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if crate::chat::agent::is_report_request(&text) {
                continue;
            }
            self.private_user_messages.insert(id.clone());
        }
    }
}

#[cfg(test)]
mod tests;
