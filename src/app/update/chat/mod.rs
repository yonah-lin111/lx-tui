//! lx 聊天状态机更新：发送、流事件收敛、终态与 `/` 命令。
//!
//! 只产生效应/更新纯状态；IO（DB/HTTP）由事件层执行，结果经 `AppEvent` 回投。
//! 按 `(pane, session, generation)` 过滤过期结果；`Finishing` 期间不允许新一轮。

use std::time::{Duration, Instant};

use crate::app::state::lx::slash::{SlashCommand, SlashPanel};
use crate::app::state::{
    AppState, ChatMessageView, ChatPartView, FrozenModel, ModelPicker, ModelPickerStage,
    RequestKey, RequestState,
};
use crate::app::toast::{Toast, ToastKind};
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::config::ai::ModelSelection;
use crate::db::chat::{ChatMessage, ModelSnapshot, TurnOutcome, Usage};
use crate::layout::PaneId;
use crate::ui::text;

/// 尝试发送：校验 Native 入口、非空草稿、请求空闲与有效模型后冻结快照并入队效应。
///
/// 返回是否产生状态变化。DB ack 成功后才清草稿；失败保留草稿可重试。
pub fn try_send(state: &mut AppState, pane: PaneId) -> bool {
    // 先收集纯数据（避免长持窗格可变借用）。
    let context = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        if target.view != crate::app::state::PaneView::Lx {
            return false;
        }
        let entry = target.lx.entry;
        let draft = target.lx.input.trim().to_string();
        let busy = !target.lx.chat.request.can_send();
        let selection = target.lx.chat.effective_selection(&state.models);
        let session_id = target.lx.chat.session_id.clone();
        let generation = next_generation(state, pane);
        (entry, draft, busy, selection, session_id, generation)
    };
    let (entry, draft, busy, selection, session_id, generation) = context;
    // 只有 Native Agent 发送；Hybrid/Automation 显示不可用提示。
    if entry != crate::app::state::LxEntry::Native {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_ENTRY_UNAVAILABLE);
        return true;
    }
    if draft.is_empty() || busy {
        // 流中 Enter 不入队、不造消息。
        return false;
    }
    // 解析有效模型（窗格选择或配置默认）。
    let Some(selection) = selection else {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    // 冻结草稿并进入 Committing：清空输入由 DB ack 成功后执行。
    let workspace = state
        .workspace_cwd_for_pane(pane)
        .or_else(|| state.active_workspace().cwd.clone());
    let Some(workspace) = workspace else {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_NO_WORKSPACE);
        return true;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.generation_counter = generation;
    target.lx.chat.request = RequestState::Committing {
        generation,
        draft: draft.clone(),
    };
    state
        .effects
        .push(crate::app::effects::Effect::BeginChatTurn {
            pane,
            generation,
            workspace,
            session_id,
            draft,
            model: ModelSnapshot {
                provider: frozen.provider.clone(),
                model: frozen.model.clone(),
                variant: frozen.variant.clone(),
            },
        });
    // 立即进入流中态：loading 行（转轮 + Esc 待命提示）从按下 Enter 起就可见，
    // 不必等首个 delta。
    refresh_lx_status(state, pane);
    true
}

