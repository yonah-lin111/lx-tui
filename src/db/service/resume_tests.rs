//! `/resume` 端到端：列表回投 → 回车 → 会话真的装回窗格。
//!
//! 走生产路径（`open_history` → `Effect::ListChatSessions` → `apply_slash_enter`
//! → `Effect::LoadChatSession`），断言窗格的历史视图与绑定会话都被重建，
//! 而不只是"面板关掉了"。

use super::tests::trusted_service;
use super::*;
use crate::app::effects::Effect;
use crate::app::state::lx::slash::{SlashCommand, SlashPanel};
use crate::layout::PaneId;

/// 在 `cwd` 落一轮完整会话，返回 session id。
fn seed_session(database: &Database, cwd: &std::path::Path, text: &str) -> String {
    let committed = database
        .begin_turn(
            cwd,
            None,
            text,
            &crate::db::chat::ModelSnapshot {
                provider: "example".into(),
                model: "example-chat".into(),
                variant: Some("high".into()),
            },
        )
        .expect("begin turn");
    database
        .finish(
            cwd,
            &committed.assistant_message_id,
            &[crate::db::chat::ChatPart {
                position: 0,
                kind: "text".into(),
                content: format!("re: {text}"),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
            &crate::db::chat::TurnOutcome::Complete {
                stop_reason: Some("end_turn".into()),
            },
            crate::db::chat::Usage::default(),
        )
        .expect("finish turn");
    committed.session_id
}

/// 焦点窗格标识（`/resume` 的发起窗格）。
fn focused_pane(state: &AppState) -> PaneId {
    state.active_tab().layout.focus()
}

/// 打开 `/resume` 并跑完列表回投，返回条目数。
fn open_history_list(service: &mut Persistence, state: &mut AppState) -> usize {
    let pane = focused_pane(state);
    assert!(
        crate::app::update::open_history(state, pane),
        "打开历史面板"
    );
    service.pump(state);
    match state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.slash.clone())
    {
        Some(SlashPanel::History(panel)) => panel.entries.len(),
        other => panic!("历史面板未填充: {other:?}"),
    }
}

#[test]
fn resume_selected_session_loads_history_into_pane() {
    let (mut service, mut state, cwd, _) = trusted_service("resume-enter");
    let session_id = seed_session(service.db.as_ref().expect("db"), &cwd, "first question");

    let count = open_history_list(&mut service, &mut state);
    assert_eq!(count, 1, "列表应含刚播种的会话");

    // 高亮唯一条目后回车（等价于 `/resume` 面板上按 Enter）。
    let pane = focused_pane(&state);
    assert!(crate::app::update::apply_slash_enter(&mut state, pane));
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadChatSession { .. })),
        "回车须入队 LoadChatSession: {:?}",
        state.effects
    );
    service.pump(&mut state);

    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.session_id.as_deref(),
        Some(session_id.as_str()),
        "会话必须真的绑定回窗格（不只是关掉面板）"
    );
    assert!(
        !target.lx.chat.messages.is_empty(),
        "历史消息必须真的装回窗格，而不是空视图"
    );
    assert!(target.lx.slash.is_none(), "面板关闭");
}

#[test]
fn resume_enter_binds_model_selection_from_snapshot() {
    let (mut service, mut state, cwd, _) = trusted_service("resume-model");
    seed_session(service.db.as_ref().expect("db"), &cwd, "hello");
    open_history_list(&mut service, &mut state);

    let pane = focused_pane(&state);
    crate::app::update::apply_slash_enter(&mut state, pane);
    service.pump(&mut state);

    let selection = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.selection.clone())
        .expect("会话模型快照恢复为窗格选择");
    assert_eq!(selection.provider, "example");
    assert_eq!(selection.model, "example-chat");
}

#[test]
fn resume_command_opens_history_panel_from_slash_command() {
    let (mut service, mut state, cwd, _) = trusted_service("resume-command");
    seed_session(service.db.as_ref().expect("db"), &cwd, "hello");
    let pane = focused_pane(&state);
    assert!(crate::app::update::apply_slash_command(
        &mut state,
        pane,
        SlashCommand::Resume
    ));
    service.pump(&mut state);
    assert!(
        matches!(
            state.pane_anywhere(pane).and_then(|t| t.lx.slash.as_ref()),
            Some(SlashPanel::History(panel)) if panel.entries.len() == 1
        ),
        "`/resume` 命令打开并填充历史列表"
    );
}

#[test]
fn resume_enter_through_key_route_binds_session() {
    // 真实按键路径：`/resume` → ↑/↓ 高亮 → Enter。按键路由是 `Routed::Lx`，
    // 这里直接复用 `input::route` 产出的 `LxInput`，断言它落到同一套 update。
    let (mut service, mut state, cwd, _) = trusted_service("resume-keys");
    let session_id = seed_session(service.db.as_ref().expect("db"), &cwd, "via keys");
    open_history_list(&mut service, &mut state);
    let pane = focused_pane(&state);

    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );
    let routed = crate::input::route(
        enter,
        alacritty_terminal::term::TermMode::empty(),
        false,
        None,
        true,
    );
    let Some(crate::input::Routed::Lx(command)) = routed else {
        panic!("Enter 在 lx 页应路由为 LxInput");
    };
    crate::app::update::apply_lx_input(&mut state, command);
    service.pump(&mut state);

    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.session_id.as_deref(),
        Some(session_id.as_str()),
        "按键路径同样必须真的绑定会话"
    );
    assert!(!target.lx.chat.messages.is_empty(), "历史必须装回");
}

