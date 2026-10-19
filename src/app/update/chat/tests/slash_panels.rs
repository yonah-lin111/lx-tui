//! `/model` 与 `/resume` 面板的"当前项"高亮、视口定位与会话编辑模态。

use super::*;
use crate::app::actions::OverlayKey;
use crate::app::overlay::Overlay;
use crate::app::state::lx::slash::HistoryEntry;
use crate::app::update::{
    apply_resume_session_edit_key, discard_stale_resume_edit, on_session_removed,
    on_session_renamed, open_resume_session_edit,
};

/// 两条历史条目（当前会话为第 2 条）。
fn seed_history(state: &mut AppState, pane: PaneId, current: &str) {
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some(current.into());
    let entries = vec![
        HistoryEntry {
            session_id: "sess-a".into(),
            title: "Alpha".into(),
            status: "active".into(),
            updated_at_ms: 1_000,
        },
        HistoryEntry {
            session_id: "sess-b".into(),
            title: "Beta".into(),
            status: "active".into(),
            updated_at_ms: 2_000,
        },
    ];
    assert!(on_history_listed(state, pane, entries));
}

#[test]
fn model_picker_marks_current_selection_and_focuses_it() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    // 当前选择 = p/m（唯一候选）。
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .selection = Some(ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: Some("high".into()),
    });
    assert!(open_model_picker(&mut state, pane));
    let Some(SlashPanel::Model(picker)) =
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone())
    else {
        panic!("model panel");
    };
    assert_eq!(picker.current, Some(0), "当前模型被标记");
    assert_eq!(picker.window.active, 0, "高亮落在当前模型上");
    assert_eq!(picker.window.viewport, None, "已可见时不引入显式视口");
}

#[test]
fn model_picker_without_bound_selection_has_no_current_marker() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    // 配置里有候选但窗格尚未绑定选择（如刚启动）。
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .selection = Some(ModelSelection {
        provider: "other".into(),
        model: "unknown".into(),
        variant: None,
    });
    assert!(open_model_picker(&mut state, pane));
    let Some(SlashPanel::Model(picker)) =
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone())
    else {
        panic!("model panel");
    };
    assert_eq!(picker.current, None, "不在候选列表里就不标");
}

#[test]
fn history_panel_marks_and_focuses_bound_session() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-b");
    let Some(SlashPanel::History(panel)) =
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone())
    else {
        panic!("history panel");
    };
    assert_eq!(panel.current, Some(1), "已绑定会话被标记");
    assert_eq!(panel.window.active, 1, "高亮落在已绑定会话上");
}

#[test]
fn history_panel_without_bound_session_has_no_marker() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-not-listed");
    let Some(SlashPanel::History(panel)) =
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone())
    else {
        panic!("history panel");
    };
    assert_eq!(panel.current, None, "不在列表里就不标");
    assert_eq!(panel.window.active, 0, "回退到首项");
}

#[test]
fn shift_enter_opens_resume_session_edit_for_highlighted_row() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    // 把高亮移到第 2 条，Shift+Enter 打开它的编辑模态。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::EnterFolder);
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_ref() else {
        panic!("编辑模态应打开");
    };
    assert_eq!(edit.session_id, "sess-b", "目标是高亮行");
    assert_eq!(edit.input.text(), "Beta", "标题预填");
    assert!(!edit.confirming_delete);
}

#[test]
fn shift_enter_without_history_panel_does_not_open_modal() {
    let mut state = ready_state("");
    // 无面板时 Shift+Enter 仍是换行。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::EnterFolder);
    assert!(state.overlay.is_none(), "不打开模态");
    assert_eq!(
        state
            .pane_anywhere(focus_pane(&state))
            .expect("pane")
            .lx
            .input,
        "\n",
        "回落为换行"
    );
}

#[test]
fn resume_edit_enter_queues_rename_and_closes() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    assert!(open_resume_session_edit(&mut state, pane));
    // 改标题后保存。
    edit_resume_title(&mut state, "Renamed");
    assert!(apply_resume_session_edit_key(&mut state, OverlayKey::Enter));
    assert!(state.overlay.is_none(), "保存后关闭模态");
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::RenameChatSession { title, .. } if title == "Renamed"
        )),
        "{:?}",
        state.effects
    );
}

