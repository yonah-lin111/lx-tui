//! ask 面板状态迁移：单选/多选/手写规则与草稿不变量。

use super::*;
use crate::chat::tools::ask::AskOption;

fn key() -> RequestKey {
    RequestKey {
        pane: crate::layout::PaneId::alloc(),
        session: "s".into(),
        generation: 1,
    }
}

fn option(label: &str) -> AskOption {
    AskOption {
        label: label.into(),
        description: format!("{label} 的说明"),
        preview: None,
    }
}

fn question(id: &str, multi_select: bool, labels: &[&str]) -> AskQuestion {
    AskQuestion {
        id: id.into(),
        header: id.into(),
        question: format!("{id}?"),
        multi_select,
        options: labels.iter().map(|label| option(label)).collect(),
    }
}

fn state(questions: Vec<AskQuestion>) -> AskState {
    AskState::new(key(), "call".into(), questions)
}

#[test]
fn single_select_is_exclusive_and_clears_custom_text() {
    let mut ask = state(vec![question("q", false, &["A", "B"])]);
    assert!(ask.choose(0));
    assert_eq!(ask.current_answers(), vec!["A".to_string()]);
    // 再选另一项：单选只保留一个答案。
    ask.choose(1);
    assert_eq!(ask.current_answers(), vec!["B".to_string()]);
    // 手写文本会把单选勾选清掉，反之亦然。
    assert!(ask.begin_edit());
    ask.editor_mut().expect("editor").insert_str("自己写");
    assert!(ask.commit_edit());
    assert_eq!(ask.current_answers(), vec!["自己写".to_string()]);
    assert!(ask.selected[0].is_empty(), "单选下提交手写必须清空勾选");
    ask.choose(0);
    assert_eq!(
        ask.current_answers(),
        vec!["A".to_string()],
        "单选下勾选必须清空手写"
    );
    assert!(!ask.custom_on[0]);
}

#[test]
fn multi_select_toggles_and_coexists_with_custom_text() {
    let mut ask = state(vec![question("q", true, &["A", "B", "C"])]);
    ask.choose(2);
    ask.choose(0);
    assert_eq!(
        ask.current_answers(),
        vec!["C".to_string(), "A".to_string()],
        "多选按勾选顺序，不按选项下标"
    );
    // 再点同一项取消。
    ask.choose(2);
    assert_eq!(ask.current_answers(), vec!["A".to_string()]);
    // 手写追加在最后。
    ask.begin_edit();
    ask.editor_mut().expect("editor").insert_str("补充");
    ask.commit_edit();
    assert_eq!(
        ask.current_answers(),
        vec!["A".to_string(), "补充".to_string()]
    );
    assert_eq!(ask.selected[0], vec![0], "多选下勾选与手写共存");
}

#[test]
fn other_row_enters_edit_and_empty_text_is_not_an_answer() {
    let mut ask = state(vec![question("q", false, &["A"])]);
    assert_eq!(ask.other_row(), 1);
    assert!(ask.choose(1), "「其他」行进入编辑态");
    assert!(ask.editing);
    assert!(!ask.commit_edit(), "空文本不算作答");
    assert!(ask.current_answers().is_empty());
    assert!(ask.begin_edit());
    assert!(ask.cancel_edit(), "Esc 只退出编辑");
    assert!(!ask.editing);
    assert!(!ask.commit_edit(), "已退出编辑时提交无动作");
}

#[test]
fn zero_option_question_only_has_the_other_row() {
    let mut ask = state(vec![question("q", false, &[])]);
    assert_eq!(ask.row_count(), 1);
    assert_eq!(ask.other_row(), 0);
    assert!(!ask.move_highlight(-1), "已到顶");
    assert!(!ask.move_highlight(1), "无下一行可移动");
}

#[test]
fn navigation_keeps_per_question_state_and_is_clamped() {
    let mut ask = state(vec![
        question("a", false, &["A1", "A2"]),
        question("b", true, &["B1"]),
    ]);
    ask.choose(1);
    assert!(ask.next_step());
    ask.choose(0);
    assert_eq!(ask.current, 1);
    assert!(ask.next_step(), "末题之后进入 Confirm");
    assert!(ask.is_confirm());
    assert!(!ask.next_step(), "Confirm 已是最后一步");
    assert!(ask.prev_step(), "Confirm 回到末题");
    assert!(!ask.is_confirm());
    assert!(ask.prev_step());
    assert_eq!(
        ask.current_answers(),
        vec!["A2".to_string()],
        "切题保留各题已选状态"
    );
    // 高亮不循环：顶/底钳制。
    ask.set_highlight(0);
    assert!(!ask.move_highlight(-3));
    assert_eq!(ask.highlight[0], 0);
    assert!(ask.move_highlight(9));
    assert_eq!(ask.highlight[0], ask.row_count() - 1);
}

#[test]
fn preview_scroll_is_clamped_by_preview_lines() {
    let mut previewed = question("q", false, &["A", "B"]);
    previewed.options[0].preview = Some("one\ntwo\nthree".into());
    let mut ask = state(vec![previewed]);
    assert_eq!(ask.preview(), Some("one\ntwo\nthree"));
    assert_eq!(ask.preview_max_scroll(), 2);
    assert!(ask.scroll_preview(99), "一次大滚动钳到上限");
    assert_eq!(ask.preview_scroll[0], 2);
    assert!(!ask.scroll_preview(1), "已到上限不再变化");
    assert!(ask.scroll_preview(-9));
    assert_eq!(ask.preview_scroll[0], 0);
    // 切到无预览项：预览为空，滚动偏移不再增长。
    ask.set_highlight(1);
    assert_eq!(ask.preview(), None);
    assert_eq!(ask.preview_max_scroll(), 0);
}

#[test]
fn answer_editing_never_touches_the_prompt_draft() {
    let mut lx = Lx::new();
    lx.insert_str("草稿不能动");
    let draft = lx.input.clone();
    let cursor = lx.cursor;
    // 面板入口：只写 `lx.ask`，不碰 `input`。
    lx.ask = Some(state(vec![question("q", true, &["A"])]));
    {
        let ask = lx.ask.as_mut().expect("ask");
        ask.choose(0);
        ask.begin_edit();
        ask.editor_mut().expect("editor").insert_str("手写答案");
        ask.commit_edit();
        ask.next_step();
        ask.cancel_edit();
    }
    assert_eq!(lx.input, draft, "作答全程不许改草稿");
    assert_eq!(lx.cursor, cursor, "作答全程不许移动草稿光标");
    // 清空面板也不改草稿。
    lx.ask = None;
    assert_eq!(lx.input, draft);
    assert_eq!(lx.cursor, cursor);
}
