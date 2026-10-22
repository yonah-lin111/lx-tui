//! 撤销交互的纯状态测试：双 Esc、面板确认、成功回填与上下文隔离。

use super::*;
use crate::app::actions::LxInput;
use crate::app::state::{PaneView, RewindAction, SlashCommand};
use crate::app::update::pane::apply_lx_input;
use crate::db::chat::FileAvailability;

fn user_message(id: &str, text: &str, sequence: i64) -> crate::app::state::ChatMessageView {
    crate::app::state::ChatMessageView {
        sequence,
        id: id.into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
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
    }
}

fn assistant_message(id: &str, sequence: i64) -> crate::app::state::ChatMessageView {
    crate::app::state::ChatMessageView {
        sequence,
        id: id.into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}

fn rewound_state() -> (AppState, PaneId) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.chat.session_id = Some("sess".into());
    target.lx.chat.revision = 1;
    target.lx.chat.messages = vec![user_message("u1", "原文", 0), assistant_message("a1", 1)];
    (state, pane)
}

fn prepare_known_empty(state: &mut AppState, pane: PaneId) {
    assert!(crate::app::update::on_rewind_prepared(
        state,
        pane,
        "sess",
        "u1",
        &FileAvailability::KnownEmpty,
        1,
        "原文",
    ));
}

#[test]
fn double_escape_on_empty_input_selects_latest_user_turn() {
    let (mut state, pane) = rewound_state();
    apply_lx_input(&mut state, LxInput::Escape);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .rewind
            .armed_at
            .is_some(),
        "第一次 Esc 只待命"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .rewind
            .selected
            .is_none()
    );
    apply_lx_input(&mut state, LxInput::Escape);
    let target = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .selected
        .clone()
        .expect("selected");
    assert_eq!(target.user_message_id, "u1");
    assert_eq!(target.session_id, "sess");
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::PrepareRewind { .. })),
        "选中后必须查询快照可用性"
    );
}

#[test]
fn non_empty_draft_never_arms_rewind() {
    let (mut state, pane) = rewound_state();
    state.pane_mut_anywhere(pane).expect("pane").lx.input = "draft".into();
    apply_lx_input(&mut state, LxInput::Escape);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .rewind
            .armed_at
            .is_none(),
        "有草稿时 Esc 不进入撤销"
    );
}

#[test]
fn enter_opens_panel_then_second_enter_executes_once() {
    let (mut state, pane) = rewound_state();
    apply_lx_input(&mut state, LxInput::Escape);
    apply_lx_input(&mut state, LxInput::Escape);
    prepare_known_empty(&mut state, pane);
    apply_lx_input(&mut state, LxInput::Enter);
    assert!(
        matches!(
            state.pane_anywhere(pane).expect("pane").lx.slash,
            Some(SlashPanel::Rewind(_))
        ),
        "第一次 Enter 只打开四项面板"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .rewind
            .executing
            .is_none()
    );
    apply_lx_input(&mut state, LxInput::Enter);
    let executing = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .executing
        .clone()
        .expect("executing");
    assert_eq!(
        executing.action,
        RewindAction::RevertAndDelete,
        "默认第一项"
    );
    assert_eq!(
        state
            .effects
            .iter()
            .filter(|effect| matches!(effect, crate::app::effects::Effect::ExecuteRewind(_)))
            .count(),
        1,
        "第二次 Enter 只提交一次执行"
    );
}

#[test]
fn escape_in_panel_returns_to_selection_and_second_escape_cancels() {
    let (mut state, pane) = rewound_state();
    apply_lx_input(&mut state, LxInput::Escape);
    apply_lx_input(&mut state, LxInput::Escape);
    prepare_known_empty(&mut state, pane);
    apply_lx_input(&mut state, LxInput::Enter);
    apply_lx_input(&mut state, LxInput::Escape);
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.rewind.selected.is_some(), "第一次 Esc 退回选中态");
    assert!(target.lx.slash.is_none());
    apply_lx_input(&mut state, LxInput::Escape);
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .rewind
            .selected
            .is_none(),
        "再次 Esc 取消选择"
    );
}

#[test]
fn successful_delete_backfills_text_without_sending() {
    let (mut state, pane) = rewound_state();
    apply_lx_input(&mut state, LxInput::Escape);
    apply_lx_input(&mut state, LxInput::Escape);
    prepare_known_empty(&mut state, pane);
    apply_lx_input(&mut state, LxInput::Enter);
    apply_lx_input(&mut state, LxInput::Enter);
    assert!(crate::app::update::on_rewind_succeeded(
        &mut state,
        pane,
        RewindAction::RevertAndDelete,
        "原始提问\n第二行",
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, "原始提问\n第二行");
    assert!(target.lx.rewind.executing.is_none());
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::BeginChatTurn { .. })),
        "回填不自动发送"
    );
}

