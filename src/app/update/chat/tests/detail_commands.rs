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

fn push_text_message(run: &mut SubagentRun, role: &str, text: &str, sequence: i64) {
    run.chat.messages.push(crate::app::state::ChatMessageView {
        id: format!("m{sequence}"),
        sequence,
        kind: "chat".into(),
        role: role.into(),
        status: "complete".into(),
        turn_index: 0,
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
    });
}

fn seed_last_report(state: &mut AppState, pane: PaneId, agent_id: &str) {
    let run = state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_agent_mut(agent_id)
        .expect("run");
    push_text_message(run, "user", crate::chat::agent::PANEL_REPORT_MARKER, 0);
    push_text_message(run, "assistant", "previous report body", 1);
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
        vec!["/rewind", "/compact", "/report", "/new"]
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
    assert_eq!(
        editor_text(&state, pane),
        "/nope",
        "未知命令保留草稿，便于继续编辑"
    );
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentReport { .. }
                | crate::app::effects::Effect::SendSubagentUserInput { .. }
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
fn report_idle_agent_emits_fill_input_effect() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/report check the log");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    let note = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::SendSubagentReport { agent_id, note, .. }
                if agent_id == "sub-idle" =>
            {
                Some(note.clone())
            }
            _ => None,
        })
        .expect("报告轮入队");
    assert_eq!(note, "check the log");
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput { .. }
        )),
        "报告不再走自动投递父轮链路"
    );
    assert_eq!(editor_text(&state, pane), "", "命令文本已消费");
    assert!(panel(&state, pane).is_none());
}

#[test]
fn report_with_previous_opens_resend_panel_and_resend_fills_input() {
    let (mut state, pane) = agents_editing_state();
    seed_last_report(&mut state, pane, "sub-idle");
    type_str(&mut state, "/report focus");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    let Some(SlashPanel::Report(report_panel)) = panel(&state, pane) else {
        panic!("expected report panel");
    };
    assert_eq!(
        report_panel.active_action(),
        crate::app::state::ReportAction::Resend
    );
    assert!(
        report_panel
            .last_report
            .as_deref()
            .is_some_and(|body| body.contains("previous report body"))
    );

    // Enter 直接重发：不产生模型效应，只填主输入框。
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    assert!(panel(&state, pane).is_none(), "执行后关闭面板");
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentReport { .. }
        )),
        "Resend 不调用模型"
    );
    let input = state.pane_anywhere(pane).unwrap().lx.input.clone();
    let tokens = crate::app::paste::folded_tokens(&input);
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        crate::app::paste::FoldedKind::SubagentReport
    );
    assert!(
        tokens[0].body.contains("previous report body"),
        "芯片正文包含上一轮报告"
    );
}

#[test]
fn report_regenerate_uses_note_and_emits_effect() {
    let (mut state, pane) = agents_editing_state();
    seed_last_report(&mut state, pane, "sub-idle");
    type_str(&mut state, "/report new focus");
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    let mut found = false;
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .with_active_editor(|editor| {
            if let Some(SlashPanel::Report(panel)) = editor.slash.as_mut() {
                panel.window.active = 1;
                found = true;
            }
        });
    assert!(found, "expected report panel");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::SendSubagentReport { agent_id, note, .. }
            if agent_id == "sub-idle" && note == "new focus"
    )));
}

#[test]
fn btw_report_submits_flag_with_report_marker() {
    let (mut state, pane) = btw_editing_state();
    {
        let thread = state
            .pane_mut_anywhere(pane)
            .unwrap()
            .lx
            .btw
            .active_thread_mut()
            .expect("thread");
        thread.thread_id = Some("thread-1".into());
        thread.child_session_id = Some("child-1".into());
        thread.title = "Side thread".into();
    }
    type_str(&mut state, "/report summarize");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    let (flag, text) = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::Btw(crate::app::effects::BtwEffect::Submit(
                submission,
            )) => Some((submission.report_to_main_input, submission.text.clone())),
            _ => None,
        })
        .expect("btw report submission");
    assert!(flag);
    assert!(text.starts_with(crate::chat::agent::PANEL_REPORT_MARKER));
    assert!(text.contains("User note: summarize"));
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
            crate::app::effects::Effect::SendSubagentReport { .. }
                | crate::app::effects::Effect::SendSubagentUserInput { .. }
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
    assert!(message.starts_with(crate::chat::agent::PANEL_REPORT_MARKER));
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
            .and_then(|target| target.lx.detail_rewind.executing.as_ref())
            .map(|exec| exec.operation_id.as_str()),
        Some(request.operation_id.as_str())
    );
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.detail_rewind.pending.as_ref())
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
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.detail_rewind.executing = Some(crate::app::state::DetailRewindExecution {
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
            .is_some_and(|target| target.lx.detail_rewind.pending.is_none())
}

#[test]
fn moving_cursor_refreshes_slash_candidates() {
    let (mut state, pane) = agents_editing_state();
    type_str(&mut state, "/co");
    assert_eq!(file_entries(&state, pane), vec!["/compact"]);
    // 光标移到行首：不在命令位置，候选必须关闭。
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Home);
    assert!(panel(&state, pane).is_none(), "离开命令位置必须关面板");
    // 回到行尾：命令重新成立，候选回来。
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::End);
    assert_eq!(file_entries(&state, pane), vec!["/compact"]);
}

