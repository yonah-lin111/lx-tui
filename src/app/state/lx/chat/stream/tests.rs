//! 流事件 → 步骤时间线的纯状态测试：思考独立完成态、显式结束幂等、
//! 多步顺序、工具状态流转与终态兜底。

use super::*;
use crate::app::state::ChatMessageView;
use crate::chat::events::{MAX_TOOL_ARGUMENTS_BYTES, StopReason};

/// 建一个只有 streaming assistant 占位的聊天状态。
fn streaming_chat() -> LxChat {
    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        id: "a".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: 0,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    });
    chat
}

/// 最后一个步骤的 reasonig 完成时刻（断言用）。
fn last_reasoning_completed(chat: &LxChat) -> Option<i64> {
    chat.messages[0]
        .reasoning_parts()
        .last()
        .and_then(|part| part.completed_at_ms)
}

fn tool<'a>(chat: &'a LxChat, call_id: &str) -> &'a crate::app::state::ToolCallView {
    chat.messages[0].tool(call_id).expect("tool step")
}

/// 复现并验证核心修复：`reasoning -> text`（assistant 仍 streaming）时思考已完成。
#[test]
fn reasoning_to_text_closes_reasoning_while_message_still_streaming() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "想", None);
    // 思考生成中：未完成，reasoning_streaming 为真。
    assert!(chat.reasoning_streaming(0, 0));
    assert_eq!(last_reasoning_completed(&chat), None);

    chat.append_stream_delta(PartKind::Text, "答", None);

    assert_eq!(chat.messages[0].status, "streaming", "消息仍在流中");
    assert!(
        !chat.reasoning_streaming(0, 0),
        "正文开始后思考不再是流中态"
    );
    let completed = last_reasoning_completed(&chat).expect("思考已完成");
    assert!(completed > 0, "完成时刻为真实时间戳");
    // 正文继续增长。
    chat.append_stream_delta(PartKind::Text, "续", None);
    assert_eq!(chat.messages[0].text(), "答续");
    assert_eq!(last_reasoning_completed(&chat), Some(completed), "只写一次");
}

/// `reasoning -> ToolInputStarted`：思考完成时间早于工具步骤（pending 之前）。
#[test]
fn reasoning_to_tool_start_closes_reasoning_before_tool_step() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "先想", None);
    chat.start_tool_input("call-1", "read", 0);

    let completed = last_reasoning_completed(&chat).expect("思考已完成");
    assert!(!chat.reasoning_streaming(0, 0));
    let call = tool(&chat, "call-1");
    assert_eq!(call.status, crate::app::state::ToolStatus::InputStreaming);
    assert_eq!(call.position, 1, "工具步骤位置在思考之后");
    // 思考完成时刻不晚于工具步骤出现（同帧顺序：先关闭思考再插入工具）。
    assert!(completed > 0);
}

/// 无显式 end 事件：由后续正文关闭；显式 `ReasoningEnded` 幂等。
#[test]
fn explicit_reasoning_ended_is_idempotent() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "想", None);
    assert!(chat.reasoning_ended(None));
    let first = last_reasoning_completed(&chat).expect("已完成");
    assert!(!chat.reasoning_ended(None), "重复结束事件无变化");
    assert_eq!(last_reasoning_completed(&chat), Some(first), "完成时间不变");
    assert!(!chat.reasoning_ended(Some("item-1")), "无开放块时幂等");
    assert_eq!(chat.messages[0].steps.len(), 1, "不创建空思考块");
}

/// 思考后开始**下一个**思考块也应关闭旧块。
#[test]
fn second_reasoning_block_closes_previous() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "一", None);
    // 带元数据的空 delta 绑定到同块，不新建、不关闭。
    chat.append_stream_delta(
        PartKind::Reasoning,
        "",
        Some(serde_json::json!({"signature": "sig"})),
    );
    assert_eq!(chat.messages[0].steps.len(), 1);
    assert_eq!(last_reasoning_completed(&chat), None);

    // 正文打断后再来一个思考块：新块开放，旧块保持已完成。
    chat.append_stream_delta(PartKind::Text, "答", None);
    chat.append_stream_delta(PartKind::Reasoning, "二", None);
    let parts: Vec<_> = chat.messages[0].reasoning_parts().collect();
    assert_eq!(parts.len(), 2);
    assert!(parts[0].completed_at_ms.is_some());
    assert!(parts[1].completed_at_ms.is_none());
    assert!(chat.reasoning_streaming(0, 2), "新思考块仍在生成");
}

