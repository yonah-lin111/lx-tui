//! 流中 steer 的 app 层状态转换：提交、撤回、终态提升与退回。
//!
//! 只改纯状态与入队效应；DB/网络由事件层执行。生效路径见
//! `event::chat::steer`，安全边界见 `chat::agent_loop`。

use std::path::Path;

use crate::app::state::lx::chat::PendingSteer;
use crate::app::state::{AppState, FrozenModel, InputHintLevel, RequestState};
use crate::chat::prepare::SkillNotice;
use crate::db::chat::ModelSnapshot;
use crate::tiling::PaneId;
use crate::ui::text;

use super::session::show_lx_hint;
use super::{next_generation, refresh_lx_status};

/// 流中 Enter：在 `Streaming` 且会话归属匹配时接受为 pending steer。
///
/// 返回 true 表示已经处理（接受或给出提示）；false 表示不适用，交回普通发送路径。
pub(super) fn try_submit(state: &mut AppState, pane: PaneId) -> bool {
    let context = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        if target.view != crate::app::state::PaneView::Lx
            || target.lx.entry != crate::app::state::LxEntry::Native
            || target.lx.shell_mode()
            || target.lx.compact.is_running()
            || target.lx.chat.has_running_shell()
        {
            return false;
        }
        let RequestState::Streaming { key, .. } = &target.lx.chat.request else {
            return false;
        };
        if target.lx.chat.session_id.as_deref() != Some(key.session.as_str()) {
            return false;
        }
        let draft = target.lx.input.trim().to_string();
        if draft.is_empty() {
            return false;
        }
        if draft.starts_with('/') {
            show_lx_hint(
                state,
                pane,
                InputHintLevel::Warning,
                text::LX_STEER_SLASH_UNSUPPORTED,
            );
            return true;
        }
        if draft.len() > crate::db::chat::MAX_MESSAGE_BYTES {
            show_lx_hint(
                state,
                pane,
                InputHintLevel::Warning,
                text::LX_STEER_TEXT_LIMIT,
            );
            return true;
        }
        let pending_count = target.lx.chat.pending_steers.len();
        if pending_count >= crate::app::state::lx::chat::steer::MAX_PENDING_STEERS {
            show_lx_hint(
                state,
                pane,
                InputHintLevel::Warning,
                text::LX_STEER_TOO_MANY,
            );
            return true;
        }
        let selection = target.lx.chat.effective_selection(&state.models);
        let reference_root = state
            .workspace_cwd_for_pane(pane)
            .or_else(|| state.chat_session_key_for_pane(pane));
        (draft, selection, reference_root)
    };
    let (draft, selection, reference_root) = context;
    let Some(selection) = selection else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Some(reference_root) = reference_root else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_NO_WORKSPACE,
        );
        return true;
    };
    let prepared = match crate::chat::prepare_user_message(&reference_root, &draft, &[]) {
        Ok(prepared) => prepared,
        Err(err) => {
            let msg = crate::ui::text::chat::image_error_message(&err);
            show_lx_hint(state, pane, InputHintLevel::Warning, msg);
            return true;
        }
    };
    if !prepared.model_images.is_empty() && !frozen.supports_image_input {
        let msg = crate::ui::text::chat::image_error_message(
            &crate::platform::ImagePrepareError::ImageUnsupportedByModel,
        );
        show_lx_hint(state, pane, InputHintLevel::Warning, msg);
        return true;
    }
    if let Some(notice) = prepared.skill_notices.first() {
        let message = match notice {
            SkillNotice::Truncated { name } => text::skills::truncated(name),
            SkillNotice::LoadFailed { name, reason } => text::skills::read_failed(name, reason),
        };
        show_lx_hint(state, pane, InputHintLevel::Warning, message);
    }
    let image_bytes = prepared
        .model_images
        .iter()
        .map(|image| u64::try_from(image.bytes.len()).unwrap_or(u64::MAX))
        .sum::<u64>();
    let pending_image_bytes = state
        .pane_anywhere(pane)
        .map(|target| {
            target
                .lx
                .chat
                .pending_steers
                .iter()
                .map(PendingSteer::image_bytes)
                .sum::<u64>()
        })
        .unwrap_or(0);
    if pending_image_bytes.saturating_add(image_bytes)
        > crate::app::state::lx::chat::steer::MAX_PENDING_IMAGE_BYTES
    {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_STEER_IMAGE_LIMIT,
        );
        return true;
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.pending_steers.push(PendingSteer {
        text: prepared.raw_text,
        attachments: prepared.attachments,
        model_images: prepared.model_images,
        model: ModelSnapshot {
            provider: frozen.provider.clone(),
            model: frozen.model.clone(),
            variant: frozen.variant.clone(),
        },
    });
    target.lx.chat.stop_armed_at = None;
    target.lx.clear();
    refresh_lx_status(state, pane);
    true
}

/// 流中单次 `Esc`：按 LIFO 弹出最近一条 pending steer 并直接回显到输入框（不与现有内容拼接合并，也不提前污染持久化历史）。
pub fn recall_last(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !matches!(target.lx.chat.request, RequestState::Streaming { .. }) {
        return false;
    }
    let Some(pending) = target.lx.chat.pending_steers.pop() else {
        return false;
    };
    target.lx.end_history_browse();
    target.lx.import_full_text(&pending.text);
    target.lx.chat.stop_armed_at = None;
    refresh_lx_status(state, pane);
    true
}