#[test]
fn multiline_draft_does_not_execute_last_line_command() {
    let (mut state, pane) = btw_editing_state();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_composer_mut()
        .expect("composer")
        .insert_str("keep this\n/new");
    crate::app::update::chat::refresh_slash_panel(&mut state, pane);
    assert!(panel(&state, pane).is_some(), "末行命令照常出面板");
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, LxInput::Enter);
    assert_eq!(
        editor_text(&state, pane),
        "keep this\n/new",
        "命令夹在多行草稿里不得执行，也不得清空整框"
    );
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::Btw(crate::app::effects::BtwEffect::DeleteThread { .. })
        )),
        "不得删除线程"
    );
}

#[test]
fn btw_rewind_receipt_targets_the_planned_thread_only() {
    use crate::app::state::{DetailRewindPlan, DetailTarget, RewindAction, RewindTargetKind};

    let (mut state, pane) = btw_editing_state();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.input = "main draft".into();
    target.lx.btw.ensure_thread("qa-2");
    {
        let first = target.lx.btw.thread_mut("qa-1").expect("qa-1");
        first.thread_id = Some("t1".into());
        first.child_session_id = Some("s1".into());
        first.loaded = true;
        let second = target.lx.btw.thread_mut("qa-2").expect("qa-2");
        second.thread_id = Some("t2".into());
        second.child_session_id = Some("s2".into());
        second.loaded = true;
    }
    target.lx.btw.active = Some("qa-2".into());
    let plan = DetailRewindPlan {
        target: DetailTarget::Btw("t1".into()),
        workspace: std::path::PathBuf::new(),
        child_session: "s1".into(),
        user_message_id: "u1".into(),
        expected_revision: 0,
        input_at_open: String::new(),
        user_text: "hello".into(),
        action: RewindAction::RevertAndDelete,
        snapshot: crate::app::state::SnapshotState::KnownEmpty,
        kind: RewindTargetKind::Turn,
    };
    state.effects.clear();
    assert!(crate::app::update::on_detail_rewind_succeeded(
        &mut state,
        pane,
        &plan,
        &[],
        &[],
        true,
        0,
        false,
        "hello",
    ));
    let target = state.pane_mut_anywhere(pane).expect("pane");
    let first = target.lx.btw.thread_mut("qa-1").expect("qa-1");
    assert!(first.thread_id.is_none(), "目标线程已重置");
    let second = target.lx.btw.thread_mut("qa-2").expect("qa-2");
    assert_eq!(
        second.thread_id.as_deref(),
        Some("t2"),
        "无关线程不能被清空"
    );
    assert_eq!(target.lx.input, "main draft", "草稿不得回填进主输入框");
    assert_eq!(
        target
            .lx
            .btw
            .thread_mut("qa-1")
            .expect("qa-1")
            .composer
            .text(),
        "hello",
        "原文回填到目标线程自己的 composer"
    );
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::Btw(crate::app::effects::BtwEffect::LoadQaList { .. })
        )),
        "删空线程后必须刷新 QA 列表标记"
    );
}

#[test]
fn detail_compact_finished_without_readback_clears_running() {
    let (mut state, pane) = agents_editing_state();
    {
        let run = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .by_agent_mut("sub-idle")
            .expect("run");
        assert!(run.compact.start("op-readback".into(), 0));
        run.rows_dirty = false;
    }
    assert!(crate::app::update::on_detail_compact_finished(
        &mut state,
        pane,
        "sess-idle",
        "op-readback",
        &crate::app::update::chat::DetailCompactResult::Committed,
        None,
    ));
    let run = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_agent("sub-idle")
        .expect("run");
    assert!(!run.compact.is_running(), "读回失败也必须结束 compacting");
    assert!(run.rows_dirty, "运行中检查点行消失后必须重量");
}

#[test]
fn btw_search_paste_loads_the_switched_thread() {
    let (mut state, pane) = btw_editing_state();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.editing = false;
        target.lx.btw.qa = vec![
            crate::app::state::lx::btw::BtwQaSummary {
                qa_id: "qa-1".into(),
                title: "alpha".into(),
                has_thread: true,
                status: "idle".into(),
            },
            crate::app::state::lx::btw::BtwQaSummary {
                qa_id: "qa-2".into(),
                title: "beta".into(),
                has_thread: true,
                status: "idle".into(),
            },
        ];
        target.lx.btw.active = Some("qa-1".into());
    }
    state.effects.clear();
    assert!(crate::app::update::apply_task_paste(
        &mut state, pane, "beta"
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.btw.active.as_deref(),
        Some("qa-2"),
        "过滤后高亮切到唯一匹配项"
    );
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::Btw(crate::app::effects::BtwEffect::LoadThread { qa_id, .. })
                if qa_id == "qa-2"
        )),
        "粘贴触发的切换也必须加载线程：{:?}",
        state.effects
    );
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

#[test]
fn btw_streaming_thread_drives_loading_frame() {
    let (mut state, pane) = btw_editing_state();
    {
        let thread = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread_mut()
            .expect("thread");
        thread.loaded = true;
        thread
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                sequence: 0,
                id: "a1".into(),
                role: "assistant".into(),
                status: "streaming".into(),
                turn_index: 0,
                kind: "chat".into(),
                shell: None,
                steps: Vec::new(),
                model_label: None,
                error_code: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                duration_ms: None,
                reports: Vec::new(),
                attachments: Vec::new(),
            });
        thread.chat.loading_frame = 0;
    }
    assert!(state.has_streaming_lx(), "BTW 在途回答必须驱动心跳");
    let before = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread")
        .chat
        .loading_frame;
    assert!(crate::app::update::chat::advance_lx_loading(&mut state));
    let after = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread")
        .chat
        .loading_frame;
    assert_eq!(after, before + 1, "BTW 线程帧必须随心跳推进");
}
