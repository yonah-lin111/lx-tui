//! 流中 steer 的注入点编排测试：安全边界、顺序与撤回竞态。

use super::*;
use crate::chat::events::{ModelImage, ModelItem, RequestPurpose};

fn steer_text(text: &str) -> SteerInput {
    SteerInput {
        text: text.into(),
        images: Vec::new(),
        skill_blocks: Vec::new(),
    }
}

fn tool_then_final() -> Arc<Script> {
    let read_args = r#"{"path":"."}"#.to_string();
    Script::new(
        vec![
            tool_calls(&[("c1", "read", &read_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    )
}

/// 首次采样前不注入：即使事件层已有 pending，也要等第一次流返回后才 take。
#[tokio::test]
async fn steer_is_not_injected_before_the_first_sample() {
    let fixture = Fixture::new("steer-first-sample");
    let script = Script::new(
        vec![
            final_response("first", None),
            final_response("second", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new()
        .with_pending_steers([true])
        .with_steer_batch(vec![steer_text("STEER_ONE")]);
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(script.request_count(), 2, "第一次 Final 后继续采样");
    assert!(
        !format!("{:?}", script.items_of(0)).contains("STEER_ONE"),
        "第一次采样前不得注入 steer"
    );
    assert!(
        format!("{:?}", script.items_of(1)).contains("STEER_ONE"),
        "回到 loop 顶部后才注入"
    );
    assert_eq!(
        host.steer_trace(),
        vec![
            SteerTrace::HasPending(true),
            SteerTrace::Take,
            SteerTrace::HasPending(false),
        ]
    );
}

/// 工具批次全部结算后才注入：steer 排在本批工具结果之后、下一次采样之前。
#[tokio::test]
async fn steer_is_injected_after_the_tool_batch() {
    let fixture = Fixture::new("steer-after-tool");
    let script = tool_then_final();
    let host = RecordingHost::new().with_steer_batch(vec![steer_text("AFTER_TOOL")]);
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(script.request_count(), 2);
    let items = script.items_of(1);
    let tool_index = items
        .iter()
        .position(|item| matches!(item, ModelItem::ToolResult { call_id, .. } if call_id == "c1"))
        .expect("tool result present");
    let steer_index = items
        .iter()
        .position(|item| matches!(item, ModelItem::UserText(text) if text == "AFTER_TOOL"))
        .expect("steer present");
    assert!(tool_index < steer_index, "steer 必须在工具结果之后");
}

/// auto-compact 必须先于 take_steer_inputs：压缩期间到达的 steer 不进摘要。
#[tokio::test]
async fn auto_compact_runs_before_take_steer_inputs() {
    let fixture = Fixture::new("steer-after-compact");
    let read_args = r#"{"path":"."}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(&[("c1", "read", &read_args, 0)], None),
            final_response("summary of the read", None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let request = ChatRequest {
        turns: Vec::new(),
        items: vec![ModelItem::UserText("investigate".into())],
        model: crate::db::chat::ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
        variant_params: None,
        purpose: RequestPurpose::Subagent(crate::chat::agent::AgentRole::Explorer),
        system_prompt: Some("explorer prompt".into()),
        tools: std::sync::Arc::new(Vec::new()),
    };
    let ctx = ModelContext::new(request, fixture.path()).with_context_limit(Some(1));
    let host = RecordingHost::new().with_steer_batch(vec![steer_text("STEER_AFTER_COMPACT")]);
    let summary = run_agent_turn(
        ctx,
        host.clone(),
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(
        host.steer_trace(),
        vec![
            SteerTrace::AutoCompact,
            SteerTrace::Take,
            SteerTrace::HasPending(false),
        ],
        "压缩检查点必须先于 steer 注入"
    );
    assert_eq!(script.request_count(), 3, "工具步 + 摘要 + 收尾");
    let prompts = script.system_prompts();
    assert!(
        prompts[2]
            .as_deref()
            .unwrap_or_default()
            .contains("<compact_summary>"),
        "摘要进入下一次采样：{prompts:?}"
    );
    assert!(
        format!("{:?}", script.items_of(2)).contains("STEER_AFTER_COMPACT"),
        "注入后的 steer 才能进入采样"
    );
}

/// Final 后仍有 pending：只 continue，不就地注入；下一次顶部先压缩再 take。
#[tokio::test]
async fn final_with_pending_steer_compacts_then_injects() {
    let fixture = Fixture::new("steer-after-final");
    let script = Script::new(
        vec![
            final_response("first answer", None),
            final_response("summary of earlier work", None),
            final_response("second answer", None),
        ],
        StopReason::Complete,
    );
    let request = ChatRequest {
        turns: Vec::new(),
        // 只有当前轮任务：首次采样前不满足压缩条件，Final 之后才越过阈值。
        items: vec![ModelItem::UserText("CURRENT_TURN".into())],
        model: crate::db::chat::ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
        variant_params: None,
        purpose: RequestPurpose::Subagent(crate::chat::agent::AgentRole::Explorer),
        system_prompt: Some("explorer prompt".into()),
        tools: std::sync::Arc::new(Vec::new()),
    };
    let ctx = ModelContext::new(request, fixture.path()).with_context_limit(Some(1));
    let host = RecordingHost::new()
        .with_pending_steers([true, false])
        .with_steer_batch(vec![steer_text("STEER_AFTER_FINAL")]);
    let summary = run_agent_turn(
        ctx,
        host.clone(),
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(
        host.steer_trace(),
        vec![
            SteerTrace::HasPending(true),
            SteerTrace::AutoCompact,
            SteerTrace::Take,
            SteerTrace::HasPending(false),
        ],
        "Final 只探测；回到顶部先压缩再注入"
    );
    assert_eq!(script.request_count(), 3, "收尾 + 摘要 + steer 后采样");
    let requests = script.requests.lock().expect("lock");
    let purposes: Vec<RequestPurpose> = requests.iter().map(|request| request.purpose).collect();
    assert_eq!(
        purposes,
        vec![
            RequestPurpose::Subagent(crate::chat::agent::AgentRole::Explorer),
            RequestPurpose::CompactSummary,
            RequestPurpose::Subagent(crate::chat::agent::AgentRole::Explorer),
        ]
    );
    assert_eq!(
        requests[2].items,
        vec![
            ModelItem::UserText("CURRENT_TURN".into()),
            ModelItem::UserText("STEER_AFTER_FINAL".into()),
        ],
        "压缩保留当前轮任务，steer 紧随其后"
    );
}

/// 等待期间用户撤回 pending：take 为空且 awaited_steer 为真，直接收轮不空跑模型。
#[tokio::test]
async fn withdrawn_pending_steer_completes_without_another_sample() {
    let fixture = Fixture::new("steer-withdrawn");
    let script = Script::new(vec![final_response("first", None)], StopReason::Complete);
    let host = RecordingHost::new()
        .with_pending_steers([true])
        .with_steer_batch(Vec::new());
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(script.request_count(), 1, "撤回后不得再发起采样");
    assert_eq!(
        host.steer_trace(),
        vec![SteerTrace::HasPending(true), SteerTrace::Take]
    );
}

/// 多批 steer 按 loop 顶部顺序依次注入。
#[tokio::test]
async fn multiple_steer_batches_are_injected_in_order() {
    let fixture = Fixture::new("steer-multiple");
    let read_args = r#"{"path":"."}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(&[("c1", "read", &read_args, 0)], None),
            tool_calls(&[("c2", "read", &read_args, 1)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new()
        .with_steer_batch(vec![steer_text("STEER_A")])
        .with_steer_batch(vec![steer_text("STEER_B")]);
    let summary = run_agent_turn(
        context(fixture.path()),
        host,
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(script.request_count(), 3);
    let first = format!("{:?}", script.items_of(1));
    assert!(first.contains("STEER_A") && !first.contains("STEER_B"));
    let second = format!("{:?}", script.items_of(2));
    let a = second.find("STEER_A").expect("A present");
    let b = second.find("STEER_B").expect("B present");
    assert!(a < b, "两批 steer 顺序必须保持");
}

/// 图片进入 UserContent；skill 正文块紧随 steer 用户消息之后。
#[tokio::test]
async fn steer_images_and_skill_blocks_are_injected_in_order() {
    let fixture = Fixture::new("steer-images");
    let script = tool_then_final();
    let host = RecordingHost::new().with_steer_batch(vec![SteerInput {
        text: "LOOK".into(),
        images: vec![ModelImage {
            mime: "image/png".into(),
            bytes: Arc::new(vec![1, 2, 3]),
        }],
        skill_blocks: vec!["SKILL_A".into(), "SKILL_B".into()],
    }]);
    let summary = run_agent_turn(
        context(fixture.path()),
        host,
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    let items = script.items_of(1);
    let index = items
        .iter()
        .position(|item| {
            matches!(item, ModelItem::UserContent { text, images } if text == "LOOK" && images.len() == 1)
        })
        .expect("steer user content present");
    assert_eq!(
        items.get(index + 1),
        Some(&ModelItem::UserText("SKILL_A".into()))
    );
    assert_eq!(
        items.get(index + 2),
        Some(&ModelItem::UserText("SKILL_B".into()))
    );

    let requests = script.requests.lock().expect("lock");
    let last_turn = requests[1].turns.last().expect("steer turn recorded");
    assert_eq!(last_turn.role, "user");
    assert_eq!(last_turn.text, "LOOK");
}

/// take 回执丢失 = 取消：本轮按中断收尾，pending 留给终态 handler。
#[tokio::test]
async fn cancelled_steer_stops_the_turn() {
    let fixture = Fixture::new("steer-cancelled");
    let script = tool_then_final();
    let host = RecordingHost::new().with_steer_error(HostError::Cancelled);
    let summary = run_agent_turn(
        context(fixture.path()),
        host,
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Interrupted);
    assert_eq!(script.request_count(), 1, "取消后不再采样");
}

/// 事件层回执失败：沿用 TurnSummary 的失败映射并停止本轮。
#[tokio::test]
async fn failed_steer_stops_the_turn() {
    let fixture = Fixture::new("steer-failed");
    let script = tool_then_final();
    let host = RecordingHost::new().with_steer_error(HostError::Failed {
        code: "steer turn begin failed".into(),
    });
    let summary = run_agent_turn(
        context(fixture.path()),
        host,
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(
        summary.outcome,
        TurnOutcome::Failed {
            code: "steer turn begin failed".into()
        }
    );
    assert_eq!(script.request_count(), 1, "失败后不再采样");
}
