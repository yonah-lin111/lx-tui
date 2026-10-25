//! `ask` 面板的输入与提交：键位表、鼠标语义、`Esc` 双击与草稿不变量。
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
        "input={:?} cursor={} rev={} ask={:?} editing={:?} current={:?} confirm={:?} selected={:?} custom_on={:?}",
        target.lx.input,
        target.lx.cursor,
        target.lx.revision(),
        ask.map(|ask| ask.call_id.clone()),
        ask.map(|ask| ask.editing),
        ask.map(|ask| ask.current),
        ask.map(|ask| ask.confirm),
        ask.map(|ask| ask.selected.clone()),
        ask.map(|ask| ask.custom_on.clone()),
    )
}

// ===== 列表态键位（design §7.1，逐行）=====

#[test]
fn list_keys_move_highlight_with_arrows_and_jk() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    press(&mut state, LxInput::Down);
    assert_eq!(panel(&state, pane).highlight[0], 1);
    press(&mut state, LxInput::Char('j'));
    assert_eq!(panel(&state, pane).highlight[0], 2, "j 移动到「其他」行");
    press(&mut state, LxInput::Up);
    assert_eq!(panel(&state, pane).highlight[0], 1);
    press(&mut state, LxInput::Char('k'));
    assert_eq!(panel(&state, pane).highlight[0], 0);
    // 顶部再按 ↑ 不变化，也不会漏进草稿。
    press(&mut state, LxInput::Up);
    assert_eq!(panel(&state, pane).highlight[0], 0);
}

#[test]
fn space_selects_highlighted_row_and_other_enters_edit() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    press(&mut state, LxInput::Char(' '));
    assert_eq!(panel(&state, pane).selected[0], vec![0]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Char(' '));
    assert_eq!(panel(&state, pane).selected[0], vec![1], "单选只保留一个");
    // 「其他」行：Space 进入编辑态。
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Char(' '));
    assert!(panel(&state, pane).editing);
}

#[test]
fn digits_select_by_index_and_toggle_in_multi_select() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A", "B", "C"])]);
    press(&mut state, LxInput::Char('2'));
    assert_eq!(panel(&state, pane).selected[0], vec![1]);
    press(&mut state, LxInput::Char('1'));
    assert_eq!(panel(&state, pane).selected[0], vec![1, 0], "按勾选顺序");
    press(&mut state, LxInput::Char('2'));
    assert_eq!(panel(&state, pane).selected[0], vec![0], "再按同一数字取消");
    // 越界数字吞掉，不改状态、不进草稿。
    press(&mut state, LxInput::Char('9'));
    assert_eq!(panel(&state, pane).selected[0], vec![0]);
}

#[test]
fn enter_submits_and_tab_arrows_switch_questions() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    // 单选：Enter 直接选中高亮项并前进（无需先按 Space）。
    press(&mut state, LxInput::Enter);
    assert_eq!(panel(&state, pane).current, 1, "非末题 Enter 前进");
    assert_eq!(
        panel(&state, pane).answers_at(0),
        vec!["A".to_string()],
        "Enter 即选中"
    );
    press(&mut state, LxInput::Left);
    assert_eq!(panel(&state, pane).current, 0, "← 上一题");
    press(&mut state, LxInput::Right);
    assert_eq!(panel(&state, pane).current, 1, "→ 下一题");
    press(&mut state, LxInput::Tab);
    assert!(panel(&state, pane).is_confirm(), "末题 Tab 进入 Confirm");
    press(&mut state, LxInput::PrevStep);
    assert!(!panel(&state, pane).is_confirm(), "Shift+Tab 退回末题");
    assert_eq!(panel(&state, pane).current, 1);
    press(&mut state, LxInput::PrevStep);
    assert_eq!(panel(&state, pane).current, 0, "Shift+Tab 继续退到上一题");
    press(&mut state, LxInput::PrevStep);
    assert_eq!(panel(&state, pane).current, 0, "首题再退不越界");
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
fn page_keys_scroll_preview_only() {
    let mut q = question("q", false, &["A", "B"]);
    q.options[0].preview = Some("1\n2\n3\n4\n5\n6".into());
    let (mut state, pane, _key) = asking(vec![q]);
    assert_eq!(panel(&state, pane).highlight[0], 0);
    press(&mut state, LxInput::PageDown);
    assert_eq!(panel(&state, pane).preview_scroll[0], 3);
    press(&mut state, LxInput::PageUp);
    assert_eq!(panel(&state, pane).preview_scroll[0], 0);
    assert_eq!(panel(&state, pane).highlight[0], 0, "翻页不动高亮");
}

