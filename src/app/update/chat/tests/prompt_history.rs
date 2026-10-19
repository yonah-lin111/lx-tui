//! 已发送 prompt 写入全局历史的测试：ack 成功才记录、失败不记录、
//! 发送后退出浏览态、重复发送去重置顶。

use super::*;

/// 发送并走完 DB ack（成功）。
///
/// 末尾把请求状态机复位为 Idle，模拟"上一轮已收流结束"，以便同状态连续发多轮。
fn send_and_commit(state: &mut AppState, draft: &str) {
    let pane = focus_pane(state);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.request = RequestState::Idle;
        target.lx.input = draft.into();
    }
    assert!(try_send(state, pane), "发送受理");
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        other => panic!("expected committing, got {other:?}"),
    };
    assert!(on_turn_committed(
        state,
        pane,
        generation,
        &committed("sess-1")
    ));
}

#[test]
fn commit_ack_records_prompt_into_global_history() {
    let mut state = ready_state("");
    send_and_commit(&mut state, "第一问");
    assert_eq!(
        state.prompt_history.entries(),
        &["第一问".to_string()],
        "ack 成功后写入全局历史"
    );
    // 同时入队持久化效应。
    let recorded = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                crate::app::effects::Effect::RecordPromptHistory { .. }
            )
        })
        .count();
    assert_eq!(recorded, 1, "恰好一条持久化效应");
}

#[test]
fn commit_failure_does_not_record_history() {
    let mut state = ready_state("不会入库");
    let pane = focus_pane(&state);
    assert!(try_send(&mut state, pane));
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        other => panic!("expected committing, got {other:?}"),
    };
    // DB ack 失败。
    assert!(on_turn_failed(&mut state, pane, generation));
    assert!(
        state.prompt_history.is_empty(),
        "提交失败不写入历史（草稿保留可重试）"
    );
}

#[test]
fn repeated_prompt_is_deduplicated_to_newest() {
    let mut state = ready_state("");
    send_and_commit(&mut state, "alpha");
    send_and_commit(&mut state, "beta");
    send_and_commit(&mut state, "alpha");
    assert_eq!(
        state.prompt_history.entries(),
        &["beta".to_string(), "alpha".to_string()],
        "重复 prompt 去重并置顶"
    );
}

#[test]
fn commit_ack_clears_history_browse() {
    let mut state = ready_state("");
    state.prompt_history.record("旧历史");
    let pane = focus_pane(&state);
    // 进入浏览态。
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    assert!(state.pane_anywhere(pane).unwrap().lx.history_browsing());
    // 浏览到历史文本后直接发送（模拟用户从历史里重发）。
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        other => panic!("expected committing, got {other:?}"),
    };
    assert!(on_turn_committed(
        &mut state,
        pane,
        generation,
        &committed("sess-2")
    ));
    assert!(
        !state.pane_anywhere(pane).unwrap().lx.history_browsing(),
        "发送成功后退出浏览态，下一次 `↑` 从最新一条开始"
    );
}

#[test]
fn oversized_prompt_is_sent_but_not_recorded() {
    let mut state = ready_state("");
    let oversized = "x".repeat(crate::app::state::PROMPT_HISTORY_MAX_BYTES + 1);
    send_and_commit(&mut state, &oversized);
    assert!(
        state.prompt_history.is_empty(),
        "超长 prompt 仍可发送，但不写入历史"
    );
}
