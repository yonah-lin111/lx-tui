//! 工具步骤几何一致性测试：`layout` 行数 vs `render` 实际推进行数、滚动条、
//! 置顶用户行、thinking 命中矩形在含工具步骤时仍与 `messages` 行区间一致。
//!
//! 这些断言针对的正是设计 §10 的硬不变量：**布局与绘制共用同一份步骤行数口径**。

use ratatui::layout::Rect;

use super::*;

/// 测试用缓存：每次调用独立，避免测试之间共享几何快照。
fn cache() -> PaneCache {
    PaneCache::default()
}
use crate::app::state::{
    AskAnsweredView, AskDetailView, AssistantStepView, ChatMessageView, CommandView,
    InteractionAction, PlanSnapshotView, PlanStepView, ProcessInteractionView, ProcessStateView,
    SearchDetailView, ToolCallView, ToolDetailView, ToolStatus, ToolSummary,
};

/// 构造一个 `read` 工具步骤（可选状态）。
fn read_step(position: i64, status: ToolStatus) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-{position}"),
            name: "read".into(),
            position,
            arguments_json: String::new(),
            status,
            summary: Some(Box::new(ToolSummary {
                path: Some("src/ui/style.rs".into()),
                range: Some((12, 40)),
                ..ToolSummary::default()
            })),
            error: None,
            result_json: None,
            detail: None,
        },
    }
}

/// 把工具步骤追加到消息末尾（position 顺延）。
fn push_tool(message: &mut ChatMessageView, status: ToolStatus) {
    let position = message.next_position();
    message.steps.push(read_step(position, status));
}

/// 渲染整块历史并返回绘制推进的行数（与 `layout(...).total_rows` 比对用）。
fn render_rows(area: Rect, lx: &Lx, scroll: usize) -> usize {
    let mut grid = Grid::new(area);
    render(&mut grid, area, lx, scroll, &mut cache())
}

/// 测试用历史：3 条消息 + 多个工具 + thinking 交错，足以溢出小视口。
fn mixed_lx() -> Lx {
    let mut first = message("user", "complete", vec![text_part("第一轮的问题")]);
    let mut assistant = message(
        "assistant",
        "complete",
        vec![reasoning_part("先看看样式"), text_part("A1\nA2\nA3\nA4")],
    );
    push_tool(&mut assistant, ToolStatus::Completed);
    push_tool(&mut assistant, ToolStatus::Failed);
    push_tool(&mut assistant, ToolStatus::Pending);
    let mut second = message("user", "complete", vec![text_part("第二轮的问题")]);
    push_tool(&mut second, ToolStatus::Interrupted);
    first.turn_index = 0;
    assistant.turn_index = 1;
    second.turn_index = 2;
    lx_with(vec![first, assistant, second])
}

#[test]
fn layout_rows_equal_rendered_rows_with_tools_and_thinking() {
    // 多个宽度（含极窄）与展开/折叠组合：布局总行数与实际绘制推进行数逐一相等。
    for width in [4u16, 8, 20, 40, 72] {
        for expanded in [false, true] {
            let mut lx = mixed_lx();
            if expanded {
                // 展开第一条消息里的思考块（步骤下标 0）。
                lx.chat.toggle_reasoning(1, 0);
            }
            let area = Rect::new(0, 0, width, 30);
            let layout = layout(area, &lx, &mut cache());
            assert_eq!(
                render_rows(area, &lx, 0),
                layout.total_rows,
                "宽 {width} / 展开 {expanded}：布局与渲染行数必须相等"
            );
        }
    }
}

#[test]
fn tool_steps_are_counted_inside_message_row_ranges() {
    let lx = mixed_lx();
    let area = Rect::new(0, 0, 60, 30);
    let computed = layout(area, &lx, &mut cache());
    assert_eq!(computed.messages.len(), 3, "三条消息各有行区间");
    // 行区间首尾相接、单调不减，且最后一条覆盖到总行数（含呼吸行）。
    let mut previous_end = 0usize;
    for rows in &computed.messages {
        assert!(rows.start >= previous_end, "行区间不得重叠：{computed:?}");
        assert!(rows.end >= rows.start, "行区间不得倒挂：{computed:?}");
        previous_end = rows.end;
    }
    assert_eq!(previous_end, computed.total_rows, "末条消息含呼吸行");
    // 第二条消息（含 3 个工具步骤）的行区间必须覆盖工具行。
    let assistant = computed.messages[1];
    assert!(
        assistant.end - assistant.start >= 6,
        "3 个工具步骤 + 思考头 + 正文必须都落在区间内：{assistant:?}"
    );
}

