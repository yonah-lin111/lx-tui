use super::*;
use crate::app::state::{ChatMessageView, Lx, LxEntry, PaneView, RequestKey, RequestState};
use crate::chat::events::PartKind;
use crate::db::chat::MessageRecap;

fn pane_id(state: &AppState) -> PaneId {
    state.active_tab().layout.focus()
}

fn text_message(id: &str, turn: i64, role: &str, text: &str) -> ChatMessageView {
    ChatMessageView {
        id: id.to_string(),
        sequence: turn * 2 + i64::from(role == "assistant"),
        kind: "chat".into(),
        role: role.into(),
        status: "complete".into(),
        turn_index: turn,
        shell: None,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: PartKind::Text,
                content: text.into(),
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

fn record(message_id: &str) -> MessageRecap {
    MessageRecap {
        id: "recap-1".into(),
        message_id: message_id.into(),
        summary: "summary".into(),
        next_action: "next".into(),
        model_provider: Some("p".into()),
        model_id: Some("m".into()),
        variant: None,
        model_label: "Model".into(),
        end_turn: 1,
        end_sequence: 3,
        created_at_ms: 1,
    }
}

#[test]
fn exchanges_keep_only_complete_visible_turns() {
    let messages = vec![
        text_message("u0", 0, "user", "first"),
        text_message("a0", 0, "assistant", "answer"),
        text_message("u1", 1, "user", "second"),
        text_message("a1", 1, "assistant", "answer2"),
    ];
    let exchanges = recap_exchanges(&messages);
    assert_eq!(exchanges.len(), 2);
    assert_eq!(exchanges[0].user, "first");
    assert_eq!(exchanges[1].assistant, "answer2");
}

#[test]
fn set_message_recaps_rebuilds_progress() {
    let mut state = AppState::demo();
    let pane = pane_id(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.chat.messages = vec![
        text_message("u0", 0, "user", "first"),
        text_message("a0", 0, "assistant", "answer"),
        text_message("a0b", 0, "assistant", "report auto-turn"),
    ];
    let changed = set_message_recaps(&mut state, pane, &[record("a0")]);
    assert!(changed);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.recap.entries.len(), 1);
    assert_eq!(target.lx.recap.completed_turns, 1);
    assert_eq!(target.lx.recap.last_recap_turn, Some(1));
}

#[test]
fn auto_eligible_counts_turns_after_last_recap_by_index() {
    let mut recap = crate::app::state::lx::LxRecap::default();
    for turn in [0, 2, 5] {
        recap.on_turn_completed(turn);
    }
    assert!(recap.auto_eligible(), "3 轮且从未 recap");
    recap.last_recap_turn = Some(2);
    assert!(!recap.auto_eligible(), "上次覆盖到 turn 2，只剩 turn 5");
    recap.on_turn_completed(7);
    assert!(recap.auto_eligible(), "turn 5/7 两轮新增");
}

#[test]
fn focus_lost_sets_deadline_and_focus_gained_cancels() {
    let mut state = AppState::demo();
    let now = Instant::now();
    on_terminal_focus_lost(&mut state, now);
    assert_eq!(state.recap_due_at, Some(now + RECAP_IDLE));
    assert!(on_terminal_focus_gained(&mut state));
    assert_eq!(state.recap_due_at, None);
}

#[test]
fn focus_gained_cancels_auto_but_keeps_manual_pending() {
    let mut state = AppState::demo();
    let pane = pane_id(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.recap.pending = Some(RecapPending {
        generation: 1,
        manual: false,
        message_id: "a0".into(),
        started_at: Instant::now(),
        elapsed_ms: 0,
    });
    assert!(on_terminal_focus_gained(&mut state));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .recap
            .pending
            .is_none(),
        "自动请求聚焦后取消"
    );

    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.recap.pending = Some(RecapPending {
        generation: 2,
        manual: true,
        message_id: "a0".into(),
        started_at: Instant::now(),
        elapsed_ms: 0,
    });
    on_terminal_focus_gained(&mut state);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .recap
            .pending
            .is_some(),
        "手动请求不因聚焦被取消"
    );
}

#[test]
fn manual_recap_without_session_warns() {
    let mut state = AppState::demo();
    let pane = pane_id(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.entry = LxEntry::Native;
    assert!(recap_command(&mut state, pane));
    assert!(state.pane_anywhere(pane).expect("pane").lx.toast.is_some());
}

#[test]
fn exchanges_skip_subagent_report_auto_turns() {
    let mut report = text_message(
        "report",
        0,
        "user",
        "<subagent_report>done</subagent_report>",
    );
    report.reports.push(crate::chat::agent::SubagentReportView {
        agent_id: "agent-1".into(),
        nickname: None,
        title: "report".into(),
        role: crate::chat::agent::AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 0,
        changed_paths: Vec::new(),
        duration_ms: None,
        body: "done".into(),
        delivered_at_ms: 0,
    });
    let messages = vec![
        report,
        text_message("a0", 0, "assistant", "answer"),
        text_message("u1", 1, "user", "second"),
        text_message("a1", 1, "assistant", "answer2"),
    ];
    let exchanges = recap_exchanges(&messages);
    assert_eq!(exchanges.len(), 1);
    assert_eq!(exchanges[0].user, "second");
}

#[test]
fn busy_turn_at_deadline_retries_instead_of_dropping_recap() {
    let mut state = AppState::demo();
    let pane = pane_id(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.entry = LxEntry::Native;
    target.lx.chat.session_id = Some("sess-1".into());
    target.lx.recap.on_turn_completed(0);
    target.lx.recap.on_turn_completed(1);
    target.lx.chat.request = RequestState::Streaming {
        key: RequestKey {
            pane,
            session: "sess-1".into(),
            generation: 1,
        },
        assistant_id: "a2".into(),
    };
    let now = Instant::now();
    state.recap_due_at = Some(now);
    assert!(tick_recap(&mut state, now));
    assert_eq!(state.recap_due_at, Some(now + RECAP_RETRY));
}

#[test]
fn recap_entries_are_empty_by_default() {
    let lx = Lx::new();
    assert!(lx.recap.is_empty());
}

#[test]
fn recap_is_prohibited_when_compact_is_at_bottom() {
    let mut state = AppState::demo();
    let pane = pane_id(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.entry = LxEntry::Native;
    target.lx.chat.session_id = Some("sess-1".into());
    target.lx.chat.messages = vec![
        text_message("u0", 0, "user", "first"),
        text_message("a0", 0, "assistant", "answer"),
    ];
    target
        .lx
        .compact
        .replace(vec![crate::app::state::CompactNoticeView {
            id: "compact-1".into(),
            sequence: 2,
            model_label: "M".into(),
            created_at_ms: 0,
            start_sequence: 0,
            end_sequence: 1,
            summary: "summary".into(),
            active: true,
            auto: false,
            report: false,
        }]);
    for turn in [0, 1, 2] {
        target.lx.recap.on_turn_completed(turn);
    }
    assert!(compact_at_bottom(&target.lx));
    assert_eq!(auto_eligibility(&state, pane), Eligibility::Ineligible);

    assert!(recap_command(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.recap.pending.is_none());
    assert_eq!(
        target.lx.toast.as_ref().map(|toast| toast.message.as_str()),
        Some(text::LX_RECAP_AFTER_COMPACT)
    );
}
