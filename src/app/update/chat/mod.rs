//! lx 聊天状态机更新：发送、流事件收敛、终态与 `/` 命令。
//!
//! 只产生效应/更新纯状态；IO（DB/HTTP）由事件层执行，结果经 `AppEvent` 回投。
//! 按 `(pane, session, generation)` 过滤过期结果；`Finishing` 期间不允许新一轮。

use std::path::Path;
use std::time::{Duration, Instant};

use crate::app::state::{
    AppState, AssistantStepView, ChatMessageView, ChatPartView, FrozenModel, InputHintLevel,
    RequestKey, RequestState, ToolStatus, ToolSummary,
};
use crate::app::toast::{Toast, ToastKind};
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::db::chat::{ModelSnapshot, TurnOutcome, Usage};
use crate::layout::PaneId;
use crate::ui::text;

mod process;
pub mod rewind;
mod session;
mod view;
mod viewport;

pub use process::{
    apply_interaction_exit, apply_process_exit, apply_process_output, apply_process_snapshot,
    commit_plan, open_task_for_selected_tool, reconcile_process_records, restore_plan,
    select_adjacent_tool, settle_tool_detail, toggle_tool_detail,
};
pub use rewind::{
    apply_enter as apply_rewind_enter, apply_input as apply_rewind_input, clear_expired_arms,
    close_panel as close_rewind_panel, move_active as move_rewind_active,
    on_failed as on_rewind_failed, on_prepared as on_rewind_prepared,
    on_session_reloaded as on_rewind_session_reloaded, on_succeeded as on_rewind_succeeded,
    open_from_command as open_rewind_from_command, set_active as set_rewind_active,
};
pub use session::{
    apply_models_changed, apply_slash_command, apply_slash_enter, close_task_page, delete_session,
    new_session, on_history_listed, on_process_listed, on_session_deleted, on_session_loaded,
    on_session_loaded_full, on_task_output, open_history, open_model_picker, open_task_page,
    open_task_page_at, persist_default_model, rename_session, resume_session, scroll_task_list,
    scroll_task_output, select_task_tab, set_task_output_scroll, stop_selected_process,
};
pub(crate) use view::view_message_of;
pub use viewport::{
    hover_lx_chat, hover_lx_jump_bottom, jump_lx_history_bottom, jump_lx_history_to_prompt,
    resolve_lx_sticky_prompt, resolve_lx_viewport_top, resolve_task_output_visible,
    scroll_lx_history, set_lx_history_scroll, toggle_lx_card,
};

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
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_ENTRY_UNAVAILABLE,
        );
        return true;
    }
    if draft.is_empty() || busy {
        // 流中 Enter 不入队、不造消息。
        return false;
    }
    // 以 `/` 开头的草稿若**不是**任何内置命令（命令面板里也没有它），不要当成
    // 普通消息发给模型——那会得到一段莫名其妙的自问自答。改为黄色警告提示，
    // 并**保留草稿**让用户改。（命令面板的采纳走 `apply_slash_enter`，不经过这里。）
    if let Some(command) = unknown_slash_command(&draft) {
        // 提示显示在**输入框上方那一行**（与 loading / jump 同一个位置），黄色。
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::chat::unknown_command(&command),
        );
        return true;
    }
    // 解析有效模型（窗格选择或配置默认）。
    let Some(selection) = selection else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    // 冻结草稿并进入 Committing：清空输入由 DB ack 成功后执行。
    // 会话落库用仓库键（worktree 与主 checkout 共用列表），执行 cwd 另行解析。
    let workspace = state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.active_workspace().repo_key().map(Path::to_path_buf));
    let Some(workspace) = workspace else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_NO_WORKSPACE,
        );
        return true;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.rewind.clear();
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
    // 先只读取出冻结草稿：历史记录要在拿窗格可变借用之前完成，避免同时借用 `state` 两处。
    let frozen_draft = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        match &target.lx.chat.request {
            RequestState::Committing {
                generation: g,
                draft,
            } if *g == generation => draft.clone(),
            _ => return false,
        }
    };
    // 记录到全局 prompt 历史：DB ack 已确认这一轮真正入库，才值得留在历史里。
    // 内存先记录（`↑` 立即可用），持久化走独立效应；两者互不阻塞。
    if state.prompt_history.record(&frozen_draft) {
        state
            .effects
            .push(crate::app::effects::Effect::RecordPromptHistory {
                text: frozen_draft.trim().to_string(),
            });
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 只清仍与冻结草稿相同的输入（期间用户编辑则保留新草稿）。
    if target.lx.input.trim() == frozen_draft.trim() {
        target.lx.clear();
    }
    // 发送成功：退出历史浏览态，否则下一次 `↑` 会从旧游标继续而非最新一条。
    target.lx.end_history_browse();
    // 展示：user 消息 + streaming assistant 占位。
    target.lx.chat.session_id = Some(committed.session_id.clone());
    target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
    // 本轮模型展示名：发送时冻结的选择（设置页缺失时回退 provider/model）。
    let model_label = target
        .lx
        .chat
        .effective_selection(&models)
        .map(|selection| models.model_display_name(&selection.provider, &selection.model));
    target.lx.chat.revision = u64::try_from(committed.revision).unwrap_or(0);
    target.lx.chat.messages.push(ChatMessageView {
        sequence: committed.user_sequence,
        id: committed.user_message_id.clone(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: committed.turn_index,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: frozen_draft,
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    });
    target.lx.chat.messages.push(ChatMessageView {
        sequence: committed.assistant_sequence,
        id: committed.assistant_message_id.clone(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: committed.turn_index,
        steps: Vec::new(),
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
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Error,
        text::LX_CHAT_COMMIT_FAILED,
    );
    refresh_lx_status(state, pane);
    true
}

/// 流 delta 回投：只更新匹配 `(pane, session, generation)` 的视图。
///
/// 整轮终态 `Finished` 由 `on_stream_finished` 处理，这里只做增量与响应收尾；
/// 旧 generation / 其它窗格的事件在状态机校验处被丢弃（不回写任何状态）。
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
                changed |= target.lx.chat.append_stream_delta(
                    part.kind,
                    &part.text,
                    part.metadata.clone(),
                );
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
            ChatDelta::ReasoningEnded { item_id } => {
                changed |= target.lx.chat.reasoning_ended(item_id.as_deref());
            }
            ChatDelta::ToolInputStarted {
                call_id,
                name,
                ordinal,
            } => {
                changed |= target.lx.chat.start_tool_input(call_id, name, *ordinal);
            }
            ChatDelta::ToolInputDelta { call_id, chunk } => {
                changed |= target.lx.chat.append_tool_input(call_id, chunk);
            }
            ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            } => {
                changed |= target.lx.chat.end_tool_input(call_id, arguments_json);
            }
            ChatDelta::ResponseFinished { stop } => {
                // 一次模型响应结束（可能是工具阶段）：不结束消息、不产生工具结果。
                changed |= target.lx.chat.finish_response(*stop);
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
    // 先按终态收尾步骤时间线：关闭仍开放的思考块、未结算工具标取消/中断（不伪造结果），
    // 再从时间线快照 parts（position 用真实分配值，不能按 enumerate 重新压平）。
    target.lx.chat.close_stream(reason);
    let parts: Vec<crate::db::chat::ChatPart> = target
        .lx
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.id == assistant_id)
        .map(crate::db::chat::chat_message_part_views)
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

/// agent loop 的 BeginTool ack 第 ② 步：把该工具步骤推进到 `Pending`（DB 已提交）。
///
/// 只作用于**在途**轮次（`(pane, session, generation)` 与当前 `RequestState::Streaming`
/// 一致）；会话切换/窗格关闭/旧 generation 一律忽略，调用方据此拒绝执行文件 IO。
/// 返回是否产生状态变化（供事件层安排下一帧渲染）。
pub fn on_tool_pending(state: &mut AppState, key: &RequestKey, call_id: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Streaming { key: active, .. } if active == key
    ) {
        return false;
    }
    let changed = target.lx.chat.mark_tool_pending(call_id);
    if changed {
        refresh_lx_status(state, key.pane);
    }
    changed
}

/// agent loop 的 BeginTool ack 第 ③ 步：`Pending -> Running`（DB running 已推进）。
///
/// 只有 pending 可推进；ack 顺序保证先 pending 后 running，此处的失败只表示状态已变，
/// 事件层据此拒绝执行文件 IO。
pub fn on_tool_running(state: &mut AppState, key: &RequestKey, call_id: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Streaming { key: active, .. } if active == key
    ) {
        return false;
    }
    let changed = target.lx.chat.mark_tool_running(call_id);
    if changed {
        refresh_lx_status(state, key.pane);
    }
    changed
}

