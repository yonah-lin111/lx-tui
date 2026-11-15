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
        vec!["/rewind", "/compact", "/report"]
    );
}

#[test]
fn typing_slash_in_btw_composer_opens_three_commands() {
    let (mut state, pane) = btw_editing_state();
    type_str(&mut state, "/");
    assert_eq!(
        file_entries(&state, pane),
        vec!["/rewind", "/compact", "/new"]
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

#[test]
fn report_idle_agent_emits_send_input_with_notify_parent_true() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/report check the log");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    let message = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::SendSubagentUserInput {
                agent_id,
                text,
                notify_parent,
                ..
            } if agent_id == "sub-idle" => Some((text.clone(), *notify_parent)),
            _ => None,
        })
        .expect("汇报轮入队");
    assert!(message.1, "显式 /report 必须 notify_parent=true");
    assert!(
        message
            .0
            .starts_with(crate::chat::agent::REPORT_REQUEST_MARKER),
        "汇报请求带固定标记：{}",
        message.0
    );
    assert!(message.0.contains("User note: check the log"));
    assert_eq!(editor_text(&state, pane), "", "命令文本已消费");
    assert!(panel(&state, pane).is_none());
}

#[test]
fn report_running_agent_is_rejected_without_effect() {
    let (mut state, pane) = agents_editing_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_agent_mut("sub-idle")
        .expect("run")
        .status = "running".into();
    type_str(&mut state, "/report now");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput { .. }
                | crate::app::effects::Effect::SteerSubagent { .. }
        )),
        "运行中拒绝汇报：{:?}",
        state.effects
    );
    assert_eq!(editor_text(&state, pane), "");
}

#[test]
fn private_follow_up_never_emits_report() {
    let (mut state, _pane) = agents_editing_state();
    type_str(&mut state, "今天天气不错");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput {
                notify_parent: false,
                text,
                ..
            } if text == "今天天气不错"
        )),
        "闲聊按私聊追问发送：{:?}",
        state.effects
    );
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput {
                notify_parent: true,
                ..
            }
        )),
        "不得产生汇报轮"
    );
}

#[test]
fn report_message_contains_marker_and_note() {
    let message = crate::chat::agent::report_request_message("  定位登录超时  ");
    assert!(message.starts_with(crate::chat::agent::REPORT_REQUEST_MARKER));
    assert!(message.contains("User note: 定位登录超时"));
    assert!(message.contains("Do not include greetings"));
    let bare = crate::chat::agent::report_request_message("  ");
    assert!(!bare.contains("User note:"));
}

fn rewind_plan(child_session: &str) -> crate::app::state::DetailRewindPlan {
    crate::app::state::DetailRewindPlan {
        target: crate::app::state::DetailTarget::Agent("sub-idle".into()),
        workspace: std::path::PathBuf::from("/work/demo"),
        child_session: child_session.into(),
        user_message_id: "msg-1".into(),
        expected_revision: 3,
        input_at_open: String::new(),
        user_text: "previous question".into(),
        action: crate::app::state::RewindAction::RevertAndDelete,
        snapshot: crate::app::state::SnapshotState::KnownEmpty,
        kind: crate::app::state::RewindTargetKind::Turn,
    }
}

fn compact_rewind_plan(child_session: &str) -> crate::app::state::DetailRewindPlan {
    crate::app::state::DetailRewindPlan {
        target: crate::app::state::DetailTarget::Agent("sub-idle".into()),
        workspace: std::path::PathBuf::from("/work/demo"),
        child_session: child_session.into(),
        user_message_id: "compact-1".into(),
        expected_revision: 4,
        input_at_open: String::new(),
        user_text: String::new(),
        action: crate::app::state::RewindAction::RestoreCompact,
        snapshot: crate::app::state::SnapshotState::KnownEmpty,
        kind: crate::app::state::RewindTargetKind::Compact,
    }
}

#[test]
fn rewind_prepare_shows_four_actions_with_restore_first() {
    let (mut state, pane) = agents_editing_state();
    assert!(crate::app::update::on_detail_rewind_prepared(
        &mut state,
        pane,
        rewind_plan("sess-idle")
    ));
    let Some(SlashPanel::Rewind(panel)) = panel(&state, pane) else {
        panic!("expected rewind panel");
    };
    assert_eq!(
        panel.actions,
        crate::app::state::RewindAction::ALL.to_vec(),
        "与主 /rewind 相同的四项动作"
    );
    assert_eq!(
        panel.active_action(),
        crate::app::state::RewindAction::RevertAndDelete,
        "默认第一项"
    );
    assert_eq!(panel.scope(), crate::app::state::RewindScope::Detail);
    assert_eq!(
        panel.detail.as_ref().map(|detail| detail.kind),
        Some(crate::app::state::DetailRewindKind::Agent)
    );
}

