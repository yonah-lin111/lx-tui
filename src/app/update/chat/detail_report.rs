//! `/report`：让 Agents / BTW 面板生成一份报告，正文只填入主输入框。
//!
//! - 无上一轮报告：发起一轮专门的报告模型调用（不自动投递父轮）；
//! - 最后一轮已是报告：打开 `Resend / Regenerate / Cancel` 二级面板；
//! - 报告正文以折叠芯片落在主输入框顶部，同源原位替换。

use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::paste::FoldedKind;
use crate::app::state::{AppState, DetailTarget, ReportAction, ReportPanel, SlashPanel};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::show_lx_toast;
use crate::tiling::PaneId;
use crate::ui::text;

use super::report_fill::{fill_main_input, last_report_body};

/// 提交 `/report [note]`。
pub(crate) fn submit(state: &mut AppState, pane: PaneId, note: &str) -> bool {
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.agents_editing())
    {
        return submit_agent(state, pane, note);
    }
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.btw_editing())
    {
        return submit_btw(state, pane, note);
    }
    show_unavailable(state, pane);
    true
}

fn submit_agent(state: &mut AppState, pane: PaneId, note: &str) -> bool {
    let Some((agent_id, session_id, busy, last_report)) =
        state.pane_anywhere(pane).and_then(|target| {
            let agent_id = target.lx.agent_edit_id()?.to_string();
            let run = target.lx.agents.by_agent(&agent_id)?;
            let busy = run.is_running()
                || run.queued_count > 0
                || !run.chat.pending_steers.is_empty()
                || run.compact.is_running();
            let last_report = if busy { None } else { last_agent_report(run) };
            Some((
                agent_id,
                target.lx.chat.session_id.clone(),
                busy,
                last_report,
            ))
        })
    else {
        show_unavailable(state, pane);
        return true;
    };

    if busy {
        super::composer::clear(state, pane);
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_DETAIL_REPORT_BUSY,
                Some(pane),
                Instant::now(),
            ),
        );
        return true;
    }
    if let Some(last) = last_report {
        return open_panel(state, pane, DetailTarget::Agent(agent_id), note, Some(last));
    }
    let Some(session_id) = session_id else {
        show_unavailable(state, pane);
        return true;
    };
    let message = crate::chat::agent::report_request_message(note);
    if message.len() > crate::db::chat::MAX_MESSAGE_BYTES {
        super::composer::clear(state, pane);
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_REPORT_NOTE_LIMIT,
                Some(pane),
                Instant::now(),
            ),
        );
        return true;
    }
    super::composer::clear(state, pane);
    state.effects.push(Effect::SendSubagentReport {
        pane,
        session_id,
        agent_id,
        note: note.to_string(),
    });
    true
}

fn submit_btw(state: &mut AppState, pane: PaneId, note: &str) -> bool {
    let Some((thread_id, busy, last_report)) = state.pane_anywhere(pane).and_then(|target| {
        let thread = target.lx.btw.active_thread()?;
        // 尚未提问过的 QA 也能直接 `/report`：报告在 BTW 线程里就地创建。
        let thread_id = thread.thread_id.clone();
        let busy = thread.is_loading() || thread.compact.is_running();
        let last_report = if busy { None } else { last_btw_report(thread) };
        Some((thread_id, busy, last_report))
    }) else {
        show_unavailable(state, pane);
        return true;
    };
    if busy {
        super::composer::clear(state, pane);
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_DETAIL_REPORT_BUSY,
                Some(pane),
                Instant::now(),
            ),
        );
        return true;
    }
    if let Some(last) = last_report
        && let Some(thread_id) = thread_id
    {
        return open_panel(state, pane, DetailTarget::Btw(thread_id), note, Some(last));
    }
    super::composer::clear(state, pane);
    super::btw::submit_report(state, pane, note);
    true
}

/// 打开二级面板：在激活的详情 composer 上放 `ReportPanel`。
fn open_panel(
    state: &mut AppState,
    pane: PaneId,
    target: DetailTarget,
    note: &str,
    last_report: Option<String>,
) -> bool {
    let Some(target_state) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let panel = ReportPanel::new(target, note.to_string(), last_report);
    target_state.lx.with_active_editor(|editor| {
        editor.slash = Some(SlashPanel::Report(Box::new(panel)));
    });
    true
}

