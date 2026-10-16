//! lx 聊天状态机更新：发送、流事件收敛、终态与 `/` 命令。
//!
//! 只产生效应/更新纯状态；IO（DB/HTTP）由事件层执行，结果经 `AppEvent` 回投。
//! 按 `(pane, session, generation)` 过滤过期结果；`Finishing` 期间不允许新一轮。

use std::time::Instant;

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
    target.lx.chat.messages.push(ChatMessageView {
        id: committed.user_message_id.clone(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: committed.turn_index,
        parts: vec![ChatPartView {
            kind: PartKind::Text,
            content: frozen_draft,
            provider_metadata_json: None,
        }],
        model_label: None,
        error_code: None,
    });
    target.lx.chat.messages.push(ChatMessageView {
        id: committed.assistant_message_id.clone(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: committed.turn_index,
        parts: Vec::new(),
        model_label: None,
        error_code: None,
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
                // 真实输入 token：同模型口径，切模型时分子清空（键由 refresh 维护）。
                if let Some(input) = usage.input {
                    target.lx.chat.last_input_tokens = Some(input as u64);
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
    // 快照当前 parts（最终 DB 写入的完整内容）。
    let parts: Vec<crate::db::chat::ChatPart> = target
        .lx
        .chat
        .messages
        .last()
        .map(|message| {
            message
                .parts
                .iter()
                .map(|part| crate::db::chat::ChatPart {
                    position: 0, // 位置由消息内顺序决定，存储层按枚举顺序写入。
                    kind: part.kind_str().into(),
                    content: part.content.clone(),
                    provider_metadata_json: part.provider_metadata_json.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    // 视图先行进入终态（DB ack 后确认；失败时保留内存 partial）。
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

/// 停止生成：显式控件/动作触发；流取消后走正常 Finishing 收尾。
pub fn stop_stream(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
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
    target.lx.chat.expanded_thinking.clear();
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
                candidate.provider == selection.provider && candidate.model == selection.model
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

/// `/history` 打开历史面板（事件层提供当前工作区分页列表）。
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

/// 历史列表回投：填充 `/history` 面板。
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
pub fn on_session_loaded(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    messages: &[ChatMessage],
    selection: Option<ModelSelection>,
) -> bool {
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
                })
                .collect(),
            model_label: message
                .model_provider
                .as_deref()
                .zip(message.model_id.as_deref())
                .map(|(provider, model)| format!("{provider}/{model}")),
            error_code: message.error_code.clone(),
        })
        .collect();
    target.lx.chat.follow_bottom = true;
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
/// - `/history` → 打开高亮会话。
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
                let Some(selection) = picker.candidates.get(picker.window.active).cloned() else {
                    // 空态占位：吞掉 Enter，不误采纳。
                    return true;
                };
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
        target.lx.chat.last_input_tokens = None;
        target.lx.chat.last_tokens_model = None;
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
        SlashCommand::History => open_history(state, pane),
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

/// 滚动历史视图（正数向下）；返回是否移动。
pub fn scroll_lx_history(state: &mut AppState, pane: PaneId, delta: isize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let total = target.lx.chat.history_height();
    let current = target.lx.chat.history_scroll as isize;
    let next = (current + delta).max(0) as usize;
    // 到底时恢复跟随底部。
    let next = next.min(total.saturating_sub(1));
    target.lx.chat.history_scroll = next;
    target.lx.chat.follow_bottom = next == 0 && delta < 0;
    true
}

#[cfg(test)]
mod tests;
