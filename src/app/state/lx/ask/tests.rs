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
fn submit_row_follows_the_tab_style() {
    // 单题单选：没有 tab、没有提交胶囊，选中即提交。
    let single = state(vec![question("q", false, &["A", "B"])]);
    assert!(single.is_single_select_only());
    assert_eq!(single.tab_len(), 1);
    assert!(!single.is_confirm());
    assert!(!single.has_submit_row());
    assert_eq!(single.submit_row(), None);
    assert_eq!(single.row_count(), 3, "2 选项 + Other");

    // 单题多选：题目 + Submit 两个 tab，选项底部有提交胶囊。
    let multi = state(vec![question("q", true, &["A", "B"])]);
    assert!(!multi.is_single_select_only());
    assert_eq!(multi.tab_len(), 2);
    assert!(multi.has_submit_row());
    assert_eq!(multi.other_row(), 2);
    assert_eq!(multi.submit_row(), Some(3));
    assert_eq!(multi.row_count(), 4, "2 选项 + Other + Submit");

    // 多题：每题一个 tab，末尾是 Submit 汇总；单选的题目没有选项底部按钮。
    let mut flow = state(vec![
        question("a", false, &["A1", "A2"]),
        question("b", false, &["B1"]),
    ]);
    assert_eq!(flow.tab_len(), 3, "两题 + Submit");
    assert!(!flow.has_submit_row());
    assert_eq!(flow.submit_row(), None);
    assert!(flow.next_step());
    assert_eq!(flow.current, 1);
    assert!(!flow.has_submit_row(), "多题里的单选不显示选项提交按钮");
    assert!(flow.next_step());
    assert!(flow.is_confirm(), "末题前进到 Submit 汇总");
    assert!(flow.has_submit_row(), "汇总 tab 显示提交胶囊");
    assert_eq!(flow.row_count(), 0, "汇总 tab 没有可高亮项");
    assert_eq!(flow.submit_row(), None, "汇总 tab 的胶囊不是高亮行");
}

#[test]
fn question_has_preview_ignores_blank_previews() {
    let mut plain = question("q", false, &["A"]);
    plain.options[0].preview = Some("   ".into());
    let ask = state(vec![plain]);
    assert!(!ask.question_has_preview(), "空白 preview 不占预览区");

    let mut previewed = question("q", false, &["A", "B"]);
    previewed.options[1].preview = Some("┌──┐".into());
    let ask = state(vec![previewed]);
    assert!(ask.question_has_preview());
}

#[test]
fn goto_tab_switches_between_questions_and_the_submit_tab() {
    let mut ask = state(vec![
        question("a", false, &["A1"]),
        question("b", false, &["B1"]),
    ]);
    assert!(ask.begin_edit());
    assert!(ask.goto_tab(9), "越界 tab 进入 Submit 汇总");
    assert!(ask.is_confirm());
    assert!(!ask.editing, "切 tab 退出编辑态");
    assert!(!ask.goto_tab(9), "已在汇总 tab");
    assert!(ask.goto_tab(0));
    assert!(!ask.is_confirm());
    assert_eq!(ask.current, 0);
    assert!(!ask.goto_tab(0), "同题不产生变化");
}

#[test]
fn navigation_cycles_tabs_and_wraps_the_highlight() {
    let mut ask = state(vec![
        question("a", false, &["A1", "A2"]),
        question("b", true, &["B1"]),
    ]);
    ask.choose(1);
    assert!(ask.next_step());
    ask.choose(0);
    assert_eq!(ask.current, 1);
    assert!(ask.next_step(), "末题前进到 Submit 汇总");
    assert!(ask.is_confirm());
    assert!(ask.next_step(), "汇总循环回第一题");
    assert_eq!(ask.current, 0);
    assert!(!ask.is_confirm());
    assert_eq!(
        ask.current_answers(),
        vec!["A2".to_string()],
        "切题保留各题已选状态"
    );
    // `Shift+Tab`：第一题后退回到汇总，再后退落到末题。
    assert!(ask.prev_step());
    assert!(ask.is_confirm());
    assert!(ask.prev_step());
    assert_eq!(ask.current, 1);
    // 高亮循环：顶/底互相绕回（当前题是第 2 题）。
    ask.highlight[1] = 0;
    assert!(ask.move_highlight(-1));
    assert_eq!(ask.highlight[1], ask.row_count() - 1);
    assert!(ask.move_highlight(1));
    assert_eq!(ask.highlight[1], 0);
}

