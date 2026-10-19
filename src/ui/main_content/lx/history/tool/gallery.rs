//! UI gallery 入口：把**生产渲染路径**直接画进 `Buffer`，并提供三种新增工具的样例卡片。
//!
//! gallery 不复制"好看但假"的样例：尺寸与绘制都调用 `history/tool.rs` 的同一套
//! `rows` / `draw`，所以查看器里看到的就是聊天历史里实际显示的东西。

use ratatui::layout::Rect;

use crate::app::state::ToolCallView;

use super::super::Grid;
use super::{draw, rows};

/// 把一张卡片的**生产渲染路径**画进 `Buffer`（UI gallery 复用同一行生成函数）。
///
/// 这是唯一给 gallery 的入口：不复制一份"好看但假"的样例，实际显示与聊天历史
/// 里的一模一样。返回实际占用的行数（便于 gallery 计算自然高度）。
pub(crate) fn render_card_into(
    buf: &mut ratatui::buffer::Buffer,
    rect: Rect,
    call: &ToolCallView,
    expanded: bool,
) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, call, 0, expanded);
    for dy in 0..rect.height {
        for dx in 0..rect.width {
            let glyph = grid.cell(rect.x + dx, rect.y + dy);
            if let Some(cell) = buf.cell_mut((rect.x + dx, rect.y + dy)) {
                if glyph.ch == crate::ui::main_content::lx::grid::CONTINUATION {
                    cell.set_diff_option(ratatui::buffer::CellDiffOption::Skip);
                    continue;
                }
                cell.set_char(glyph.ch);
                cell.set_style(glyph.style);
                cell.set_diff_option(ratatui::buffer::CellDiffOption::None);
            }
        }
    }
    row_index
}

/// 卡片在给定宽度下需要的渲染行数（gallery 计算自然高度用）。
pub(crate) fn card_rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    rows(call, width, expanded)
}

/// 三种新增工具的样例卡片（gallery 与文档共用同一份构造）。
pub(crate) fn sample_card(
    kind: crate::app::ui_gallery::AgentStepId,
) -> Option<(ToolCallView, bool)> {
    use crate::app::state::{
        CommandView, InteractionAction, PlanSnapshotView, PlanStepView, ProcessInteractionView,
        ProcessStateView, ToolDetailView, ToolStatus,
    };
    use crate::app::ui_gallery::AgentStepId;
    let mut call = ToolCallView::new("gallery", "exec_command", 0);
    match kind {
        AgentStepId::Command => {
            call.name = "exec_command".into();
            call.status = ToolStatus::Running;
            call.detail = Some(Box::new(ToolDetailView::Command(CommandView {
                process_session_id: Some(12),
                command: "cargo test chat::tools".into(),
                cwd: "~/projects/lx-tui".into(),
                tty: false,
                state: ProcessStateView::Running,
                output_preview: Vec::new(),
                truncated: false,
                dropped_bytes: 0,
                output_tail: "running 24 tests\ntest read::empty_file ... ok\ntest write::created ... ok\ntest result: ok. 24 passed\n".into(),
                started_at_ms: Some(0),
                ended_at_ms: None,
            })));
            Some((call, false))
        }
        AgentStepId::ProcessInteraction => {
            call.name = "write_stdin".into();
            call.status = ToolStatus::Completed;
            call.detail = Some(Box::new(ToolDetailView::Interaction(
                ProcessInteractionView {
                    action: InteractionAction::SendInput,
                    process_session_id: 12,
                    command: "cargo test chat::tools".into(),
                    input_preview: Some("y\\n".into()),
                    output: "test result: ok. 24 passed\n".into(),
                    state: ProcessStateView::Running,
                    exit_code: None,
                    truncated: false,
                    dropped_bytes: 0,
                },
            )));
            Some((call, false))
        }
        AgentStepId::Plan => {
            call.name = "update_plan".into();
            call.status = ToolStatus::Completed;
            call.detail = Some(Box::new(ToolDetailView::Plan(PlanSnapshotView {
                revision: 1,
                explanation: Some("Implementation finished; checking behavior.".into()),
                steps: vec![
                    ("Implement command tools".into(), PlanStepView::Completed),
                    ("Verify process cleanup".into(), PlanStepView::InProgress),
                    ("Review terminal display".into(), PlanStepView::Pending),
                ],
            })));
            Some((call, true))
        }
        _ => None,
    }
}