// ===== 编辑态（design §7.2）=====

#[test]
fn edit_mode_enter_commits_text_and_escape_only_leaves_edit() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "自己写".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::Enter);
    let current = panel(&state, pane);
    assert!(!current.editing, "Enter 提交文本并退出编辑");
    assert_eq!(current.current_answers(), vec!["自己写".to_string()]);
    assert_eq!(current.current, 0, "提交文本不提交整题");

    // 再进编辑（已提交的手写答案之上回车会提交，故用 Space 重进编辑）：
    // Esc 只退出，文本保留，整轮不取消。
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Char(' '));
    press(&mut state, LxInput::Char('x'));
    press(&mut state, LxInput::Escape);
    let panel = panel(&state, pane);
    assert!(!panel.editing);
    assert!(state.pane_anywhere(pane).expect("pane").lx.ask.is_some());
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
fn edit_mode_swallows_tab_and_keeps_question() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    press(&mut state, LxInput::Tab);
    assert_eq!(panel(&state, pane).current, 0, "编辑态 Tab 不切题");
    assert!(panel(&state, pane).editing);
}

#[test]
fn shift_enter_newline_in_edit_mode() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "ab".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::EnterFolder);
    press(&mut state, LxInput::Char('c'));
    assert_eq!(
        panel(&state, pane).custom[0].text(),
        "ab\nc",
        "Shift+Enter 换行"
    );
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
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("超限必须有窗格内 toast");
    assert_eq!(
        toast.message,
        crate::app::update::chat::ask::ASK_HINT_TOO_LONG
    );

    // 超限粘贴整次拒绝，已写内容一个字节不变。
    assert!(!crate::app::update::apply_ask_paste(&mut state, pane, "cc"));
    assert_eq!(panel(&state, pane).custom[0].text().len(), limit);
}

// ===== 提交（design §6.3）=====

#[test]
fn empty_answer_shows_toast_and_does_not_submit() {
    // 多选 + 单题：Enter 不会自动勾选，因此答案为空 → toast。
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A"])]);
    press(&mut state, LxInput::Enter);
    let current = panel(&state, pane);
    assert!(current.answers_at(0).is_empty());
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("空答案必须给出窗格内 toast");
    assert_eq!(toast.message, crate::app::update::chat::ask::ASK_HINT_EMPTY);
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::AnswerAsk { .. }))
    );
    assert!(state.pane_anywhere(pane).expect("pane").lx.ask.is_some());
}

#[test]
fn last_question_submit_pushes_answer_effect_and_clears_panel() {
    let (mut state, pane, key) = asking(vec![
        question("a", false, &["A"]),
        question("b", true, &["B1", "B2"]),
    ]);
    press(&mut state, LxInput::Char(' '));
    press(&mut state, LxInput::Enter);
    assert_eq!(panel(&state, pane).current, 1, "第 1 题作答后前进");
    press(&mut state, LxInput::Char(' '));
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Char(' '));
    press(&mut state, LxInput::Enter);
    assert!(panel(&state, pane).is_confirm(), "末题后进入 Confirm");
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.ask.is_some(),
        "Confirm 之前绝不提交"
    );
    press(&mut state, LxInput::Enter);
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.ask.is_none(),
        "Confirm 提交后清面板"
    );
    let effect = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::AnswerAsk {
                key: effect_key,
                call_id,
                answers,
            } => Some((effect_key.clone(), call_id.clone(), answers.clone())),
            _ => None,
        })
        .expect("必须推 AnswerAsk");
    assert_eq!(effect.0, key);
    assert_eq!(effect.1, "call-1", "效应必须带 call_id 防串台");
    assert_eq!(effect.2.get("a"), Some(&vec!["A".to_string()]));
    assert_eq!(
        effect.2.get("b"),
        Some(&vec!["B1".to_string(), "B2".to_string()])
    );
}

