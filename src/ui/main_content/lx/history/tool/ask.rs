//! `ask` 卡片：标题（方块 + 动词 + 状态）+ 每题摘要与题干节点。
//!
//! ```text
//! ■ Ask · 2 questions
//!   ├─ Q1: 布局
//!   │  A1: 双栏
//!   └─ Q2: 主题
//!      A2: 深色, 用青色
//! ```
//!
//! - 运行中只写 `waiting for you`：参数还在流式接收，不解析、不猜题数；
//! - 每题是一块多行节点：题号/答案标签在首列，答案与题干是同一块的续行；
//! - 展开态在题号后追加完整题干（题干最多 500 字，折行由树体统一处理）；
//! - **永不展示 preview**：字符画只服务于决策现场，历史里是噪音；
//! - `rows` 与 `draw` 必须严格同源（`history::layout` 与 `render` 共用）。

use ratatui::layout::Rect;

use crate::app::state::{AskDetailView, ToolCallView, ToolDetailView, ToolStatus};
use crate::ui::style;
use crate::ui::text::agent as a;

use super::super::{Grid, visible_row};
use super::tree::{self, Node, Section};
use super::{Segment, header_with_verb, put_segments, single_line, title_rows};

/// 单行文本取样上限（限住超长答案的每帧成本）。
const SAMPLE_CHARS: usize = 512;

/// 每题一块：`Qn: 标签` +（展开时）题干 + `An: 答案`。
fn nodes(view: &AskDetailView, expanded: bool) -> Vec<Node> {
    view.questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let mut lines = vec![vec![
                Segment::new(a::ask_question_prefix(index), style::agent_step_detail()),
                Segment::new(
                    single_line(&question.header, SAMPLE_CHARS),
                    style::chat_tool_param(),
                ),
            ]];
            if expanded {
                lines.push(vec![Segment::new(
                    single_line(&question.question, SAMPLE_CHARS),
                    style::text(),
                )]);
            }
            let answers = if question.answers.is_empty() {
                a::ASK_NO_ANSWER.to_string()
            } else {
                question
                    .answers
                    .iter()
                    .map(|answer| single_line(answer, SAMPLE_CHARS))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            lines.push(vec![
                Segment::new(a::ask_answer_prefix(index), style::agent_step_detail()),
                Segment::new(answers, style::text()),
            ]);
            Node::block(lines)
        })
        .collect()
}

/// 卡片占用的渲染行数；与 [`draw`] 严格相等。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    let Some(ToolDetailView::Ask(view)) = call.detail.as_deref() else {
        return title_rows();
    };
    title_rows()
        + tree::layout(&[Section::fixed(nodes(view, expanded))], width)
            .rows
            .len()
}

/// 绘制卡片；推进行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    animating: bool,
    expanded: bool,
) {
    let width = rect.width;
    if width == 0 {
        return;
    }
    let view = match call.detail.as_deref() {
        Some(ToolDetailView::Ask(view)) => Some(view),
        _ => None,
    };
    let running = matches!(
        call.status,
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running
    );
    let status = if running {
        Some(a::ASK_WAITING.to_string())
    } else {
        view.map(|view| a::ask_question_count(view.questions.len()))
    };
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header_with_verb(
            call,
            width,
            loading_frame,
            animating,
            a::VERB_TOOL_ASK,
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
        &tree::layout(&[Section::fixed(nodes(view, expanded))], width).rows,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AskAnsweredView, ToolDetailView, ToolStatus};
    use crate::ui::text::agent as text_agent;

    fn ask() -> ToolCallView {
        ToolCallView {
            call_id: "call-ask".into(),
            name: "ask".into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Ask(AskDetailView {
                questions: vec![
                    AskAnsweredView {
                        header: "布局".into(),
                        question: "你要哪种布局？".into(),
                        answers: vec!["双栏".into()],
                    },
                    AskAnsweredView {
                        header: "主题".into(),
                        question: "深色还是浅色？".into(),
                        answers: vec!["深色".into(), "用青色".into()],
                    },
                ],
            }))),
        }
    }

    fn flat(call: &ToolCallView, width: u16, expanded: bool) -> String {
        let height = rows(call, width, expanded).max(1) as u16;
        let rect = Rect::new(0, 0, width, height);
        let mut grid = Grid::new(rect);
        let mut row = 0;
        draw(&mut grid, rect, 0, &mut row, call, 0, false, expanded);
        assert_eq!(row, rows(call, width, expanded));
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
    fn folded_answers_do_not_use_dot_separator() {
        let call = ask();
        let text = flat(&call, 60, false);
        assert!(text.contains("布局"), "{text}");
        assert!(text.contains("双栏"), "{text}");
        let body: String = text.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(!body.contains('·'), "问答摘要不拼 `·`：{text}");
        assert!(!text.contains("你要哪种布局"), "折叠态不显示题干：{text}");
    }

    #[test]
    fn expanded_adds_the_question_body() {
        let call = ask();
        let collapsed = rows(&call, 60, false);
        let expanded = rows(&call, 60, true);
        assert!(expanded > collapsed, "展开必须多出题干");
        let text = flat(&call, 60, true);
        assert!(text.contains("你要哪种布局"), "{text}");
        assert!(text.contains("深色还是浅色"), "{text}");
    }

    #[test]
    fn answers_are_tree_nodes() {
        let call = ask();
        let text = flat(&call, 60, false);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[1].starts_with(text_agent::TREE_BRANCH), "{text}");
        assert!(lines[1].contains("Q1: 布局"), "{text}");
        assert!(lines[2].starts_with(text_agent::TREE_BAR), "{text}");
        assert!(lines[2].contains("A1: 双栏"), "{text}");
        assert!(lines[3].starts_with(text_agent::TREE_LEAF), "{text}");
        assert!(lines[3].contains("Q2: 主题"), "{text}");
        assert!(lines[4].starts_with(text_agent::TREE_PAD), "{text}");
        assert!(lines[4].contains("A2: 深色, 用青色"), "{text}");
    }
}