/// DB begin_turn 成功回执：绑定 session/message、清冻结草稿、启动网络流。
///
/// 若期间用户继续编辑，新草稿不被清掉（只清与冻结草稿相同的输入）。
pub fn on_turn_committed(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    committed: &crate::db::chat::TurnCommitted,
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Committing { generation: g, .. } if *g == generation
    ) {
        return false;
    }
    let frozen_draft = match &target.lx.chat.request {
        RequestState::Committing { draft, .. } => draft.clone(),
        _ => return false,
    };
    // 只清仍与冻结草稿相同的输入（期间用户编辑则保留新草稿）。
    if target.lx.input.trim() == frozen_draft.trim() {
        target.lx.clear();
    }
    // 展示：user 消息 + streaming assistant 占位。
    target.lx.chat.session_id = Some(committed.session_id.clone());
    target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
    // 本轮模型展示名：发送时冻结的选择（设置页缺失时回退 provider/model）。
    let model_label = target
        .lx
        .chat
        .effective_selection(&models)
        .map(|selection| models.model_display_name(&selection.provider, &selection.model));
    target.lx.chat.messages.push(ChatMessageView {
        id: committed.user_message_id.clone(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: committed.turn_index,
        parts: vec![ChatPartView {
            kind: PartKind::Text,
            content: frozen_draft,
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    });
    target.lx.chat.messages.push(ChatMessageView {
        id: committed.assistant_message_id.clone(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: committed.turn_index,
        parts: Vec::new(),
        model_label,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    });
    target.lx.chat.follow_bottom = true;
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation,
    };
    target.lx.chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id: committed.assistant_message_id.clone(),
    };
    // 启动网络流：由事件层按 key 执行；先入队效应。
    state
        .effects
        .push(crate::app::effects::Effect::StartChatStream {
            key,
            assistant_id: committed.assistant_message_id.clone(),
        });
    true
}

/// DB begin_turn 失败：回 Idle 保留草稿并提示可重试。
pub fn on_turn_failed(state: &mut AppState, pane: PaneId, generation: u64) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Committing { generation: g, .. } if *g == generation
    ) {
        return false;
    }
    target.lx.chat.request = RequestState::Idle;
    show_chat_toast(state, ToastKind::Error, text::LX_CHAT_COMMIT_FAILED);
    refresh_lx_status(state, pane);
    true
}

/// 流 delta 回投：只更新匹配 `(pane, session, generation)` 的视图。
pub fn on_stream_delta(state: &mut AppState, key: &RequestKey, deltas: &[ChatDelta]) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Streaming {
            key: active,
            ..
        } if active == key
    ) {
        return false;
    }
    let mut changed = false;
    for delta in deltas {
        match delta {
            ChatDelta::Part(part) => {
                target
                    .lx
                    .chat
                    .append_stream_delta(part.kind, &part.text, part.metadata.clone());
                changed = true;
            }
            ChatDelta::Usage(usage) => {
                // 上下文占用按 codex 口径：上一轮请求的输入 + 输出（下一轮请求的输入近似
                // 为其上一轮输入加上该轮输出）。切模型时分子清空（键由 refresh 维护）。
                if let Some(input) = usage.input {
                    let context = input.saturating_add(usage.output.unwrap_or(0));
                    target.lx.chat.context_tokens = Some(context.max(0) as u64);
                }
                changed = true;
            }
            ChatDelta::Finished { .. } => {
                // 终态事件由 on_stream_finished 处理；此处只标记。
                changed = true;
            }
        }
    }
    if changed {
        refresh_lx_status(state, key.pane);
    }
    changed
}

