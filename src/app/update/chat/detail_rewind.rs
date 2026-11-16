//! 详情 composer 的 `/rewind`：预检回投、确认面板与成功回填。
//!
//! 与主聊天 `/rewind` 共用 `execute_rewind` / `commit_rewind`；这里只负责目标
//! （子会话 / BTW 线程）与回投后的视图重载。compact 检查点走同一份面板状态，
//! 执行时由事件层改走 `revert_compact`。

use crate::app::effects::{BtwEffect, DetailRewindRequest, Effect, RewindRequest};
use crate::app::state::{
    AppState, DetailRewindExecution, DetailRewindKind, DetailRewindPlan, DetailTarget,
    RewindAction, RewindPanel, RewindTargetKind, SlashPanel,
};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::{show_lx_toast, show_toast_at};
use crate::tiling::PaneId;
use crate::ui::text;

/// `/rewind` 命令入口：消费草稿并请求事件层预检。
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
        // 上下文在、目标不在（新 QA 尚无线程 / agent 已被移除）：命令已消费，
        // 必须给出可见反馈，不能静默吞掉。
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_DETAIL_REWIND_NOTHING,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return true;
    };
    let executing = state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.detail_rewind.is_executing());
    if executing {
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_DETAIL_REWIND_BUSY,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return true;
    }
    // 命令文本先消费；若执行成功，撤回原文只在草稿仍为空时回填。
    super::composer::clear(state, pane);
    state.effects.push(Effect::PrepareDetailRewind {
        pane,
        target,
        input_at_open: String::new(),
    });
    true
}

/// 预检回执：冻结计划并打开动作面板（轮次四项；compact 恢复与取消）。
pub fn on_prepared(state: &mut AppState, pane: PaneId, plan: DetailRewindPlan) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !plan.target.is_active(&target.lx) {
        return false;
    }
    let kind = match plan.target {
        DetailTarget::Agent(_) => DetailRewindKind::Agent,
        DetailTarget::Btw(_) => DetailRewindKind::Btw,
    };
    let panel = match plan.kind {
        RewindTargetKind::Compact => RewindPanel::for_detail_compact(kind),
        _ => RewindPanel::for_detail(kind, plan.snapshot),
    };
    target
        .lx
        .with_active_editor(|editor| editor.slash = Some(SlashPanel::Rewind(Box::new(panel))));
    target.lx.detail_rewind.pending = Some(plan);
    true
}

/// 确认面板的 Enter：执行或取消；禁用动作与过期计划直接作废。
pub(crate) fn apply_confirm(state: &mut AppState, pane: PaneId, panel: &RewindPanel) -> bool {
    let Some(plan) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.detail_rewind.pending.clone())
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
    let action = panel.active_action();
    // 含文件动作在快照不可用时禁用：吞掉 Enter，不降级成删对话。
    if !panel.enabled(action) {
        return true;
    }
    if action != RewindAction::Cancel
        && (action == RewindAction::RestoreCompact) != (plan.kind == RewindTargetKind::Compact)
    {
        return true;
    }
    if action == RewindAction::Cancel {
        clear_pending(state, pane);
        close_panel(state, pane);
        return true;
    }
    let mut plan = plan;
    // 记录用户实际选择：回投据此决定草稿回填与提示文案。
    plan.action = action;
    let operation_id = crate::db::new_external_id("detail-rewind");
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
        .push(Effect::ExecuteDetailRewind(DetailRewindRequest {
            pane,
            target: plan.target.clone(),
            operation_id: operation_id.clone(),
            request,
        }));
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.detail_rewind.pending = Some(plan.clone());
        target.lx.detail_rewind.executing = Some(DetailRewindExecution {
            operation_id,
            target: plan.target,
        });
    }
    close_panel(state, pane);
    true
}

/// 执行回执成功：重载详情会话并回填草稿。
///
/// 回填只落在 `plan.target` 自己的 composer 上：用户可能在回执到达前退出编辑、
/// 切到别的 agent / QA，绝不能把被撤回的原文写进主输入框。
#[allow(clippy::too_many_arguments)]
pub fn on_succeeded(
    state: &mut AppState,
    pane: PaneId,
    plan: &DetailRewindPlan,
    messages: &[crate::db::chat::ChatMessage],
    checkpoints: &[crate::db::chat::CompactCheckpoint],
    session_deleted: bool,
    restored_files: usize,
    already_restored: bool,
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
    target.lx.detail_rewind.clear();
    let mut load_qa_list = false;
    match &plan.target {
        DetailTarget::Agent(agent_id) => {
            {
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
            if let Some(composer) = target.lx.agents.composer_mut(agent_id) {
                backfill(composer.editor_mut(), plan, user_text);
            }
        }
        DetailTarget::Btw(thread_id) => {
            let Some(thread) = target.lx.btw.thread_by_thread_id_mut(thread_id) else {
                return false;
            };
            if session_deleted {
                thread.reset_after_delete();
                // 线程行已随子会话级联删除；QA 列表的 `● side thread` 必须重载才消失。
                load_qa_list = true;
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
            backfill(thread.composer.editor_mut(), plan, user_text);
        }
    }
    if load_qa_list && let Some(parent_session_id) = target.lx.chat.session_id.clone() {
        state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
            pane,
            parent_session_id,
        }));
    }
    let message = if plan.action == RewindAction::RestoreCompact {
        text::LX_COMPACT_REWOUND.to_string()
    } else {
        text::rewind_notice(plan.action.as_str(), restored_files, already_restored)
    };
    show_toast_at(
        state,
        Some(pane),
        Toast::new(
            ToastKind::Default,
            message,
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

/// 草稿回填：只有删对话的动作才回填，且仅当该 composer 仍等于发起的原文。
fn backfill(editor: &mut crate::app::state::Lx, plan: &DetailRewindPlan, user_text: &str) {
    if editor.input == plan.input_at_open && plan.action.fills_draft() && !user_text.is_empty() {
        editor.replace_input(user_text);
    }
    editor.slash = None;
}

/// 执行失败：清执行态、保留对话与草稿、提示原因。
pub fn on_failed(state: &mut AppState, pane: PaneId, message: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.detail_rewind.clear();
    if target.lx.agents_editing() || target.lx.btw_editing() {
        target.lx.with_active_editor(|editor| editor.slash = None);
    }
    show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Warning,
            format!("{} ({message})", text::detail::LX_DETAIL_REWIND_FAILED),
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

/// 执行成功但读回失败：提交已落库，保留当前视图并提示重载失败。
pub fn on_reload_failed(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.detail_rewind.clear();
    if target.lx.agents_editing() || target.lx.btw_editing() {
        target.lx.with_active_editor(|editor| editor.slash = None);
    }
    show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Warning,
            text::detail::LX_DETAIL_REWIND_RELOAD_FAILED,
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

/// 清掉等待确认的计划（面板关闭时）。
pub(crate) fn clear_pending(state: &mut AppState, pane: PaneId) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.detail_rewind.pending = None;
    }
}

fn close_panel(state: &mut AppState, pane: PaneId) {
    super::composer::close(state, pane);
}
