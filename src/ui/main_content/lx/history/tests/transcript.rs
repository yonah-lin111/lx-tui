//! 真实工具步骤行的整块观感：占行数、步骤标记与 2 列缩进不漂移。

use super::*;

/// 构造工具步骤（`read` / Pending，带路径与行范围摘要）。
fn tool_step(position: i64) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: crate::app::state::ToolCallView {
            call_id: format!("call-{position}"),
            name: "read".into(),
            position,
            arguments_json: r#"{"path":"src/a.rs","offset":3,"limit":5}"#.into(),
            status: crate::app::state::ToolStatus::Pending,
            summary: Some(Box::new(crate::app::state::ToolSummary {
                path: Some("src/a.rs".into()),
                range: Some((3, 7)),
                ..Default::default()
            })),
            error: None,
            result_json: None,
            detail: None,
        },
    }
}

/// 工具步骤占两行（`■ 动词` + `  └─ 详情`）：布局行数与渲染行数一致，
/// 且文本可见（不吞掉、不错位）。
#[test]
fn tool_step_occupies_two_rows_and_renders_summary() {
    let mut message = message("assistant", "complete", vec![text_part("正文")]);
    message.steps.push(tool_step(1));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 20);
    let computed = layout(area, &lx, &mut cache());
    // 正文 1 + 块间空行 1 + 工具 2 行 + 消息间空行 1 = 5。
    assert_eq!(computed.total_rows, 5);

    let mut grid = Grid::new(area);
    render(&mut grid, area, &lx, 0, &mut cache());
    let mut rows: Vec<String> = Vec::new();
    for y in 0..7u16 {
        let mut row = String::new();
        for x in 0..area.right() {
            row.push(grid.cell(x, y).ch);
        }
        rows.push(row);
    }
    let painted = rows.join("\n");
    let squeezed: String = painted
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '\0')
        .collect();
    assert!(squeezed.contains("正文"), "正文仍渲染：{painted}");
    assert!(
        squeezed.contains(text::agent::VERB_TOOL_READ),
        "工具行动词可见：{painted}"
    );
    // 第 1 行只有方块 + 动词（无状态文字、无路径）。
    let header_row = rows
        .iter()
        .find(|row| row.contains(text::agent::VERB_TOOL_READ))
        .expect("工具标题行");
    assert_eq!(
        header_row.trim_end(),
        format!(
            "{} {}",
            text::agent::STEP_MARK_FILLED,
            text::agent::VERB_TOOL_READ
        ),
        "标题行只有方块 + 动词：{header_row:?}"
    );
    // 第 2 行是 `└─` + 参数 + 状态词兜底（pending 没有参数摘要之外的状态字段，
    // 这里 pending 有摘要 → 只显示参数与范围）。
    let detail_row = rows
        .iter()
        .find(|row| row.contains("src/a.rs"))
        .expect("详情行");
    assert!(
        detail_row
            .trim_start()
            .starts_with(text::agent::MARKER_TREE_LEAF),
        "详情行以 └─ 起头：{detail_row:?}"
    );
    assert!(squeezed.contains("3-7"), "行范围可见：{painted}");
}

/// 仅工具步骤的消息也占行（不会因“没有 text part”而整条消失）。
#[test]
fn tool_only_message_still_occupies_rows() {
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(tool_step(0));
    let lx = lx_with(vec![message]);
    let computed = layout(Rect::new(0, 0, 40, 10), &lx, &mut cache());
    // 工具 2 行 + 消息间空行/呼吸行 1 = 3。
    assert_eq!(computed.total_rows, 3);
}

#[test]
fn transcript_lays_out_step_marks_with_two_column_gutter() {
    // 一份完整观感锁定：用户消息 `> `、每个步骤的方块标记与 2 列内容缩进
    // （思考头 / 思考正文 / 正文块 / 工具行）、块间空行。
    let mut assistant = message(
        "assistant",
        "streaming",
        vec![
            reasoning_part("思考甲\n思考乙"),
            text_part("正文第一行\n正文第二行"),
        ],
    );
    assistant.steps.push(tool_step(2));
    let mut lx = lx_with(vec![
        message("user", "complete", vec![text_part("用户问题")]),
        assistant,
    ]);
    lx.chat.toggle_reasoning(1, 0);
    lx.chat.loading_frame = text::agent::STEP_BREATH_TICKS; // 呼吸进入空心相位
    // 动画推进中才呼吸（停止后恒实心，见 `StepMark::glyph`）。
    lx.chat.status.streaming = true;
    let area = Rect::new(0, 0, 56, 12);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    // 10 行内容 + 1 行呼吸位：工具行改成两行后多出 1 行。
    let rows: Vec<String> = (0..11).map(|row| row_text(&buf, area, row)).collect();
    assert_eq!(
        rows,
        vec![
            "> 用户问题",
            "",
            "○ Thinking",
            "  思考甲",
            "  思考乙",
            "",
            "● 正文第一行",
            "  正文第二行",
            "",
            "□ Read",
            "  └─ src/a.rs · 3-7",
        ],
        "步骤方块、2 列缩进与两行式工具行的整块观感"
    );
}