#[test]
fn thinking_header_hit_rect_matches_message_row_range_with_tools() {
    let mut lx = mixed_lx();
    lx.chat.toggle_reasoning(1, 0);
    let area = Rect::new(2, 3, 60, 30);
    let computed = layout(area, &lx, &mut cache());
    // thinking 步骤在第二条消息（index 1）的步骤 0，头部必须在它的行区间内。
    let (message_index, step_index, header) = computed
        .thinking
        .first()
        .copied()
        .expect("thinking 头部矩形");
    assert_eq!((message_index, step_index), (1, 0));
    let rows = computed.messages[message_index];
    let row = usize::from(header.y - area.y);
    assert!(
        row >= rows.start && row < rows.end,
        "thinking 头部行 {row} 必须落在消息区间 {rows:?} 内"
    );
    assert_eq!(header.x, area.x);
    assert_eq!(header.width, area.width - SCROLLBAR_COLUMNS);
}

#[test]
fn scrollbar_and_sticky_row_stay_consistent_with_tools() {
    let mut lx = mixed_lx();
    lx.chat.follow_bottom = false;
    let area = Rect::new(0, 0, 24, 6);
    let layout = layout(area, &lx, &mut cache());
    // 顶行落在第一条 assistant 回复里 → 需要钉第一轮 prompt，body 让位一行。
    lx.chat.history_scroll = layout.messages[1].start;
    let computed = view(area, &lx, &mut cache());
    assert_eq!(computed.sticky, Some(0), "含工具步骤时仍能钉住用户消息");
    assert_eq!(computed.body.height, area.height - 1, "让位一行");
    // 滚动条按 body 几何给出，且与渲染同一可见行数。
    let bar = scrollbar(
        computed.body,
        computed.total_rows,
        usize::from(computed.body.height),
        computed.scroll,
    );
    let bar = bar.expect("内容溢出应有滚动条");
    assert_eq!(bar.total, computed.total_rows);
    assert_eq!(bar.visible, usize::from(computed.body.height));
    assert!(bar.thumb.height >= 1 && bar.thumb.bottom() <= computed.body.bottom());
    // 渲染推进的行数与 view 的总行数一致（含工具步骤）。
    assert_eq!(
        render_rows(computed.body, &lx, computed.scroll),
        computed.total_rows,
        "滚动后渲染行数仍与布局一致"
    );
}

#[test]
fn narrow_and_short_viewports_never_panic_with_tools() {
    let lx = mixed_lx();
    for width in [1u16, 2, 4, 8] {
        for height in [0u16, 1, 2, 3] {
            let area = Rect::new(0, 0, width, height);
            let computed = layout(area, &lx, &mut cache());
            let rendered = render_rows(area, &lx, 0);
            if width == 0 {
                assert_eq!(computed.total_rows, 0, "零宽不产生行");
                assert_eq!(rendered, 0);
            } else if height == 0 {
                // 无可见行：渲染不写任何内容（布局仍给出滚动口径，与既有实现一致）。
                assert_eq!(rendered, 0, "零高不高绘制");
            } else {
                assert_eq!(rendered, computed.total_rows, "宽 {width} 高 {height}");
            }
            // 视图 / 滚动条 / 命中都不 panic。
            let view = view(area, &lx, &mut cache());
            let _ = scrollbar(area, view.total_rows, usize::from(area.height), 0);
            let _ = sticky_rect(&view);
        }
    }
}

