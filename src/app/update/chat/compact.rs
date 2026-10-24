//! `/compact` 的状态转换：空闲检查、冻结请求、回执处理与撤销。
//!
//! 只改纯状态并入队 `Effect`：摘要请求、DB 事务与视图重载都在事件层执行。
//! - 忙碌（流式回复 / 工具执行 / 已有压缩在跑）时立即拒绝，不中断当前任务；
//! - 成功后回填检查点列表与模型名；失败**不改**任何持久化状态与折叠状态；
//! - `/rewind` 对 compact 只切上下文指针，不触碰文件 scope。

use crate::app::effects::{CompactRequest, CompactRevertRequest, Effect};
use crate::app::state::{AppState, CompactNoticeView, InputHintLevel, RequestState};
use crate::db::chat::CompactCheckpoint;
use crate::layout::PaneId;
use crate::ui::text;

use super::{refresh_lx_status, show_lx_hint};

/// `/compact` 采纳：空闲检查 → 冻结请求 → 入队效应。
///
/// 返回是否产生状态变化（拒绝也算已处理，避免命令文本落进草稿）。
pub fn compact_command(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    // 只允许空闲时压缩：流中/等待工具/已在压缩都拒绝，不打断当前任务。
    if !matches!(target.lx.chat.request, RequestState::Idle) || target.lx.compact.is_running() {
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_COMPACT_BUSY);
        return true;
    }
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_COMPACT_NO_SESSION,
        );
        return true;
    };
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_NO_WORKSPACE,
        );
        return true;
    };
    // 模型：与下一轮聊天同一解析口径（窗格选择优先，回退配置默认）。
    let Some(selection) = target.lx.chat.effective_selection(&state.models) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Some(frozen) = crate::app::state::FrozenModel::resolve(&state.models, &selection) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let model_label = state
        .models
        .model_display_name(&frozen.provider, &frozen.model);
    let base_checkpoint_id = target.lx.compact.active().map(|item| item.id.clone());
    let expected_revision = target.lx.chat.revision;
    let operation_id = crate::db::new_external_id("compact-op");
    // 计时起点与幂等键一起冻结：输入框上方那一行据此显示「压缩中 + 已耗时」。
    let started_at_ms = crate::db::now_ms();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.compact.start(operation_id.clone(), started_at_ms);
    state.effects.push(Effect::ExecuteCompact(CompactRequest {
        pane,
        workspace,
        session_id,
        expected_revision,
        base_checkpoint_id,
        model: crate::db::chat::ModelSnapshot {
            provider: frozen.provider.clone(),
            model: frozen.model.clone(),
            variant: frozen.variant.clone(),
        },
        model_label,
        transport: frozen.transport.clone(),
        base_url: frozen.base_url.clone(),
        api_key: frozen.api_key.clone(),
        variant_params: frozen.variant_params.clone(),
        idle_timeout_ms: frozen.idle_timeout_ms,
        operation_id,
    }));
    true
}

/// 压缩成功：回填检查点列表并展示实际模型名。
pub fn on_compact_finished(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    checkpoints: Vec<CompactCheckpoint>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 迟到回执：窗格已切到别的会话时不得覆盖。
    if target.lx.chat.session_id.as_deref() != Some(session_id) {
        return false;
    }
    let views: Vec<CompactNoticeView> = checkpoints
        .iter()
        .map(CompactNoticeView::from_record)
        .collect();
    target.lx.compact.replace(views);
    target.lx.compact.finish();
    let active = target.lx.compact.active().cloned();
    // 新检查点写入后 revision 前进：旧选中目标/撤销面板全部作废。
    target.lx.clear_rewind();
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Warning,
        &match active {
            Some(checkpoint) => {
                format!(
                    "{} model: {}",
                    text::LX_COMPACT_DONE,
                    checkpoint.model_label
                )
            }
            None => text::LX_COMPACT_DONE.to_string(),
        },
    );
    // 压缩后视图只显示摘要条目与其后消息：跟随底部重新计算几何。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.follow_bottom = true;
        target.lx.chat.suppressed_prompt_header = None;
    }
    refresh_lx_status(state, pane);
    true
}

/// 压缩没有可压缩的增量（成功后无写入）：只清执行态并提示。
pub fn on_compact_empty(state: &mut AppState, pane: PaneId, session_id: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() != Some(session_id) {
        return false;
    }
    target.lx.compact.finish();
    show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_COMPACT_EMPTY);
    true
}

/// 压缩失败：清执行态、保留全部历史与折叠状态、不写检查点。
pub fn on_compact_failed(state: &mut AppState, pane: PaneId, message: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.compact.finish();
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Error,
        format!("{} ({message})", text::LX_COMPACT_FAILED),
    );
    true
}

/// `/rewind` 选择到 compact 时构造撤销请求（冻结 revision 与检查点 id）。
///
/// 只切上下文指针：不建文件 scope、不读文件、不请求快照预检。
pub fn revert_request(
    state: &AppState,
    pane: PaneId,
) -> Option<(Effect, crate::app::state::RewindTarget)> {
    let target = state.pane_anywhere(pane)?;
    if !matches!(target.lx.chat.request, RequestState::Idle) {
        return None;
    }
    let checkpoint = target.lx.compact.active()?.clone();
    let session_id = target.lx.chat.session_id.clone()?;
    let workspace = state.chat_session_key_for_pane(pane)?;
    let revision = target.lx.chat.revision;
    let operation_id = crate::db::new_external_id("compact-rewind-op");
    let effect = Effect::RevertCompact(CompactRevertRequest {
        pane,
        workspace,
        session_id: session_id.clone(),
        checkpoint_id: checkpoint.id.clone(),
        expected_revision: revision,
        operation_id,
    });
    Some((
        effect,
        crate::app::state::RewindTarget {
            session_id,
            user_message_id: checkpoint.id.clone(),
            expected_revision: revision,
            pane,
            input_at_open: target.lx.input.clone(),
            user_text: String::new(),
            kind: crate::app::state::RewindTargetKind::Compact,
        },
    ))
}

/// compact 撤销成功：回填检查点列表并提示（不改草稿：没有可回填的用户原文）。
pub fn on_compact_reverted(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    checkpoints: Vec<CompactCheckpoint>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() != Some(session_id) {
        return false;
    }
    let views: Vec<CompactNoticeView> = checkpoints
        .iter()
        .map(CompactNoticeView::from_record)
        .collect();
    target.lx.compact.replace(views);
    target.lx.rewind.clear();
    if matches!(
        target.lx.slash,
        Some(crate::app::state::SlashPanel::Rewind(_))
    ) {
        target.lx.slash = None;
    }
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Warning,
        text::LX_COMPACT_REWOUND,
    );
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.follow_bottom = true;
    }
    refresh_lx_status(state, pane);
    true
}

/// 会话重载/切换时替换检查点列表（保留仍存在的展开状态）。
pub fn set_checkpoints(
    state: &mut AppState,
    pane: PaneId,
    checkpoints: &[CompactCheckpoint],
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let views: Vec<CompactNoticeView> = checkpoints
        .iter()
        .map(CompactNoticeView::from_record)
        .collect();
    if target.lx.compact.checkpoints == views {
        return false;
    }
    target.lx.compact.replace(views);
    true
}

/// 切换检查点条目的展开状态（`Enter`/点击；默认折叠）。
pub fn toggle_checkpoint(state: &mut AppState, pane: PaneId, checkpoint_id: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !target
        .lx
        .compact
        .checkpoints
        .iter()
        .any(|item| item.id == checkpoint_id)
    {
        return false;
    }
    super::pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.compact.toggle(checkpoint_id);
    true
}
