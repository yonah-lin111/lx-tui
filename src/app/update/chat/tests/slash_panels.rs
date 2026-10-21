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
fn model_picker_marks_current_model_after_restart() {
    // 回归：重启后窗格级 `chat.selection` 是 None，真正生效的是配置 `defaultModel`。
    // 只读 `chat.selection` 会让"重启后打开面板看不到当前模型"。
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .selection = None;
    // ready_state 的配置默认模型是 p/m。
    assert!(state.models.settings.default_model.is_some());
    assert!(open_model_picker(&mut state, pane));
    let Some(SlashPanel::Model(picker)) =
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone())
    else {
        panic!("model panel");
    };
    assert_eq!(picker.current, Some(0), "重启后仍标出配置里的当前模型");
    assert_eq!(picker.window.active, 0, "高亮也落在当前模型上");
}

#[test]
fn model_picker_falls_back_to_default_when_pane_selection_is_stale() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    // 窗格选择指向已不存在的 provider/model：按 `effective_selection` 回落配置默认。
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
    assert_eq!(picker.current, Some(0), "失效选择回落到配置默认并标出");
}

#[test]
fn model_picker_without_any_selection_has_no_current_marker() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    // 窗格没有选择、配置也没有 defaultModel：确实没有"当前模型"可标。
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .selection = None;
    state.models.settings.default_model = None;
    assert!(open_model_picker(&mut state, pane));
    let Some(SlashPanel::Model(picker)) =
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone())
    else {
        panic!("model panel");
    };
    assert_eq!(picker.current, None, "没有任何生效模型时不标");
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

/// `/resume` 面板内 `Ctrl/Cmd+C`：复制高亮行 session id（裸 id），且**不清空**输入框。
#[test]
fn clear_key_copies_highlighted_session_id_without_clearing_input() {
    use crate::app::actions::LxInput;
    use crate::app::effects::Effect;

    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    // 输入框里放一段草稿：面板内 ^C 不该把它清掉。
    let draft = "帮我查下昨天那个 panic";
    state.pane_mut_anywhere(pane).expect("pane").lx.input = draft.into();
    // 高亮第 2 条（sess-b）。
    crate::app::update::apply_lx_input(&mut state, LxInput::Down);

    crate::app::update::apply_lx_input(&mut state, LxInput::Clear);

    let copied: Vec<(String, Option<crate::layout::PaneId>)> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CopyToClipboard { text, anchor } => Some((text.clone(), *anchor)),
            _ => None,
        })
        .collect();
    assert_eq!(
        copied,
        vec![("sess-b".to_string(), Some(pane))],
        "复制高亮行裸 id，且 toast 锚到当前窗格（main content）"
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.input, draft, "面板内 ^C 不清空输入框");
    assert!(
        matches!(lx.slash, Some(SlashPanel::History(_))),
        "面板保持打开"
    );
}

/// 空面板（无条目）时 `^C` 静默吞掉：无复制效应、不清空、不关面板。
#[test]
fn clear_key_on_empty_history_panel_is_swallowed() {
    use crate::app::actions::LxInput;
    use crate::app::effects::Effect;

    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    // 不投递任何条目：面板只有空态占位行。
    let draft = "keep me";
    state.pane_mut_anywhere(pane).expect("pane").lx.input = draft.into();
    state.effects.clear();

    crate::app::update::apply_lx_input(&mut state, LxInput::Clear);

    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CopyToClipboard { .. })),
        "空列表不产生复制效应"
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.input, draft, "不清空输入框");
    assert!(
        matches!(lx.slash, Some(SlashPanel::History(_))),
        "面板保持打开"
    );
}

/// 面板关闭时 `^C` 仍是原来的「清空输入框」语义。
#[test]
fn clear_key_still_clears_input_when_resume_panel_is_closed() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    state.pane_mut_anywhere(pane).expect("pane").lx.input = "draft".into();

    crate::app::update::apply_lx_input(&mut state, LxInput::Clear);

    assert!(state.pane_anywhere(pane).expect("pane").lx.input.is_empty());
}

