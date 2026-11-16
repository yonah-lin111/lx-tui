//! 停止面板与双 Esc 待命测试。

use super::super::stop as stop_panel;
use super::*;
use crate::app::state::{
    QueuedSubmission, SlashPanel, StopAction, StopAvailability, StopPanel, SubagentRun,
};
use crate::app::update::pane::apply_lx_input;
use crate::chat::agent::AgentRole;

/// 最小 subagent run：running + 指定排队数。
fn agent_run(agent_id: &str, running: bool, queued: usize) -> SubagentRun {
    let mut run = SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: agent_id.into(),
        nickname: Some(agent_id.into()),
        agent_path: format!("/root/{agent_id}"),
        child_session_id: format!("child-{agent_id}"),
        parent_session_id: "sess-stop".into(),
        spawn_message_id: None,
        role: AgentRole::Executor,
        model_provider: "p".into(),
        model_id: "m".into(),
        model_variant: None,
        title: "task".into(),
        status: if running { "running" } else { "idle" }.into(),
        created_at_ms: 1,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    });
    run.queued_count = queued;
    run
}

fn open_stop_panel(state: &mut AppState, pane: PaneId, availability: StopAvailability) {
    state.pane_mut_anywhere(pane).unwrap().lx.slash =
        Some(SlashPanel::Stop(Box::new(StopPanel::new(availability))));
}

#[test]
fn first_escape_arms_and_second_opens_panel_without_stopping() {
    let (mut state, pane, _key) = streaming_state("sess-stop-panel");
    state.effects.clear();
    assert!(escape_stop(&mut state, pane), "首次 Esc 待命");
    assert!(state.effects.is_empty(), "首次 Esc 不停止");
    assert!(escape_stop(&mut state, pane), "二次 Esc 打开面板");
    let target = state.pane_anywhere(pane).unwrap();
    assert!(
        matches!(
            target.lx.slash,
            Some(SlashPanel::Stop(ref panel)) if panel.active_action() == StopAction::All
        ),
        "第二次 Esc 只打开面板，默认 Stop all"
    );
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::CancelChatStream { .. })),
        "打开面板本身不得发停止效应"
    );
}

#[test]
fn escape_stays_idle_when_nothing_to_stop() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(!escape_stop(&mut state, pane), "空闲不进入停止待命");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .stop_armed_at
            .is_none()
    );
}

#[test]
fn subagent_only_opens_stop_panel_and_disables_main() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .agents
        .runs
        .push(agent_run("curie", true, 0));
    assert!(escape_stop(&mut state, pane));
    assert!(escape_stop(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    let Some(SlashPanel::Stop(panel)) = target.lx.slash.as_ref() else {
        panic!("应打开停止面板");
    };
    assert!(!panel.enabled(StopAction::Main));
    assert!(panel.enabled(StopAction::Subagents));
}

#[test]
fn queued_idle_agent_counts_as_stoppable() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .agents
        .runs
        .push(agent_run("curie", false, 2));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.agents.stoppable_count(), 1);
    assert!(StopAvailability::from_lx(&target.lx).any());
}

#[test]
fn stop_all_emits_main_subagent_and_shell_effects() {
    let (mut state, pane, _key) = streaming_state("sess-stop-all");
    {
        let target = state.pane_mut_anywhere(pane).unwrap();
        target.lx.agents.runs.push(agent_run("curie", true, 0));
        target
            .lx
            .chat
            .pending_shells
            .push(crate::app::state::PendingShell {
                generation: 1,
                revision: 0,
                command: "sleep 1".into(),
                session: Some("sess-stop-all".into()),
            });
    }
    open_stop_panel(
        &mut state,
        pane,
        StopAvailability {
            main: true,
            subagents: 1,
            shells: 1,
            processes: 0,
        },
    );
    state.effects.clear();
    assert!(stop_panel::apply_enter(&mut state, pane));
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::CancelChatStream { .. }))
    );
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::StopSubagentsByUser { agent_id: None, .. }
    )));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::CancelUserShells {
            scope: crate::app::effects::UserShellStopScope::All,
            ..
        }
    )));
    assert!(
        state.pane_anywhere(pane).unwrap().lx.slash.is_none(),
        "执行后关闭面板"
    );
}

#[test]
fn stop_shells_keeps_chat_queue() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    {
        let target = state.pane_mut_anywhere(pane).unwrap();
        target.lx.chat.queue.push(QueuedSubmission::Chat {
            draft: "keep".into(),
        });
        target.lx.chat.queue.push(QueuedSubmission::Shell {
            full_text: "drop".into(),
            cwd: "/tmp".into(),
        });
        target
            .lx
            .chat
            .pending_shells
            .push(crate::app::state::PendingShell {
                generation: 1,
                revision: 0,
                command: "drop".into(),
                session: None,
            });
    }
    open_stop_panel(
        &mut state,
        pane,
        StopAvailability {
            shells: 1,
            ..StopAvailability::default()
        },
    );
    if let Some(SlashPanel::Stop(panel)) = state.pane_mut_anywhere(pane).unwrap().lx.slash.as_mut()
    {
        panel.window.active = 3;
    }
    state.effects.clear();
    assert!(stop_panel::apply_enter(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.queue.len(), 1);
    assert!(matches!(
        target.lx.chat.queue.first(),
        Some(QueuedSubmission::Chat { .. })
    ));
    assert!(target.lx.chat.pending_shells.is_empty());
}

#[test]
fn stop_main_only_stops_response() {
    let (mut state, pane, _key) = streaming_state("sess-stop-main");
    open_stop_panel(
        &mut state,
        pane,
        StopAvailability {
            main: true,
            ..StopAvailability::default()
        },
    );
    if let Some(SlashPanel::Stop(panel)) = state.pane_mut_anywhere(pane).unwrap().lx.slash.as_mut()
    {
        panel.window.active = 1;
    }
    state.effects.clear();
    assert!(stop_panel::apply_enter(&mut state, pane));
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::CancelChatStream { .. }))
    );
    assert!(!state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::StopSubagentsByUser { .. }
    )));
}

#[test]
fn expired_panel_target_reports_nothing_to_stop() {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    open_stop_panel(
        &mut state,
        pane,
        StopAvailability {
            main: true,
            ..StopAvailability::default()
        },
    );
    state.effects.clear();
    assert!(stop_panel::apply_enter(&mut state, pane));
    assert!(state.effects.is_empty(), "目标已消失不产生停止效应");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .toast
            .as_ref()
            .is_some_and(|toast| toast.message == text::LX_NOTHING_TO_STOP)
    );
}

#[test]
fn stop_panel_escape_closes_without_clearing_draft() {
    let mut state = ready_state("keep me");
    let pane = focus_pane(&state);
    open_stop_panel(
        &mut state,
        pane,
        StopAvailability {
            main: true,
            ..StopAvailability::default()
        },
    );
    apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.slash.is_none());
    assert_eq!(target.lx.input, "keep me");
}
