//! 工具步骤几何一致性测试：`layout` 行数 vs `render` 实际推进行数、滚动条、
//! 置顶用户行、thinking 命中矩形在含工具步骤时仍与 `messages` 行区间一致。
//!
//! 这些断言针对的正是设计 §10 的硬不变量：**布局与绘制共用同一份步骤行数口径**。

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    AssistantStepView, ChatMessageView, ToolCallView, ToolStatus, ToolSummary,
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
            summary: Some(ToolSummary {
                path: Some("src/ui/style.rs".into()),
                range: Some((12, 40)),
                ..ToolSummary::default()
            }),
            error: None,
            result_json: None,
        },
    }
}

/// 把工具步骤追加到消息末尾（position 顺延）。
fn push_tool(message: &mut ChatMessageView, status: ToolStatus) {
    let position = message.next_position();
    message.steps.push(read_step(position, status));
}

/// 渲染整块历史并返回绘制推进的行数（与 `layout().total_rows` 比对用）。
fn render_rows(area: Rect, lx: &Lx, scroll: usize) -> usize {
    let mut grid = Grid::new(area);
    render(&mut grid, area, lx, scroll)
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
            let layout = layout(area, &lx);
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
    let computed = layout(area, &lx);
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
    let computed = layout(area, &lx);
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
    let layout = layout(area, &lx);
    // 顶行落在第一条 assistant 回复里 → 需要钉第一轮 prompt，body 让位一行。
    lx.chat.history_scroll = layout.messages[1].start;
    let computed = view(area, &lx);
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
            let computed = layout(area, &lx);
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
            let view = view(area, &lx);
            let _ = scrollbar(area, view.total_rows, usize::from(area.height), 0);
            let _ = sticky_rect(&view);
        }
    }
}

#[test]
fn tool_steps_with_all_statuses_stay_one_row_each() {
    // 每个状态都必须只占 1 行：布局行数 = 工具数 × 1（其余块固定）。
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
        let computed = layout(area, &lx);
        // 工具行 1 + 消息间空行（呼吸行）1 = 2。
        assert_eq!(computed.total_rows, 2, "{status:?} 仍只占 1 行");
        assert_eq!(render_rows(area, &lx, 0), 2, "{status:?} 渲染行数一致");
    }
}

#[test]
fn completed_thinking_has_no_spinner_even_after_tool_steps() {
    // 思考完成（`completed_at_ms` 有值）后即便消息仍在流中，也不再渲染转轮；
    // 工具步骤的存在不影响该判断。
    let mut part = reasoning_part("已完成思考");
    part.started_at_ms = Some(1_000);
    part.ended_at_ms = Some(2_000);
    part.completed_at_ms = Some(2_000);
    let mut message = message("assistant", "streaming", vec![part, text_part("接着说话")]);
    push_tool(&mut message, ToolStatus::Running);
    let lx = lx_with(vec![message]);
    assert!(
        !lx.chat.reasoning_streaming(0, 0),
        "思考块已有完成时刻 → 不再进行中"
    );
    let area = Rect::new(0, 0, 48, 8);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let header = row_text(&buf, area, 0);
    assert!(
        header.starts_with("▸ Thought"),
        "完成态显示折叠头而非转轮：{header}"
    );
    for frame in text::LX_LOADING_FRAMES {
        assert!(
            !header.contains(frame),
            "思考完成态不得出现转轮 {frame}：{header}"
        );
    }
    // 但工具行自身仍在进行中 → 它自己的转轮保留（状态是步骤各自的）。
    let joined = (0..6)
        .map(|row| row_text(&buf, area, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains(text::tool_status_label(ToolStatus::Running)),
        "工具行显示自己的状态：{joined}"
    );
    assert!(
        joined
            .lines()
            .any(|line| line.starts_with(text::LX_LOADING_FRAMES[0])),
        "工具仍在运行 → 工具行保留自己的转轮：{joined}"
    );
}
