//! 文案派生（A01–A11）：todo 优先级、阶段判定、assistant 定位与会话隔离。

use super::*;

// ===== 文案派生（A01–A11）=====

/// A01：计划有 InProgress 时显示 todo，即使当前正在思考。
#[test]
fn todo_wins_over_thinking_and_tools() {
    let mut lx = streaming_lx(
        "a1",
        vec![reasoning_step(0, true), tool_step(1, ToolStatus::Running)],
    );
    lx.chat.plan = Some(plan(&[
        ("第一步", PlanStepView::Completed),
        ("修复输入框布局", PlanStepView::InProgress),
    ]));
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains("修复输入框布局"), "{row:?}");
    assert!(!row.contains(text::chat::THINKING_LABEL), "{row:?}");
    assert!(!row.contains(text::chat::RUNNING_TOOLS_LABEL), "{row:?}");
    assert!(!row.contains(text::chat::LOADING_LABEL), "{row:?}");
}

/// A02：只有 Pending 步骤 + 开放思考 → Thinking（不把 Pending 当当前任务）。
#[test]
fn pending_plan_steps_fall_back_to_thinking() {
    let mut lx = streaming_lx("a1", vec![reasoning_step(0, true)]);
    lx.chat.plan = Some(plan(&[
        ("待办一", PlanStepView::Pending),
        ("待办二", PlanStepView::Pending),
    ]));
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains(text::chat::THINKING_LABEL), "{row:?}");
    assert!(!row.contains("待办"), "Pending 不是当前任务：{row:?}");
}

/// A02 续：计划全部 Completed 或已清空时同样回退实际阶段。
#[test]
fn completed_or_cleared_plan_falls_back_to_stage() {
    let mut lx = streaming_lx("a1", vec![tool_step(0, ToolStatus::Running)]);
    lx.chat.plan = Some(plan(&[("做完的", PlanStepView::Completed)]));
    let (grid, bar) = draw_bar(&lx, 40);
    assert!(row_text(&grid, bar).contains(text::chat::RUNNING_TOOLS_LABEL));
    lx.chat.plan = None;
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains(text::chat::RUNNING_TOOLS_LABEL), "{row:?}");
    assert!(!row.contains("做完的"), "{row:?}");
}

/// A03：无计划，当前 tool 为 Pending 或 Running → Running tools。
#[test]
fn pending_and_running_tools_show_running_tools() {
    for status in [ToolStatus::Pending, ToolStatus::Running] {
        let lx = streaming_lx("a1", vec![tool_step(0, status)]);
        let (grid, bar) = draw_bar(&lx, 40);
        let row = row_text(&grid, bar);
        assert!(
            row.contains(text::chat::RUNNING_TOOLS_LABEL),
            "{status:?}: {row:?}"
        );
    }
}

/// A04：只有 InputStreaming 工具（参数还在接收）不算「在运行」。
#[test]
fn input_streaming_tool_is_not_running() {
    let lx = streaming_lx("a1", vec![tool_step(0, ToolStatus::InputStreaming)]);
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains(text::chat::LOADING_LABEL), "{row:?}");
    assert!(!row.contains(text::chat::RUNNING_TOOLS_LABEL), "{row:?}");
}

/// A05：工具已终态、最后一步是普通正文 → Working。
#[test]
fn settled_tools_fall_back_to_working() {
    let lx = streaming_lx(
        "a1",
        vec![
            tool_step(0, ToolStatus::Completed),
            text_step("继续输出正文"),
        ],
    );
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains(text::chat::LOADING_LABEL), "{row:?}");
    assert!(!row.contains(text::chat::RUNNING_TOOLS_LABEL), "{row:?}");
}

/// A06：历史有思考、当前思考已关闭 → 不显示 Thinking。
#[test]
fn closed_reasoning_is_not_thinking() {
    let lx = streaming_lx(
        "a1",
        vec![reasoning_step(0, false), text_step("想完了，开始写")],
    );
    let (grid, bar) = draw_bar(&lx, 40);
    assert!(row_text(&grid, bar).contains(text::chat::LOADING_LABEL));
}