/// 流终态回投（完整 deltas 先经 on_stream_delta 消费；本函数只处理结束）。
///
/// 取消/失败/完成均先转 Finishing，再入队最终 DB 写入。
pub fn on_stream_finished(
    state: &mut AppState,
    key: &RequestKey,
    reason: &StopReason,
    usage: Usage,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    let assistant_id = match &target.lx.chat.request {
        RequestState::Streaming {
            key: active,
            assistant_id,
        } if active == key => assistant_id.clone(),
        _ => return false,
    };
    // 快照当前 parts（最终 DB 写入的完整内容）；position 按时间线顺序给出。
    let parts: Vec<crate::db::chat::ChatPart> = target
        .lx
        .chat
        .messages
        .last()
        .map(|message| {
            crate::db::chat::chat_part_views(
                message
                    .parts
                    .iter()
                    .enumerate()
                    .map(|(position, part)| (position as i64, part)),
            )
        })
        .unwrap_or_default();
    // 视图先行进入终态（DB ack 后确认；失败时保留内存 partial）。
    target.lx.chat.stop_armed_at = None;
    let duration_ms = target
        .lx
        .chat
        .turn_started_at_ms
        .map(|started| crate::db::now_ms().saturating_sub(started).max(0));
    if let Some(message) = target
        .lx
        .chat
        .messages
        .last_mut()
        .filter(|message| message.id == assistant_id)
    {
        message.status = match reason {
            StopReason::Complete => "complete".into(),
            StopReason::Cancelled => "interrupted".into(),
            StopReason::Failed { .. } => "failed".into(),
        };
        // 失败原因（脱敏码）随消息保留，供历史行与提示显示。
        message.error_code = match reason {
            StopReason::Failed { code } => Some(code.clone()),
            _ => None,
        };
        // 本轮统计：供应商用量 + 耗时（tok/s 由渲染层按输出/耗时派生）。
        message.input_tokens = usage.input;
        message.output_tokens = usage.output;
        message.cache_read_tokens = usage.cache_read;
        message.duration_ms = duration_ms;
    }
    target.lx.chat.request = RequestState::Finishing {
        key: key.clone(),
        reason: reason.clone(),
    };
    let outcome = match reason {
        StopReason::Complete => TurnOutcome::Complete {
            stop_reason: Some("complete".into()),
        },
        StopReason::Cancelled => TurnOutcome::Interrupted,
        StopReason::Failed { code } => TurnOutcome::Failed {
            error_code: code.clone(),
        },
    };
    state
        .effects
        .push(crate::app::effects::Effect::FinishChatTurn {
            key: key.clone(),
            assistant_id,
            parts,
            outcome,
            usage,
        });
    true
}

/// 最终 DB 写入成功：回 Idle；失败保留内存 partial 并提示保存失败。
pub fn on_finish_ack(state: &mut AppState, key: &RequestKey, success: bool) -> bool {
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
    if success {
        target.lx.chat.request = RequestState::Idle;
    } else {
        // Finishing（保存失败）：保留内存 partial，不能伪装成可恢复历史。
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_SAVE_FAILED);
    }
    refresh_lx_status(state, key.pane);
    true
}

/// Esc 停止的待命窗口：窗口内再按一次才真正停止（对齐 opencode 的 5 秒）。
pub const STOP_ARM_WINDOW: Duration = Duration::from_secs(5);

/// 流中按 `Esc`（对齐 opencode 的双击停止）：首次只**待命**并显示提示，
/// 窗口内再按一次才停止；非流中吞掉（返回 false 不置脏）。
pub fn escape_stop(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !target.lx.chat.request.is_busy() {
        return false;
    }
    let now = Instant::now();
    let armed = target
        .lx
        .chat
        .stop_armed_at
        .is_some_and(|at| now.saturating_duration_since(at) <= STOP_ARM_WINDOW);
    if !armed {
        target.lx.chat.stop_armed_at = Some(now);
        return true;
    }
    target.lx.chat.stop_armed_at = None;
    stop_stream(state, pane);
    true
}

/// 停止生成：Esc 双击确认后触发；流取消后走正常 Finishing 收尾。
pub fn stop_stream(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.stop_armed_at = None;
    match target.lx.chat.request.clone() {
        RequestState::Streaming { key, .. } => {
            state
                .effects
                .push(crate::app::effects::Effect::CancelChatStream { key });
            true
        }
        RequestState::Committing { generation, .. } => {
            // 等 DB ack 后直接落 interrupted（不发 HTTP）：标记待取消。
            state
                .effects
                .push(crate::app::effects::Effect::CancelChatCommit { pane, generation });
            true
        }
        _ => false,
    }
}

