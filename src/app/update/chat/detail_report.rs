//! `/report`：让 Agents / BTW 面板生成一份报告，正文只填入主输入框。
//!
//! - 无上一份报告：发起一次独立总结请求（与 `/compact` 同管线，不落消息）；
//! - 已有报告检查点：打开 `Resend / Regenerate / Cancel` 二级面板；
//! - 报告正文以折叠芯片落在主输入框顶部，同源原位替换。

use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::paste::FoldedKind;
use crate::app::state::{AppState, DetailTarget, ReportAction, ReportPanel, SlashPanel};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::show_lx_toast;
use crate::tiling::PaneId;
use crate::ui::text;

use super::report_fill::{fill_main_input, format_report_body};

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
    if note.len() > crate::db::chat::MAX_MESSAGE_BYTES {
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
        // 总结必须有自己的线程会话；新 QA 无历史时没有可总结的内容。
        let thread_id = thread.thread_id.clone()?;
        let busy = thread.is_loading() || thread.compact.is_running();
        let last_report = if busy { None } else { last_btw_report(thread) };
        Some((thread_id, busy, last_report))
    }) else {
        show_btw_unavailable(state, pane);
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
    if note.len() > crate::db::chat::MAX_MESSAGE_BYTES {
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
    if let Some(last) = last_report {
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

fn show_btw_unavailable(state: &mut AppState, pane: PaneId) {
    show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Warning,
            text::detail::LX_DETAIL_REPORT_UNAVAILABLE_BTW,
            Some(pane),
            Instant::now(),
        ),
    );
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

/// 最近一次 `/report` 报告的芯片正文（Agents）。
///
/// 只有报告本身就是时间线最后一条时才返回：它后面若还有消息、撤销提示或其它
/// 检查点，`/report` 应直接重新总结，而不是弹“重发上一份报告”的二级面板。
fn last_agent_report(run: &crate::app::state::SubagentRun) -> Option<String> {
    let checkpoint = run
        .compact
        .checkpoints
        .iter()
        .rev()
        .find(|checkpoint| checkpoint.report && checkpoint.active)?;
    let latest = run
        .chat
        .messages
        .iter()
        .map(|message| message.sequence)
        .chain(run.chat.notices.iter().map(|notice| notice.sequence))
        .chain(
            run.compact
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.sequence),
        )
        .max()
        .unwrap_or(checkpoint.sequence);
    if checkpoint.sequence < latest {
        return None;
    }
    Some(format_report_body(
        &agent_report_header(run),
        &checkpoint.summary,
        &[],
    ))
}

/// 来源头：`[Subagent report] 名字 · 角色 [· 标题]`。
fn agent_report_header(run: &crate::app::state::SubagentRun) -> String {
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
    header
}

/// 最近一次 `/report` 报告的芯片正文（BTW）；同样只认时间线最后一条。
fn last_btw_report(thread: &crate::app::state::lx::BtwThreadState) -> Option<String> {
    let checkpoint = thread
        .compact
        .checkpoints
        .iter()
        .rev()
        .find(|checkpoint| checkpoint.report && checkpoint.active)?;
    let latest = thread
        .chat
        .messages
        .iter()
        .map(|message| message.sequence)
        .chain(thread.chat.notices.iter().map(|notice| notice.sequence))
        .chain(
            thread
                .compact
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.sequence),
        )
        .max()
        .unwrap_or(checkpoint.sequence);
    if checkpoint.sequence < latest {
        return None;
    }
    Some(format_report_body(
        &format!("[BTW report] {}", thread.title.trim()),
        &checkpoint.summary,
        &[],
    ))
}

/// 一次性总结提交成功：把最新报告检查点写成主输入框芯片。
///
/// 目标（Agents / BTW）在提交期间可能已被切走；找不到目标时只 Toast，不写错会话。
pub fn finish_generated(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    checkpoints: &[crate::db::chat::CompactCheckpoint],
    changed_paths: Vec<String>,
) -> bool {
    let Some(checkpoint) = checkpoints
        .iter()
        .rev()
        .find(|checkpoint| checkpoint.kind.is_report() && checkpoint.is_active())
    else {
        show_unavailable(state, pane);
        return false;
    };
    let Some((parent_session, kind, source, header)) =
        state.pane_anywhere(pane).and_then(|target| {
            let parent_session = target.lx.chat.session_id.clone()?;
            if let Some(run) = target.lx.agents.by_session(session_id) {
                return Some((
                    parent_session,
                    FoldedKind::SubagentReport,
                    run.agent_id.clone(),
                    agent_report_header(run),
                ));
            }
            let thread = target.lx.btw.thread_by_child_session(session_id)?;
            Some((
                parent_session,
                FoldedKind::BtwReport,
                thread.thread_id.clone().unwrap_or_default(),
                format!("[BTW report] {}", thread.title.trim()),
            ))
        })
    else {
        show_unavailable(state, pane);
        return false;
    };
    crate::app::update::on_generated_report_ready(
        state,
        crate::app::update::GeneratedReportReady {
            pane,
            parent_session,
            kind,
            source,
            header,
            body: checkpoint.summary_text.clone(),
            changed_paths,
        },
    )
}
