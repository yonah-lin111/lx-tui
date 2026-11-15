//! 详情 composer 的 `/undo`：预检回投、两行确认与成功回填。
//!
//! 与主聊天 `/rewind` 共用 `execute_rewind` / `commit_rewind`；这里只负责目标
//! （子会话 / BTW 线程）与回投后的视图重载。

use crate::app::effects::{DetailUndoRequest, Effect, RewindRequest};
use crate::app::state::{
    AppState, DetailTarget, DetailUndoExecution, DetailUndoKind, DetailUndoPlan, RewindAction,
    RewindPanel, SlashPanel,
};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::{show_lx_toast, show_toast_at};
use crate::tiling::PaneId;
use crate::ui::text;

/// `/undo` 命令入口：消费草稿并请求事件层预检。
pub(crate) fn prepare(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane).and_then(|target| {
        if target.lx.agents_editing() {
            target
                .lx
                .agent_edit_id()
                .map(|agent_id| DetailTarget::Agent(agent_id.to_string()))
        } else if target.lx.btw_editing() {
            target
                .lx
                .btw
                .active_thread()
                .and_then(|thread| thread.thread_id.clone())
                .map(DetailTarget::Btw)
        } else {
            None
        }
    }) else {
        return false;
    };
    let executing = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .is_some_and(|page| page.detail_undo_executing.is_some());
    if executing {
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_DETAIL_UNDO_BUSY,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return true;
    }
    // 命令文本先消费；若执行成功，撤回原文只在草稿仍为空时回填。
    super::composer::clear(state, pane);
    state.effects.push(Effect::PrepareDetailUndo {
        pane,
        target,
        input_at_open: String::new(),
    });
    true
}

/// 预检回执：冻结计划并打开两行确认（默认高亮 Cancel）。
pub fn on_prepared(state: &mut AppState, pane: PaneId, plan: DetailUndoPlan) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !plan.target.is_active(&target.lx) {
        return false;
    }
    let kind = match plan.target {
        DetailTarget::Agent(_) => DetailUndoKind::Agent,
        DetailTarget::Btw(_) => DetailUndoKind::Btw,
    };
    let snapshot = plan.snapshot;
    let action = plan.action;
    target.lx.with_active_editor(|editor| {
        editor.slash = Some(SlashPanel::Rewind(Box::new(RewindPanel::for_detail(
            kind, snapshot, action,
        ))));
    });
    if let Some(page) = target.lx.task.as_mut() {
        page.detail_undo_pending = Some(plan);
    }
    true
}

/// 确认面板的 Enter：执行或取消；过期计划直接作废。
pub(crate) fn apply_confirm(state: &mut AppState, pane: PaneId, panel: &RewindPanel) -> bool {
    let Some(plan) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.detail_undo_pending.clone())
    else {
        close_panel(state, pane);
        return true;
    };
    // 面板打开后目标可能被切走（切 agent / QA）：过期计划直接作废。
    if !state
        .pane_anywhere(pane)
        .is_some_and(|target| plan.target.is_active(&target.lx))
    {
        clear_pending(state, pane);
        close_panel(state, pane);
        return true;
    }
    match panel.active_action() {
        RewindAction::Cancel => {
            clear_pending(state, pane);
            close_panel(state, pane);
        }
        action if Some(action) == panel.detail.as_ref().map(|detail| detail.action) => {
            let operation_id = crate::db::new_external_id("detail-undo");
            let request = RewindRequest {
                pane,
                session_id: plan.child_session.clone(),
                target_user_message_id: plan.user_message_id.clone(),
                expected_revision: plan.expected_revision,
                action,
                workspace: plan.workspace.clone(),
                input_at_open: plan.input_at_open.clone(),
                user_text: plan.user_text.clone(),
                operation_id: operation_id.clone(),
            };
            state
                .effects
                .push(Effect::ExecuteDetailUndo(DetailUndoRequest {
                    pane,
                    target: plan.target.clone(),
                    operation_id: operation_id.clone(),
                    request,
                }));
            if let Some(page) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
            {
                page.detail_undo_executing = Some(DetailUndoExecution {
                    operation_id,
                    target: plan.target,
                });
            }
            close_panel(state, pane);
        }
        _ => {}
    }
    true
}

/// 执行回执成功：重载详情会话并回填草稿。
pub fn on_succeeded(
    state: &mut AppState,
    pane: PaneId,
    plan: &DetailUndoPlan,
    messages: &[crate::db::chat::ChatMessage],
    checkpoints: &[crate::db::chat::CompactCheckpoint],
    session_deleted: bool,
    user_text: &str,
) -> bool {
    let models = state.models.clone();
    let project_root = state.project_root_for_pane(pane);
    let notices = checkpoints
        .iter()
        .map(crate::app::state::CompactNoticeView::from_record)
        .collect::<Vec<_>>();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    page.detail_undo_executing = None;
    page.detail_undo_pending = None;
    match &plan.target {
        DetailTarget::Agent(agent_id) => {
            let Some(run) = target.lx.agents.by_agent_mut(agent_id) else {
                return false;
            };
            run.chat.messages = messages
                .iter()
                .map(|message| super::view::view_message_of(message, &models))
                .collect();
            run.chat.rebuild_steered_user_messages(messages);
            run.chat.set_project_root(project_root);
            run.loaded = true;
            run.compact.replace(notices);
            run.rows_dirty = true;
            run.follow = true;
            run.scroll = usize::MAX;
        }
        DetailTarget::Btw(thread_id) => {
            let Some(thread) = target.lx.btw.active_thread_mut() else {
                return false;
            };
            if thread.thread_id.as_deref() != Some(thread_id.as_str()) && !session_deleted {
                return false;
            }
            if session_deleted {
                thread.reset_after_delete();
            } else {
                thread.chat.messages = messages
                    .iter()
                    .map(|message| super::view::view_message_of(message, &models))
                    .collect();
                thread.chat.rebuild_steered_user_messages(messages);
                thread.chat.set_project_root(project_root);
                thread.loaded = true;
                thread.compact.replace(notices);
                thread.rows_dirty = true;
                thread.follow = true;
                thread.scroll = usize::MAX;
            }
        }
    }
    // 草稿回填：仅当 composer 仍为空（未被用户改过）时写入撤回原文。
    let unchanged = target
        .lx
        .active_editor()
        .input
        .as_str()
        .eq(plan.input_at_open.as_str());
    if unchanged && !user_text.is_empty() {
        target.lx.with_active_editor(|editor| {
            editor.replace_input(user_text);
            editor.slash = None;
        });
    } else {
        target.lx.with_active_editor(|editor| editor.slash = None);
    }
    show_toast_at(
        state,
        Some(pane),
        Toast::new(
            ToastKind::Default,
            text::detail::LX_DETAIL_UNDO_DONE,
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

/// 执行失败：清执行态、保留对话与草稿、提示原因。
pub fn on_failed(state: &mut AppState, pane: PaneId, message: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if let Some(page) = target.lx.task.as_mut() {
        page.detail_undo_executing = None;
        page.detail_undo_pending = None;
    }
    target.lx.with_active_editor(|editor| editor.slash = None);
    show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Warning,
            format!("{} ({message})", text::detail::LX_DETAIL_UNDO_FAILED),
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

/// 清掉等待确认的计划（面板关闭时）。
pub(crate) fn clear_pending(state: &mut AppState, pane: PaneId) {
    if let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.detail_undo_pending = None;
    }
}

fn close_panel(state: &mut AppState, pane: PaneId) {
    super::composer::close(state, pane);
}
