//! `ask` 面板的输入与提交：键位表、鼠标语义、`Esc` 双击、计时补偿与草稿不变量。
//!
//! 全部走 `apply_lx_input`（生产路由），不直接调内部函数：这里要锁住的正是
//! 「ask 分支排在后台任务页之后、rewind 之前」这条路由。

use super::*;
use crate::app::actions::LxInput;
use crate::app::state::{AskHit, AskState, AskWheelZone};
use crate::chat::tools::ask::{AskOption, AskQuestion};

fn question(id: &str, multi_select: bool, labels: &[&str]) -> AskQuestion {
    AskQuestion {
        id: id.into(),
        header: id.into(),
        question: format!("{id}?"),
        multi_select,
        options: labels
            .iter()
            .map(|label| AskOption {
                label: (*label).into(),
                description: format!("{label} 的说明"),
                preview: None,
            })
            .collect(),
    }
}

fn key_for(pane: PaneId) -> RequestKey {
    RequestKey {
        pane,
        session: "sess-ask".into(),
        generation: 1,
    }
}

/// 流中窗格 + 挂起面板；返回（状态，窗格，key）。
fn asking(questions: Vec<AskQuestion>) -> (AppState, PaneId, RequestKey) {
    let mut state = ready_state("草稿 abc");
    let pane = focus_pane(&state);
    let key = key_for(pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(key.session.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: "a1".into(),
        };
        target.lx.ask = Some(AskState::new(key.clone(), "call-1".into(), questions));
        target.lx.cursor = target.lx.input.len();
    }
    (state, pane, key)
}

fn panel(state: &AppState, pane: PaneId) -> &AskState {
    state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
        .expect("面板挂起")
}

fn ask_present(state: &AppState, pane: PaneId) -> bool {
    state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.ask.is_some())
}

fn toast_message(state: &AppState, pane: PaneId) -> Option<String> {
    state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .map(|toast| toast.message.clone())
}

fn answer_effect(state: &AppState) -> Option<crate::chat::tools::AskAnswers> {
    state.effects.iter().find_map(|effect| match effect {
        crate::app::effects::Effect::AnswerAsk { answers, .. } => Some(answers.clone()),
        _ => None,
    })
}

/// 一次性按下若干键（走真实路由）。
fn press(state: &mut AppState, command: LxInput) -> bool {
    let before = state.pane_anywhere(focus_pane(state)).is_some();
    assert!(before, "窗格存在");
    let pane = focus_pane(state);
    let previous = snapshot(state, pane);
    crate::app::update::apply_lx_input(state, command);
    snapshot(state, pane) != previous
}

/// 面板 + 草稿的可比较快照。
fn snapshot(state: &AppState, pane: PaneId) -> String {
    let target = state.pane_anywhere(pane).expect("pane");
    let ask = target.lx.ask.as_ref();
    format!(
        "input={:?} cursor={} rev={} ask={:?} editing={:?} current={:?} highlight={:?} selected={:?} custom_on={:?}",
        target.lx.input,
        target.lx.cursor,
        target.lx.revision(),
        ask.map(|ask| ask.call_id.clone()),
        ask.map(|ask| ask.editing),
        ask.map(|ask| ask.current),
        ask.map(|ask| ask.highlight.clone()),
        ask.map(|ask| ask.selected.clone()),
        ask.map(|ask| ask.custom_on.clone()),
    )
}

// ===== 列表态键位（design §4.3.3）=====

#[test]
fn list_navigation_honors_arrows_and_swallows_other_keys() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    press(&mut state, LxInput::Down);
    assert_eq!(panel(&state, pane).highlight[0], 1);
    press(&mut state, LxInput::Up);
    assert_eq!(panel(&state, pane).highlight[0], 0);
    // `↑` 从首行绕到末行（对齐 opencode 的循环高亮）。
    press(&mut state, LxInput::Up);
    assert_eq!(
        panel(&state, pane).highlight[0],
        panel(&state, pane).row_count() - 1
    );
    // 回到普通选项行：未绑定的字符键一律吞掉，不动高亮、也不漏进草稿。
    press(&mut state, LxInput::Down);
    assert_eq!(panel(&state, pane).highlight[0], 0);
    for command in [LxInput::Char('j'), LxInput::Char('k')] {
        assert!(!press(&mut state, command), "按键必须被吞掉");
    }
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "草稿 abc",
        "草稿不变量"
    );
}