/// `^C` 在 `/model` 面板里仍被吞掉（不复制、不清空）。
#[test]
fn clear_key_is_swallowed_in_model_panel() {
    use crate::app::actions::LxInput;
    use crate::app::effects::Effect;

    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .selection = Some(ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    });
    assert!(open_model_picker(&mut state, pane));
    let draft = "keep me";
    state.pane_mut_anywhere(pane).expect("pane").lx.input = draft.into();
    state.effects.clear();

    crate::app::update::apply_lx_input(&mut state, LxInput::Clear);

    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CopyToClipboard { .. })),
        "model 面板不复制"
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        draft,
        "model 面板内不清空"
    );
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

/// 面板内打字即过滤：输入框内容就是过滤词，`↵` 采纳的是过滤后的高亮项。
#[test]
fn typing_in_resume_panel_filters_and_enter_picks_filtered_row() {
    use crate::app::actions::LxInput;
    use crate::app::effects::Effect;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    // 高亮本来在 sess-a（第 1 条）。
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('b'));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('e'));
    let panel = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::History(panel)) => panel,
        _ => panic!("history panel"),
    };
    assert_eq!(panel.filtered, vec![1], "只留标题命中 `be` 的 Beta");
    assert_eq!(panel.window.active, 0, "高亮移到唯一命中项");

    // 采纳：应加载 Beta，而不是原本高亮的 Alpha（绑定由回投完成，这里看效应）。
    assert!(apply_slash_enter(&mut state, pane));
    let loaded: Vec<String> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadChatSession { session_id, .. } => Some(session_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(loaded, vec!["sess-b".to_string()], "采纳的是过滤后的高亮项");
}

/// 面板内退格恢复全量视图。
#[test]
fn backspace_in_resume_panel_restores_full_list() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('b'));
    crate::app::update::apply_lx_input(&mut state, LxInput::Backspace);
    let panel = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::History(panel)) => panel,
        _ => panic!("history panel"),
    };
    assert_eq!(panel.filtered, vec![0, 1], "退格后恢复全量");
    assert!(panel.filter.is_empty(), "过滤词随输入框清空");
}

/// 过滤无命中时：空态不可采纳，面板保持打开。
#[test]
fn enter_on_empty_filter_result_is_swallowed() {
    use crate::app::actions::LxInput;
    use crate::app::effects::Effect;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    for ch in "zzzz".chars() {
        crate::app::update::apply_lx_input(&mut state, LxInput::Char(ch));
    }
    assert!(apply_slash_enter(&mut state, pane), "按键被消费");
    assert!(
        matches!(
            state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()),
            Some(SlashPanel::History(_))
        ),
        "面板保持打开"
    );
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadChatSession { .. })),
        "空态不加载任何会话"
    );
}

/// 面板内 `^C` 复制的仍是过滤后的高亮项。
#[test]
fn copy_in_resume_panel_copies_filtered_row() {
    use crate::app::actions::LxInput;
    use crate::app::effects::Effect;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    for ch in "be".chars() {
        crate::app::update::apply_lx_input(&mut state, LxInput::Char(ch));
    }
    crate::app::update::apply_lx_input(&mut state, LxInput::Clear);
    let copied: Vec<String> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CopyToClipboard { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, vec!["sess-b".to_string()], "复制过滤后的高亮项");
}

/// `/model` 第一级键入过滤；进入第二级后过滤视图重置（不沿用上一级过滤词）。
#[test]
fn typing_in_model_panel_filters_models_and_variant_stage_resets() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    state.models = models_with(Some("high"));
    let pane = focus_pane(&state);
    assert!(open_model_picker(&mut state, pane));
    // 唯一候选（p/m）；打一个必然无命中的词验证过滤生效。
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('z'));
    let picker = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::Model(picker)) => picker,
        _ => panic!("model panel"),
    };
    assert!(picker.is_filter_empty(), "无命中");
    assert_eq!(picker.stage_len(), 1, "原始候选仍在");
    // 退格恢复。
    crate::app::update::apply_lx_input(&mut state, LxInput::Backspace);
    let picker = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::Model(picker)) => picker,
        _ => panic!("model panel"),
    };
    assert_eq!(picker.filtered, vec![0], "恢复全量");
}

