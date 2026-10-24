//! 上下文装配测试：轮次分组、成对工具回放、整轮裁剪、回放上限与视图往返。
//!
//! 分文件组织（单文件上限）：
//! - [`legacy`]：旧纯文本会话等价性、complete-only 轮次、草稿保留与 reasoning 排除；
//! - [`tools`]：text/tool 顺序、失败结果、不可回放状态、非法参数、跨 provider 规范化；
//! - [`budget`]：整轮裁剪、`shrink_once`、token 估算含工具内容、单条结果上限。
//! - [`roundtrip`]：`chat_part_views_to_db`（含 tools）→ `assemble` 往返。

use super::*;

mod budget;
mod compact;
mod legacy;
mod roundtrip;
mod shell;
mod tools;

/// 一条测试消息；`text` 写入单个 text part（position 0）。
fn message(
    external_id: &str,
    sequence: i64,
    turn_index: i64,
    role: &str,
    status: &str,
    text: &str,
) -> ChatMessage {
    ChatMessage {
        external_id: external_id.into(),
        sequence,
        turn_index,
        kind: "chat".into(),
        role: role.into(),
        status: status.into(),
        model_provider: (role == "assistant").then(|| "example".to_string()),
        model_id: (role == "assistant").then(|| "m".to_string()),
        variant: None,
        stop_reason: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        created_at_ms: 0,
        completed_at_ms: None,
        user_shell: None,
        parts: vec![part(0, "text", text)],
        tools: Vec::new(),
    }
}

/// 一个内容块。
fn part(position: i64, kind: &str, content: &str) -> crate::db::chat::ChatPart {
    crate::db::chat::ChatPart {
        position,
        kind: kind.into(),
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

/// 一条工具调用记录（成败由 `status` / `result_json` / `error_text` 决定）。
fn tool(
    position: i64,
    call_id: &str,
    name: &str,
    status: &str,
    arguments_json: &str,
    result_json: Option<&str>,
    error_text: Option<&str>,
) -> ToolCallRecord {
    ToolCallRecord {
        call_id: call_id.into(),
        message_id: "a".into(),
        name: name.into(),
        arguments_json: arguments_json.into(),
        position,
        status: status.into(),
        result_json: result_json.map(str::to_string),
        error_text: error_text.map(str::to_string),
        started_at_ms: 0,
        completed_at_ms: None,
        duration_ms: None,
    }
}

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "example".into(),
        model: "m".into(),
        variant: None,
    }
}

/// 装配一次（不限预算）。
fn assemble_unbounded(messages: &[ChatMessage], draft: &str) -> Assembly {
    assemble(&AssemblyInput {
        messages,
        draft,
        model: &model(),
        context_limit: None,
        output_limit: None,
        variant_params: None,
        compact: None,
        environment: None,
    })
}

/// 装配一次（给定上下文/输出上限）。
fn assemble_with_limit(messages: &[ChatMessage], draft: &str, limit: u64, output: u64) -> Assembly {
    assemble(&AssemblyInput {
        messages,
        draft,
        model: &model(),
        context_limit: Some(limit),
        output_limit: Some(output),
        variant_params: None,
        compact: None,
        environment: None,
    })
}

/// 两轮完整历史（无工具）。
fn two_complete_turns() -> Vec<ChatMessage> {
    vec![
        message("u0", 0, 0, "user", "complete", "第一问"),
        message("a0", 1, 0, "assistant", "complete", "第一答"),
        message("u1", 2, 1, "user", "complete", "第二问"),
        message("a1", 3, 1, "assistant", "complete", "第二答"),
    ]
}

/// 一轮含 text + 工具 + text 的 assistant 消息（position 0/1/2）。
fn turn_with_tool(text_before: &str, call_id: &str, text_after: &str) -> Vec<ChatMessage> {
    let mut user = message("u", 0, 0, "user", "complete", "读文件");
    user.parts = vec![part(0, "text", "读文件")];
    let mut assistant = message("a", 1, 0, "assistant", "complete", "");
    assistant.parts = vec![part(0, "text", text_before), part(2, "text", text_after)];
    assistant.tools = vec![tool(
        1,
        call_id,
        "read",
        "completed",
        r#"{"path":"src/a.rs"}"#,
        Some(r#"{"tool":"read","content":"1: fn main() {}"}"#),
        None,
    )];
    vec![user, assistant]
}

/// items 摘要（断言用）：类型标签 + 关键字段，便于逐项比较顺序。
#[derive(Debug, PartialEq, Eq)]
enum ItemView {
    User(String),
    Assistant(String),
    Call(String, String),
    Result(String, bool),
}

/// 把 items 映射成可断言视图。
fn item_views(items: &[ModelItem]) -> Vec<ItemView> {
    items
        .iter()
        .map(|item| match item {
            ModelItem::UserText(text) => ItemView::User(text.clone()),
            ModelItem::AssistantText(text) => ItemView::Assistant(text.clone()),
            ModelItem::AssistantToolCall { call_id, name, .. } => {
                ItemView::Call(call_id.clone(), name.clone())
            }
            ModelItem::ToolResult {
                call_id, is_error, ..
            } => ItemView::Result(call_id.clone(), *is_error),
        })
        .collect()
}

/// 断言 items 中不存在 orphan result：每个结果**紧邻**同 call_id 的调用，且调用/结果一一对应。
fn assert_no_orphan_results(items: &[ModelItem]) {
    let mut calls: Vec<&str> = Vec::new();
    let mut results: Vec<&str> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            ModelItem::AssistantToolCall { call_id, .. } => calls.push(call_id.as_str()),
            ModelItem::ToolResult { call_id, .. } => {
                let previous = index.checked_sub(1).and_then(|at| items.get(at));
                assert!(
                    matches!(previous, Some(ModelItem::AssistantToolCall { call_id: at, .. }) if at == call_id),
                    "结果必须紧邻其调用（无 orphan result）: {call_id}"
                );
                assert!(
                    !results.contains(&call_id.as_str()),
                    "同一调用不得有第二个结果: {call_id}"
                );
                results.push(call_id.as_str());
            }
            ModelItem::UserText(_) | ModelItem::AssistantText(_) => {}
        }
    }
    assert_eq!(calls.len(), results.len(), "每个调用恰有一个结果");
}