/// agent loop 的 FinishTool ack 第 ② 步：把工具步骤结算为终态并写入摘要/错误码。
///
/// 摘要已由事件层从 `chat::tools::to_summary` 映射成纯数据；本函数不执行 IO。
/// `result_json` 是刚落库的规范化结果体（上下文回放用，UI 不铺开全文）。
/// 迟到/重复结果（已终态）不覆盖真实结果。
pub fn on_tool_settled(
    state: &mut AppState,
    key: &RequestKey,
    call_id: &str,
    status: ToolStatus,
    summary: Option<ToolSummary>,
    result_json: Option<String>,
    error: Option<String>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Streaming { key: active, .. } if active == key
    ) {
        return false;
    }
    let changed = target
        .lx
        .chat
        .settle_tool_result(call_id, status, summary, result_json, error);
    if changed {
        refresh_lx_status(state, key.pane);
    }
    changed
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

/// 在**指定窗格**的输入框上方显示一条临时提示（错误红 / 默认黄）。
///
/// lx 页的所有反馈都走这里而不是 toast：位置贴着用户刚敲字的输入框，2 秒后自动消失。
/// 展示一条 lx 反馈 toast，锚定在触发动作的窗格（`Some(pane)`）。
///
/// 用户要求「移除 lx 输入框上方的通知显示，全部改为 main content 的 toast」：
/// 提示原先贴在输入框上沿，会和 loading / 回到底部抢同一行；toast 覆盖在窗格内，
/// 只挡一小块、自动消失，且能携带三档语义色（default / warning / danger）。
pub(crate) fn show_lx_hint(
    state: &mut AppState,
    pane: PaneId,
    level: InputHintLevel,
    message: impl Into<String>,
) {
    let kind = match level {
        InputHintLevel::Error => ToastKind::Danger,
        InputHintLevel::Warning => ToastKind::Warning,
    };
    let toast = Toast::new(kind, message, Some(pane), Instant::now());
    crate::app::update::show_toast(state, toast);
}