#[test]
fn tool_steps_occupy_a_header_plus_one_or_two_detail_rows() {
    // 工具步骤 = 1 行标题 + 1~4 行详情（路径/范围/错误/取消各成节点，
    // 折叠态最多 2 行内容 + 1 行省略提示）。布局与渲染必须一致。
    for status in [
        ToolStatus::InputStreaming,
        ToolStatus::Pending,
        ToolStatus::Running,
        ToolStatus::Completed,
        ToolStatus::Failed,
        ToolStatus::Cancelled,
        ToolStatus::Interrupted,
    ] {
        let mut message = message("assistant", "complete", Vec::new());
        push_tool(&mut message, status);
        let lx = lx_with(vec![message]);
        let area = Rect::new(0, 0, 40, 10);
        let computed = layout(area, &lx, &mut cache());
        // `read`：标题 1 + 路径 1 +（错误/取消 0~1 行）+ 呼吸 1。
        assert!(
            (3..=4).contains(&computed.total_rows),
            "{status:?} 应为 3~4 行，实际 {}",
            computed.total_rows
        );
        assert_eq!(
            render_rows(area, &lx, 0),
            computed.total_rows,
            "{status:?} 渲染行数一致"
        );
    }
}

/// 连续同名工具合并为一组：一个组头 + 逐调用树体，组内不空行。
#[test]
fn consecutive_tool_steps_share_one_group_header() {
    let mut message = message("assistant", "complete", Vec::new());
    for (index, status) in [
        ToolStatus::Completed,
        ToolStatus::Failed,
        ToolStatus::Running,
    ]
    .into_iter()
    .enumerate()
    {
        let mut step = read_step(index as i64 + 1, status);
        if let AssistantStepView::Tool { call, .. } = &mut step
            && status == ToolStatus::Failed
        {
            call.error = Some("boom".into());
        }
        message.steps.push(step);
    }
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 14);
    let computed = layout(area, &lx, &mut cache());
    // 组头 1 + 三个调用体各 1 + 呼吸行 1 = 5。
    assert_eq!(computed.total_rows, 5, "合并组只用一个组头");
    assert_eq!(render_rows(area, &lx, 0), 5);

    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rows: Vec<String> = (0..5).map(|row| row_text(&buf, area, row)).collect();
    assert!(rows[0].contains("3 calls"), "组头写调用数：{rows:?}");
    assert!(rows[0].contains("1 failed"), "组头聚合失败：{rows:?}");
    assert!(rows[0].contains("1 running"), "组头聚合运行中：{rows:?}");
    assert!(
        rows[1].starts_with(text::agent::TREE_BRANCH),
        "非末调用用 ├─：{rows:?}"
    );
    assert!(rows[1].contains("src/ui/style.rs"), "{rows:?}");
    assert!(rows[2].contains("boom"), "{rows:?}");
    assert!(
        rows[3].starts_with(text::agent::TREE_LEAF),
        "末调用用 └─：{rows:?}"
    );
    assert!(rows[4].trim().is_empty(), "末尾呼吸行：{rows:?}");
}

/// 合并后的命令组保留每个调用的退出状态，组头只出现一次动词。
#[test]
fn command_group_keeps_each_exit_status_on_its_body() {
    let mut message = message("assistant", "complete", Vec::new());
    for (position, id, code) in [(1, 0, 0), (2, 1, 1)] {
        message.steps.push(command_step(position, id, code));
    }
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 70, 12);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_EXEC_COMMAND).count(),
        1,
        "组头只出现一次 Run command：{text}"
    );
    assert!(text.contains("2 calls"), "{text}");
    assert!(text.contains("$ cargo check"), "{text}");
    assert!(text.contains("$ cargo test"), "{text}");
    assert!(text.contains("Exited 0"), "{text}");
    assert!(text.contains("exit 1"), "{text}");
    let area = Rect::new(0, 0, 70, 12);
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows,
        "组内每行都计入快照"
    );
}