#[test]
fn space_toggles_selection_and_opens_the_inline_editor() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    press(&mut state, LxInput::Char(' '));
    assert_eq!(panel(&state, pane).selected[0], vec![0]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Char(' '));
    assert_eq!(panel(&state, pane).selected[0], vec![1], "单选只保留一个");
    // 「其他」行：Space 进入原地编辑态（不提交）。
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Char(' '));
    assert!(panel(&state, pane).editing);
    assert_eq!(
        panel(&state, pane).highlight[0],
        panel(&state, pane).other_row()
    );
}

#[test]
fn single_question_single_select_enter_submits_immediately() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    assert!(!ask_present(&state, pane), "单题单选：回车选中后直接提交");
    assert_eq!(
        answer_effect(&state).and_then(|answers| answers.get("q").cloned()),
        Some(vec!["B".to_string()])
    );
}

#[test]
fn multi_question_single_select_advances_then_enters_the_submit_tab() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    // 第 1 题：回车选中并自动前进。
    press(&mut state, LxInput::Enter);
    assert_eq!(panel(&state, pane).current, 1);
    assert_eq!(panel(&state, pane).answers_at(0), vec!["A".to_string()]);
    // 末题：回车选中并进入 Submit 汇总，不自动提交。
    press(&mut state, LxInput::Enter);
    assert!(ask_present(&state, pane), "必须进入 Submit 汇总再提交");
    let ask = panel(&state, pane);
    assert_eq!(ask.answers_at(1), vec!["B".to_string()]);
    assert!(ask.is_confirm());
    // 再回车才真正提交。
    press(&mut state, LxInput::Enter);
    assert!(!ask_present(&state, pane));
    let answers = answer_effect(&state).expect("AnswerAsk");
    assert_eq!(answers.get("a"), Some(&vec!["A".to_string()]));
    assert_eq!(answers.get("b"), Some(&vec!["B".to_string()]));
}

#[test]
fn tab_keys_cycle_between_questions_and_the_submit_tab() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    press(&mut state, LxInput::Tab);
    assert_eq!(panel(&state, pane).current, 1);
    press(&mut state, LxInput::Tab);
    assert!(panel(&state, pane).is_confirm(), "Tab 前进到 Submit 汇总");
    press(&mut state, LxInput::Tab);
    assert_eq!(panel(&state, pane).current, 0, "汇总循环回第一题");
    assert!(!panel(&state, pane).is_confirm());
    // Shift+Tab（`PrevStep`）与 ← 反向循环。
    press(&mut state, LxInput::PrevStep);
    assert!(panel(&state, pane).is_confirm());
    press(&mut state, LxInput::Left);
    assert_eq!(panel(&state, pane).current, 1, "汇总后退落到末题");
    press(&mut state, LxInput::Right);
    assert!(panel(&state, pane).is_confirm());
}

#[test]
fn single_select_question_has_no_tab_cycling() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    assert!(!press(&mut state, LxInput::Tab));
    assert!(!press(&mut state, LxInput::PrevStep));
    assert!(!panel(&state, pane).is_confirm());
}

#[test]
fn digit_shortcut_selects_and_confirms_a_single_select_answer() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    press(&mut state, LxInput::Char('2'));
    assert!(!ask_present(&state, pane), "数字键选中后立即提交");
    assert_eq!(
        answer_effect(&state).and_then(|answers| answers.get("q").cloned()),
        Some(vec!["B".to_string()])
    );
}

#[test]
fn multi_select_enter_only_toggles_and_the_submit_row_commits() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A", "B"])]);
    press(&mut state, LxInput::Enter);
    assert_eq!(panel(&state, pane).selected[0], vec![0]);
    assert!(ask_present(&state, pane), "多选回车绝不自动提交");
    // 移到提交按钮（选项 2 + Other + Submit）。
    while panel(&state, pane).highlight[0] != panel(&state, pane).submit_row().expect("submit") {
        press(&mut state, LxInput::Down);
    }
    press(&mut state, LxInput::Enter);
    assert!(!ask_present(&state, pane), "聚焦提交按钮才提交");
    assert_eq!(
        answer_effect(&state).and_then(|answers| answers.get("q").cloned()),
        Some(vec!["A".to_string()])
    );
}