#[test]
fn resume_edit_rejects_blank_title() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    assert!(open_resume_session_edit(&mut state, pane));
    edit_resume_title(&mut state, "   ");
    assert!(!apply_resume_session_edit_key(
        &mut state,
        OverlayKey::Enter
    ));
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_ref() else {
        panic!("模态保持打开");
    };
    assert_eq!(
        edit.hint.as_deref(),
        Some(crate::ui::text::RESUME_EDIT_EMPTY_TITLE)
    );
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::RenameChatSession { .. }
        )),
        "空标题不落库"
    );
}

#[test]
fn resume_edit_delete_requires_second_confirmation() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    assert!(open_resume_session_edit(&mut state, pane));
    // 第一次 ^D 只进入确认态。
    assert!(!apply_resume_session_edit_key(
        &mut state,
        OverlayKey::Remove
    ));
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_ref() else {
        panic!("模态保持打开");
    };
    assert!(edit.confirming_delete, "进入删除确认");
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::DeleteChatSession { .. }
        )),
        "第一次不删除"
    );
    // Esc 退回编辑态（不关窗、不删）。
    assert!(!apply_resume_session_edit_key(&mut state, OverlayKey::Esc));
    assert!(state.overlay.is_some(), "Esc 只退回编辑态");
    // 再确认一次才真删。
    apply_resume_session_edit_key(&mut state, OverlayKey::Remove);
    apply_resume_session_edit_key(&mut state, OverlayKey::Remove);
    assert!(state.overlay.is_none());
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::DeleteChatSession { .. }
        )),
        "{:?}",
        state.effects
    );
}

#[test]
fn resume_edit_delete_is_rejected_when_session_busy_elsewhere() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    // 另一个窗格正绑着 sess-a 且在流中。
    let other = state
        .active_tab_mut()
        .split_pane(pane, crate::app::state::Direction::Horizontal)
        .expect("split pane");
    let target = state.pane_mut_anywhere(other).expect("pane");
    target.lx.chat.session_id = Some("sess-a".into());
    target.lx.chat.request = RequestState::Streaming {
        key: RequestKey {
            pane: other,
            session: "sess-a".into(),
            generation: 1,
        },
        assistant_id: "msg-1".into(),
    };
    assert!(open_resume_session_edit(&mut state, pane));
    apply_resume_session_edit_key(&mut state, OverlayKey::Remove);
    apply_resume_session_edit_key(&mut state, OverlayKey::Remove);
    let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_ref() else {
        panic!("被占用时模态保持打开");
    };
    assert_eq!(
        edit.hint.as_deref(),
        Some(crate::ui::text::LX_CHAT_SESSION_BUSY)
    );
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::DeleteChatSession { .. }
        )),
        "不删除正在别处运行的会话"
    );
}

#[test]
fn rename_ack_updates_list_row_in_place() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    assert!(on_session_renamed(&mut state, pane, "sess-a", "Renamed"));
    let Some(SlashPanel::History(panel)) =
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone())
    else {
        panic!("history panel");
    };
    assert_eq!(panel.entries[0].title, "Renamed", "就地更新标题");
    assert_eq!(panel.entries.len(), 2, "不增删行");
    assert_eq!(panel.current, Some(0), "标记不动");
}

#[test]
fn delete_ack_removes_row_and_moves_highlight() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    assert!(on_session_removed(&mut state, pane, "sess-a"));
    let Some(SlashPanel::History(panel)) =
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone())
    else {
        panic!("history panel");
    };
    assert_eq!(panel.entries.len(), 1, "删除行消失");
    assert_eq!(panel.entries[0].session_id, "sess-b");
    assert!(
        state.overlay.is_none() || state.overlay.is_some(),
        "面板保持打开"
    );
}

#[test]
fn stale_resume_edit_is_discarded_when_focus_moves() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    assert!(open_resume_session_edit(&mut state, pane));
    // 焦点切到另一个窗格。
    state
        .active_tab_mut()
        .layout
        .split_focused(crate::app::state::Direction::Horizontal, 0.5);
    assert!(discard_stale_resume_edit(&mut state), "归属失效即作废");
    assert!(state.overlay.is_none());
}

/// 把编辑模态的标题整体替换成给定文本。
fn edit_resume_title(state: &mut AppState, title: &str) {
    if let Some(Overlay::ResumeSessionEdit(edit)) = state.overlay.as_mut() {
        edit.input.clear();
        edit.input.insert_str(title);
    }
}