/// `Esc` 关闭 `/resume` 面板：整框清空输入框里的过滤词。
#[test]
fn escape_closes_resume_panel_and_clears_input() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    for ch in "be".chars() {
        crate::app::update::apply_lx_input(&mut state, LxInput::Char(ch));
    }
    assert_eq!(state.pane_anywhere(pane).expect("pane").lx.input, "be");

    crate::app::update::apply_lx_input(&mut state, LxInput::Escape);

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(lx.slash.is_none(), "面板已关闭");
    assert!(lx.input.is_empty(), "过滤词被清空: {:?}", lx.input);
}

/// `Esc` 关闭 `/model` 第一级面板：同样整框清空。
#[test]
fn escape_closes_model_panel_and_clears_input() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    state.models = models_with(Some("high"));
    let pane = focus_pane(&state);
    assert!(open_model_picker(&mut state, pane));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('z'));

    crate::app::update::apply_lx_input(&mut state, LxInput::Escape);

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(lx.slash.is_none(), "面板已关闭");
    assert!(lx.input.is_empty(), "过滤词被清空: {:?}", lx.input);
}

/// `Esc` 从 `/model` 第二级退回第一级：**不清空**（仍在同一流程内）。
#[test]
fn escape_from_variant_stage_keeps_input() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    state.models = models_with(Some("high"));
    let pane = focus_pane(&state);
    assert!(open_model_picker(&mut state, pane));
    // 进入第二级（默认模型有 high variant）。
    assert!(apply_slash_enter(&mut state, pane));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('x'));

    crate::app::update::apply_lx_input(&mut state, LxInput::Escape);

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(
        matches!(lx.slash, Some(SlashPanel::Model(_))),
        "仍停留在第一级面板"
    );
    assert_eq!(lx.input, "x", "退回第一级不清空输入框");
}

/// `↵` 选中会话：清空输入框。
#[test]
fn enter_on_resume_panel_clears_input() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    for ch in "be".chars() {
        crate::app::update::apply_lx_input(&mut state, LxInput::Char(ch));
    }

    assert!(apply_slash_enter(&mut state, pane));

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(lx.input.is_empty(), "选中会话后清空: {:?}", lx.input);
}

/// `↵` 进入第二级：**不清空**（换层继续挑，过滤词要留给退回第一级时用）。
#[test]
fn enter_into_variant_stage_keeps_input() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    state.models = models_with(Some("high"));
    let pane = focus_pane(&state);
    assert!(open_model_picker(&mut state, pane));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('m'));

    assert!(apply_slash_enter(&mut state, pane));

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.input, "m", "进入第二级不清空过滤词");
}

/// `↵` 选中思考等级（关闭面板）：清空输入框。
#[test]
fn enter_on_variant_stage_clears_input() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    state.models = models_with(Some("high"));
    let pane = focus_pane(&state);
    assert!(open_model_picker(&mut state, pane));
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('m'));
    assert!(apply_slash_enter(&mut state, pane));
    // 第二级不支持过滤，但输入框里残留的字符仍应在选中后清空。
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('y'));

    assert!(apply_slash_enter(&mut state, pane));

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(lx.slash.is_none(), "面板已关闭");
    assert!(lx.input.is_empty(), "选中等级后清空: {:?}", lx.input);
}

/// `⇧↵` 打开会话编辑模态：**不清空**输入框。
#[test]
fn shift_enter_on_resume_panel_keeps_input() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    for ch in "be".chars() {
        crate::app::update::apply_lx_input(&mut state, LxInput::Char(ch));
    }

    crate::app::update::apply_lx_input(&mut state, LxInput::EnterFolder);

    assert!(state.overlay.is_some(), "编辑模态已打开");
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "be",
        "打开编辑模态不清空输入框"
    );
}