/// 合并后的搜索组把总命中数放组头，单调用命中数留在各自首行。
#[test]
fn search_group_aggregates_matches_and_keeps_each_status() {
    let mut message = message("assistant", "complete", Vec::new());
    for (position, matches) in [(1, 3), (2, 2)] {
        message.steps.push(search_step(position, matches));
    }
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 80, 10);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_TOOL_SEARCH).count(),
        1,
        "组头只出现一次 Search：{text}"
    );
    assert!(text.contains("2 calls"), "{text}");
    assert!(text.contains("5 matches"), "组头汇总命中：{text}");
    assert!(text.contains("3 matches"), "首调用保留命中：{text}");
    assert!(text.contains("2 matches"), "次调用保留命中：{text}");
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows
    );
}

/// 组头不可点击，调用体各自可点。
#[test]
fn group_header_has_no_tool_card_region() {
    let mut message = message("assistant", "complete", Vec::new());
    message
        .steps
        .push(file_step(1, "call-a", "edit", ToolStatus::Completed));
    message
        .steps
        .push(file_step(2, "call-b", "edit", ToolStatus::Completed));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 10);
    let mut cache = cache();
    let snapshot = cache.snapshot(super::content_rect(area).width, &lx);
    let header_start = snapshot
        .entries
        .first()
        .and_then(|entry| {
            entry.blocks.iter().find_map(|block| match block.source {
                BlockSource::ToolGroupHeader { .. } => Some(block.start),
                _ => None,
            })
        })
        .expect("组头块");
    assert!(
        snapshot.cards.iter().all(|card| match card.kind {
            CardKind::Tool { .. } => card.card_start != header_start,
            _ => true,
        }),
        "组头不得登记工具点击面：{:?}",
        snapshot.cards
    );
    assert_eq!(
        snapshot
            .cards
            .iter()
            .filter(|card| matches!(card.kind, CardKind::Tool { .. }))
            .count(),
        2,
        "两个调用体各自一个点击面"
    );
}

/// `write_stdin` 按显示动词分组：读输出与写入不混组。
#[test]
fn write_stdin_groups_by_display_verb() {
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(write_stdin_step(
        1,
        InteractionAction::ReadOutput,
        "cargo test",
    ));
    message.steps.push(write_stdin_step(
        2,
        InteractionAction::ReadOutput,
        "cargo test",
    ));
    message.steps.push(write_stdin_step(
        3,
        InteractionAction::SendInput,
        "cargo test",
    ));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 80, 16);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_READ_OUTPUT).count(),
        1,
        "读输出组头只出现一次：{text}"
    );
    assert!(text.contains("2 calls"), "{text}");
    assert_eq!(
        text.matches(text::agent::VERB_SEND_INPUT).count(),
        1,
        "写入单独成组：{text}"
    );
}

/// `ask` 是阻塞交互，连续两次也各自一张卡。
#[test]
fn ask_calls_are_never_grouped() {
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(ask_step(1, "first?"));
    message.steps.push(ask_step(2, "second?"));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 12);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_TOOL_ASK).count(),
        2,
        "每次 ask 都有自己的标题：{text}"
    );
    assert!(!text.contains("2 calls"), "{text}");
}

fn write_stdin_step(position: i64, action: InteractionAction, command: &str) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-stdin-{position}"),
            name: "write_stdin".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Interaction(
                ProcessInteractionView {
                    action,
                    process_session_id: 1,
                    command: command.into(),
                    input_preview: (action == InteractionAction::SendInput)
                        .then(|| "y\\n".to_string()),
                    output: "ok".into(),
                    state: ProcessStateView::Running,
                    exit_code: None,
                    duration_ms: None,
                    truncated: false,
                    dropped_bytes: 0,
                },
            ))),
        },
    }
}

fn ask_step(position: i64, question: &str) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-ask-{position}"),
            name: "ask".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Ask(AskDetailView {
                questions: vec![AskAnsweredView {
                    header: "Q".into(),
                    question: question.into(),
                    answers: vec!["yes".into()],
                }],
            }))),
        },
    }
}

/// 计划快照只显示最新一次，组头写 updates 而不是 calls。
#[test]
fn plan_snapshot_group_shows_only_the_latest_update() {
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(plan_step(1, "old step"));
    message.steps.push(plan_step(2, "new step"));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 10);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_UPDATE_PLAN).count(),
        1,
        "组头只出现一次 Plan：{text}"
    );
    assert!(text.contains("2 updates"), "{text}");
    assert!(text.contains("new step"), "{text}");
    assert!(!text.contains("old step"), "旧快照不铺列表：{text}");
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows
    );
}