#[test]
fn rewind_prepare_for_compact_shows_restore_and_cancel() {
    let (mut state, pane) = agents_editing_state();
    assert!(crate::app::update::on_detail_rewind_prepared(
        &mut state,
        pane,
        compact_rewind_plan("sess-idle")
    ));
    let Some(SlashPanel::Rewind(panel)) = panel(&state, pane) else {
        panic!("expected compact rewind panel");
    };
    assert_eq!(
        panel.actions,
        vec![
            crate::app::state::RewindAction::RestoreCompact,
            crate::app::state::RewindAction::Cancel
        ]
    );
    assert_eq!(
        panel.active_action(),
        crate::app::state::RewindAction::RestoreCompact
    );
    assert_eq!(panel.scope(), crate::app::state::RewindScope::Detail);
}

#[test]
fn rewind_confirm_cancel_has_no_effect() {
    let (mut state, pane) = agents_editing_state();
    assert!(crate::app::update::on_detail_rewind_prepared(
        &mut state,
        pane,
        rewind_plan("sess-idle")
    ));
    let Some(SlashPanel::Rewind(mut panel)) = panel(&state, pane) else {
        panic!("expected rewind panel");
    };
    panel.window.active = panel.actions.len() - 1;
    state.effects.clear();
    assert!(super::super::detail_rewind::apply_confirm(
        &mut state, pane, &panel
    ));
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::ExecuteDetailRewind(_))),
        "Cancel 不执行：{:?}",
        state.effects
    );
    assert!(panel_after_confirm_closed(&state, pane));
}

#[test]
fn rewind_confirm_default_enter_emits_execute_effect() {
    let (mut state, pane) = agents_editing_state();
    assert!(crate::app::update::on_detail_rewind_prepared(
        &mut state,
        pane,
        rewind_plan("sess-idle")
    ));
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    let request = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::ExecuteDetailRewind(request) => Some(request),
            _ => None,
        })
        .expect("execute effect");
    assert_eq!(
        request.target,
        crate::app::state::DetailTarget::Agent("sub-idle".into())
    );
    assert_eq!(request.request.session_id, "sess-idle");
    assert_eq!(
        request.request.action,
        crate::app::state::RewindAction::RevertAndDelete
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .and_then(|page| page.detail_rewind_executing.as_ref())
            .map(|exec| exec.operation_id.as_str()),
        Some(request.operation_id.as_str())
    );
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .and_then(|page| page.detail_rewind_pending.as_ref())
            .is_some_and(|plan| plan.action == crate::app::state::RewindAction::RevertAndDelete),
        "回投记录实际选择"
    );
}

#[test]
fn rewind_confirm_disabled_file_action_is_swallowed() {
    let (mut state, pane) = agents_editing_state();
    let mut plan = rewind_plan("sess-idle");
    plan.snapshot = crate::app::state::SnapshotState::Unavailable;
    assert!(crate::app::update::on_detail_rewind_prepared(
        &mut state, pane, plan
    ));
    // 默认项含文件恢复：快照不可用时 Enter 被吞掉。
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::ExecuteDetailRewind(_))),
        "禁用动作不得执行：{:?}",
        state.effects
    );
    // ↓ 到 DeleteOnly 后仍可只删对话。
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Down);
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    let request = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::ExecuteDetailRewind(request) => Some(request),
            _ => None,
        })
        .expect("DeleteOnly 可执行");
    assert_eq!(
        request.request.action,
        crate::app::state::RewindAction::DeleteOnly
    );
}

#[test]
fn rewind_busy_agent_does_not_open_confirm() {
    let (mut state, pane) = agents_editing_state();
    // 已有执行中的撤销：再次 /rewind 只提示，不覆盖。
    if let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.detail_rewind_executing = Some(crate::app::state::DetailRewindExecution {
            operation_id: "op-1".into(),
            target: crate::app::state::DetailTarget::Agent("sub-idle".into()),
        });
    }
    assert!(super::super::detail_rewind::prepare(&mut state, pane));
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::PrepareDetailRewind { .. }
        )),
        "执行中不得重复预检"
    );
    assert!(panel(&state, pane).is_none());
}