/// `/new`：解绑当前窗格会话；在途请求继续按原 session 完成写入。
pub fn new_session(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 旧请求仍在途时只解绑并取消其请求（写入继续按原 ID 完成）。
    if target.lx.chat.request.is_busy() {
        stop_stream(state, pane);
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.session_id = None;
    target.lx.chat.messages.clear();
    target.lx.chat.follow_bottom = true;
    target.lx.chat.input_tokens_reset();
    target.lx.chat.thinking_toggled.clear();
    target.lx.chat.turn_started_at_ms = None;
    refresh_lx_status(state, pane);
    true
}

/// `/model` 打开模型面板（第一级）。
pub fn open_model_picker(state: &mut AppState, pane: PaneId) -> bool {
    if !state
        .pane_anywhere(pane)
        .is_some_and(|target| target.view == crate::app::state::PaneView::Lx)
    {
        return false;
    }
    let candidates = ModelPicker::candidates(&state.models.settings);
    if candidates.is_empty() {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_NO_MODEL);
        return true;
    }
    // 高亮对齐当前选择。
    let active = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.selection.as_ref())
        .and_then(|selection| {
            candidates.iter().position(|candidate| {
                candidate.selection.provider == selection.provider
                    && candidate.selection.model == selection.model
            })
        })
        .unwrap_or(0);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.slash = Some(SlashPanel::Model(Box::new(ModelPicker {
        stage: ModelPickerStage::Models,
        candidates,
        window: crate::app::state::lx::slash::PanelWindow::new(active),
    })));
    true
}

/// `/resume` 打开历史面板（事件层提供当前工作区分页列表）。
pub fn open_history(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 列表由事件层按需加载（Effect::ListChatSessions）；先打开面板骨架。
    target.lx.slash = Some(SlashPanel::History(Box::default()));
    state
        .effects
        .push(crate::app::effects::Effect::ListChatSessions { pane });
    true
}

/// 历史列表回投：填充 `/resume` 面板。
pub fn on_history_listed(
    state: &mut AppState,
    pane: PaneId,
    entries: Vec<crate::app::state::lx::slash::HistoryEntry>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(SlashPanel::History(panel)) = target.lx.slash.as_mut() else {
        return false;
    };
    panel.entries = entries;
    panel.window = crate::app::state::lx::slash::PanelWindow::new(0);
    true
}

/// 恢复会话：绑定当前窗格并重建展示历史（DB 行转视图）。
pub fn resume_session(state: &mut AppState, pane: PaneId, session_id: &str) -> bool {
    // 同一 session 已被另一窗格占用（正在生成）时拒绝。
    let occupied = state
        .all_pane_ids()
        .into_iter()
        .filter(|other| *other != pane)
        .any(|other| {
            state.pane_anywhere(other).is_some_and(|p| {
                p.lx.chat.session_id.as_deref() == Some(session_id) && p.lx.chat.request.is_busy()
            })
        });
    if occupied {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_SESSION_BUSY);
        return true;
    }
    state
        .effects
        .push(crate::app::effects::Effect::LoadChatSession {
            pane,
            session_id: session_id.to_string(),
        });
    // 面板先行关闭；列表数据到达后填充。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
    }
    true
}