/// 一轮多步：text → reasoning → text 的顺序与 position 单调唯一。
#[test]
fn multi_step_timeline_positions_are_monotonic() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Text, "正文1", None);
    chat.append_stream_delta(PartKind::Reasoning, "想", None);
    chat.append_stream_delta(PartKind::Text, "正文2", None);

    let message = &chat.messages[0];
    assert_eq!(message.steps.len(), 3);
    assert!(positions_are_monotonic(&message.steps));
    assert_eq!(
        message
            .steps
            .iter()
            .map(|step| step.position())
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    // 中间思考块已完成，最后正文在增长。
    assert!(
        message
            .reasoning_parts()
            .next()
            .unwrap()
            .completed_at_ms
            .is_some()
    );
    assert!(!chat.reasoning_streaming(0, 0));
}

/// reasoning → 2 tools：顺序与 position 单调唯一，工具之间无覆盖。
#[test]
fn reasoning_then_two_tools_keeps_order() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "计划", None);
    chat.start_tool_input("call-a", "read", 0);
    chat.append_tool_input("call-a", r#"{"path":"#);
    chat.append_tool_input("call-a", r#""a.rs"}"#);
    chat.end_tool_input("call-a", r#"{"path":"a.rs"}"#);
    chat.start_tool_input("call-b", "search", 1);
    chat.end_tool_input("call-b", r#"{"pattern":"x"}"#);

    let message = &chat.messages[0];
    assert_eq!(message.steps.len(), 3);
    assert!(positions_are_monotonic(&message.steps));
    let tools: Vec<_> = message.tools().collect();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].call_id, "call-a");
    assert_eq!(tools[0].position, 1);
    assert_eq!(tools[1].call_id, "call-b");
    assert_eq!(tools[1].position, 2);
    assert_eq!(tools[0].status, crate::app::state::ToolStatus::Pending);
    assert_eq!(
        tools[0].summary.as_ref().and_then(|s| s.path.as_deref()),
        Some("a.rs")
    );
}

/// 工具状态流转 InputStreaming → Pending，重复 Started/Ended 幂等。
#[test]
fn tool_input_lifecycle_and_duplicates_are_ignored() {
    let mut chat = streaming_chat();
    assert!(chat.start_tool_input("c1", "read", 0));
    assert_eq!(
        tool(&chat, "c1").status,
        crate::app::state::ToolStatus::InputStreaming
    );
    // 同 call_id 重复 Started：忽略，不覆盖已有步骤、不产生第二行。
    assert!(!chat.start_tool_input("c1", "write", 0));
    assert_eq!(chat.messages[0].steps.len(), 1);
    assert_eq!(tool(&chat, "c1").name, "read");

    assert!(chat.append_tool_input("c1", r#"{"path":"#));
    assert!(chat.append_tool_input("c1", r#""a.rs"}"#));
    assert!(chat.end_tool_input("c1", r#"{"path":"a.rs"}"#));
    assert_eq!(
        tool(&chat, "c1").status,
        crate::app::state::ToolStatus::Pending
    );
    // 重复 Ended：幂等，不覆盖。
    assert!(!chat.end_tool_input("c1", r#"{"path":"other.rs"}"#));
    assert_eq!(tool(&chat, "c1").arguments_json, r#"{"path":"a.rs"}"#);
}

/// 参数超限：标记失败并停止累计，不进入 Pending。
#[test]
fn tool_arguments_over_limit_fail_the_call() {
    let mut chat = streaming_chat();
    chat.start_tool_input("c1", "write", 0);
    let chunk = "x".repeat(MAX_TOOL_ARGUMENTS_BYTES);
    assert!(chat.append_tool_input("c1", &chunk));
    assert!(chat.append_tool_input("c1", "y"), "超限那次返回变化");
    let call = tool(&chat, "c1");
    assert_eq!(call.status, crate::app::state::ToolStatus::Failed);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::chat::events::TOOL_ARGUMENTS_TOO_LARGE_CODE)
    );
    assert!(!chat.end_tool_input("c1", "{}"), "已失败不再接受结束");
}

/// 非法完整参数：标失败（不执行、不转 Pending）。
#[test]
fn invalid_complete_arguments_fail_the_call() {
    let mut chat = streaming_chat();
    chat.start_tool_input("c1", "read", 0);
    assert!(chat.end_tool_input("c1", "{半截"));
    let call = tool(&chat, "c1");
    assert_eq!(call.status, crate::app::state::ToolStatus::Failed);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::chat::events::TOOL_ARGUMENTS_INVALID_CODE)
    );
}

/// 未知 call_id 的增量/结束：忽略，不创建步骤。
#[test]
fn unknown_call_id_never_creates_a_step() {
    let mut chat = streaming_chat();
    assert!(!chat.append_tool_input("ghost", "x"));
    assert!(!chat.end_tool_input("ghost", "{}"));
    assert!(chat.messages[0].steps.is_empty());
}

/// `ResponseFinished` 只收尾思考与在途工具输入，不结束消息。
#[test]
fn response_finished_closes_reasoning_but_keeps_message_streaming() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "想", None);
    chat.start_tool_input("c1", "read", 0);
    chat.append_tool_input("c1", r#"{"path":"a.rs"}"#);
    assert!(chat.finish_response(crate::chat::events::ModelStepStop::ToolCalls));
    assert_eq!(
        chat.messages[0].status, "streaming",
        "响应结束不等于轮次结束"
    );
    assert!(last_reasoning_completed(&chat).is_some());
    assert_eq!(
        tool(&chat, "c1").status,
        crate::app::state::ToolStatus::Pending,
        "参数已完整的工具转 Pending（尚未执行）"
    );
}

/// `ResponseFinished` 时参数流仍未闭合（协议截断）：记失败、不执行。
#[test]
fn response_finished_marks_unclosed_tool_input_failed() {
    let mut chat = streaming_chat();
    chat.start_tool_input("c1", "read", 0);
    assert!(chat.finish_response(crate::chat::events::ModelStepStop::ToolCalls));
    let call = tool(&chat, "c1");
    assert_eq!(call.status, crate::app::state::ToolStatus::Failed);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::chat::events::TOOL_ARGUMENTS_INVALID_CODE)
    );
}

/// `Finished` 兜底：关闭仍开放的思考并把未终态工具标为中断（不伪造结果）。
#[test]
fn finished_settles_open_reasoning_and_tools_without_fake_results() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "想", None);
    chat.start_tool_input("c1", "read", 0);
    chat.end_tool_input("c1", r#"{"path":"a.rs"}"#);
    assert!(chat.close_stream(&StopReason::Failed {
        code: "timeout".into()
    }));
    assert!(last_reasoning_completed(&chat).is_some());
    let call = tool(&chat, "c1");
    assert_eq!(call.status, crate::app::state::ToolStatus::Interrupted);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::db::chat::INTERRUPTED_ERROR_CODE)
    );
    assert!(call.arguments_json.contains("a.rs"), "参数保留，不伪造结果");
}