#[test]
fn rename_does_not_reorder_sessions_by_updated_time() {
    // 改备注名不该把会话顶到"最近活动"最前：改名只改 title。
    let (mut service, mut state, cwd, _) = trusted_service("resume-rename-order");
    let older = seed_session(service.db.as_ref().expect("db"), &cwd, "older");
    let newer = seed_session(service.db.as_ref().expect("db"), &cwd, "newer");
    let db = service.db.as_ref().expect("db");
    // 让 older 的 updated_at_ms 明显更早（模拟"很久没聊"）。
    db.conn
        .execute(
            "UPDATE agent_session SET updated_at_ms = 1, last_message_at_ms = 1 WHERE external_id = ?1",
            rusqlite::params![older],
        )
        .expect("backdate older");
    let order_before: Vec<String> = service
        .db
        .as_ref()
        .expect("db")
        .list_chat_sessions(&cwd, 0, 50)
        .expect("list")
        .into_iter()
        .map(|summary| summary.external_id)
        .collect();
    assert_eq!(order_before, vec![newer.clone(), older.clone()]);

    // 通过编辑模态改名 older 并跑完落库。
    let pane = focused_pane(&state);
    assert!(crate::app::update::open_history(&mut state, pane));
    service.pump(&mut state);
    let index = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::History(panel)) => panel
            .entries
            .iter()
            .position(|entry| entry.session_id == older)
            .expect("older listed"),
        other => panic!("history panel: {other:?}"),
    };
    assert!(crate::app::update::hover_lx_slash(&mut state, pane, index));
    assert!(crate::app::update::open_resume_session_edit(
        &mut state, pane
    ));
    if let Some(crate::app::overlay::Overlay::ResumeSessionEdit(edit)) = state.overlay.as_mut() {
        edit.input.clear();
        edit.input.insert_str("renamed older");
    }
    assert!(crate::app::update::apply_resume_session_edit_key(
        &mut state,
        crate::app::actions::OverlayKey::Enter
    ));
    service.pump(&mut state);

    let order_after: Vec<String> = service
        .db
        .as_ref()
        .expect("db")
        .list_chat_sessions(&cwd, 0, 50)
        .expect("list")
        .into_iter()
        .map(|summary| summary.external_id)
        .collect();
    assert_eq!(order_after, order_before, "改名不改变排序");
    let titles: Vec<String> = service
        .db
        .as_ref()
        .expect("db")
        .list_chat_sessions(&cwd, 0, 50)
        .expect("list")
        .into_iter()
        .map(|summary| summary.title)
        .collect();
    assert!(
        titles.iter().any(|title| title == "renamed older"),
        "{titles:?}"
    );
}

#[test]
fn delete_through_modal_removes_session_and_updates_panel() {
    let (mut service, mut state, cwd, _) = trusted_service("resume-delete-e2e");
    let doomed = seed_session(service.db.as_ref().expect("db"), &cwd, "doomed");
    let keep = seed_session(service.db.as_ref().expect("db"), &cwd, "keep");
    let pane = focused_pane(&state);
    assert!(crate::app::update::open_history(&mut state, pane));
    service.pump(&mut state);
    let index = match state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()) {
        Some(SlashPanel::History(panel)) => panel
            .entries
            .iter()
            .position(|entry| entry.session_id == doomed)
            .expect("doomed listed"),
        other => panic!("history panel: {other:?}"),
    };
    assert!(crate::app::update::hover_lx_slash(&mut state, pane, index));
    assert!(crate::app::update::open_resume_session_edit(
        &mut state, pane
    ));
    // 两次 ^D：进入确认、确认删除。
    crate::app::update::apply_resume_session_edit_key(
        &mut state,
        crate::app::actions::OverlayKey::Remove,
    );
    crate::app::update::apply_resume_session_edit_key(
        &mut state,
        crate::app::actions::OverlayKey::Remove,
    );
    service.pump(&mut state);

    let remaining: Vec<String> = service
        .db
        .as_ref()
        .expect("db")
        .list_chat_sessions(&cwd, 0, 50)
        .expect("list")
        .into_iter()
        .map(|summary| summary.external_id)
        .collect();
    assert_eq!(remaining, vec![keep], "库中只剩未被删的会话");
    let target = state.pane_anywhere(pane).expect("pane");
    let Some(SlashPanel::History(panel)) = target.lx.slash.as_ref() else {
        panic!("/resume 面板保持打开");
    };
    assert!(
        panel.entries.iter().all(|entry| entry.session_id != doomed),
        "列表就地移除该行"
    );
}