#[test]
fn going_back_and_changing_answers_updates_the_submitted_payload() {
    let (mut state, pane, _key) = asking(vec![
        question("a", true, &["A1", "A2"]),
        question("b", false, &["B"]),
    ]);
    press(&mut state, LxInput::Char('1'));
    press(&mut state, LxInput::Enter);
    press(&mut state, LxInput::Left);
    // 多选：先取消 A1 再选 A2，答案才是 A2。
    press(&mut state, LxInput::Char('1'));
    press(&mut state, LxInput::Char('2'));
    press(&mut state, LxInput::Enter);
    // 第 2 题是单选：Enter 直接选中 B 并进入 Confirm。
    press(&mut state, LxInput::Enter);
    assert!(panel(&state, pane).is_confirm());
    press(&mut state, LxInput::Enter);
    assert!(state.pane_anywhere(pane).expect("pane").lx.ask.is_none());
    let answers = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::AnswerAsk { answers, .. } => Some(answers.clone()),
            _ => None,
        })
        .expect("AnswerAsk");
    assert_eq!(answers.get("a"), Some(&vec!["A2".to_string()]));
    assert_eq!(answers.get("b"), Some(&vec!["B".to_string()]));
}

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
    // 全程在面板里操作（含编辑态输入）。
    press(&mut state, LxInput::Char('1'));
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "手写".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::Enter);
    press(&mut state, LxInput::Up);
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
    // 面板里敲一堆字并提交。
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    for ch in "ask 文本".chars() {
        press(&mut state, LxInput::Char(ch));
    }
    press(&mut state, LxInput::Enter);
    press(&mut state, LxInput::Up);
    press(&mut state, LxInput::Enter);
    // 草稿的撤销栈必须仍是它自己的：撤销一次回到 "a"。
    crate::app::update::apply_lx_input(&mut state, LxInput::Undo);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, "a", "ask 编辑绝不能污染草稿的撤销栈");
    assert_eq!(target.lx.cursor, 1);
}

// ===== Esc 双击（design §7.1）=====

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
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.ask.is_some(),
        "待命不清面板"
    );
    press(&mut state, LxInput::Escape);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::CancelChatStream { key: effect_key } if *effect_key == key
        )),
        "第二次 Esc 推 CancelChatStream"
    );
}

// ===== 鼠标（design §7.3）=====

#[test]
fn click_single_select_highlights_and_selects_without_submitting() {
    let mut first = question("q", false, &["A", "B"]);
    first.options[0].preview = Some("left".into());
    first.options[1].preview = Some("right".into());
    let (mut state, pane, _key) = asking(vec![first]);
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Option(1)
    ));
    let current = panel(&state, pane);
    assert_eq!(current.highlight[0], 1, "单击切换预览");
    assert_eq!(current.preview(), Some("right"));
    assert_eq!(
        current.selected[0],
        vec![1],
        "单选单击是独占选择（否则纯鼠标用户交不出答案）"
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.ask.is_some(),
        "单击绝不提交"
    );
}

#[test]
fn click_multi_select_toggles_immediately() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A", "B"])]);
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Option(1)
    ));
    assert_eq!(panel(&state, pane).selected[0], vec![1]);
    assert!(crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Option(1)
    ));
    assert!(panel(&state, pane).selected[0].is_empty(), "再点取消");
}

#[test]
fn click_other_enters_edit_and_click_tab_switches_question() {
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
    // 越界题号吞掉。
    assert!(!crate::app::update::apply_ask_click(
        &mut state,
        pane,
        AskHit::Tab(9)
    ));
}