/// 会话加载回投：重建消息视图与模型选择。
///
/// 同时恢复**上下文占用**：按 codex 口径取最近一次带用量的 assistant 轮次
/// （其输入 + 输出 token 近似下一轮请求的输入），因此重开历史会话不再显示空上下文。
pub fn on_session_loaded(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    messages: &[ChatMessage],
    selection: Option<ModelSelection>,
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.session_id = Some(session_id.to_string());
    if let Some(selection) = selection {
        target.lx.chat.selection = Some(selection);
    }
    target.lx.chat.messages = messages
        .iter()
        .map(|message| ChatMessageView {
            id: message.external_id.clone(),
            role: message.role.clone(),
            status: message.status.clone(),
            turn_index: message.turn_index,
            parts: message
                .parts
                .iter()
                .map(|part| ChatPartView {
                    kind: if part.kind == "reasoning" {
                        PartKind::Reasoning
                    } else {
                        PartKind::Text
                    },
                    content: part.content.clone(),
                    provider_metadata_json: part.provider_metadata_json.clone(),
                    started_at_ms: part.started_at_ms,
                    ended_at_ms: part.ended_at_ms,
                    completed_at_ms: part.completed_at_ms,
                })
                .collect(),
            model_label: message
                .model_provider
                .as_deref()
                .zip(message.model_id.as_deref())
                .map(|(provider, model)| models.model_display_name(provider, model)),
            error_code: message.error_code.clone(),
            input_tokens: message.input_tokens,
            output_tokens: message.output_tokens,
            cache_read_tokens: message.cache_read_tokens,
            duration_ms: message
                .completed_at_ms
                .map(|completed| completed.saturating_sub(message.created_at_ms).max(0)),
        })
        .collect();
    // 上下文占用：最近一条带用量的 assistant 轮次（同模型键由 refresh 校验）。
    let recent = messages.iter().rev().find(|message| {
        message.role == "assistant"
            && (message.input_tokens.is_some() || message.output_tokens.is_some())
    });
    target.lx.chat.last_tokens_model = recent.and_then(|message| {
        message
            .model_provider
            .as_deref()
            .zip(message.model_id.as_deref())
            .map(|(provider, model)| format!("{provider}/{model}"))
    });
    target.lx.chat.context_tokens = recent.map(|message| {
        let input = message.input_tokens.unwrap_or(0).max(0);
        let output = message.output_tokens.unwrap_or(0).max(0);
        (input + output) as u64
    });
    target.lx.chat.follow_bottom = true;
    target.lx.chat.thinking_toggled.clear();
    target.lx.chat.turn_started_at_ms = None;
    refresh_lx_status(state, pane);
    true
}

/// 重命名会话。
pub fn rename_session(state: &mut AppState, pane: PaneId, session_id: &str, title: &str) -> bool {
    let workspace = state
        .workspace_cwd_for_pane(pane)
        .map(|path| path.to_path_buf());
    let Some(workspace) = workspace else {
        return false;
    };
    state
        .effects
        .push(crate::app::effects::Effect::RenameChatSession {
            workspace,
            session_id: session_id.to_string(),
            title: title.to_string(),
        });
    true
}

/// 删除会话（有确认；先取消在途请求，等最终写入后事务级删除）。
pub fn delete_session(state: &mut AppState, pane: PaneId, session_id: &str) -> bool {
    // 在途请求先取消（事件层等 finish 后再删除）。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && target.lx.chat.session_id.as_deref() == Some(session_id)
    {
        stop_stream(state, pane);
    }
    let workspace = state
        .workspace_cwd_for_pane(pane)
        .map(|path| path.to_path_buf());
    let Some(workspace) = workspace else {
        return false;
    };
    state
        .effects
        .push(crate::app::effects::Effect::DeleteChatSession {
            workspace,
            session_id: session_id.to_string(),
        });
    true
}

/// 删除完成：解绑当前窗格（若正是该会话）。
pub fn on_session_deleted(state: &mut AppState, pane: PaneId, session_id: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() == Some(session_id) {
        target.lx.chat.session_id = None;
        target.lx.chat.messages.clear();
        return true;
    }
    false
}

