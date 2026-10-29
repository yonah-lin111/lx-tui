//! `update_plan` 卡片：完成计数、三态步骤与解释各自成树节点。
//!
//! ```text
//! ■ Plan · 1/3 completed
//!   └─ [x] Implement command tools
//!      [>] Verify process cleanup
//!      [ ] Review terminal display
//!      Implementation finished; checking behavior.
//! ```
//!
//! - 完成计数在标题右侧；步骤正文是一块多行节点，只在首行画 `└─`；
//! - 计数只认 `completed`，`in_progress` 不算完成；
//! - 三种状态都用 ASCII 标记（`[ ]` / `[>]` / `[x]`），不只靠颜色；
//! - 长步骤按显示列折行，续行由共用树体补 `│` / 空白；
//! - 空计划只有一个 `Plan cleared` 节点；
//! - 不提供用户点击改状态：这里是模型列出的待办。

use ratatui::layout::Rect;

use crate::app::state::{PlanSnapshotView, PlanStepView, ToolCallView};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::tree::{self, Node, Section};
use super::{Segment, header_with_verb, put_segments, single_line, title_rows};

/// 计划解释文字的单行取样上限。
const EXPLANATION_SAMPLE_CHARS: usize = 512;

/// 计划卡片的全部节点（标题之外）。
fn nodes(view: &PlanSnapshotView) -> Vec<Node> {
    if view.is_empty() {
        return vec![Node::text(
            text::agent::PLAN_EMPTY,
            style::agent_step_detail(),
        )];
    }
    let mut lines: Vec<Vec<Segment>> = Vec::new();
    for (step, status) in &view.steps {
        let marker = text::agent::plan_marker(*status);
        // 「进行中」与其它运行中工具同档（橙黄）；完成用 dim，待办用默认前景。
        let step_style = match status {
            PlanStepView::Completed => style::agent_step_detail(),
            PlanStepView::InProgress => style::chat_tool_verb_running(),
            PlanStepView::Pending => style::text(),
        };
        let marker_style = match status {
            PlanStepView::InProgress => style::chat_step_label_alive(),
            _ => style::agent_step_detail(),
        };
        lines.push(vec![
            Segment::new(format!("{marker} "), marker_style),
            Segment::new(single_line(step, EXPLANATION_SAMPLE_CHARS), step_style),
        ]);
    }
    if let Some(explanation) = explanation_of(view) {
        lines.push(vec![Segment::new(explanation, style::agent_step_detail())]);
    }
    vec![Node::block(lines)]
}

/// 解释文本（单行化、限长）；空解释返回 None。
fn explanation_of(view: &PlanSnapshotView) -> Option<String> {
    let explanation = view.explanation.as_deref()?;
    let line = single_line(explanation, EXPLANATION_SAMPLE_CHARS);
    (!line.is_empty()).then_some(line)
}

/// 计划卡片占用的渲染行数。
pub(super) fn rows(call: &ToolCallView, width: u16, _expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    let Some(crate::app::state::ToolDetailView::Plan(view)) = call.detail.as_deref() else {
        return title_rows();
    };
    title_rows()
        + tree::layout(&[Section::fixed(nodes(view))], width)
            .rows
            .len()
}

/// 绘制计划卡片；推进行数必须与 [`rows`] 严格相等。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    animating: bool,
) {
    let width = rect.width;
    if width == 0 {
        return;
    }
    let view = match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Plan(view)) => Some(view),
        _ => None,
    };
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let status = view
            .map(|view| text::agent::plan_progress(view.completed(), view.steps.len()))
            .filter(|_| view.is_some_and(|view| !view.is_empty()));
        let segments = header_with_verb(
            call,
            width,
            loading_frame,
            animating,
            text::agent::VERB_UPDATE_PLAN,
            status,
        );
        put_segments(grid, rect.x, y, width, &segments);
    }
    *row_index += 1;
    let Some(view) = view else {
        return;
    };
    tree::draw(
        grid,
        rect,
        scroll,
        row_index,
        &tree::layout(&[Section::fixed(nodes(view))], width).rows,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{PlanSnapshotView, PlanStepView, ToolCallView, ToolDetailView};

    fn plan(steps: Vec<(&str, PlanStepView)>, explanation: Option<&str>) -> ToolCallView {
        ToolCallView {
            call_id: "call-plan".into(),
            name: "update_plan".into(),
            position: 0,
            arguments_json: "{}".into(),
            status: crate::app::state::ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Plan(PlanSnapshotView {
                revision: 1,
                explanation: explanation.map(str::to_string),
                steps: steps
                    .into_iter()
                    .map(|(step, status)| (step.to_string(), status))
                    .collect(),
            }))),
        }
    }

    fn flat(call: &ToolCallView, width: u16) -> String {
        let height = rows(call, width, false).max(1) as u16;
        let rect = Rect::new(0, 0, width, height);
        let mut grid = Grid::new(rect);
        let mut row = 0;
        draw(&mut grid, rect, 0, &mut row, call, 0, false);
        assert_eq!(row, rows(call, width, false));
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                let glyph = grid.cell(x, y);
                if glyph.ch != crate::ui::main_content::lx::grid::CONTINUATION {
                    out.push(glyph.ch);
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn title_has_progress_and_steps_form_one_block() {
        let call = plan(
            vec![
                ("Implement command tools", PlanStepView::Completed),
                ("Verify process cleanup", PlanStepView::InProgress),
                ("Review terminal display", PlanStepView::Pending),
            ],
            Some("checking behavior"),
        );
        let text = flat(&call, 70);
        assert!(text.contains("1/3 completed"), "{text}");
        assert!(text.contains("[x] Implement command tools"), "{text}");
        assert!(text.contains("[>] Verify process cleanup"), "{text}");
        assert!(text.contains("[ ] Review terminal display"), "{text}");
        assert!(text.contains("checking behavior"), "{text}");
        let title = text.lines().next().unwrap_or("");
        assert!(title.contains("1/3 completed"), "计数在标题：{title}");
        let body: String = text.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(!body.contains('·'), "步骤区不拼 `·`：{text}");
        let leaves = text.matches(text::agent::TREE_LEAF).count();
        assert_eq!(leaves, 1, "整块步骤只有一个收口：{text}");
        assert!(
            text.contains(text::agent::TREE_PAD),
            "步骤续行左对齐：{text}"
        );
    }

    #[test]
    fn empty_plan_closes_with_cleared_node() {
        let call = plan(Vec::new(), Some("wrapped up"));
        let text = flat(&call, 50);
        assert!(text.contains(text::agent::PLAN_EMPTY), "{text}");
        assert!(!text.contains("completed"), "{text}");
        assert!(!text.contains("wrapped up"), "空计划不画解释：{text}");
    }

    #[test]
    fn long_chinese_step_wraps_and_geometry_holds() {
        let call = plan(
            vec![(
                "执行命令工具并验证进程清理与终端显示的中文步骤名称",
                PlanStepView::InProgress,
            )],
            None,
        );
        for width in [10u16, 20, 30] {
            let text = flat(&call, width);
            assert_eq!(rows(&call, width, false), text.lines().count());
            assert!(!text.contains('\u{fffd}'), "{text}");
            assert!(
                text.lines()
                    .any(|line| line.starts_with(text::agent::TREE_PAD)),
                "步骤续行左对齐：{text}"
            );
        }
    }
}