/// A06 续：开放思考不是**最后一步**（后面已有正文/工具）也不算正在思考。
#[test]
fn open_reasoning_must_be_the_last_step() {
    let lx = streaming_lx(
        "a1",
        vec![reasoning_step(0, true), text_step("已经有正文了")],
    );
    let (grid, bar) = draw_bar(&lx, 40);
    assert!(row_text(&grid, bar).contains(text::chat::LOADING_LABEL));
}

/// A07：Committing / Finishing 不读取上一轮阶段。
#[test]
fn committing_and_finishing_never_show_stale_stage() {
    use crate::chat::events::StopReason;
    let key = RequestKey {
        pane: PaneId::alloc(),
        session: "session".into(),
        generation: 2,
    };
    for request in [
        RequestState::Committing {
            generation: 2,
            draft: "下一轮".into(),
        },
        RequestState::Finishing {
            key: key.clone(),
            reason: StopReason::Complete,
        },
    ] {
        let mut lx = streaming_lx("a1", vec![reasoning_step(0, true)]);
        // 上一条历史仍是「开放思考」，但本轮没有 Streaming assistant。
        lx.chat.request = request;
        let (grid, bar) = draw_bar(&lx, 40);
        let row = row_text(&grid, bar);
        assert!(row.contains(text::chat::LOADING_LABEL), "{row:?}");
        assert!(!row.contains(text::chat::THINKING_LABEL), "{row:?}");
    }
}

/// A08：本地命令记录追加在当前 assistant 之后，仍按 assistant_id 派生阶段。
#[test]
fn shell_record_after_assistant_keeps_model_stage() {
    let mut lx = streaming_lx("a1", vec![reasoning_step(0, true)]);
    lx.chat.messages.push(shell_message("shell-1"));
    let (grid, bar) = draw_bar(&lx, 40);
    assert!(
        row_text(&grid, bar).contains(text::chat::THINKING_LABEL),
        "必须定位 assistant 而不是最后一条消息"
    );
}

/// A09：计划更新 / 清空 / 全部完成后，下一帧按新计划或实际阶段显示（无缓存）。
#[test]
fn plan_changes_apply_on_the_next_frame() {
    let mut lx = streaming_lx("a1", vec![reasoning_step(0, true)]);
    lx.chat.plan = Some(plan(&[("旧任务", PlanStepView::InProgress)]));
    let (grid, bar) = draw_bar(&lx, 40);
    assert!(row_text(&grid, bar).contains("旧任务"));

    lx.chat.plan = Some(plan(&[("新任务", PlanStepView::InProgress)]));
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains("新任务"), "{row:?}");
    assert!(!row.contains("旧任务"), "不得残留旧文案：{row:?}");

    lx.chat.plan = Some(plan(&[("新任务", PlanStepView::Completed)]));
    let (grid, bar) = draw_bar(&lx, 40);
    assert!(row_text(&grid, bar).contains(text::chat::THINKING_LABEL));

    lx.chat.plan = None;
    let (grid, bar) = draw_bar(&lx, 40);
    assert!(row_text(&grid, bar).contains(text::chat::THINKING_LABEL));
}

/// A10：新一轮沿用未完成计划（会话级 canonical plan）。
#[test]
fn new_turn_keeps_the_unfinished_plan() {
    let mut lx = streaming_lx("a2", vec![reasoning_step(0, true)]);
    lx.chat.plan = Some(plan(&[("修复输入框布局", PlanStepView::InProgress)]));
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains("修复输入框布局"), "{row:?}");
    assert!(!row.contains(text::chat::THINKING_LABEL), "{row:?}");
}

/// A11：切会话后只读当前会话的 canonical plan（旧计划已清空时不显示）。
#[test]
fn session_switch_does_not_leak_previous_plan() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("上个会话的任务", PlanStepView::InProgress)]));
    lx.chat.reset_session_view();
    lx.chat.request = streaming_request("b1");
    lx.chat.status.streaming = true;
    lx.chat
        .messages
        .push(assistant("b1", "streaming", vec![text_step("答")]));
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(!row.contains("上个会话的任务"), "{row:?}");
    assert!(row.contains(text::chat::LOADING_LABEL), "{row:?}");
}