#[test]
fn submit_with_unanswered_questions_toasts_and_jumps_back() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    // 直接跳到末题作答并进入 Submit 汇总：第 1 题仍未答。
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Tab(1)
    ));
    press(&mut state, LxInput::Enter);
    assert!(panel(&state, pane).is_confirm());
    press(&mut state, LxInput::Enter);
    assert!(ask_present(&state, pane), "未答完不得提交");
    assert_eq!(
        toast_message(&state, pane).as_deref(),
        Some(crate::ui::text::agent::ASK_TOAST_ANSWER_REQUIRED)
    );
    assert_eq!(panel(&state, pane).current, 0, "自动跳回首个未答题");
    assert!(!panel(&state, pane).is_confirm());
}

#[test]
fn enter_on_other_row_enters_edit_instead_of_submitting() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    assert!(panel(&state, pane).editing);
    assert!(
        panel(&state, pane).answers_at(0).is_empty(),
        "进入编辑不等于提交"
    );
}

#[test]
fn space_on_the_submit_row_behaves_like_enter() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A"])]);
    press(&mut state, LxInput::Char(' '));
    let submit = panel(&state, pane).submit_row().expect("submit");
    {
        let ask = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .expect("ask");
        ask.highlight[0] = submit;
    }
    press(&mut state, LxInput::Char(' '));
    assert!(!ask_present(&state, pane), "提交行 Space 等价回车");
}

#[test]
fn confirm_tab_arrows_scroll_the_review_instead_of_moving_the_highlight() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    press(&mut state, LxInput::Tab);
    press(&mut state, LxInput::Tab);
    assert!(panel(&state, pane).is_confirm());
    {
        let ask = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .expect("ask");
        ask.sync_list_window(0, 20, 5);
    }
    press(&mut state, LxInput::Down);
    assert_eq!(panel(&state, pane).viewport(), Some(1), "汇总里 ↓ 是滚动");
    press(&mut state, LxInput::Up);
    assert_eq!(panel(&state, pane).viewport(), Some(0));
}

#[test]
fn page_keys_scroll_the_unified_list() {
    let mut q = question("q", false, &["A", "B"]);
    q.options[0].preview = Some("1\n2\n3\n4\n5\n6".into());
    let (mut state, pane, _key) = asking(vec![q]);
    // 事件循环每帧把几何落进状态；测试里手动落同一份窗口（内容 20 行 / 视口 5 行）。
    {
        let ask = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .expect("ask");
        ask.sync_list_window(0, 20, 5);
    }
    press(&mut state, LxInput::PageDown);
    assert_eq!(panel(&state, pane).viewport(), Some(5), "PageDown 前进一屏");
    assert_eq!(panel(&state, pane).highlight[0], 0, "翻页不动高亮");
    press(&mut state, LxInput::PageUp);
    assert_eq!(panel(&state, pane).viewport(), Some(0));
}

// ===== 编辑态（design §4.3.4）=====

#[test]
fn edit_mode_enter_commits_and_single_select_submits() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "自己写".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::Enter);
    assert!(!ask_present(&state, pane), "单题单选：确认手写文本即提交");
    assert_eq!(
        answer_effect(&state).and_then(|answers| answers.get("q").cloned()),
        Some(vec!["自己写".to_string()])
    );
}

#[test]
fn edit_mode_enter_on_multi_select_only_commits_the_text() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "补充".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::Enter);
    let ask = panel(&state, pane);
    assert!(!ask.editing, "Enter 退出编辑态");
    assert!(ask_present(&state, pane), "多选不自动提交");
    assert_eq!(ask.answers_at(0), vec!["补充".to_string()]);
}

#[test]
fn edit_mode_newline_and_escape_keep_the_text() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "ab".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::EnterFolder);
    press(&mut state, LxInput::Char('c'));
    assert_eq!(panel(&state, pane).custom[0].text(), "ab\nc");
    // Esc 只退出编辑：文本保留，整轮不取消。
    press(&mut state, LxInput::Escape);
    assert!(!panel(&state, pane).editing);
    assert!(ask_present(&state, pane));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .stop_armed_at
            .is_none(),
        "编辑态 Esc 不得进入停止待命"
    );
}