/// 取消：未开始执行的工具标 `Cancelled`。
#[test]
fn cancelled_turn_marks_open_tools_cancelled() {
    let mut chat = streaming_chat();
    chat.start_tool_input("c1", "read", 0);
    chat.end_tool_input("c1", r#"{"path":"a.rs"}"#);
    chat.close_stream(&StopReason::Cancelled);
    let call = tool(&chat, "c1");
    assert_eq!(call.status, crate::app::state::ToolStatus::Cancelled);
    assert!(call.error.is_none(), "取消不写错误原因");
}

/// 已完成工具不被终态兜底覆盖。
#[test]
fn terminal_tools_are_not_overwritten_on_finish() {
    let mut chat = streaming_chat();
    chat.start_tool_input("c1", "read", 0);
    chat.end_tool_input("c1", r#"{"path":"a.rs"}"#);
    if let Some(call) = chat.messages[0].tool_mut("c1") {
        call.settle(crate::app::state::ToolStatus::Completed, None);
    }
    chat.close_stream(&StopReason::Complete);
    assert_eq!(
        tool(&chat, "c1").status,
        crate::app::state::ToolStatus::Completed
    );
}

/// 非 streaming 消息不接受任何增量（晚到事件不污染终态）。
#[test]
fn deltas_are_dropped_once_message_left_streaming() {
    let mut chat = streaming_chat();
    chat.messages[0].status = "complete".into();
    assert!(!chat.append_stream_delta(PartKind::Text, "迟到", None));
    assert!(!chat.start_tool_input("c1", "read", 0));
    assert!(chat.messages[0].steps.is_empty());
}

/// 签名元数据仍绑定到原思考块（provider 元数据不丢）。
#[test]
fn signature_metadata_binds_to_reasoning_part() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "思考", None);
    chat.append_stream_delta(
        PartKind::Reasoning,
        "",
        Some(serde_json::json!({"signature": "sig-1"})),
    );
    assert_eq!(chat.messages[0].steps.len(), 1, "不生成空 part");
    let metadata = chat.messages[0]
        .reasoning_parts()
        .next()
        .and_then(|part| part.provider_metadata_json.clone())
        .expect("metadata");
    let value: serde_json::Value = serde_json::from_str(&metadata).expect("json");
    assert_eq!(value.get("signature"), Some(&serde_json::json!("sig-1")));
}