/// `/` 面板确认（Enter / Tab / 鼠标点击）：按当前视图与高亮项推进。
///
/// - 命令候选 → 消费命令文本并执行（`/model`、`/resume`、`/new`）。
/// - `/model` 第一级 → 有 variants 进入第二级，否则直接完成。
/// - `/model` 第二级 → 以高亮 variant 完成选择（真正写入下一轮选择）。
/// - `/resume` → 打开高亮会话。
pub fn apply_slash_enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(panel) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.slash.clone())
    else {
        return false;
    };
    match panel {
        SlashPanel::Commands { .. } => super::pane::accept_slash_command(state, pane),
        SlashPanel::Model(picker) => match &picker.stage {
            ModelPickerStage::Models => {
                let Some(candidate) = picker.candidates.get(picker.window.active).cloned() else {
                    // 空态占位：吞掉 Enter，不误采纳。
                    return true;
                };
                let selection = candidate.selection;
                let variants = variant_keys(state, &selection);
                if variants.is_empty() {
                    finish_model_selection(state, pane, selection);
                } else {
                    let active = default_variant_index(state, &selection, &variants);
                    if let Some(target) = state.pane_mut_anywhere(pane)
                        && let Some(SlashPanel::Model(current)) = target.lx.slash.as_mut()
                    {
                        current.stage = ModelPickerStage::Variants {
                            model: selection,
                            variants,
                        };
                        current.window = crate::app::state::lx::slash::PanelWindow::new(active);
                    }
                }
                true
            }
            ModelPickerStage::Variants { model, variants } => {
                let mut selection = model.clone();
                // 采纳高亮等级（此前只完成默认 variant，未应用用户选择）。
                selection.variant = variants.get(picker.window.active).cloned();
                finish_model_selection(state, pane, selection);
                true
            }
        },
        SlashPanel::History(history) => {
            if let Some(entry) = history.entries.get(history.window.active) {
                let session_id = entry.session_id.clone();
                resume_session(state, pane, &session_id);
            }
            true
        }
    }
}

/// 模型默认 variant 在候选中的索引（缺失或不在列表则 0）。
fn default_variant_index(
    state: &AppState,
    selection: &ModelSelection,
    variants: &[String],
) -> usize {
    state
        .models
        .settings
        .providers
        .get(&selection.provider)
        .and_then(|provider| provider.models.get(&selection.model))
        .and_then(|model| model.variant.clone())
        .and_then(|variant| variants.iter().position(|item| *item == variant))
        .unwrap_or(0)
}

/// 选择完成后写入窗格选择；已保存 session 同步 DB 三列；并写回配置默认（下次启动沿用）。
fn finish_model_selection(state: &mut AppState, pane: PaneId, selection: ModelSelection) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
        target.lx.chat.selection = Some(selection.clone());
        // 切模型时旧分子清空。
        target.lx.chat.input_tokens_reset();
        // 已保存 session：选择动作排队写 DB。
        if let Some(session_id) = target.lx.chat.session_id.clone() {
            let workspace = state
                .workspace_cwd_for_pane(pane)
                .map(|path| path.to_path_buf());
            if let Some(workspace) = workspace {
                state
                    .effects
                    .push(crate::app::effects::Effect::SetChatSessionModel {
                        workspace,
                        session_id,
                        model: ModelSnapshot {
                            provider: selection.provider.clone(),
                            model: selection.model.clone(),
                            variant: selection.variant.clone(),
                        },
                    });
            }
        }
    }
    // 持久化：写回 `ai.json` defaultModel（关闭重开仍是这个模型）。
    persist_default_model(state, &selection);
    refresh_lx_status(state, pane);
}

/// 模型的 variants 键列表。
fn variant_keys(state: &AppState, selection: &ModelSelection) -> Vec<String> {
    state
        .models
        .settings
        .providers
        .get(&selection.provider)
        .and_then(|provider| provider.models.get(&selection.model))
        .map(|model| model.variants.keys().cloned().collect())
        .unwrap_or_default()
}

/// 设置页模型配置保存后更新快照（在途请求继续持有旧快照到结束）。
pub fn apply_models_changed(
    state: &mut AppState,
    settings: &crate::config::ai::AiSettings,
) -> bool {
    let changed = state.models.settings != *settings;
    state.models.settings = settings.clone();
    // 立即更新所有窗格的上限/标签（旧模型分子清理由 refresh 内部处理）。
    refresh_all_lx_status(state);
    changed
}

