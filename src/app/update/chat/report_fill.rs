//! `/report` 报告落位：来源头组装、主输入框折叠芯片与报告轮回执。

use std::time::Instant;

use crate::app::paste::FoldedKind;
use crate::app::state::{AppState, ChatMessageView};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::show_lx_toast;
use crate::chat::agent::AgentRole;
use crate::tiling::PaneId;
use crate::ui::text;

/// 报告正文里最多列出的变更文件数。
const MAX_REPORT_CHANGED_PATHS: usize = 8;

/// 事件层回投：Agents 面板 `/report` 轮完成。
#[derive(Debug, Clone)]
pub struct SubagentReportReady {
    pub pane: PaneId,
    pub parent_session: String,
    pub agent_id: String,
    pub nickname: Option<String>,
    pub title: String,
    pub role: AgentRole,
    pub body: String,
    pub changed_paths: Vec<String>,
}

impl SubagentReportReady {
    /// 带来源头的完整报告正文：名字 · 角色 [· 标题] + 正文 + 变更文件摘要。
    fn formatted_body(&self) -> String {
        let name = self
            .nickname
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(self.agent_id.as_str());
        let mut header = format!("[Subagent report] {name} · {}", self.role.as_str());
        if !self.title.trim().is_empty() {
            header.push_str(" · ");
            header.push_str(self.title.trim());
        }
        let mut out = format!("{header}\n{}", self.body.trim_end());
        if !self.changed_paths.is_empty() {
            let shown = self
                .changed_paths
                .iter()
                .take(MAX_REPORT_CHANGED_PATHS)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str("\nChanged files: ");
            out.push_str(&shown);
            if self.changed_paths.len() > MAX_REPORT_CHANGED_PATHS {
                out.push_str(&format!(
                    " (+{} more)",
                    self.changed_paths.len() - MAX_REPORT_CHANGED_PATHS
                ));
            }
        }
        out
    }
}

/// 报告轮完成：把正文填进主输入框（只填不发）。
pub fn on_subagent_report_ready(state: &mut AppState, ready: SubagentReportReady) -> bool {
    let body = ready.formatted_body();
    if body.trim().is_empty() {
        show_lx_toast(
            state,
            ready.pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_REPORT_UNAVAILABLE,
                Some(ready.pane),
                Instant::now(),
            ),
        );
        return false;
    }
    if !fill_main_input(
        state,
        ready.pane,
        &ready.parent_session,
        FoldedKind::SubagentReport,
        &ready.agent_id,
        &body,
    ) {
        tracing::debug!(
            agent_id = %ready.agent_id,
            pane = ready.pane.raw(),
            "subagent report dropped: pane session changed"
        );
        return false;
    }
    show_lx_toast(
        state,
        ready.pane,
        Toast::new(
            ToastKind::Default,
            text::detail::LX_REPORT_FILLED,
            Some(ready.pane),
            Instant::now(),
        ),
    );
    true
}

/// BTW `/report` 合并报告：成功段 + 失败占位。
#[derive(Debug, Clone)]
pub struct BtwReportReady {
    pub pane: PaneId,
    pub parent_session: String,
    pub thread_id: String,
    pub title: String,
    pub sections: Vec<(String, String)>,
    pub failed: Vec<String>,
}

impl BtwReportReady {
    fn formatted_body(&self) -> Option<String> {
        let mut out = format!("[BTW report] {}", self.title.trim());
        let mut any_ok = false;
        for (label, body) in &self.sections {
            let body = body.trim();
            if body.is_empty() {
                continue;
            }
            any_ok = true;
            out.push_str(&format!("\n[{label}] {body}"));
        }
        for label in &self.failed {
            out.push_str(&format!("\n[{label}] report unavailable (failed)"));
        }
        any_ok.then_some(out)
    }
}

/// BTW 报告全部收口：填主输入框；全失败只 Toast。
pub fn on_btw_report_ready(state: &mut AppState, ready: BtwReportReady) -> bool {
    let Some(body) = ready.formatted_body() else {
        show_lx_toast(
            state,
            ready.pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_BTW_REPORT_UNAVAILABLE,
                Some(ready.pane),
                Instant::now(),
            ),
        );
        return false;
    };
    if !fill_main_input(
        state,
        ready.pane,
        &ready.parent_session,
        FoldedKind::BtwReport,
        &ready.thread_id,
        &body,
    ) {
        tracing::debug!(
            thread_id = %ready.thread_id,
            pane = ready.pane.raw(),
            "btw report dropped: pane session changed"
        );
        return false;
    }
    show_lx_toast(
        state,
        ready.pane,
        Toast::new(
            ToastKind::Default,
            text::detail::LX_REPORT_FILLED,
            Some(ready.pane),
            Instant::now(),
        ),
    );
    true
}

/// 把报告折叠芯片写到主输入框：同源原位替换，否则插到顶部并与草稿空一行。
pub fn fill_main_input(
    state: &mut AppState,
    pane: PaneId,
    parent_session: &str,
    kind: FoldedKind,
    source: &str,
    body: &str,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() != Some(parent_session) {
        return false;
    }
    target.lx.insert_report_chip(kind, source, body)
}

/// 从面板最后一条 assistant 报告里取正文（用于二级面板的 `Resend`）。
pub(crate) fn last_report_body(messages: &[ChatMessageView]) -> Option<String> {
    let assistant = messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")?;
    let body = assistant
        .parts()
        .filter(|part| part.kind == crate::chat::events::PartKind::Text)
        .map(|part| part.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!body.is_empty()).then_some(body)
}