/// 草稿是「行首 `/` 开头的一行命令」但**不是**任何内置命令时，返回那段命令文本。
///
/// 判定口径与命令面板一致（复用 [`slash_trigger`] 与 [`SlashPanel::commands_matching`]），
/// 保证「面板里能选中的」永远不会被这里拦下：
/// - 只有**单行**命令算数：多行草稿可能是用户在写 markdown 引用或路径；
/// - **带空格一律不拦**：`/q a`、`/path/to/file.rs 说明` 这种是普通输入，不是命令；
/// - 围栏代码块内不拦（`slash_trigger` 已经处理）。
///
/// 返回值保留用户输入的原文（含 `/`），由 `text::lx_unknown_command` 负责截断与拼句。
fn unknown_slash_command(draft: &str) -> Option<String> {
    use crate::app::state::lx::slash::{SlashPanel, slash_trigger};
    // 只处理「整段就是一行」的草稿。
    let line = draft.trim();
    if line.contains('\n') {
        return None;
    }
    // 与面板同口径：光标在末尾时的触发前缀。
    let query = slash_trigger(line, line.len())?;
    let trimmed = query.trim();
    // 单独一个 `/` 说明用户还没敲完；不拦（面板此时是完整列表）。
    if trimmed == "/" {
        return None;
    }
    // **带空格一律不拦**（`/q a`、`/path/to/file.rs 说明`、`/etc/hosts 是什么`）：
    // 用户在敲命令时不会带空格（面板采纳的是一条无参命令），带空格说明这是
    // 普通输入（路径、引用、`/` 开头的自然语言），按内容原样发送。
    let name = trimmed.trim_start_matches('/');
    if name.is_empty() || name.contains(char::is_whitespace) || name.contains('/') {
        return None;
    }
    // 面板能匹配到就交给面板（用户可能想选中后 Enter）。
    if !SlashPanel::commands_matching(&format!("/{name}")).is_empty() {
        return None;
    }
    // 回显整段原文（含参数），让用户确认自己敲了什么。
    Some(trimmed.to_string())
}