/// 把窗格当前选择的模型写回 `ai.json` 的 `defaultModel`（下次启动沿用）。
///
/// 只在选择变化或首次采用默认时入队，避免重复落盘。
pub fn persist_default_model(state: &mut AppState, selection: &ModelSelection) -> bool {
    if state.models.settings.default_model.as_ref() == Some(selection) {
        return false;
    }
    state
        .effects
        .push(crate::app::effects::Effect::SaveDefaultModel {
            selection: selection.clone(),
        });
    // 快照同步更新：内存立即生效，避免同一会话内再次入队。
    state.models.settings.default_model = Some(selection.clone());
    true
}

/// 刷新单窗格状态行展示数据（设置页保存、选择变更、usage 到达后调用）。
pub fn refresh_lx_status(state: &mut AppState, pane: PaneId) {
    let models = state.models.clone();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.refresh_status(&models);
    }
}

/// 启动/配置加载后刷新所有窗格状态行。
pub fn refresh_all_lx_status(state: &mut AppState) {
    for pane in state.all_pane_ids() {
        refresh_lx_status(state, pane);
    }
}

/// 窗格内下一个 generation（单调递增；从窗格计数器取）。
fn next_generation(state: &AppState, pane: PaneId) -> u64 {
    state
        .pane_anywhere(pane)
        .map(|target| target.lx.chat.generation_counter)
        .map(|counter| counter.wrapping_add(1))
        .unwrap_or(1)
        .max(1)
}

/// 显示聊天相关 toast。
fn show_chat_toast(state: &mut AppState, kind: ToastKind, message: &'static str) {
    crate::app::update::show_toast(state, Toast::new(kind, message, None, Instant::now()));
}

/// 命令采纳：`/model`、`/resume`、`/new` 的命令文本不被发送。
pub fn apply_slash_command(state: &mut AppState, pane: PaneId, command: SlashCommand) -> bool {
    match command {
        SlashCommand::Model => open_model_picker(state, pane),
        SlashCommand::Resume => open_history(state, pane),
        SlashCommand::New => new_session(state, pane),
    }
}

/// 测试钩子：直接完成模型选择（绕过两级面板交互）。
#[cfg(test)]
pub(crate) mod test_hooks {
    use super::*;

    pub(crate) fn finish_model_selection(
        state: &mut AppState,
        pane: PaneId,
        selection: ModelSelection,
    ) {
        super::finish_model_selection(state, pane, selection)
    }
}

/// 切换指定 thinking 块展开状态。
pub fn toggle_thinking_at(
    state: &mut AppState,
    pane: PaneId,
    message_index: usize,
    part_index: usize,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.toggle_reasoning(message_index, part_index);
    target.lx.chat.follow_bottom = false;
    true
}

/// 切换最近一条 assistant 消息里的最后一个 thinking 块（Ctrl+T）。
pub fn toggle_latest_thinking(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let mut found = None;
    'outer: for (message_index, message) in target.lx.chat.messages.iter().enumerate().rev() {
        for (part_index, part) in message.parts.iter().enumerate().rev() {
            if part.kind == PartKind::Reasoning {
                found = Some((message_index, part_index));
                break 'outer;
            }
        }
    }
    let Some((message_index, part_index)) = found else {
        return false;
    };
    target.lx.chat.toggle_reasoning(message_index, part_index);
    true
}

/// 滚动历史视图（正数向下）；`total`/`visible` 为渲染行数与视口行数；返回是否移动。
///
/// 状态里的 `history_scroll` 是**顶部行索引**（0 = 最早）；`follow_bottom` 表示
/// 视口钉在最新一屏（顶部行 = `total - visible`）。
pub fn scroll_lx_history(
    state: &mut AppState,
    pane: PaneId,
    delta: isize,
    total: usize,
    visible: usize,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let max_top = total.saturating_sub(visible);
    let current = if target.lx.chat.follow_bottom {
        max_top
    } else {
        target.lx.chat.history_scroll.min(max_top)
    };
    let next = (current as isize + delta).clamp(0, max_top as isize) as usize;
    let moved = next != current || target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = next;
    target.lx.chat.follow_bottom = next >= max_top;
    moved
}