/// 面板 Enter：重发 / 重新生成 / 取消。
pub(crate) fn apply_confirm(state: &mut AppState, pane: PaneId, panel: &ReportPanel) -> bool {
    let active = state
        .pane_anywhere(pane)
        .is_some_and(|target| panel.target.is_active(&target.lx));
    if !active {
        close_panel(state, pane);
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_REPORT_TARGET_CHANGED,
                Some(pane),
                Instant::now(),
            ),
        );
        return true;
    }

    match panel.active_action() {
        ReportAction::Cancel => {
            close_panel(state, pane);
            true
        }
        ReportAction::Resend => {
            let Some(body) = panel.last_report.clone() else {
                return true;
            };
            let Some(parent_session) = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.session_id.clone())
            else {
                return true;
            };
            let Some((kind, source)) = target_chip(&panel.target) else {
                return true;
            };
            fill_main_input(state, pane, &parent_session, kind, source, &body);
            close_panel(state, pane);
            true
        }
        ReportAction::Regenerate => {
            let note = panel.note.clone();
            match &panel.target {
                DetailTarget::Agent(agent_id) => {
                    let agent_id = agent_id.clone();
                    close_panel(state, pane);
                    let Some(session_id) = state
                        .pane_anywhere(pane)
                        .and_then(|target| target.lx.chat.session_id.clone())
                    else {
                        return true;
                    };
                    state.effects.push(Effect::SendSubagentReport {
                        pane,
                        session_id,
                        agent_id,
                        note,
                    });
                }
                DetailTarget::Btw(_) => {
                    close_panel(state, pane);
                    super::btw::submit_report(state, pane, &note);
                }
            }
            true
        }
    }
}

fn close_panel(state: &mut AppState, pane: PaneId) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.with_active_editor(|editor| editor.slash = None);
    }
}

fn target_chip(target: &DetailTarget) -> Option<(FoldedKind, &str)> {
    match target {
        DetailTarget::Agent(agent_id) => Some((FoldedKind::SubagentReport, agent_id.as_str())),
        DetailTarget::Btw(thread_id) => Some((FoldedKind::BtwReport, thread_id.as_str())),
    }
}

fn show_unavailable(state: &mut AppState, pane: PaneId) {
    show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Warning,
            text::detail::LX_DETAIL_REPORT_UNAVAILABLE,
            Some(pane),
            Instant::now(),
        ),
    );
}

/// 最后一轮是 `/report` 时返回带来源头的报告正文（Agents）。
fn last_agent_report(run: &crate::app::state::SubagentRun) -> Option<String> {
    let messages = &run.chat.messages;
    let assistant_index = messages
        .iter()
        .rposition(|message| message.role == "assistant")?;
    let user_index = messages[..assistant_index]
        .iter()
        .rposition(|message| message.role == "user")?;
    if !crate::chat::agent::is_report_request(&messages[user_index].text()) {
        return None;
    }
    let body = last_report_body(&messages[assistant_index..])?;
    let name = run
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(run.agent_id.as_str());
    let mut header = format!("[Subagent report] {name} · {}", run.role.as_str());
    if !run.title.trim().is_empty() {
        header.push_str(" · ");
        header.push_str(run.title.trim());
    }
    Some(format!("{header}\n{body}"))
}

/// 最后一轮是 `/report` 时返回合并正文（BTW）。
fn last_btw_report(thread: &crate::app::state::lx::BtwThreadState) -> Option<String> {
    let messages = &thread.chat.messages;
    let user_index = messages
        .iter()
        .rposition(|message| message.role == "user")?;
    if !crate::chat::agent::is_report_request(&messages[user_index].text()) {
        return None;
    }
    let bodies: Vec<String> = messages[user_index + 1..]
        .iter()
        .filter(|message| message.role == "assistant")
        .map(|message| message.text().trim().to_string())
        .filter(|body| !body.is_empty())
        .collect();
    if bodies.is_empty() {
        return None;
    }
    Some(format!(
        "[BTW report] {}\n{}",
        thread.title.trim(),
        bodies.join("\n\n")
    ))
}