#[test]
fn edit_mode_tab_commits_and_moves_to_the_next_tab() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "自己写".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::Tab);
    let ask = panel(&state, pane);
    assert!(!ask.editing, "Tab 先离开编辑框");
    assert_eq!(ask.current, 1, "再切到下一题");
    assert_eq!(ask.answers_at(0), vec!["自己写".to_string()]);
}

#[test]
fn typing_on_the_other_row_starts_editing_with_that_character() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A"])]);
    press(&mut state, LxInput::Down);
    assert_eq!(
        panel(&state, pane).highlight[0],
        panel(&state, pane).other_row()
    );
    press(&mut state, LxInput::Char('x'));
    let ask = panel(&state, pane);
    assert!(ask.editing, "在 Other 行直接键入即开始编辑");
    assert_eq!(ask.custom[0].text(), "x", "首字符不能丢");
}

#[test]
fn custom_text_stops_at_the_byte_limit_with_a_toast() {
    let limit = crate::chat::tools::ask::MAX_ASK_CUSTOM_BYTES;
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    assert!(panel(&state, pane).editing);

    // 恰好填满上限：整次写入放行。
    assert!(crate::app::update::apply_ask_paste(
        &mut state,
        pane,
        &"a".repeat(limit)
    ));
    assert_eq!(panel(&state, pane).custom[0].text().len(), limit);

    // 再键入一个字符：拒绝写入（绝不截断）并给出窗格内提示。
    press(&mut state, LxInput::Char('b'));
    assert_eq!(panel(&state, pane).custom[0].text().len(), limit);
    assert_eq!(
        toast_message(&state, pane).as_deref(),
        Some(crate::app::update::chat::ask::ASK_HINT_TOO_LONG)
    );

    // 超限粘贴整次拒绝，已写内容一个字节不变。
    assert!(!crate::app::update::apply_ask_paste(&mut state, pane, "cc"));
    assert_eq!(panel(&state, pane).custom[0].text().len(), limit);
}

#[test]
fn paste_budget_uses_the_sanitized_length() {
    // `\t` 写入时会展开成两个空格：按原始长度预检会让上限翻倍。
    let limit = crate::chat::tools::ask::MAX_ASK_CUSTOM_BYTES;
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);

    let tabs = "\t".repeat(limit);
    assert!(
        !crate::app::update::apply_ask_paste(&mut state, pane, &tabs),
        "净化后超限必须整次拒绝"
    );
    assert_eq!(panel(&state, pane).custom[0].text(), "");

    // 半个上限的 tab 净化后正好填满。
    let half = "\t".repeat(limit / 2);
    assert!(crate::app::update::apply_ask_paste(&mut state, pane, &half));
    assert_eq!(panel(&state, pane).custom[0].text().len(), limit);
}

#[test]
fn list_mode_paste_is_swallowed_and_never_touches_the_draft() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A"])]);
    assert!(!crate::app::update::apply_ask_paste(
        &mut state, pane, "hidden"
    ));
    assert_eq!(panel(&state, pane).custom[0].text(), "");
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "草稿 abc",
        "草稿不变量"
    );
}

// ===== 提交（design §4.3.3）=====

#[test]
fn submit_never_touches_the_draft() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A"])]);
    let before = {
        let target = state.pane_anywhere(pane).expect("pane");
        (
            target.lx.input.clone(),
            target.lx.cursor,
            target.lx.revision(),
        )
    };
    // 全程在面板里操作（含原地编辑输入）。
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "手写".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::Enter);
    while panel(&state, pane).highlight[0] != panel(&state, pane).submit_row().expect("submit") {
        press(&mut state, LxInput::Down);
    }
    press(&mut state, LxInput::Enter);
    let after = {
        let target = state.pane_anywhere(pane).expect("pane");
        (
            target.lx.input.clone(),
            target.lx.cursor,
            target.lx.revision(),
        )
    };
    assert_eq!(before, after, "作答全程草稿/光标/修订号不变");
    assert!(before.0.contains("草稿"));
    assert!(!ask_present(&state, pane));
}

