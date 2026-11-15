//! 详情 composer 的 `/` 命令面板：候选集合、按键采纳、关闭与提交拦截。

use super::*;
use crate::app::actions::LxInput;
use crate::app::state::{PaneView, SlashEntry, SlashPanel, SubagentRun, TaskTab};

/// 最小运行记录：Agents composer 只要 agent_id / child session 存在即可。
fn agent_run(agent_id: &str, child_session: &str) -> SubagentRun {
    SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: agent_id.into(),
        nickname: None,
        agent_path: format!("/root/{agent_id}"),
        child_session_id: child_session.into(),
        parent_session_id: "sess-1".into(),
        spawn_message_id: None,
        role: crate::chat::agent::AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "idle".into(),
        title: "investigate".into(),
        created_at_ms: 0,
        completed_at_ms: Some(1_000),
        report_json: None,
        report_delivered: false,
    })
}

/// 打开 Agents 页并进入 `sub-idle` 的追问编辑态。
fn agents_editing_state() -> (AppState, PaneId) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.chat.session_id = Some("sess-1".into());
        target.lx.agents.runs = vec![agent_run("sub-idle", "sess-idle")];
        target.lx.agents.ensure_composer("sub-idle");
        let page = target.lx.task.get_or_insert_with(Default::default);
        page.tab = TaskTab::Agents;
        page.agent_edit = Some("sub-idle".into());
        target.lx.sync_agent_composer_context();
    }
    (state, pane)
}

/// 打开 BTW 页并进入 `qa-1` 的编辑态。
fn btw_editing_state() -> (AppState, PaneId) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.entry = crate::app::state::LxEntry::Native;
    target.lx.chat.session_id = Some("sess-1".into());
    target.lx.task = Some(crate::app::state::TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    target.lx.btw.ensure_thread("qa-1");
    target.lx.btw.editing = true;
    target.lx.btw.qa_loaded = true;
    target
        .lx
        .sync_btw_composer_context(Some(std::env::temp_dir()), None);
    (state, pane)
}

fn panel(state: &AppState, pane: PaneId) -> Option<SlashPanel> {
    state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.active_editor().slash.clone())
}

fn file_entries(state: &AppState, pane: PaneId) -> Vec<&'static str> {
    let Some(SlashPanel::Commands { items, .. }) = panel(state, pane) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|entry| match entry {
            SlashEntry::Detail(command) => command.text(),
            SlashEntry::Command(command) => command.text(),
        })
        .collect()
}

fn editor_text(state: &AppState, pane: PaneId) -> String {
    state
        .pane_anywhere(pane)
        .map(|target| target.lx.active_editor().input.clone())
        .unwrap_or_default()
}

fn type_str(state: &mut AppState, text: &str) {
    for ch in text.chars() {
        crate::app::update::pane::apply_lx_input(state, crate::app::actions::LxInput::Char(ch));
    }
}

#[test]
fn typing_slash_in_agent_composer_opens_three_commands() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/");
    assert_eq!(
        file_entries(&state, pane),
        vec!["/undo", "/compact", "/report"]
    );
}

#[test]
fn typing_slash_in_btw_composer_opens_three_commands() {
    let (mut state, pane) = btw_editing_state();
    type_str(&mut state, "/");
    assert_eq!(
        file_entries(&state, pane),
        vec!["/undo", "/compact", "/new"]
    );
}

#[test]
fn enter_accepts_report_and_echoes_argument_hint() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/rep");
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    assert_eq!(editor_text(&state, pane), "/report ");
    assert!(panel(&state, pane).is_none(), "采纳后面板关闭");
    assert_eq!(
        crate::app::state::lx::slash::active_argument_hint("/report "),
        Some("[note]")
    );
}

#[test]
fn escape_closes_panel_and_keeps_edit_mode() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/");
    assert!(panel(&state, pane).is_some());
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Escape);
    assert!(panel(&state, pane).is_none());
    let editing = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .is_some_and(|page| page.agent_edit.is_some());
    assert!(editing, "第一下 Esc 只关面板");
}

#[test]
fn slash_panel_is_closed_when_leaving_edit_or_switching_tab() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/");
    assert!(panel(&state, pane).is_some());
    // 退出编辑态：面板随之关闭。
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Escape);
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Escape);
    assert!(panel(&state, pane).is_none());
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .is_some_and(|page| page.agent_edit.is_none())
    );

    // 重新打开面板后切换 tab：close_all 清掉所有 composer 面板。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        if let Some(composer) = target.lx.agents.composer_mut("sub-idle") {
            composer.clear();
        }
        target.lx.task.as_mut().expect("task").agent_edit = Some("sub-idle".into());
    }
    type_str(&mut state, "/");
    assert!(panel(&state, pane).is_some());
    crate::app::update::pane::cycle_task_tab(&mut state, pane, 1);
    assert!(panel(&state, pane).is_none());
}

#[test]
fn unknown_slash_is_not_sent_to_model() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/nope");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    assert_eq!(editor_text(&state, pane), "", "未知命令不留在草稿里");
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput { .. }
                | crate::app::effects::Effect::SteerSubagent { .. }
        )),
        "未知 / 命令不得发给模型：{:?}",
        state.effects
    );
}

#[test]
fn paste_into_composer_refreshes_slash_panel() {
    let (mut state, pane) = agents_editing_state();
    assert!(crate::app::update::apply_task_paste(
        &mut state, pane, "/rep"
    ));
    assert_eq!(file_entries(&state, pane), vec!["/report"]);
}
