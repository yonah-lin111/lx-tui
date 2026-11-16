//! 双击 Esc 停止面板的执行：动作分派、作用域校验与数量反馈。

use std::time::Instant;

use crate::app::effects::{Effect, UserShellStopScope};
use crate::app::state::{AppState, SlashPanel, StopAction, StopAvailability};
use crate::app::toast::{Toast, ToastKind};
use crate::tiling::PaneId;
use crate::ui::text;

/// 执行停止面板当前高亮动作；面板未打开或动作失效时只关闭面板。
pub fn apply_enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(panel) = state.pane_anywhere(pane).and_then(|target| {
        match target.lx.slash.as_ref() {
            Some(SlashPanel::Stop(panel)) => Some((**panel).clone()),
            _ => None,
        }
    }) else {
        return false;
    };

    // 面板可能停留数秒：执行前按当前状态重算可用性。
    let availability = state
        .pane_anywhere(pane)
        .map_or_else(StopAvailability::default, |target| {
            StopAvailability::from_lx(&target.lx)
        });
    let action = panel.active_action();

    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
    }

    if !availability.enabled(action) {
        crate::app::update::show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::LX_NOTHING_TO_STOP,
                Some(pane),
                Instant::now(),
            ),
        );
        return true;
    }

    match action {
        StopAction::Cancel => {}
        StopAction::All => {
            // 先停主轮，避免继续派生；再停 subagent；最后停 shell/进程。
            if availability.main {
                super::stop_stream(state, pane);
            }
            push_stop_all_subagents_by_user(state, pane);
            stop_shells(state, pane, UserShellStopScope::All);
        }
        StopAction::Main => {
            super::stop_stream(state, pane);
        }
        StopAction::Subagents => {
            push_stop_all_subagents_by_user(state, pane);
        }
        StopAction::Shells => {
            stop_shells(state, pane, UserShellStopScope::ShellsOnly);
        }
    }
    true
}

/// 入队“停止当前会话树全部 subagent”效应（真正的取消在事件层）。
fn push_stop_all_subagents_by_user(state: &mut AppState, pane: PaneId) {
    let Some(session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return;
    };
    state.effects.push(Effect::StopSubagentsByUser {
        session_id,
        agent_id: None,
    });
}

/// 停止用户 shell（按范围）与受管后台进程。
fn stop_shells(state: &mut AppState, pane: PaneId, scope: UserShellStopScope) {
    let processes = state
        .pane_anywhere(pane)
        .map_or(0, |target| target.lx.background_tasks);
    super::user_shell::cancel_user_shells_with_scope(state, pane, scope);
    if processes > 0 {
        state.effects.push(Effect::StopAllSessionProcesses { pane });
    } else if scope == UserShellStopScope::ShellsOnly {
        let has_shells = state
            .pane_anywhere(pane)
            .is_some_and(|target| target.lx.chat.has_running_shell());
        if !has_shells {
            crate::app::update::show_lx_toast(
                state,
                pane,
                Toast::new(
                    ToastKind::Warning,
                    text::LX_NOTHING_TO_STOP,
                    Some(pane),
                    Instant::now(),
                ),
            );
        }
    }
}