fn panel_after_confirm_closed(state: &AppState, pane: PaneId) -> bool {
    panel(state, pane).is_none()
        && state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .is_some_and(|page| page.detail_rewind_pending.is_none())
}

#[test]
fn compact_command_emits_detail_effect_once() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/compact");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    let targets: Vec<_> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            crate::app::effects::Effect::CompactDetail { target, .. } => Some(target.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        targets,
        vec![crate::app::state::DetailTarget::Agent("sub-idle".into())]
    );
    assert_eq!(editor_text(&state, pane), "");
}

#[test]
fn detail_compact_finished_replaces_checkpoints_and_clears_running() {
    let (mut state, pane) = agents_editing_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target
            .lx
            .agents
            .by_agent_mut("sub-idle")
            .expect("run")
            .compact
            .start("op-1".into(), 0);
    }
    assert!(crate::app::update::on_detail_compact_finished(
        &mut state,
        pane,
        "sess-idle",
        "op-1",
        &crate::app::update::DetailCompactResult::Committed,
        Some(&[]),
    ));
    let run = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_agent("sub-idle")
        .expect("run");
    assert!(!run.compact.is_running());
    assert!(run.compact.checkpoints.is_empty());
}

#[test]
fn late_detail_compact_finished_is_ignored() {
    let (mut state, pane) = agents_editing_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target
            .lx
            .agents
            .by_agent_mut("sub-idle")
            .expect("run")
            .compact
            .start("current-op".into(), 0);
    }
    assert!(!crate::app::update::on_detail_compact_finished(
        &mut state,
        pane,
        "sess-idle",
        "stale-op",
        &crate::app::update::DetailCompactResult::Empty,
        None,
    ));
    let running = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_agent("sub-idle")
        .expect("run")
        .compact
        .running
        .clone();
    assert_eq!(running.as_deref(), Some("current-op"));
}

#[test]
fn rewind_command_and_hidden_alias_emit_prepare_effect() {
    for command in ["/rewind", "/undo"] {
        let (mut state, pane) = agents_editing_state();
        type_str(&mut state, command);
        state.effects.clear();
        crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
        assert!(
            state.effects.iter().any(|effect| matches!(
                effect,
                crate::app::effects::Effect::PrepareDetailRewind {
                    target: crate::app::state::DetailTarget::Agent(agent_id),
                    ..
                } if agent_id == "sub-idle"
            )),
            "{command} 应进入详情撤销预检：{:?}",
            state.effects
        );
        assert_eq!(editor_text(&state, pane), "", "{command} 已消费");
    }
}

#[test]
fn detail_compact_ticks_elapsed_and_breathing_frame() {
    let (mut state, pane) = agents_editing_state();
    {
        let run = &mut state.pane_mut_anywhere(pane).expect("pane").lx.agents.runs[0];
        run.compact
            .start("op-detail".into(), crate::db::now_ms() - 1_500);
        run.chat.loading_frame = 0;
        run.rows_dirty = false;
    }
    assert!(state.has_streaming_lx(), "详情压缩必须驱动心跳");
    assert!(crate::app::update::chat::advance_lx_loading(&mut state));
    let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
    assert!(
        run.compact
            .elapsed_ms
            .is_some_and(|elapsed| elapsed >= 1_000),
        "压缩耗时必须随心跳刷新：{:?}",
        run.compact.elapsed_ms
    );
    assert_eq!(run.chat.loading_frame, 1, "运行中检查点行必须呼吸");
}

#[test]
fn detail_btw_compact_ticks_elapsed_and_breathing_frame() {
    let (mut state, pane) = btw_editing_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let thread = target.lx.btw.active_thread_mut().expect("thread");
        thread.loaded = true;
        thread
            .compact
            .start("op-btw".into(), crate::db::now_ms() - 1_500);
        thread.chat.loading_frame = 0;
    }
    assert!(state.has_streaming_lx());
    assert!(crate::app::update::chat::advance_lx_loading(&mut state));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert!(
        thread
            .compact
            .elapsed_ms
            .is_some_and(|elapsed| elapsed >= 1_000)
    );
    assert_eq!(thread.chat.loading_frame, 1);
}
