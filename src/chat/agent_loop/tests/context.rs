//! 当前轮上下文的累计语义（对应 design §7.3 第二段与 G6 的“当前轮不丢步骤”）。
//!
//! 断言的是一件事：本轮在工具之间多次采样时，`ModelContext` 必须把**历史里已回放的
//! 成对调用**与**本轮刚发生的调用/结果**都保留在同一份 items 里，不从 DB 重新装配。

use super::*;
use crate::chat::context::MAX_TOOL_RESULT_REPLAY_BYTES;

/// 初始请求已含一段历史（含一对已完成工具）+ 末尾当前用户消息。
fn context_with_history(cwd: &Path) -> ModelContext {
    ModelContext::new(
        ChatRequest {
            turns: Vec::new(),
            items: vec![
                ModelItem::UserText("历史问".into()),
                ModelItem::AssistantText("历史答前".into()),
                ModelItem::AssistantToolCall {
                    call_id: "call_history".into(),
                    name: "read".into(),
                    input: serde_json::json!({"path": "old.txt"}),
                },
                ModelItem::ToolResult {
                    call_id: "call_history".into(),
                    content: "old content".into(),
                    is_error: false,
                },
                ModelItem::AssistantText("历史答后".into()),
                ModelItem::UserText("本轮问".into()),
            ],
            model: crate::db::chat::ModelSnapshot {
                provider: "example".into(),
                model: "example".into(),
                variant: None,
            },
            variant_params: None,
            purpose: crate::chat::events::RequestPurpose::Chat,
            system_prompt: None,
        },
        cwd.to_path_buf(),
    )
}

/// 本轮发生 call/result 后，下一次采样仍保留历史成对项 + 本轮成对项（顺序不乱）。
#[tokio::test]
async fn current_turn_keeps_history_pairs_and_appends_new_ones() {
    let fixture = Fixture::new("context-accumulate");
    let write_args = r#"{"path":"fresh.txt","content":"v"}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(&[("call_now", "write", &write_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = RecordingExecutor::new();
    let summary = run_agent_turn(
        context_with_history(fixture.path()),
        host,
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    let items = script.items_of(1);
    // 历史前缀原样保留（含历史成对项）。
    let ids: Vec<&str> = items
        .iter()
        .filter_map(|item| match item {
            ModelItem::AssistantToolCall { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        vec!["call_history", "call_now"],
        "历史调用在前、本轮调用追加在后"
    );
    let results: Vec<&str> = items
        .iter()
        .filter_map(|item| match item {
            ModelItem::ToolResult { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(results, vec!["call_history", "call_now"]);
    // 每个结果都紧跟其调用（成对相邻，无 orphan）。
    for (index, item) in items.iter().enumerate() {
        if let ModelItem::ToolResult { call_id, .. } = item {
            let at = items
                .iter()
                .position(|candidate| {
                    matches!(candidate, ModelItem::AssistantToolCall { call_id: id, .. } if id == call_id)
                })
                .expect("call present");
            assert_eq!(at + 1, index, "结果必须紧邻其调用: {call_id}");
        }
    }
    // 历史正文仍按原顺序在原位。
    assert!(matches!(&items[0], ModelItem::UserText(text) if text == "历史问"));
    assert!(matches!(&items[5], ModelItem::UserText(text) if text == "本轮问"));
}

/// 冻结的系统提示（环境快照 + compact 摘要）必须原样进入**每一次**采样。
///
/// 回归：`next_request` 曾把 `system_prompt` 写成 `None`，transport 于是现场重采集，
/// compact 摘要与冻结环境全部丢失。
#[tokio::test]
async fn frozen_system_prompt_is_reused_by_every_sampling() {
    let fixture = Fixture::new("context-frozen-prompt");
    let mut request = ChatRequest {
        turns: Vec::new(),
        items: vec![ModelItem::UserText("do it".into())],
        model: crate::db::chat::ModelSnapshot {
            provider: "example".into(),
            model: "example".into(),
            variant: None,
        },
        variant_params: None,
        purpose: crate::chat::events::RequestPurpose::Chat,
        system_prompt: Some("FROZEN PROMPT WITH COMPACT SUMMARY".into()),
    };
    request.turns.push(crate::chat::events::ChatTurn {
        role: "user".into(),
        text: "do it".into(),
        reasoning_metadata: None,
    });
    let write_args = r#"{"path":"fresh.txt","content":"v"}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(&[("call_now", "write", &write_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let summary = run_agent_turn(
        ModelContext::new(request, fixture.path()),
        RecordingHost::new(),
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(script.request_count(), 2, "一次工具轮 + 一次收尾采样");
    assert_eq!(
        script.system_prompts(),
        vec![
            Some("FROZEN PROMPT WITH COMPACT SUMMARY".into()),
            Some("FROZEN PROMPT WITH COMPACT SUMMARY".into()),
        ],
        "每一次采样都必须复用 turn 创建时冻结的系统提示"
    );
}

/// 本轮单条工具结果超 64 KiB：进入下一次采样时截断并显式标记。
#[tokio::test]
async fn current_turn_truncates_oversized_tool_result() {
    let fixture = Fixture::new("context-truncate");
    // 写一个多行大文件：每行 60 字符 × 4000 行 ≈ 240 KiB，read 默认取 2000 行仍超 64 KiB。
    let line = "x".repeat(59);
    let big = format!("{line}\n").repeat(4_000);
    std::fs::write(fixture.path().join("big.txt"), &big).expect("write fixture file");
    let read_args = r#"{"path":"big.txt"}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(&[("call_big", "read", &read_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = RecordingExecutor::new();
    let summary = run_agent_turn(
        context(fixture.path()),
        host,
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    let content = script
        .items_of(1)
        .into_iter()
        .find_map(|item| match item {
            ModelItem::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .expect("read result present");
    assert!(
        content.len() <= MAX_TOOL_RESULT_REPLAY_BYTES,
        "本轮结果同样受回放上限约束: {}",
        content.len()
    );
    assert!(content.ends_with("[truncated]"), "截断必须显式标记");
}

/// 直接调用累计接口：结果截断口径与历史装配一致（同一常量、同一标记）。
#[test]
fn push_tool_result_uses_the_same_replay_limit() {
    let mut context = context_helper();
    let oversized = "y".repeat(MAX_TOOL_RESULT_REPLAY_BYTES + 1);
    context.push_tool_result("call_x", oversized, false);
    let ModelItem::ToolResult {
        content,
        call_id,
        is_error,
    } = context.items().last().cloned().expect("result appended")
    else {
        panic!("expected tool result item");
    };
    assert_eq!(call_id, "call_x");
    assert!(!is_error);
    assert!(content.len() <= MAX_TOOL_RESULT_REPLAY_BYTES);
    assert!(content.ends_with("[truncated]"));
}

/// 空上下文（累计接口直测用）。
fn context_helper() -> ModelContext {
    ModelContext::new(
        ChatRequest {
            turns: Vec::new(),
            items: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "example".into(),
                model: "example".into(),
                variant: None,
            },
            variant_params: None,
            purpose: crate::chat::events::RequestPurpose::Chat,
            system_prompt: None,
        },
        std::env::temp_dir(),
    )
}