#[test]
fn confirm_only_submits_after_every_question_is_answered() {
    let (mut state, pane, _key) = asking(vec![
        question("a", false, &["A"]),
        question("b", false, &["B"]),
    ]);
    // 直接跳过第 1 题点 Confirm：全部作答未满足 → toast，不提交。
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Tab(2));
    assert!(panel(&state, pane).is_confirm());
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Submit);
    assert!(state.pane_anywhere(pane).expect("pane").lx.ask.is_some());
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.toast.as_ref())
            .is_some()
    );
    // 逐题作答后 Confirm 才放行。
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Tab(0));
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Option(0));
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Tab(1));
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Option(0));
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Tab(2));
    crate::app::update::apply_ask_click(&mut state, pane, AskHit::Submit);
    assert!(state.pane_anywhere(pane).expect("pane").lx.ask.is_none());
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::AnswerAsk { call_id, .. } if call_id == "call-1"
    )));
}

#[test]
fn wheel_preview_scrolls_and_options_move_highlight() {
    let mut q = question("q", false, &["A", "B"]);
    q.options[0].preview = Some("1\n2\n3\n4\n5\n6\n7\n8".into());
    let (mut state, pane, _key) = asking(vec![q]);
    assert!(crate::app::update::apply_ask_wheel(
        &mut state,
        pane,
        AskWheelZone::Preview,
        1
    ));
    assert_eq!(panel(&state, pane).preview_scroll[0], 3);
    assert_eq!(panel(&state, pane).highlight[0], 0, "预览滚轮不改高亮");
    assert!(crate::app::update::apply_ask_wheel(
        &mut state,
        pane,
        AskWheelZone::Options,
        1
    ));
    assert_eq!(panel(&state, pane).highlight[0], 1);
    assert_eq!(panel(&state, pane).preview_scroll[0], 3, "选项滚轮不改预览");
}

// ===== 与其它面板/路由的关系 =====

#[test]
fn ask_branch_precedes_history_browsing() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    // 草稿有内容、光标在末尾：无 ask 时 ↑ 会被历史接管；有 ask 时必须归面板。
    press(&mut state, LxInput::Up);
    assert_eq!(panel(&state, pane).highlight[0], 0, "↑ 归面板");
    press(&mut state, LxInput::Down);
    assert_eq!(panel(&state, pane).highlight[0], 1);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, "草稿 abc", "面板接管期间草稿不变");
    assert_eq!(target.lx.cursor, "草稿 abc".len());
}

#[test]
fn single_question_enter_selects_highlight_and_submits_immediately() {
    let (mut state, pane, _key) = asking(vec![question("q", false, &["A", "B"])]);
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Enter);
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.ask.is_none(),
        "单题单选：回车选中后直接提交"
    );
    let answers = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::AnswerAsk { answers, .. } => Some(answers.clone()),
            _ => None,
        })
        .expect("AnswerAsk");
    assert_eq!(answers.get("q"), Some(&vec!["B".to_string()]));
}

#[test]
fn single_question_multi_select_submits_with_enter_after_space() {
    let (mut state, pane, _key) = asking(vec![question("q", true, &["A", "B"])]);
    // 多选：回车不会自动勾选，需要先按空格。
    press(&mut state, LxInput::Enter);
    assert!(state.pane_anywhere(pane).expect("pane").lx.ask.is_some());
    press(&mut state, LxInput::Char(' '));
    press(&mut state, LxInput::Down);
    press(&mut state, LxInput::Char(' '));
    press(&mut state, LxInput::Enter);
    assert!(state.pane_anywhere(pane).expect("pane").lx.ask.is_none());
    let answers = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::AnswerAsk { answers, .. } => Some(answers.clone()),
            _ => None,
        })
        .expect("AnswerAsk");
    assert_eq!(
        answers.get("q"),
        Some(&vec!["A".to_string(), "B".to_string()])
    );
}