/// 完整状态流转：InputStreaming → Pending → Running → Completed / Failed。
#[test]
fn tool_status_transitions_pending_running_terminal() {
    use crate::app::state::ToolStatus;

    // 成功路径。
    let mut chat = streaming_chat();
    chat.start_tool_input("c1", "read", 0);
    chat.end_tool_input("c1", r#"{"path":"a.rs"}"#);
    assert_eq!(tool(&chat, "c1").status, ToolStatus::Pending);
    assert!(chat.mark_tool_running("c1"), "Pending → Running");
    assert_eq!(tool(&chat, "c1").status, ToolStatus::Running);
    assert!(!chat.mark_tool_running("c1"), "重复推进幂等 false");
    assert!(chat.settle_tool("c1", ToolStatus::Completed, None));
    assert_eq!(tool(&chat, "c1").status, ToolStatus::Completed);
    assert!(
        !chat.settle_tool("c1", ToolStatus::Failed, None),
        "终态不可覆盖"
    );
    assert_eq!(
        tool(&chat, "c1").status,
        ToolStatus::Completed,
        "已终态不被改回"
    );

    // 失败路径：错误原因可见，且不伪造结果体。
    let mut chat = streaming_chat();
    chat.start_tool_input("c2", "write", 0);
    chat.end_tool_input("c2", r#"{"path":"b.rs","content":"x"}"#);
    chat.mark_tool_running("c2");
    assert!(chat.settle_tool("c2", ToolStatus::Failed, Some("permission denied".into())));
    let call = tool(&chat, "c2");
    assert_eq!(call.status, ToolStatus::Failed);
    assert_eq!(call.error.as_deref(), Some("permission denied"));
    assert!(call.arguments_json.contains("b.rs"), "参数保留");

    // 非终态不作为 settle 目标。
    assert!(
        !chat.settle_tool("c2", ToolStatus::Running, None),
        "拒绝非法目标态"
    );
}

/// 同一帧内 `ReasoningEnded` + `ToolInputStarted`：按帧内顺序处理，完成时间只写一次。
#[test]
fn same_frame_reasoning_end_then_tool_start_is_ordered() {
    let mut chat = streaming_chat();
    chat.append_stream_delta(PartKind::Reasoning, "想", None);
    // 显式结束（幂等）后插入工具步骤：思考完成时间不被二次改写。
    assert!(chat.reasoning_ended(None));
    let first = last_reasoning_completed(&chat).expect("已完成");
    assert!(chat.start_tool_input("c1", "read", 0));
    assert_eq!(last_reasoning_completed(&chat), Some(first), "只写一次");
    assert_eq!(chat.messages[0].steps.len(), 2);
    assert!(positions_are_monotonic(&chat.messages[0].steps));
}

/// 结算路径与重载路径共用一套合并：参数只有 `path` 的 `read` 结算后仍有区间与行数。
#[test]
fn settle_merges_argument_and_result_summaries() {
    use crate::app::state::ToolStatus;
    use crate::app::state::ToolSummary;

    let mut chat = streaming_chat();
    chat.start_tool_input("c1", "read", 0);
    chat.end_tool_input("c1", r#"{"path":"src/a.rs"}"#);
    chat.mark_tool_running("c1");
    // 结果侧摘要（`to_summary` 的口径）：区间与行数只来自结果体。
    let result = ToolSummary {
        path: Some("src/a.rs".into()),
        range: Some((1, 40)),
        read_lines: Some(40),
        total_lines: Some(40),
        ..ToolSummary::default()
    };
    assert!(chat.settle_tool_result("c1", ToolStatus::Completed, Some(result), None, None));
    let summary = tool(&chat, "c1").summary.as_ref().expect("摘要");
    assert_eq!(summary.path.as_deref(), Some("src/a.rs"), "路径保留");
    assert_eq!(summary.range, Some((1, 40)), "区间来自结果侧");
    assert_eq!(summary.read_lines, Some(40), "实际行数来自结果侧");

    // `search`：pattern 只在参数里，结算后不能被结果侧的空 pattern 覆盖掉。
    let mut chat = streaming_chat();
    chat.start_tool_input("c2", "search", 0);
    chat.end_tool_input("c2", r#"{"pattern":"fn render","path":"src"}"#);
    chat.mark_tool_running("c2");
    let result = ToolSummary {
        matches: Some(3),
        ..ToolSummary::default()
    };
    assert!(chat.settle_tool_result("c2", ToolStatus::Completed, Some(result), None, None));
    let summary = tool(&chat, "c2").summary.as_ref().expect("摘要");
    assert_eq!(
        summary.pattern.as_deref(),
        Some("fn render"),
        "pattern 来自参数"
    );
    assert_eq!(summary.path.as_deref(), Some("src"), "path 来自参数");
    assert_eq!(summary.matches, Some(3), "命中数来自结果侧");
}
