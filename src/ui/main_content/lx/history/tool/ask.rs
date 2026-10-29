//! `ask` 卡片：标题（方块 + 动词 + 状态）+ 每题问答节点。
//!
//! ```text
//! ■ Ask · 2 questions
//!   ├─ Q1: 你要哪种布局？
//!   │  A1: 双栏
//!   └─ Q2: 深色还是浅色？
//!      A2: 深色, 用青色
//! ```
//!
//! - 运行中只写 `waiting for you`：参数还在流式接收，不解析、不猜题数；
//! - 每题是一块多行节点：直接显示 `Qn: 题干` 与 `An: 答案`，不显示弹窗 tab 短标签（`header`）；
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

/// 每题一块：`Qn: 题干` + `An: 答案`（不显示弹窗 tab 标签，仅在题干为空时回退 `header`）。
fn nodes(view: &AskDetailView) -> Vec<Node> {
    view.questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let prompt = if question.question.trim().is_empty() {
                &question.header
            } else {
                &question.question
            };
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
            Node::block(vec![
                vec![
                    Segment::new(a::ask_question_prefix(index), style::agent_step_detail()),
                    Segment::new(
                        single_line(prompt, SAMPLE_CHARS),
                        style::agent_step_detail(),
                    ),
                ],
                vec![
                    Segment::new(a::ask_answer_prefix(index), style::text()),
                    Segment::new(answers, style::text()),
                ],
            ])
        })
        .collect()
}

/// 卡片占用的渲染行数；与 [`draw`] 严格相等。
pub(super) fn rows(call: &ToolCallView, width: u16, _expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    let Some(ToolDetailView::Ask(view)) = call.detail.as_deref() else {
        return title_rows();
    };
    title_rows()
        + tree::layout(&[Section::fixed(nodes(view))], width)
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
    _expanded: bool,
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
        &tree::layout(&[Section::fixed(nodes(view))], width).rows,
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
    fn shows_question_text_directly_without_tab_header() {
        let call = ask();
        let text = flat(&call, 60, false);
        assert!(text.contains("你要哪种布局？"), "{text}");
        assert!(text.contains("深色还是浅色？"), "{text}");
        assert!(text.contains("双栏"), "{text}");
        assert!(!text.contains("Q1: 布局"), "不显示短 tab header：{text}");
        assert!(!text.contains("Q2: 主题"), "不显示短 tab header：{text}");
        let body: String = text.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(!body.contains('·'), "问答摘要不拼 `·`：{text}");
    }

    #[test]
    fn answers_are_tree_nodes() {
        let call = ask();
        let text = flat(&call, 60, false);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[1].starts_with(text_agent::TREE_BRANCH), "{text}");
        assert!(lines[1].contains("Q1: 你要哪种布局？"), "{text}");
        assert!(lines[2].starts_with(text_agent::TREE_BAR), "{text}");
        assert!(lines[2].contains("A1: 双栏"), "{text}");
        assert!(lines[3].starts_with(text_agent::TREE_LEAF), "{text}");
        assert!(lines[3].contains("Q2: 深色还是浅色？"), "{text}");
        assert!(lines[4].starts_with(text_agent::TREE_PAD), "{text}");
        assert!(lines[4].contains("A2: 深色, 用青色"), "{text}");
        let Some(ToolDetailView::Ask(view)) = call.detail.as_deref() else {
            panic!("missing ask detail");
        };
        let rendered = tree::layout(&[Section::fixed(nodes(view))], 60).rows;
        assert_eq!(rendered[0].segments[0].style, style::agent_step_detail());
        assert_eq!(rendered[0].segments[1].style, style::agent_step_detail());
        assert_eq!(rendered[1].segments[0].style, style::text());
        assert_eq!(rendered[1].segments[1].style, style::text());
    }

    #[test]
    fn running_card_only_says_waiting() {
        let mut call = ask();
        call.status = ToolStatus::Running;
        call.detail = None;
        assert_eq!(rows(&call, 60, false), 1, "运行中只有标题行");
        let text = flat(&call, 60, false);
        assert!(text.contains("Ask"), "标题必须显示动词：{text:?}");
        assert!(
            text.contains("waiting for you"),
            "运行中必须等待作答：{text:?}"
        );
    }

    #[test]
    fn empty_answers_and_zero_width_do_not_panic() {
        let mut call = ask();
        call.detail = Some(Box::new(ToolDetailView::Ask(AskDetailView {
            questions: vec![AskAnsweredView {
                header: "布局".into(),
                question: "布局？".into(),
                answers: Vec::new(),
            }],
        })));
        let text = flat(&call, 40, false);
        assert!(text.contains(text_agent::ASK_NO_ANSWER), "{text}");
        assert_eq!(rows(&call, 0, false), 0);
        call.detail = Some(Box::new(ToolDetailView::Ask(AskDetailView {
            questions: Vec::new(),
        })));
        assert_eq!(rows(&call, 40, false), 1);
    }
}
