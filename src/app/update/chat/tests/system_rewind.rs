//! 系统事件（模型切换 / recap）作为 rewind 目标的交互测试。

use super::*;
use crate::app::actions::LxInput;
use crate::app::state::lx::MessageRecapView;
use crate::app::state::{
    AssistantStepView, ChatMessageView, ChatPartView, LxEntry, ModelSwitchNoticeView, PaneView,
    RewindAction, RewindTargetKind,
};
use crate::app::update::pane::apply_lx_input;

fn text_message(id: &str, role: &str, sequence: i64, turn_index: i64) -> ChatMessageView {
    ChatMessageView {
        id: id.into(),
        sequence,
        kind: "chat".into(),
        role: role.into(),
        status: "complete".into(),
        turn_index,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: format!("{role} text"),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

fn system_state() -> (AppState, PaneId) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.entry = LxEntry::Native;
    target.lx.chat.session_id = Some("sess".into());
    target.lx.chat.revision = 1;
    target.lx.chat.messages = vec![
        text_message("u1", "user", 0, 0),
        text_message("a1", "assistant", 1, 0),
    ];
    (state, pane)
}

fn switch_record(id: &str, sequence: i64) -> crate::db::chat::ModelSwitchRecord {
    crate::db::chat::ModelSwitchRecord {
        id: id.into(),
        sequence,
        from_provider: Some("p".into()),
        from_model: Some("m".into()),
        from_variant: None,
        to_provider: "p".into(),
        to_model: "m2".into(),
        to_variant: None,
        from_label: Some("M".into()),
        to_label: "M2".into(),
        updated_sources: Vec::new(),
        injected_prompt: String::new(),
        created_at_ms: 0,
    }
}

fn recap_view() -> MessageRecapView {
    MessageRecapView {
        id: "recap-1".into(),
        summary: "summary".into(),
        next_action: String::new(),
        model_label: "M".into(),
        end_turn: 0,
        end_sequence: 1,
        created_at_ms: 0,
    }
}

#[test]
fn latest_model_switch_is_selected_and_opens_model_panel() {
    let (mut state, pane) = system_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .model_switch
        .switches = vec![ModelSwitchNoticeView::from_record(&switch_record("s1", 2))];

    crate::app::update::chat::rewind::select_latest(&mut state, pane).expect("selection");
    let selected = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .selected
        .clone()
        .expect("selected");
    assert_eq!(selected.kind, RewindTargetKind::ModelSwitch);
    assert_eq!(selected.user_message_id, "s1");

    apply_lx_input(&mut state, LxInput::Enter);
    let Some(SlashPanel::Rewind(panel)) = state.pane_anywhere(pane).expect("pane").lx.slash.clone()
    else {
        panic!("expected rewind panel");
    };
    assert_eq!(
        panel.actions,
        vec![RewindAction::RestoreModel, RewindAction::Cancel]
    );
    assert_eq!(panel.scope(), crate::app::state::RewindScope::ModelSwitch);

    apply_lx_input(&mut state, LxInput::Enter);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::RevertModelSwitch(request)
            if request.switch_id == "s1"
    )));
}

#[test]
fn latest_recap_is_selected_and_opens_recap_panel() {
    let (mut state, pane) = system_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .recap
        .entries
        .insert("a1".into(), recap_view());

    crate::app::update::chat::rewind::select_latest(&mut state, pane).expect("selection");
    let selected = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .selected
        .clone()
        .expect("selected");
    assert_eq!(selected.kind, RewindTargetKind::Recap);
    assert_eq!(selected.user_message_id, "a1");

    apply_lx_input(&mut state, LxInput::Enter);
    let Some(SlashPanel::Rewind(panel)) = state.pane_anywhere(pane).expect("pane").lx.slash.clone()
    else {
        panic!("expected rewind panel");
    };
    assert_eq!(
        panel.actions,
        vec![RewindAction::DeleteRecap, RewindAction::Cancel]
    );
    assert_eq!(panel.scope(), crate::app::state::RewindScope::Recap);

    apply_lx_input(&mut state, LxInput::Enter);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::DeleteRecap(request)
            if request.message_id == "a1"
    )));
}

#[test]
fn compact_at_bottom_is_selected_before_recap_or_user_turn() {
    let (mut state, pane) = system_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.recap.entries.insert("a1".into(), recap_view());
        target
            .lx
            .compact
            .replace(vec![crate::app::state::CompactNoticeView {
                id: "c1".into(),
                sequence: 2,
                model_label: "M".into(),
                created_at_ms: 0,
                start_sequence: 0,
                end_sequence: 1,
                summary: "sum".into(),
                active: true,
                auto: false,
            }]);
    }

    crate::app::update::chat::rewind::select_latest(&mut state, pane).expect("selection");
    let selected = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .selected
        .clone()
        .expect("selected");
    assert_eq!(selected.kind, RewindTargetKind::Compact);
    assert_eq!(selected.user_message_id, "c1");
}

#[test]
fn inactive_compact_checkpoint_is_ignored_when_selecting_latest() {
    let (mut state, pane) = system_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.recap.entries.insert("a1".into(), recap_view());
        target
            .lx
            .compact
            .replace(vec![crate::app::state::CompactNoticeView {
                id: "c1".into(),
                sequence: 2,
                model_label: "M".into(),
                created_at_ms: 0,
                start_sequence: 0,
                end_sequence: 1,
                summary: "sum".into(),
                active: false,
                auto: false,
            }]);
    }

    crate::app::update::chat::rewind::select_latest(&mut state, pane).expect("selection");
    let selected = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .selected
        .clone()
        .expect("selected");
    assert_eq!(selected.kind, RewindTargetKind::Recap);
}