/// 测试钩子：直接完成模型选择（绕过两级面板交互）。
#[cfg(test)]
pub(crate) mod test_hooks {
    use crate::app::state::AppState;
    use crate::config::ai::ModelSelection;
    use crate::layout::PaneId;

    pub(crate) fn finish_model_selection(
        state: &mut AppState,
        pane: PaneId,
        selection: ModelSelection,
    ) {
        super::session::finish_model_selection(state, pane, selection)
    }
}

/// 退出「跟随底部」前把**当前视口顶部行**固化进状态（锚定视口）。
///
/// `follow_bottom = true` 时 `history_scroll` 不参与渲染（顶部行由 `total - height`
/// 现算），因此它是**过期值**（通常是 0）。任何「切换展开状态」都必须先锚定再改动：
/// 否则下一帧按过期的 0 渲染，视口会直接跳回历史最顶部——用户看到的是
/// 「点了没反应 / 展不开」（思考块恰好滚出屏幕，屏幕上只剩对话开头）。
///
/// 顶部行取自事件循环每帧写入的 [`crate::app::state::LxChat::viewport_top`]：它是
/// **上一次绘制实际用的行号**，也就是用户此刻正看着的那一行。尚未绘制过时无从锚定，
/// 直接保持原状（此时也还没有任何可见内容可点）。
///
/// 已在阅读态（`follow_bottom == false`）时无需锚定：顶部行本来就是绝对行号。
pub fn pin_lx_viewport(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if !target.lx.chat.follow_bottom {
        return false;
    }
    let Some(scroll) = target.lx.chat.viewport_top else {
        return false;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.history_scroll = scroll;
    target.lx.chat.follow_bottom = false;
    true
}

/// 切换指定 thinking 块展开状态。
///
/// 先 [`pin_lx_viewport`] 再切换：跟随底部时 `history_scroll` 是过期值，不先锚定就会
/// 让展开后的界面跳回历史最顶部（表现为「点了没反应 / 展不开」）。
pub fn toggle_thinking_at(
    state: &mut AppState,
    pane: PaneId,
    message_index: usize,
    step_index: usize,
) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(message) = target.lx.chat.messages.get(message_index) else {
        return false;
    };
    if !matches!(
        message.step_at(step_index),
        Some(crate::app::state::AssistantStepView::Part { part, .. })
            if part.kind == crate::chat::events::PartKind::Reasoning
    ) {
        return false;
    }
    pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.toggle_reasoning(message_index, step_index);
    true
}

/// 切换最近一条 assistant 消息里的最后一个 thinking 块（Ctrl+T）。
pub fn toggle_latest_thinking(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let mut found = None;
    'outer: for (message_index, message) in target.lx.chat.messages.iter().enumerate().rev() {
        for (step_index, step) in message.steps.iter().enumerate().rev() {
            if step
                .part()
                .is_some_and(|part| part.kind == PartKind::Reasoning)
            {
                found = Some((message_index, step_index));
                break 'outer;
            }
        }
    }
    let Some((message_index, step_index)) = found else {
        return false;
    };
    pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.toggle_reasoning(message_index, step_index);
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
            // 本轮耗时：时钟只在这里读（渲染层纯只读），随同一帧刷新。
            target.lx.chat.turn_elapsed_ms = target
                .lx
                .chat
                .turn_started_at_ms
                .map(|started| crate::db::now_ms().saturating_sub(started).max(0));
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests;
