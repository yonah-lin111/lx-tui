//! 旁路问答（`/btw`）的只读上下文装配。
//!
//! 边界（design D5/D8/D9）：
//! - 主历史只到所属 QA 结束，不包含后续主 QA 或其他 BTW；
//! - 被 `@` 的实例只引入自身任务记录与状态，不复制其父主对话；
//! - BTW 自身历史来自独立 child session，不写回原任务会话；
//! - 参考内容是带来源的只读资料，不是待执行的工具调用。

use std::path::Path;
use std::sync::Arc;

use crate::chat::agent_loop::ModelContext;
use crate::chat::events::{ChatRequest, ModelItem, RequestPurpose};
use crate::db::Database;
use crate::db::chat::{BtwThreadView, ChatMessage, ModelSnapshot};

/// 参考历史总量上限（字符）；超出后从最早的主历史开始丢弃并显式标记。
const MAX_REFERENCE_CHARS: usize = 48 * 1024;
/// 单条消息正文上限。
const MAX_MESSAGE_CHARS: usize = 8 * 1024;
/// 单条工具结果回放上限。
const MAX_TOOL_RESULT_CHARS: usize = 2 * 1024;

/// 被显式 `@` 的实例记录（只读资料）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwAgentReference {
    /// 稳定实例 ID。
    pub agent_id: String,
    pub nickname: String,
    pub role: String,
    pub status: String,
    /// 该实例最近一次完整任务的摘要（不是半截回复）。
    pub summary: String,
}

/// 一次旁路问答的装配输入。
pub struct BtwContextInput<'a> {
    pub database: &'a Database,
    pub workspace: &'a Path,
    pub parent_session_id: &'a str,
    pub qa_anchor_id: &'a str,
    pub child_session_id: &'a str,
    /// 本批次的 user 消息 ID：自身从该消息起不进入历史。
    pub current_user_message_id: &'a str,
    pub question: &'a str,
    pub answerer_label: &'a str,
    pub references: &'a [BtwAgentReference],
    pub model: &'a ModelSnapshot,
    pub variant_params: Option<serde_json::Value>,
    pub cwd: &'a Path,
    pub context_limit: Option<u64>,
    pub mcp: &'a crate::mcp::McpToolSnapshot,
    pub sandbox: &'a crate::config::sandbox::SandboxSettings,
}

/// 装配 BTW 的只读上下文。
pub fn assemble_btw_context(input: BtwContextInput<'_>) -> Result<ModelContext, String> {
    let (anchor, main_messages) = input
        .database
        .load_qa_context(input.workspace, input.parent_session_id, input.qa_anchor_id)
        .map_err(|error| error.code().to_string())?
        .ok_or_else(|| "btw qa anchor missing".to_string())?;
    let thread = input
        .database
        .btw_thread_for_qa(input.workspace, input.parent_session_id, input.qa_anchor_id)
        .ok()
        .flatten()
        .and_then(|thread| {
            input
                .database
                .load_btw_thread(input.workspace, &thread.id)
                .ok()
                .flatten()
        });
    let btw_messages = input
        .database
        .load_chat_session(input.workspace, input.child_session_id)
        .map_err(|error| error.code().to_string())?
        .unwrap_or_default();

    let mut reference = String::new();
    reference.push_str(&format!(
        "<btw-context parent-session=\"{}\" qa-anchor=\"{}\" answerer=\"{}\">\n",
        escape_xml(input.parent_session_id),
        escape_xml(&anchor.message_id),
        escape_xml(input.answerer_label),
    ));
    reference.push_str("<main-reference>\n");
    reference.push_str(&render_messages(
        &main_messages,
        &std::collections::HashMap::new(),
    ));
    reference.push_str("</main-reference>\n");
    for agent in input.references {
        reference.push_str(&format!(
            "<agent-reference agent-id=\"{}\" role=\"{}\" status=\"{}\">\n{}\n</agent-reference>\n",
            escape_xml(&agent.agent_id),
            escape_xml(&agent.role),
            escape_xml(&agent.status),
            escape_xml(&agent.summary),
        ));
    }
    if let Some(thread) = &thread {
        let prior = render_btw_history(thread, &btw_messages, input.current_user_message_id);
        if !prior.trim().is_empty() {
            reference.push_str("<btw-history>\n");
            reference.push_str(&prior);
            reference.push_str("</btw-history>\n");
        }
    }
    reference.push_str("</btw-context>");
    let reference = trim_reference(reference);

    let environment = crate::chat::prompt::environment_snapshot_with_sandbox(
        &crate::platform::EnvironmentFacts::capture(),
        Some(input.cwd),
        Some(input.cwd),
        &[],
        input.sandbox,
    );
    let system_prompt = crate::chat::prompt::btw_system_prompt(Some(&environment));
    let items = vec![
        ModelItem::UserText(reference),
        ModelItem::UserText(input.question.to_string()),
    ];
    let request = ChatRequest {
        turns: Vec::new(),
        items,
        model: input.model.clone(),
        variant_params: input.variant_params,
        purpose: RequestPurpose::Btw,
        system_prompt: Some(system_prompt),
        tools: Arc::new(crate::chat::tools::turn_tools_for_btw(input.mcp)),
    };
    Ok(ModelContext::new(request, input.cwd)
        .with_subagent_depth(1, 1)
        .with_context_limit(input.context_limit))
}