/// 直接设置历史视口顶部行偏移（滚动条点击/拖拽）；`total`/`visible` 为渲染行数与视口行数。
/// 顶部行 == `total - visible` 时视为钉底跟随；返回视口是否移动。
pub fn set_lx_history_scroll(
    state: &mut AppState,
    pane: PaneId,
    offset: usize,
    total: usize,
    visible: usize,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let max_top = total.saturating_sub(visible);
    let clamped = offset.min(max_top);
    let before = target.lx.chat.history_scroll;
    let was_following = target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = clamped;
    target.lx.chat.follow_bottom = clamped >= max_top;
    before != clamped || was_following != target.lx.chat.follow_bottom
}

/// 每轮把各 lx 窗格的置顶行**抑制记忆**落状态。
///
/// 渲染层只读、不写状态（AGENTS：渲染只读），而记忆是唯一的跨帧状态，故放在
/// 事件循环里、`tui.draw` 之前由这里写；判定本身是
/// `ui::main_content::lx::history_view` 的纯函数（渲染/命中各自现算，不存第二份真相）。
/// 返回是否有窗格记忆变化。
pub fn resolve_lx_sticky_prompt(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::layout::pane_inner_rect(*rect);
        // 先只读地算出结论（借用结束后再写状态）。
        let resolved = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| crate::ui::main_content::lx::history_view(inner, &target.lx))
            .map(|view| view.suppressed);
        let Some(suppressed) = resolved else {
            continue;
        };
        let Some(target) = state.pane_mut_anywhere(*pane) else {
            continue;
        };
        if target.lx.chat.suppressed_prompt_header == suppressed {
            continue;
        }
        target.lx.chat.suppressed_prompt_header = suppressed;
        changed = true;
    }
    changed
}

/// 点击置顶行：跳到该 prompt（顶部对齐该消息起始行），并退出跟随底部。
/// `target_row` 由 `ui::main_content::lx::sticky_prompt_jump` 按当前几何给出。
pub fn jump_lx_history_to_prompt(state: &mut AppState, pane: PaneId, target_row: usize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let before = target.lx.chat.history_scroll;
    let was_following = target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = target_row;
    // 明确进入阅读态：置顶行随之按规则让位（顶行成了那条用户消息自己）。
    target.lx.chat.follow_bottom = false;
    before != target_row || was_following
}

/// 回到底部：把历史视口钉到最新一屏（跟随底部）并清除悬停态；返回是否发生变化。
pub fn jump_lx_history_bottom(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let changed = !target.lx.chat.follow_bottom || target.lx.chat.jump_bottom_hover;
    target.lx.chat.follow_bottom = true;
    target.lx.chat.jump_bottom_hover = false;
    changed
}

/// 设置「回到底部」按钮悬停态；返回是否变化。
pub fn hover_lx_jump_bottom(state: &mut AppState, pane: PaneId, hovered: bool) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.jump_bottom_hover == hovered {
        return false;
    }
    target.lx.chat.jump_bottom_hover = hovered;
    true
}

/// 推进流中窗格的 loading 转轮帧；返回是否需要重绘（仅在有流中的 lx 窗格时）。
pub fn advance_lx_loading(state: &mut AppState) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        if let Some(target) = state.pane_mut_anywhere(pane)
            && target.view == crate::app::state::PaneView::Lx
            && target.lx.chat.status.streaming
        {
            // 待命超时：提示消失（重新回到普通 `Working`）。
            if target
                .lx
                .chat
                .stop_armed_at
                .is_some_and(|at| at.elapsed() > STOP_ARM_WINDOW)
            {
                target.lx.chat.stop_armed_at = None;
            }
            target.lx.chat.loading_frame = target.lx.chat.loading_frame.wrapping_add(1);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests;