fn plan_step(position: i64, step: &str) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-plan-{position}"),
            name: "update_plan".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Plan(PlanSnapshotView {
                revision: position as u64,
                explanation: None,
                steps: vec![(step.to_string(), PlanStepView::Completed)],
            }))),
        },
    }
}

fn command_step(position: i64, id: i32, exit: i32) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-cmd-{position}"),
            name: "exec_command".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Command(CommandView {
                process_session_id: Some(id.unsigned_abs().into()),
                command: if id == 0 {
                    "cargo check".into()
                } else {
                    "cargo test".into()
                },
                cwd: "/w".into(),
                tty: false,
                state: ProcessStateView::Exited { code: Some(exit) },
                duration_ms: Some(1_200),
                output_preview: Vec::new(),
                truncated: false,
                dropped_bytes: 0,
                output_tail: String::new(),
                started_at_ms: None,
                ended_at_ms: None,
            }))),
        },
    }
}

fn search_step(position: i64, matches: usize) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-search-{position}"),
            name: "search".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: Some(Box::new(ToolSummary {
                path: Some("src".into()),
                pattern: Some("fixme".into()),
                matches: Some(matches),
                ..ToolSummary::default()
            })),
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Search(SearchDetailView {
                matches: Vec::new(),
                truncated: false,
                files_scanned: 1,
            }))),
        },
    }
}

/// 工具与正文之间的空行仍在（异构块保留空行）。
#[test]
fn tool_and_text_still_have_a_blank_line_between_them() {
    let mut message = message("assistant", "complete", vec![text_part("正文")]);
    message.steps.push(read_step(1, ToolStatus::Completed));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 12);
    let computed = layout(area, &lx, &mut cache());
    // 正文 1 + 空行 1 + read 2（标题+路径）+ 呼吸行 1 = 5。
    assert_eq!(computed.total_rows, 5, "异构块之间保留空行");
    assert_eq!(render_rows(area, &lx, 0), 5);
}

#[test]
fn completed_thinking_has_no_spinner_even_after_tool_steps() {
    // 思考完成（`completed_at_ms` 有值）后即便消息仍在流中，也不再呼吸（静态方块）；
    // 工具步骤的存在不影响该判断，工具行保留自己的呼吸方块。
    let mut part = reasoning_part("已完成思考");
    part.started_at_ms = Some(1_000);
    part.ended_at_ms = Some(2_000);
    part.completed_at_ms = Some(2_000);
    let mut message = message("assistant", "streaming", vec![part, text_part("接着说话")]);
    push_tool(&mut message, ToolStatus::Running);
    let mut lx = lx_with(vec![message]);
    assert!(
        !lx.chat.reasoning_streaming(0, 0),
        "思考块已有完成时刻 → 不再进行中"
    );
    let area = Rect::new(0, 0, 48, 8);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let header = row_text(&buf, area, 0);
    assert!(
        header.starts_with("● Thought"),
        "完成态显示静态圆点 + Thought：{header}"
    );
    assert!(
        !header
            .chars()
            .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
        "思考完成态不得出现 braille 转轮：{header}"
    );
    // 工具标题行按**动词**定位：状态词已搬到第 2 行（`  └─ running`），
    // 标题行只剩 `■ Read`，方块呼吸只发生在这里。
    let tool_line = |buf: &Buffer| -> String {
        (0..8)
            .map(|row| row_text(buf, area, row))
            .find(|line| line.contains(text::agent::VERB_TOOL_READ))
            .expect("工具标题行")
    };
    // 工具行自身仍在进行中 → 它自己的方块呼吸（帧 0 实心）。
    assert!(
        tool_line(&buf).starts_with(text::agent::STEP_MARK_FILLED),
        "工具行带呼吸方块：{}",
        tool_line(&buf)
    );
    // 相位翻到空心帧：只有运行中的工具行变空心，已完成的思考头保持实心。
    lx.chat.loading_frame = text::agent::STEP_BREATH_TICKS;
    // 动画推进中：工具行才允许进入空心相位（停止后恒实心）。
    lx.chat.status.streaming = true;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    assert!(
        row_text(&buf, area, 0).starts_with("● Thought"),
        "思考头不随 loading 帧变化：{}",
        row_text(&buf, area, 0)
    );
    assert!(
        tool_line(&buf).starts_with(text::agent::STEP_MARK_HOLLOW),
        "运行中的工具行进入空心相位：{}",
        tool_line(&buf)
    );
}

