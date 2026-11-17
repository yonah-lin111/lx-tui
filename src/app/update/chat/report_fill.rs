//! `/report` 报告落位：来源头组装、主输入框折叠芯片。
//!
//! 报告由事件层一次性总结生成（与 `/compact` 共用检查点时间线），本模块只负责
//! 把最终正文与来源头写成主输入框的折叠芯片。

use std::time::Instant;

use crate::app::paste::FoldedKind;
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::show_lx_toast;
use crate::tiling::PaneId;
use crate::ui::text;

/// 报告正文里最多列出的变更文件数。
const MAX_REPORT_CHANGED_PATHS: usize = 8;

/// 一次性 `/report` 总结完成：正文已经在提交时拼好来源头与变更文件。
#[derive(Debug, Clone)]
pub struct GeneratedReportReady {
    pub pane: PaneId,
    pub parent_session: String,
    pub kind: FoldedKind,
    pub source: String,
    /// 已包含来源头、正文与变更文件的完整芯片正文。
    pub body: String,
}

/// 组装报告芯片正文：来源头 + 总结正文 + 变更文件（最多 8 条）。
///
/// `total` 是会话内已提交变更文件总数；展示列表可能被 SQL LIMIT 截断，
/// `(+N more)` 必须用总数计算，不能拿截断后的长度冒充。
pub fn format_report_body(
    header: &str,
    body: &str,
    changed_paths: &[String],
    changed_total: usize,
) -> String {
    let mut out = format!("{}\n{}", header.trim_end(), body.trim_end());
    if !changed_paths.is_empty() {
        let shown = changed_paths
            .iter()
            .take(MAX_REPORT_CHANGED_PATHS)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str("\nChanged files: ");
        out.push_str(&shown);
        let total = changed_total.max(changed_paths.len());
        if total > MAX_REPORT_CHANGED_PATHS {
            out.push_str(&format!(" (+{} more)", total - MAX_REPORT_CHANGED_PATHS));
        }
    }
    out
}

/// 一次性总结完成：只填主输入框，不自动起父轮。
pub fn on_generated_report_ready(state: &mut AppState, ready: GeneratedReportReady) -> bool {
    if ready.body.trim().is_empty() {
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
    let body = ready.body;
    if !fill_main_input(
        state,
        ready.pane,
        &ready.parent_session,
        ready.kind,
        &ready.source,
        &body,
    ) {
        tracing::debug!(
            source = %ready.source,
            pane = ready.pane.raw(),
            "generated report dropped: pane session changed"
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
    // `/resume` 选会话或历史加载期间草稿仍属于旧会话：此时填芯片会被带进新会话。
    if target.lx.chat.loading_session
        || matches!(
            target.lx.slash,
            Some(crate::app::state::SlashPanel::History(_))
        )
    {
        return false;
    }
    target.lx.insert_report_chip(kind, source, body)
}