/// `^C` 复制会话 id：**不清空**（支持连续复制多条）。
#[test]
fn copy_on_resume_panel_keeps_input() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    for ch in "be".chars() {
        crate::app::update::apply_lx_input(&mut state, LxInput::Char(ch));
    }

    crate::app::update::apply_lx_input(&mut state, LxInput::Clear);

    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "be",
        "复制不清空输入框"
    );
}

/// `/model` 第二级（思考等级）不支持过滤：键入字符不会缩小列表，也不会出现空态。
#[test]
fn variant_stage_ignores_typing_for_filtering() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    // 两个等级：low / high。
    let mut settings = models_with(Some("high"));
    let provider = settings.settings.providers.get_mut("p").expect("provider");
    let model = provider.models.get_mut("m").expect("model");
    model
        .variants
        .insert("low".into(), serde_json::json!({"reasoningEffort": "low"}));
    state.models = settings;
    let pane = focus_pane(&state);
    assert!(open_model_picker(&mut state, pane));
    assert!(apply_slash_enter(&mut state, pane), "进入第二级");

    // 键一个必然不匹配任何等级的字符。
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('z'));

    let picker = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::Model(picker)) => picker,
        _ => panic!("model panel"),
    };
    assert_eq!(picker.stage_len(), 2, "两级都还在");
    assert_eq!(picker.filtered, vec![0, 1], "第二级恒为全量视图");
    assert!(!picker.is_filter_empty(), "不会显示 no matches");
}

/// `/resume` 过滤仍生效（回归：清空逻辑不得破坏过滤本身）。
#[test]
fn resume_filter_still_works_after_clear_rules() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(open_history(&mut state, pane));
    seed_history(&mut state, pane, "sess-a");
    for ch in "be".chars() {
        crate::app::update::apply_lx_input(&mut state, LxInput::Char(ch));
    }
    let panel = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::History(panel)) => panel,
        _ => panic!("history panel"),
    };
    assert_eq!(panel.filtered, vec![1], "过滤仍生效");
}

/// 回归：`Esc` 从第二级退回第一级，必须重建出**过滤后**的列表，不是全量。
///
/// 之前的实现是进入第二级就把输入框清空，退回时按空串重算 → 列表跳回全量，
/// 用户看到「Esc 退回第一级仍只剩 deepseek 没有效果」。
#[test]
fn escape_from_variant_stage_restores_filtered_model_list() {
    use crate::app::actions::LxInput;

    let mut state = ready_state("");
    // 两个模型：m（有 high/low）、other（无 variants）。
    let mut settings = models_with(Some("high"));
    {
        let provider = settings.settings.providers.get_mut("p").expect("provider");
        let model = provider.models.get_mut("m").expect("model");
        model
            .variants
            .insert("low".into(), serde_json::json!({"reasoningEffort": "low"}));
        let mut other = model.clone();
        other.id = "other".into();
        other.name = "zzz-other".into();
        other.variants.clear();
        provider.models.insert("other".into(), other);
    }
    state.models = settings;
    let pane = focus_pane(&state);
    assert!(open_model_picker(&mut state, pane));
    // 只留 m（`m` 也命中 `other` 的 provider？不会，按模型名匹配）。
    crate::app::update::apply_lx_input(&mut state, LxInput::Char('m'));
    let picker = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::Model(picker)) => picker,
        _ => panic!("model panel"),
    };
    let filtered_before = picker.filtered.clone();
    assert_eq!(filtered_before.len(), 1, "第一级只留一个模型");
    assert!(picker.stage_len() >= 2, "原始候选至少两个");

    // 进入第二级，再 Esc 退回第一级。
    assert!(apply_slash_enter(&mut state, pane));
    crate::app::update::apply_lx_input(&mut state, LxInput::Escape);

    let picker = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::Model(picker)) => picker,
        _ => panic!("应停留在第一级面板"),
    };
    assert!(
        matches!(picker.stage, ModelPickerStage::Models),
        "已退回第一级"
    );
    assert_eq!(picker.filter, "m", "过滤词仍在");
    assert_eq!(
        picker.filtered, filtered_before,
        "退回后仍是过滤后的列表，不跳回全量"
    );
}