/// 取消 / 完成 / 中断等**终态**的方块必须实心：空心会被读成「还在跑」。
/// 取消曾经恒空心（用户带截图要求修正），这里把两种呼吸相位都锁住。
#[test]
fn terminal_tool_states_keep_a_filled_marker_in_every_frame() {
    for status in [
        ToolStatus::Completed,
        ToolStatus::Cancelled,
        ToolStatus::Interrupted,
        ToolStatus::Failed,
    ] {
        for (frame, streaming) in [
            (0, false),
            (text::agent::STEP_BREATH_TICKS, true),
            (0, true),
        ] {
            let mut message = message("assistant", "complete", Vec::new());
            push_tool(&mut message, status);
            let mut lx = lx_with(vec![message]);
            lx.chat.loading_frame = frame;
            lx.chat.status.streaming = streaming;
            let area = Rect::new(0, 0, 40, 10);
            let mut buf = Buffer::empty(area);
            crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
            let line = row_text(&buf, area, 0);
            assert!(
                line.starts_with(text::agent::STEP_MARK_FILLED),
                "{status:?} frame={frame} streaming={streaming} 必须是实心方块：{line}"
            );
        }
    }
}

// ===== 连续同种文件调用合并成一张卡 =====

/// 构造一个带差异正文的文件工具步骤（`name` 决定 `edit` / `write`）。
fn file_step(position: i64, call_id: &str, name: &str, status: ToolStatus) -> AssistantStepView {
    use crate::app::state::{FileChangeView, FileDiffDetailView, FileDiffView};
    use crate::chat::tools::{DiffHunk, DiffLine, DiffLineKind, FileChangeKind, FileDiff};

    let diff = FileDiff::Complete {
        hunks: vec![DiffHunk {
            // 4 个增删行 > 预览的 3 行额度：折叠态出省略提示，展开态必然多行。
            lines: (1..=2)
                .flat_map(|n| {
                    [
                        DiffLine {
                            kind: DiffLineKind::Removed,
                            old_line: Some(n),
                            new_line: None,
                            text: format!("old{n}"),
                            no_final_newline: false,
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line: None,
                            new_line: Some(n),
                            text: format!("new{n}"),
                            no_final_newline: false,
                        },
                    ]
                })
                .collect(),
        }],
        simplified: false,
    };
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: call_id.into(),
            name: name.into(),
            position,
            arguments_json: String::new(),
            status,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(crate::app::state::ToolDetailView::File(
                FileDiffDetailView {
                    files: vec![FileChangeView {
                        path: format!("src/file{position}.rs"),
                        to: None,
                        kind: FileChangeKind::Updated,
                        added: Some(1),
                        removed: Some(1),
                        counts_known: true,
                        simplified: false,
                        diff: Some(FileDiffView::from(&diff)),
                    }],
                    files_total: 1,
                    added: Some(1),
                    removed: Some(1),
                    simplified: false,
                    omitted_files: 0,
                    single: false,
                },
            ))),
        },
    }
}

