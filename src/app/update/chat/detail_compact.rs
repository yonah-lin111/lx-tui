//! 详情会话的 `/compact`：命令入口与压缩开始/完成的状态回投。

use crate::app::effects::Effect;
use crate::app::state::{AppState, CompactNoticeView, DetailTarget};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::show_lx_toast;
use crate::tiling::PaneId;
use crate::ui::text;

/// `/compact` 命令入口：冻结目标并交给事件层解析会话与模型。
pub(crate) fn request(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = detail_target(state, pane) else {
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_COMPACT_UNAVAILABLE,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return true;
    };
    super::composer::clear(state, pane);
    state.effects.push(Effect::CompactDetail { pane, target });
    true
}

/// 详情目标：Agents 取编辑中的 agent，BTW 取当前线程。
fn detail_target(state: &AppState, pane: PaneId) -> Option<DetailTarget> {
    let target = state.pane_anywhere(pane)?;
    if target.lx.agents_editing() {
        return target
            .lx
            .agent_edit_id()
            .map(|agent_id| DetailTarget::Agent(agent_id.to_string()));
    }
    if target.lx.btw_editing() {
        return target
            .lx
            .btw
            .active_thread()
            .and_then(|thread| thread.thread_id.clone())
            .map(DetailTarget::Btw);
    }
    None
}

/// 压缩回执（事件层结果的应用层投影，避免 app 依赖 event 类型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailCompactResult {
    Committed,
    Empty,
    Failed(String),
}

/// 事件层已受理压缩：置执行态并关闭 composer 面板。
pub fn on_started(
    state: &mut AppState,
    pane: PaneId,
    target: &DetailTarget,
    operation_id: String,
) -> bool {
    on_started_kind(state, pane, target, operation_id, false)
}

/// 事件层已受理 `/report` 总结：置执行态（与 compact 共用 running/计时）。
pub fn on_report_started(
    state: &mut AppState,
    pane: PaneId,
    target: &DetailTarget,
    operation_id: String,
) -> bool {
    on_started_kind(state, pane, target, operation_id, true)
}

fn on_started_kind(
    state: &mut AppState,
    pane: PaneId,
    target: &DetailTarget,
    operation_id: String,
    report: bool,
) -> bool {
    let now = crate::db::now_ms();
    let Some(target_state) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let started = match target {
        DetailTarget::Agent(agent_id) => target_state
            .lx
            .agents
            .by_agent_mut(agent_id)
            .map(|run| {
                let started = if report {
                    run.compact.start_report(operation_id.clone(), now)
                } else {
                    run.compact.start(operation_id.clone(), now)
                };
                if !started {
                    return false;
                }
                run.touch_access();
                // 运行中的检查点行是新增内容：总行数必须重量，否则滚动上限、
                // 滚动条与“贴底”判定会一直沿用压缩前的旧值。
                run.rows_dirty = true;
                true
            })
            .unwrap_or(false),
        DetailTarget::Btw(thread_id) => target_state
            .lx
            .btw
            .thread_by_thread_id_mut(thread_id)
            .map(|thread| {
                let started = if report {
                    thread.compact.start_report(operation_id.clone(), now)
                } else {
                    thread.compact.start(operation_id.clone(), now)
                };
                if !started {
                    return false;
                }
                thread.rows_dirty = true;
                true
            })
            .unwrap_or(false),
    };
    if !started {
        return false;
    }
    target_state
        .lx
        .with_active_editor(|editor| editor.slash = None);
    true
}

/// 压缩完成：只回填对应详情会话，不触碰主会话检查点与 revision。
pub fn handle_finished(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    operation_id: &str,
    outcome: &DetailCompactResult,
    checkpoints: Option<&[crate::db::chat::CompactCheckpoint]>,
    report: bool,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let notices = checkpoints.map(|records| {
        records
            .iter()
            .map(CompactNoticeView::from_record)
            .collect::<Vec<_>>()
    });
    let mut found = false;
    if let Some(run) = target.lx.agents.by_session_mut(session_id) {
        if run.compact.running.as_deref() != Some(operation_id) {
            return false;
        }
        run.compact.finish();
        // 运行中的检查点行已消失：无论成功/空/失败都要重量，否则滚动上限
        // 会一直多算这一行。
        run.rows_dirty = true;
        if let Some(notices) = notices.as_ref() {
            run.compact.replace(notices.clone());
            run.rows_dirty = true;
            run.follow = true;
            run.scroll = usize::MAX;
        }
        found = true;
    } else if let Some(thread) = target.lx.btw.thread_by_child_session_mut(session_id) {
        if thread.compact.running.as_deref() != Some(operation_id) {
            return false;
        }
        thread.compact.finish();
        thread.rows_dirty = true;
        if let Some(notices) = notices {
            thread.compact.replace(notices);
            thread.rows_dirty = true;
            thread.follow = true;
            thread.scroll = usize::MAX;
        }
        found = true;
    }
    if !found {
        return false;
    }
    match outcome {
        // 报告成功由事件层继续填主输入框，这里不重复 Toast。
        DetailCompactResult::Committed if report => {}
        DetailCompactResult::Committed => show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Default,
                text::LX_COMPACT_DONE,
                Some(pane),
                std::time::Instant::now(),
            ),
        ),
        DetailCompactResult::Empty if report => show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_REPORT_UNAVAILABLE,
                Some(pane),
                std::time::Instant::now(),
            ),
        ),
        DetailCompactResult::Empty => show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Default,
                text::LX_COMPACT_EMPTY,
                Some(pane),
                std::time::Instant::now(),
            ),
        ),
        DetailCompactResult::Failed(message) if report => show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                format!("{} ({message})", text::detail::LX_REPORT_UNAVAILABLE),
                Some(pane),
                std::time::Instant::now(),
            ),
        ),
        DetailCompactResult::Failed(message) => show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                format!("{} ({message})", text::LX_COMPACT_FAILED),
                Some(pane),
                std::time::Instant::now(),
            ),
        ),
    }
    true
}