/// 终态失败/取消：按顺序将每条 pending 压入输入框撤销栈并最终回显最新一条（绝不将多条合并拼接，也不丢失带附件的前序条目）。
pub fn restore_pending_to_input(state: &mut AppState, pane: PaneId) -> usize {
    let pending = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return 0;
        };
        target.lx.chat.promoting_steers = 0;
        if target.lx.chat.pending_steers.is_empty() {
            return 0;
        }
        std::mem::take(&mut target.lx.chat.pending_steers)
    };
    let count = pending.len();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.end_history_browse();
        for item in &pending {
            target.lx.import_full_text(&item.text);
        }
    }
    refresh_lx_status(state, pane);
    count
}

/// 正常完成但 pending 未生效：把队首批次提升为下一轮（复用 `BeginChatTurn`）。
pub fn promote_after_finish(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(batch) =
        crate::app::state::lx::chat::steer::build_batch(&target.lx.chat.pending_steers)
    else {
        return false;
    };
    let workspace = state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.active_workspace().repo_key().map(Path::to_path_buf));
    let Some(workspace) = workspace else {
        restore_pending_to_input(state, pane);
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_NO_WORKSPACE,
        );
        return false;
    };
    let session_id = target.lx.chat.session_id.clone();
    let Some(session_id) = session_id else {
        restore_pending_to_input(state, pane);
        return false;
    };
    // 提升为普通下一轮时优先按当前配置解析模型，无配置时回退到提交 steer 时冻结的 `batch.model`。
    let fallback_selection = crate::config::ai::ModelSelection {
        provider: batch.model.provider.clone(),
        model: batch.model.model.clone(),
        variant: batch.model.variant.clone(),
    };
    let resolved = target
        .lx
        .chat
        .effective_selection(&state.models)
        .and_then(|selection| FrozenModel::resolve(&state.models, &selection).map(|m| (m, false)))
        .or_else(|| FrozenModel::resolve(&state.models, &fallback_selection).map(|m| (m, true)));
    let Some((current, used_fallback)) = resolved else {
        restore_pending_to_input(state, pane);
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return false;
    };
    if !batch.model_images.is_empty() && !current.supports_image_input {
        restore_pending_to_input(state, pane);
        let msg = crate::ui::text::chat::image_error_message(
            &crate::platform::ImagePrepareError::ImageUnsupportedByModel,
        );
        show_lx_hint(state, pane, InputHintLevel::Warning, msg);
        return false;
    }
    let model = ModelSnapshot {
        provider: current.provider,
        model: current.model,
        variant: current.variant,
    };
    let generation = next_generation(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if used_fallback && target.lx.chat.selection.is_none() {
        target.lx.chat.selection = Some(fallback_selection);
    }
    target.lx.chat.generation_counter = generation;
    target.lx.chat.promoting_steers = batch.consumed;
    target.lx.chat.main_stopped = false;
    target.lx.chat.stop_armed_at = None;
    target.lx.chat.request = RequestState::Committing {
        generation,
        draft: batch.text.clone(),
    };
    state
        .effects
        .push(crate::app::effects::Effect::BeginChatTurn {
            pane,
            generation,
            workspace,
            session_id: Some(session_id),
            draft: batch.text,
            attachments: batch.attachments,
            model,
        });
    refresh_lx_status(state, pane);
    true
}

/// 会话切换/删除：丢弃 pending 并给出可见提示。
pub fn discard_for_session_change(state: &mut AppState, pane: PaneId) -> usize {
    let count = state
        .pane_anywhere(pane)
        .map(|target| target.lx.chat.pending_steers.len())
        .unwrap_or(0);
    if count == 0 {
        return 0;
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.pending_steers.clear();
        target.lx.chat.promoting_steers = 0;
    }
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Warning,
        text::lx_steer_discarded(count),
    );
    count
}

/// 最终 DB 写入成功：回 Idle；失败保留内存 partial 并提示保存失败。
pub fn on_finish_ack(
    state: &mut AppState,
    key: &crate::app::state::RequestKey,
    success: bool,
) -> bool {
    let reason = {
        let Some(target) = state.pane_mut_anywhere(key.pane) else {
            return false;
        };
        if !matches!(
            &target.lx.chat.request,
            RequestState::Finishing {
                key: active,
                ..
            } if active == key
        ) {
            return false;
        }
        let RequestState::Finishing { reason, .. } = &target.lx.chat.request else {
            return false;
        };
        reason.clone()
    };
    if success {
        if let Some(target) = state.pane_mut_anywhere(key.pane) {
            target.lx.chat.request = RequestState::Idle;
        }
        let promoted = matches!(reason, crate::chat::events::StopReason::Complete)
            && promote_after_finish(state, key.pane);
        if !promoted {
            if !matches!(reason, crate::chat::events::StopReason::Complete) {
                // Esc 停止或失败：未生效 steer 退回输入框，不自动发送。
                restore_pending_to_input(state, key.pane);
            }
            // 模型轮次结束后按 FIFO 分派排队项（忙时 Tab 排队的 shell 在这里开始）。
            super::user_shell::on_user_shell_terminal(state, key.pane);
            // 没有用户提交占位时，才允许后台报告自动起轮。用户输入优先。
            super::request_report_drain_if_idle(state, key.pane);
        }
    } else {
        // Finishing（保存失败）：保留内存 partial，不能伪装成可恢复历史。
        restore_pending_to_input(state, key.pane);
        show_lx_hint(
            state,
            key.pane,
            InputHintLevel::Error,
            text::LX_CHAT_SAVE_FAILED,
        );
    }
    refresh_lx_status(state, key.pane);
    true
}