/// 历史区整页文本（用于断言内容差异）。
fn rendered_text(lx: &Lx, area: Rect) -> String {
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, lx, 0);
    (0..area.height)
        .map(|y| row_text(&buf, area, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 卡片命中数（每张可折叠卡片一个）；没有动作行，因此只有一个计数。
fn card_hits(lx: &Lx, area: Rect) -> usize {
    layout(area, lx, &mut cache())
        .cards
        .iter()
        .filter(|hit| matches!(hit.kind, CardKind::Tool { .. }))
        .count()
}

/// 连续文件工具合并为一个组头；每个调用仍有独立点击面，组内不空行。
#[test]
fn consecutive_file_tools_share_a_group_header() {
    let area = Rect::new(0, 0, 60, 24);
    let mut message = message("assistant", "complete", Vec::new());
    message
        .steps
        .push(file_step(1, "call-a", "edit", ToolStatus::Completed));
    message
        .steps
        .push(file_step(2, "call-b", "edit", ToolStatus::Completed));
    let lx = lx_with(vec![message.clone()]);
    let computed = layout(area, &lx, &mut cache());
    assert_eq!(card_hits(&lx, area), 2, "两个调用各一张点击面");
    assert_eq!(render_rows(area, &lx, 0), computed.total_rows);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_TOOL_EDIT).count(),
        1,
        "组头只出现一次 Edit：{text}"
    );
    assert!(text.contains("2 calls"), "组头写调用数：{text}");
    assert!(!text.contains("2 files"), "不再出现旧合并标题：{text}");
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let tool_cards: Vec<_> = computed
        .cards
        .iter()
        .filter(|card| matches!(card.kind, CardKind::Tool { .. }))
        .collect();
    let blanks: Vec<usize> = (usize::from(tool_cards[0].card.bottom())
        ..usize::from(tool_cards[1].card.y))
        .filter(|row| {
            row_text(&buf, area, u16::try_from(*row).unwrap_or(u16::MAX))
                .trim()
                .is_empty()
        })
        .collect();
    assert_eq!(blanks.len(), 0, "组内调用之间不空行：{blanks:?}");

    // 点第一个调用只展开第一个，不影响同组第二个。
    let mut expanded = lx_with(vec![message.clone()]);
    assert!(expanded.chat.toggle_detail("assistant-1", "call-a"));
    let expanded_layout = layout(area, &expanded, &mut cache());
    assert_eq!(render_rows(area, &expanded, 0), expanded_layout.total_rows);
    assert_eq!(card_hits(&expanded, area), 2, "展开后仍是两个调用体");
}

/// 正文步骤打断即分组：`edit` → 正文 → `edit` 是两张卡。
#[test]
fn a_part_step_separates_two_file_cards() {
    let mut message = message("assistant", "complete", vec![text_part("中间说一句")]);
    message.steps = vec![
        file_step(1, "call-a", "edit", ToolStatus::Completed),
        AssistantStepView::Part {
            position: 2,
            part: text_part("中间说一句"),
        },
        file_step(3, "call-b", "edit", ToolStatus::Completed),
    ];
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 20);
    assert_eq!(card_hits(&lx, area), 2, "正文打断合并");
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows
    );
}

/// 长 markdown 正文滚动：绘制必须从增量前缀与活跃尾部无缝读取。
///
/// 正文 80 行（全部固化进稳定前缀）+ 末尾表格（未闭合前留在尾部），
/// 滚动到前缀/尾部交界处，屏幕行必须与源码行一一对应。
#[test]
fn long_markdown_scroll_reads_prefix_and_tail() {
    let mut body = (0..80)
        .map(|index| format!("line {index:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    body.push_str("\n\n| 名称 | 数值 |\n| --- | ---: |\n| 甲 | 1 |");
    let lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![text_part(&body)],
    )]);
    let area = Rect::new(0, 0, 40, 10);
    let mut cache = cache();
    let mut grid = Grid::new(area);
    // 先在顶部建快照与增量前缀，再滚到交界处：共享同一份缓存。
    let _ = render(&mut grid, area, &lx, 0, &mut cache);
    let mut grid = Grid::new(area);
    let _ = render(&mut grid, area, &lx, 78, &mut cache);
    let rows: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| grid.cell(x, y).ch).collect())
        .collect();
    assert!(rows[0].contains("line 78"), "{rows:?}");
    assert!(rows[1].contains("line 79"), "{rows:?}");
    assert!(rows[2].trim().is_empty(), "段落与表格之间空一行：{rows:?}");
    assert!(rows[3].contains('┌'), "表格尾部在空行后紧接前缀：{rows:?}");
    assert!(rows.iter().any(|row| row.contains('甲')), "{rows:?}");
}