#[test]
fn started_at_is_recorded_and_compensated_into_the_turn_clock() {
    let ask = state(vec![question("q", false, &["A"])]);
    let now = crate::db::now_ms();
    assert!(
        (now - ask.started_at_ms).abs() < 5_000,
        "创建面板必须记录挂起起点"
    );
    let mut chat = crate::app::state::LxChat::new();
    chat.turn_started_at_ms = Some(now);
    ask.compensate_turn_clock(&mut chat);
    let started = chat.turn_started_at_ms.expect("起点仍在");
    assert!(started >= now, "补偿后起点不早于提问开始");
}

#[test]
fn page_scroll_moves_the_unified_window_and_clamps() {
    let mut ask = state(vec![question("q", true, &["A", "B"])]);
    assert!(!ask.scroll_page(1), "还没有几何时不产生位移");

    ask.sync_list_window(0, 20, 5);
    assert!(ask.scroll_page(1), "PageDown 前进一屏");
    assert_eq!(ask.viewport(), Some(5));
    assert!(ask.scroll_window(-1));
    assert_eq!(ask.viewport(), Some(4));
    assert!(ask.scroll_page(9), "一次大滚动钳到上限");
    assert_eq!(ask.viewport(), Some(15));
    assert_eq!(ask.highlight[0], 0, "滚动绝不动高亮");

    // 窗口跟随高亮（无显式视口）时，滚动从上一帧的窗口起点继续。
    ask.viewport = None;
    ask.sync_list_window(3, 20, 5);
    assert!(ask.scroll_window(1));
    assert_eq!(ask.viewport(), Some(4));

    // 内容装得下时翻页无效，也不会留下虚假视口。
    ask.viewport = None;
    ask.sync_list_window(0, 4, 8);
    assert!(!ask.scroll_page(1));
    assert_eq!(ask.viewport(), None);
}

#[test]
fn custom_editor_keeps_leading_bang_as_content() {
    // `!` 是答案的一部分：不能被吸成 Shell 模式（全角 `！` 更不能被改成半角）。
    let mut ask = state(vec![question("q", true, &["A"])]);
    ask.begin_edit();
    let editor = ask.editor_mut().expect("editor");
    for ch in "！重要".chars() {
        editor.insert_char(ch);
    }
    assert_eq!(editor.text(), "！重要", "文本逐字保留");
    assert_eq!(
        editor.visual_rows().len(),
        1,
        "视觉行按原始缓冲计算，与 text() 同源"
    );
    assert!(ask.commit_edit());
    assert_eq!(ask.answers_at(0), vec!["！重要".to_string()]);
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

#[test]
fn ask_editor_selection_and_editing() {
    let mut editor = AskEditor::new();
    editor.insert_str("custom answer");
    assert_eq!(editor.text(), "custom answer");

    // 选区
    editor.begin_text_selection(0, 7);
    editor.drag_text_selection(0, 13);
    assert!(editor.has_text_selection());
    assert_eq!(editor.selected_text().as_deref(), Some("answer"));

    // 键入替换
    editor.insert_char('!');
    assert_eq!(editor.text(), "custom !");
    assert!(!editor.has_text_selection());

    // 快捷键: delete_to_line_end
    editor.home();
    editor.move_right(); // 光标在 1
    editor.delete_to_line_end();
    assert_eq!(editor.text(), "c");

    // delete_to_line_start
    editor.insert_str("omplete");
    editor.delete_to_line_start();
    assert_eq!(editor.text(), "");
}

#[test]
fn ask_editor_selection_overflow_preserves_text_and_undo() {
    let mut editor = AskEditor::new();
    let text = "a".repeat(MAX_ASK_CUSTOM_BYTES);
    assert!(editor.insert_str(&text));
    assert_eq!(editor.text().len(), MAX_ASK_CUSTOM_BYTES);

    // 选中前 10 字节，尝试粘贴 20 字节（总长将超出上限）
    editor.begin_text_selection(0, 0);
    editor.drag_text_selection(0, 10);
    assert!(editor.has_text_selection());
    let overflow = "b".repeat(20);
    assert!(!editor.insert_str(&overflow), "超限必须整次拒绝");
    assert_eq!(
        editor.text().len(),
        MAX_ASK_CUSTOM_BYTES,
        "超限拒绝后原选区文字必须完好保留"
    );

    // 粘贴 10 字节正好替换
    let exact = "b".repeat(10);
    assert!(editor.insert_str(&exact));
    assert_eq!(editor.text().len(), MAX_ASK_CUSTOM_BYTES);
    assert!(editor.text().starts_with("bbbbbbbbbb"));

    // 撤销一步恢复原状
    editor.undo();
    assert!(editor.text().starts_with("aaaaaaaaaa"));
}