#[test]
fn undo_after_submit_restores_the_draft_history_not_ask_text() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    // 草稿自身产生一次撤销点：输入 "ab" 后撤销应回到 "a"。
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('a'));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('b'));
    let key = key_for(pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(key.session.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: "a1".into(),
        };
        target.lx.ask = Some(AskState::new(
            key,
            "call-1".into(),
            vec![question("q", false, &["A"])],
        ));
    }
    // 单选单题：在「其他」里写字并提交整题。
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "ask 文本".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::Enter);
    assert!(!ask_present(&state, pane));
    // 草稿的撤销栈必须仍是它自己的：撤销一次回到 "a"。
    crate::app::update::apply_lx_input(&mut state, LxInput::Undo);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, "a", "ask 编辑绝不能污染草稿的撤销栈");
    assert_eq!(target.lx.cursor, 1);
}

#[test]
fn submit_compensates_the_wait_time_into_the_turn_clock() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A"])]);
    let now = crate::db::now_ms();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.turn_started_at_ms = Some(now);
        target.lx.ask.as_mut().expect("ask").started_at_ms = now - 10_000;
    }
    press(&mut state, LxInput::Enter);
    assert!(!ask_present(&state, pane));
    let started = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .turn_started_at_ms
        .expect("本轮起点仍在");
    assert!(
        started >= now + 9_000,
        "用户作答的 10 秒必须从本轮耗时里剔除：{started} vs {now}"
    );
}

#[test]
fn loading_tick_freezes_the_turn_clock_while_asking() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A"])]);
    let now = crate::db::now_ms();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.turn_started_at_ms = Some(now - 5_000);
        target.lx.chat.turn_elapsed_ms = Some(5_000);
        target.lx.chat.loading_frame = 0;
        target.lx.chat.status.streaming = true;
    }
    assert!(
        !crate::app::update::chat::advance_lx_loading(&mut state),
        "ask 等待期间没有任何推进"
    );
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.loading_frame, 0, "主会话扫描帧冻结");
    assert_eq!(target.lx.chat.turn_elapsed_ms, Some(5_000), "耗时冻结");
}

// ===== Esc 双击（design §4.3.3）=====

#[test]
fn escape_arms_then_cancels_on_second_press() {
    let (mut state, pane, key) = asking(vec![question("q", false, &["A"])]);
    press(&mut state, LxInput::Escape);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .stop_armed_at
            .is_some(),
        "第一次 Esc 只待命"
    );
    assert!(ask_present(&state, pane), "待命不清面板");
    press(&mut state, LxInput::Escape);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::CancelChatStream { key: effect_key } if *effect_key == key
        )),
        "第二次 Esc 推 CancelChatStream"
    );
}

// ===== 鼠标（design §4.3.3）=====

#[test]
fn click_single_select_submits_immediately() {
    let mut first = question("q", false, &["A", "B"]);
    first.options[0].preview = Some("left".into());
    first.options[1].preview = Some("right".into());
    let (mut state, pane, _key) = asking(vec![first]);
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Option(1)
    ));
    assert!(!ask_present(&state, pane), "单题单选：单击选项直接提交");
    assert_eq!(
        answer_effect(&state).and_then(|answers| answers.get("q").cloned()),
        Some(vec!["B".to_string()])
    );
}

#[test]
fn click_multi_select_toggles_without_submitting() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A", "B"])]);
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Option(1)
    ));
    assert_eq!(panel(&state, pane).selected[0], vec![1]);
    assert!(ask_present(&state, pane), "多选点击只勾选");
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Option(1)
    ));
    assert!(panel(&state, pane).selected[0].is_empty(), "再点取消");
}

#[test]
fn click_other_enters_edit_and_click_tab_commits_and_switches() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Other
    ));
    assert!(panel(&state, pane).editing);
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Tab(1)
    ));
    assert_eq!(panel(&state, pane).current, 1);
    assert!(!panel(&state, pane).editing, "切题退出编辑态");
    // 越界 tab 进入 Submit 汇总（不再钳到末题）。
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Tab(9)
    ));
    assert!(panel(&state, pane).is_confirm());
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Tab(0)
    ));
    assert_eq!(panel(&state, pane).current, 0);
    assert!(!panel(&state, pane).is_confirm());
}