/// 渲染主历史：只保留已完成正文与已封口工具结果；半截内容不进参考。
fn render_messages(
    messages: &[ChatMessage],
    labels: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = String::new();
    for message in messages {
        if message.status == "streaming" || message.kind != "chat" {
            continue;
        }
        let role = if message.role == "user" {
            "user".to_string()
        } else {
            labels
                .get(&message.external_id)
                .cloned()
                .unwrap_or_else(|| "assistant".to_string())
        };
        out.push_str(&format!("[{role}]\n"));
        for part in &message.parts {
            if part.kind != "text" {
                continue;
            }
            out.push_str(&clip(&part.content, MAX_MESSAGE_CHARS));
            out.push('\n');
        }
        if message.role == "assistant" {
            for tool in &message.tools {
                if tool.status != "completed" && tool.status != "failed" {
                    continue;
                }
                out.push_str(&format!(
                    "[tool {} {}]\n{}\n",
                    tool.name,
                    tool.status,
                    tool.result_json
                        .as_deref()
                        .map(|result| clip(result, MAX_TOOL_RESULT_CHARS))
                        .unwrap_or_else(|| "(no result)".to_string()),
                ));
            }
        }
        out.push('\n');
    }
    out
}

/// 渲染本线程已完成的历史（当前批次及其之后不进入参考）。
fn render_btw_history(
    view: &BtwThreadView,
    messages: &[ChatMessage],
    current_user_message_id: &str,
) -> String {
    let Some(current_sequence) = messages
        .iter()
        .find(|message| message.external_id == current_user_message_id)
        .map(|message| message.sequence)
    else {
        return String::new();
    };
    let mut labels = std::collections::HashMap::new();
    for batch in &view.batches {
        for recipient in &batch.recipients {
            if let Some(id) = recipient.assistant_message_id.as_deref() {
                labels.insert(
                    id.to_string(),
                    format!("{} [{}]", recipient.display_nickname, recipient.target_kind),
                );
            }
        }
    }
    let prior: Vec<ChatMessage> = messages
        .iter()
        .filter(|message| message.sequence < current_sequence)
        .cloned()
        .collect();
    render_messages(&prior, &labels)
}

/// 参考段总预算：超出时保留尾部（当前问题与最近历史优先）。
fn trim_reference(text: String) -> String {
    if text.chars().count() <= MAX_REFERENCE_CHARS {
        return text;
    }
    let tail: String = text
        .chars()
        .rev()
        .take(MAX_REFERENCE_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("<btw-context-truncated/>\n{tail}")
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str(" …[truncated]");
    out
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests;
