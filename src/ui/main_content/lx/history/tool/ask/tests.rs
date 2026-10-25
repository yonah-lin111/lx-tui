//! `ask` 卡片：行数与绘制严格一致、运行中不猜题数、空答案不 panic。

use super::*;
use crate::app::state::{AskAnsweredView, AskDetailView, ToolCallView, ToolDetailView, ToolStatus};

fn answered(header: &str, question: &str, answers: &[&str]) -> AskAnsweredView {
    AskAnsweredView {
        header: header.into(),
        question: question.into(),
        answers: answers.iter().map(|answer| (*answer).to_string()).collect(),
    }
}

fn call(status: ToolStatus, detail: Option<AskDetailView>) -> ToolCallView {
    ToolCallView {
        call_id: "call-ask".into(),
        name: "ask".into(),
        position: 0,
        arguments_json: String::new(),
        status,
        summary: None,
        error: None,
        result_json: None,
        detail: detail.map(|view| Box::new(ToolDetailView::Ask(view))),
    }
}

fn detail() -> AskDetailView {
    AskDetailView {
        questions: vec![
            answered("布局", "侧栏用哪种布局？", &["双栏"]),
            answered("主题", "主题用什么颜色？", &["深色", "青色"]),
        ],
    }
}

fn render_rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    let area = Rect::new(0, 0, width, 20);
    let mut grid = Grid::new(area);
    let mut row_index = 0;
    draw(&mut grid, area, 0, &mut row_index, call, 0, false, expanded);
    row_index
}

#[test]
fn rows_and_draw_agree_when_folded_and_expanded() {
    let call = call(ToolStatus::Completed, Some(detail()));
    for expanded in [false, true] {
        let expected = rows(&call, 60, expanded);
        assert_eq!(
            render_rows(&call, 60, expanded),
            expected,
            "expanded={expanded} 时绘制行数与布局不一致"
        );
    }
    assert_eq!(rows(&call, 60, false), 3, "标题 + 两题各一行");
    assert!(rows(&call, 60, true) > 3, "展开必须追加题干行");
}

#[test]
fn running_card_only_says_waiting() {
    let call = call(ToolStatus::Running, None);
    assert_eq!(rows(&call, 60, false), 1, "运行中只有标题行");
    let area = Rect::new(0, 0, 60, 4);
    let mut grid = Grid::new(area);
    let mut row_index = 0;
    draw(&mut grid, area, 0, &mut row_index, &call, 0, true, false);
    let text: String = (0..60).map(|x| grid.cell(x, 0).ch).collect();
    assert!(text.contains("Ask"), "标题必须显示动词：{text:?}");
    assert!(
        text.contains("waiting for you"),
        "运行中必须等待作答：{text:?}"
    );
}

#[test]
fn empty_answers_do_not_panic() {
    let call = call(
        ToolStatus::Completed,
        Some(AskDetailView {
            questions: vec![answered("布局", "布局？", &[])],
        }),
    );
    assert_eq!(rows(&call, 40, false), 2);
    assert_eq!(render_rows(&call, 40, false), 2);
}

#[test]
fn zero_width_or_empty_detail_degrades_to_title() {
    let call = call(
        ToolStatus::Completed,
        Some(AskDetailView { questions: vec![] }),
    );
    assert_eq!(rows(&call, 40, false), 1);
    assert_eq!(rows(&call, 0, false), 0);
}

#[test]
fn wide_characters_and_long_answers_stay_on_one_line() {
    let call = call(
        ToolStatus::Completed,
        Some(AskDetailView {
            questions: vec![answered(
                "主题",
                &"很长的问题".repeat(40),
                &["深色主题配青色高亮"],
            )],
        }),
    );
    let area = Rect::new(0, 0, 24, 6);
    let mut grid = Grid::new(area);
    let mut row_index = 0;
    draw(&mut grid, area, 0, &mut row_index, &call, 0, false, false);
    assert_eq!(row_index, rows(&call, 24, false));
    // 折叠行不越过卡片宽度（不写半个宽字符）。
    let mut line = String::new();
    for x in 0..24 {
        let ch = grid.cell(x, 1).ch;
        if ch != super::super::super::super::grid::CONTINUATION {
            line.push(ch);
        }
    }
    assert!(!line.is_empty());
}

#[test]
fn the_card_registers_a_click_face_only_when_expansion_adds_rows() {
    // 生产上「展开」只能经点击面触发（`history_hit::card_at` → `card_regions`）；
    // 不登记就等于 expanded 分支永远不可达。
    let call = call(ToolStatus::Completed, Some(detail()));
    let width = 60u16;
    let step_rows = rows(&call, width, false);
    let regions =
        super::super::card_regions(&call, width, super::super::Folds::default(), step_rows);
    let body = regions
        .iter()
        .find(|region| region.section == crate::app::state::CardSection::Body)
        .expect("ask 卡片必须可点击展开");
    assert!(body.clickable);
    assert_eq!(body.card.rows, step_rows, "整卡都是点击面");
    let folded = rows(&call, width, false);
    let expanded = rows(&call, width, true);
    assert!(expanded > folded, "展开必须真的多出行");
}