#[test]
fn click_submit_requires_every_answer() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    // 跳过第 1 题点提交：全部作答未满足 → toast + 跳回未答题。
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Tab(1));
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Submit);
    assert!(ask_present(&state, pane));
    assert_eq!(
        toast_message(&state, pane).as_deref(),
        Some(crate::ui::text::agent::ASK_TOAST_ANSWER_REQUIRED)
    );
    assert_eq!(panel(&state, pane).current, 0);
    // 逐题作答（单选点击后自动前进），进入汇总后提交才放行。
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Option(0));
    assert_eq!(panel(&state, pane).current, 1);
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Option(0));
    assert!(panel(&state, pane).is_confirm(), "末题答完进入 Submit 汇总");
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Submit);
    assert!(!ask_present(&state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::AnswerAsk { call_id, .. } if call_id == "call-1"
    )));
}

#[test]
fn wheel_scrolls_the_unified_list_without_moving_the_highlight() {
    let mut q = question("q", false, &["A", "B"]);
    q.options[0].preview = Some("1\n2\n3\n4\n5\n6\n7\n8".into());
    let (mut state, pane, _key) = asking(vec![q]);
    // 选项与角色画预览共用一个视口：两个分区走同一条滚动。
    assert!(crate::app::update::apply_ask_wheel(
        &mut state,
        pane,
        AskWheelZone::Preview,
        1,
        Some((0, 20, 5))
    ));
    assert_eq!(panel(&state, pane).viewport(), Some(3));
    assert!(crate::app::update::apply_ask_wheel(
        &mut state,
        pane,
        AskWheelZone::Options,
        1,
        Some((3, 20, 5))
    ));
    assert_eq!(panel(&state, pane).viewport(), Some(6));
    assert_eq!(panel(&state, pane).highlight[0], 0, "滚轮绝不动高亮");

    // 内容装得下时滚轮不产生位移，也不记视口。
    assert!(!crate::app::update::apply_ask_wheel(
        &mut state,
        pane,
        AskWheelZone::Options,
        1,
        Some((0, 4, 8))
    ));
    // 拿不到几何时吞掉滚轮，绝不猜基准。
    assert!(!crate::app::update::apply_ask_wheel(
        &mut state,
        pane,
        AskWheelZone::Options,
        1,
        None
    ));
}

#[test]
fn keyboard_navigation_clears_the_explicit_viewport() {
    // 滚轮建立的显式视口在键盘移动高亮后必须作废，窗口重新跟随高亮。
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B", "C"])]);
    assert!(crate::app::update::apply_ask_wheel(
        &mut state,
        pane,
        AskWheelZone::Options,
        1,
        Some((0, 20, 5))
    ));
    assert_eq!(panel(&state, pane).viewport, Some(3));
    press(&mut state, LxInput::Down);
    assert_eq!(panel(&state, pane).highlight[0], 1, "键盘移动高亮");
    assert_eq!(panel(&state, pane).viewport, None, "窗口重新跟随高亮");
}

// ===== 与其它面板/路由的关系 =====

#[test]
fn ask_branch_precedes_history_browsing() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    // 草稿有内容、光标在末尾：无 ask 时 ↑ 会被历史接管；有 ask 时必须归面板。
    press(&mut state, LxInput::Down);
    assert_eq!(panel(&state, pane).highlight[0], 1, "↓ 归面板");
    press(&mut state, LxInput::Up);
    assert_eq!(panel(&state, pane).highlight[0], 0, "↑ 归面板");
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, "草稿 abc", "面板接管期间草稿不变");
    assert_eq!(target.lx.cursor, "草稿 abc".len());
}

#[test]
fn arrow_after_mouse_scroll_restores_the_follow_window() {
    // 高亮被滚出视口后按方向键：窗口必须交还给高亮，否则键盘"没反应"。
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B", "C"])]);
    assert!(crate::app::update::apply_ask_wheel(
        &mut state,
        pane,
        AskWheelZone::Options,
        3,
        Some((0, 20, 5))
    ));
    assert!(panel(&state, pane).viewport().is_some());
    press(&mut state, LxInput::Up);
    assert_eq!(panel(&state, pane).viewport(), None, "窗口交还给高亮");
}