#[test]
fn edited_draft_is_not_overwritten_by_backfill() {
    let (mut state, pane) = rewound_state();
    apply_lx_input(&mut state, LxInput::Escape);
    apply_lx_input(&mut state, LxInput::Escape);
    prepare_known_empty(&mut state, pane);
    apply_lx_input(&mut state, LxInput::Enter);
    apply_lx_input(&mut state, LxInput::Enter);
    // 执行期间用户输入了新草稿（执行中吞键；直接写状态模拟迟到编辑）。
    state.pane_mut_anywhere(pane).expect("pane").lx.input = "my draft".into();
    crate::app::update::on_rewind_succeeded(&mut state, pane, RewindAction::DeleteOnly, "原文");
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "my draft",
        "非空/已改草稿不被回填覆盖"
    );
}

#[test]
fn slash_undo_is_the_same_command_without_duplicate_candidates() {
    let matches = SlashPanel::commands_matching("/undo");
    assert_eq!(matches.first(), Some(&SlashCommand::Rewind));
    assert_eq!(
        matches
            .iter()
            .filter(|command| **command == SlashCommand::Rewind)
            .count(),
        1,
        "别名不能产生重复候选"
    );
}

#[test]
fn notices_are_not_part_of_model_assembly() {
    let (mut state, pane) = rewound_state();
    state.pane_mut_anywhere(pane).expect("pane").lx.chat.notices =
        vec![crate::app::state::RewindNoticeView {
            id: "n1".into(),
            sequence: 2,
            kind: "revert_and_delete".into(),
            restored_file_count: 2,
            files_already_restored: false,
        }];
    let target = state.pane_anywhere(pane).expect("pane");
    let frozen = crate::app::state::FrozenModel {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
        transport: "openai".into(),
        base_url: "https://api.example.invalid".into(),
        api_key: "k".into(),
        context_limit: Some(100_000),
        output_limit: Some(4_096),
        variant_params: None,
        idle_timeout_ms: 30_000,
    };
    let assembled = crate::event::chat::assemble_turn_request(&target.lx.chat.messages, &frozen);
    let debug = format!("{:?}", assembled.request.items);
    assert!(debug.contains("原文"));
    assert!(!debug.contains("revert_and_delete"));
    assert!(!debug.contains("Rewind"));
}

#[test]
fn slash_command_accepts_standalone_line_and_opens_panel() {
    let (mut state, pane) = rewound_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.input = "/rewind".into();
        target.lx.cursor = target.lx.input.len();
        target.lx.slash = Some(SlashPanel::Commands {
            items: vec![SlashCommand::Rewind],
            window: crate::app::state::PanelWindow::new(0),
        });
    }
    assert!(crate::app::update::pane::accept_slash_command(
        &mut state, pane
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.input.is_empty(), "命令文本整条消费");
    assert!(matches!(target.lx.slash, Some(SlashPanel::Rewind(_))));
    assert!(
        target.lx.rewind.selected.is_some(),
        "命令采纳直接选定最新轮次"
    );
}

#[test]
fn slash_command_with_other_text_is_rejected_and_draft_kept() {
    let (mut state, pane) = rewound_state();
    let draft = "hello\n/rewind";
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.input = draft.into();
        target.lx.cursor = target.lx.input.len();
        target.lx.slash = Some(SlashPanel::Commands {
            items: vec![SlashCommand::Rewind],
            window: crate::app::state::PanelWindow::new(0),
        });
    }
    assert!(crate::app::update::pane::accept_slash_command(
        &mut state, pane
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, draft, "拒绝时保留草稿");
    assert!(target.lx.rewind.selected.is_none());
}

#[test]
fn busy_escape_keeps_stop_semantics_and_does_not_arm_rewind() {
    let (mut state, pane) = rewound_state();
    state.pane_mut_anywhere(pane).expect("pane").lx.chat.request =
        crate::app::state::RequestState::Streaming {
            key: crate::app::state::RequestKey {
                pane,
                session: "sess".into(),
                generation: 1,
            },
            assistant_id: "a1".into(),
        };
    apply_lx_input(&mut state, LxInput::Escape);
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.rewind.armed_at.is_none(), "忙时不得进入撤销");
    assert!(
        target.lx.chat.stop_armed_at.is_some(),
        "忙时第一次 Esc 仍进入停止待命"
    );
}

#[test]
fn double_escape_requests_centering_latest_user_message() {
    let (mut state, pane) = rewound_state();
    apply_lx_input(&mut state, LxInput::Escape);
    apply_lx_input(&mut state, LxInput::Escape);
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .rewind
            .center_target
            .as_deref(),
        Some("u1")
    );
}

#[test]
fn resolver_scrolls_center_request_and_consumes_it() {
    let (mut state, pane) = rewound_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let mut messages = Vec::new();
        for index in 0..30 {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            let mut message = if role == "user" {
                user_message(&format!("u{index}"), "history line", index as i64)
            } else {
                assistant_message(&format!("a{index}"), index as i64)
            };
            message.id = format!("m{index}");
            messages.push(message);
        }
        target.lx.chat.messages = messages;
        target.lx.rewind.center_target = Some("m10".into());
    }
    let changed = crate::app::update::resolve_lx_rewind_center(
        &mut state,
        &[(pane, ratatui::layout::Rect::new(0, 0, 80, 24))],
    );
    assert!(changed, "居中请求应改变视口");
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.rewind.center_target.is_none(), "请求只消费一次");
    assert!(!target.lx.chat.follow_bottom);
    assert!(target.lx.chat.history_scroll > 0);
}
