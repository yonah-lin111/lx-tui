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
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::db::chat::{ModelSnapshot, TurnOutcome, Usage};
use crate::tiling::PaneId;
use crate::ui::text;

pub mod ask;
pub mod btw;
pub mod compact;
pub(crate) mod composer;
mod connect;
mod detail_compact;
mod detail_report;
mod detail_rewind;
mod fork;
pub mod mcp;
pub mod model_switch;
mod process;
pub mod recap;
pub mod report_fill;
pub mod rewind;
mod session;
mod slash;
mod steer;
mod stop;
pub(crate) mod subagent;
mod subagent_cards;
mod task_page;
pub mod title;
pub mod user_shell;
mod view;
mod viewport;
pub mod web;

pub use ask::{
    apply_ask_click, apply_ask_paste, apply_ask_wheel, apply_input as apply_ask_input, submit_all,
};
pub use btw::{
    AcceptedBtw, BtwToolSettlement, BtwToolUpdate, apply_btw_buffer_deltas,
    apply_input as apply_btw_input, btw_message_parts as btw_parts, enter_btw_edit, leave_btw_edit,
    new_assistant_message as new_btw_assistant_message, on_accepted as on_btw_accepted,
    on_delta as on_btw_delta, on_failed as on_btw_failed, on_finished as on_btw_finished,
    on_qa_list_loaded, on_recipient_started as on_btw_recipient_started,
    on_thread_deleted as on_btw_thread_deleted, on_thread_loaded as on_btw_thread_loaded,
    on_tool_state as on_btw_tool_state, open_btw_tab, scroll_btw_detail, select_btw_qa,
    set_task_btw_scroll, submit_current as submit_btw,
    sync_search_selection as sync_btw_search_selection, toggle_btw_card, try_submit_btw_command,
};
pub use compact::{
    compact_command, on_compact_empty, on_compact_failed, on_compact_finished,
    on_compact_revert_failed, on_compact_reverted, set_checkpoints as set_compact_checkpoints,
    toggle_checkpoint,
};
pub use connect::{
    apply_connect_overlay_key, hover_connect, on_connect_failed, on_models_refresh_failed,
    on_models_refreshed, on_preset_connected, on_preset_disconnected, on_preset_oauth_failed,
    on_preset_oauth_prompt, open_connect_panel, scroll_connect, set_connect_selection,
    set_connect_viewport,
};
pub use detail_compact::{
    DetailCompactResult, handle_finished as on_detail_compact_finished,
    on_started as on_detail_compact_started,
};
pub use detail_rewind::{
    on_failed as on_detail_rewind_failed, on_prepared as on_detail_rewind_prepared,
    on_reload_failed as on_detail_rewind_reload_failed, on_succeeded as on_detail_rewind_succeeded,
};
pub use fork::{on_fork_completed, on_fork_failed, open_fork_panel};
pub use mcp::{
    build_panel_entries as build_mcp_panel_entries, mcp_root_for_pane, open_mcp_panel,
    refresh_all_open_mcp_panels, refresh_pane_mcp_panel, toggle_mcp_entry,
};
pub use model_switch::{set_switches as set_model_switches, toggle_model_switch};
pub use process::{
    activate_tool_card, apply_interaction_exit, apply_process_exit, apply_process_output,
    apply_process_snapshot, commit_plan, open_task_for_selected_tool, reconcile_process_records,
    restore_plan, select_adjacent_tool, settle_tool_detail, toggle_tool_detail,
};
pub use recap::{
    apply_recap, cancel_pending_recap, on_terminal_focus_gained, on_terminal_focus_lost,
    recap_command, recap_failed, set_message_recaps, take_pending_recap, tick_recap,
};
pub use report_fill::{
    BtwReportReady, SubagentReportReady, fill_main_input, on_btw_report_ready,
    on_subagent_report_ready,
};
pub use rewind::{
    apply_enter as apply_rewind_enter, apply_input as apply_rewind_input,
    cancel_on_scroll as cancel_rewind_on_scroll, clear_expired_arms,
    close_panel as close_rewind_panel, move_active as move_rewind_active,
    on_failed as on_rewind_failed, on_prepared as on_rewind_prepared,
    on_session_reloaded as on_rewind_session_reloaded, on_succeeded as on_rewind_succeeded,
    open_from_command as open_rewind_from_command, set_active as set_rewind_active,
};
pub(crate) use session::show_lx_hint;
pub use session::{
    delete_session, new_session, on_history_listed, on_session_deleted, on_session_loaded,
    on_session_loaded_full, rename_session, resume_session,
};
#[cfg(test)]
pub(crate) use slash::finish_model_selection;
pub(crate) use slash::refresh_slash_panel;
use slash::unknown_slash_command;
pub use slash::{
    accept_slash_command, apply_models_changed, apply_slash_command, apply_slash_enter,
    open_history, open_model_picker, persist_default_model, remember_model_used,
};
pub use steer::on_finish_ack;
pub use subagent::{
    apply_deltas as apply_subagent_deltas, flush_subagent_cards, on_report_batch_committed,
    on_subagent_auto_compact, on_subagent_delta, on_subagent_finished, on_subagent_loaded,
    on_subagent_started, on_subagent_tool_pending, on_subagent_tool_running,
    on_subagent_tool_settled, open_agent_page, refresh_subagent_cards,
    request_subagent_cards_refresh, restore_selected_task_agent, scroll_agents_detail,
    select_task_agent, set_subagent_list, set_task_agents_scroll, stop_selected_subagent,
    toggle_agent_card,
};
pub use task_page::{
    close_task_page, on_process_listed, on_task_output, open_task_page, open_task_page_at,
    scroll_task_list, scroll_task_output, select_task_tab, set_task_list_scroll,
    set_task_output_scroll, stop_selected_process,
};
pub use title::{
    apply_generated_session_title, cancel_session_title, flush_prompt_title_action,
    maybe_schedule_session_title, on_prompt_block_title_generated, take_pending_session_title,
};
pub use user_shell::{
    apply_user_shell_snapshot, cancel_user_shells, on_user_shell_begun, on_user_shell_cancelled,
    on_user_shell_rejected, on_user_shell_spawn_failed, on_user_shell_spawned,
    on_user_shell_terminal, queue_chat_while_shell_runs, queue_shell, try_submit_user_shell,
};
pub(crate) use view::view_message_of;
pub use viewport::{
    hover_lx_chat, hover_lx_jump_bottom, jump_lx_history_bottom, jump_lx_history_to_prompt,
    resolve_ask_list_window, resolve_task_output_visible, scroll_lx_history, set_lx_history_scroll,
    toggle_latest_thinking, toggle_lx_card, toggle_thinking_at,
};
pub(crate) use viewport::{
    invalidate_task_agent_rows, resolve_lx_history, resolve_lx_rewind_center,
    resolve_task_agents_visible, resolve_task_btw_visible,
};
pub use web::{
    AI_WEB_DEEPSEEK_SESSION_PREFIX, AI_WEB_GEMINI_SESSION_PREFIX, AI_WEB_GROK_SESSION_PREFIX,
    ai_web_remote_session, ai_web_remote_session_id, is_ai_web_session, on_ai_web_image_generated,
    on_ai_web_original_image_download_failed, on_ai_web_original_image_downloaded,
    on_ai_web_session_bound, on_ai_web_session_loaded, rewind_ai_web,
    trigger_ai_web_original_image_download, try_send_ai_web,
};

/// 尝试发送：校验 Native 入口、非空草稿、请求空闲与有效模型后冻结快照并入队效应。
///
/// 返回是否产生状态变化。DB ack 成功后才清草稿；失败保留草稿可重试。
pub fn try_send(state: &mut AppState, pane: PaneId) -> bool {
    // shell 分派必须早于入口/模型/busy 检查：本地命令不需要模型，模型忙时照常执行。
    if user_shell::try_submit_user_shell(state, pane) {
        return true;
    }
    // `/rename` 是纯元数据更新，同样早于入口/模型/busy 检查。
    if slash::try_submit_rename_command(state, pane) {
        return true;
    }
    // `/btw` 在排队/steer 之前拦截：生成期间也必须进入旁路，不注入主任务。
    if btw::try_submit_btw_command(state, pane) {
        return true;
    }
    // 仅本地命令在跑：普通聊天冻结入队，等结果落库后再发送。
    if user_shell::queue_chat_while_shell_runs(state, pane) {
        return true;
    }
    // 模型正在生成：Enter 作为 steer 进入待注入队列，不打断当前请求。
    if steer::try_submit(state, pane) {
        return true;
    }
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
        // 压缩进行中不能发送；同会话其它 pane 在压缩/跑轮次时，本 pane 发送会推进
        // revision，让那次压缩在 commit 阶段失败。
        let session_busy = target.lx.chat.session_id.as_deref().is_some_and(|session| {
            state.all_pane_ids().into_iter().any(|other| {
                other != pane
                    && state.pane_anywhere(other).is_some_and(|pane_state| {
                        pane_state.lx.chat.session_id.as_deref() == Some(session)
                            && (pane_state.lx.compact.is_running()
                                || pane_state.lx.chat.request.is_busy())
                    })
            })
        });
        let busy =
            !target.lx.chat.request.can_send() || target.lx.compact.is_running() || session_busy;
        let selection = target.lx.chat.effective_selection(&state.models);
        let session_id = target.lx.chat.session_id.clone();
        let generation = next_generation(state, pane);
        (entry, draft, busy, selection, session_id, generation)
    };
    let (entry, draft, busy, selection, session_id, generation) = context;
    // AI Web 走纯内存网页会话：不校验本地模型/工作区，不写 SQLite。
    if entry == crate::app::state::LxEntry::AiWeb {
        return web::try_send_ai_web(state, pane, draft, busy, generation);
    }
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
    // 以 `/` 开头的草稿若不是任何内置命令，不作为普通消息发给模型，保留草稿并黄色提示。
    if let Some(command) = unknown_slash_command(&draft) {
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
    let reference_root = state
        .workspace_cwd_for_pane(pane)
        .unwrap_or_else(|| workspace.clone());
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
            crate::chat::prepare::SkillNotice::Truncated { name } => text::skills::truncated(name),
            crate::chat::prepare::SkillNotice::LoadFailed { name, reason } => {
                text::skills::read_failed(name, reason)
            }
        };
        show_lx_hint(state, pane, InputHintLevel::Warning, message);
    }
    // 发送已通过全部校验：取消同会话在途 recap，避免旧回顾挂到推进后的时间线上。
    let _ = recap::cancel_pending_recap(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.clear_rewind();
    // 钉住本轮实际使用的模型（包括窗格选择失效后的回退结果）：流启动会按
    // `effective_selection` 重新解析，若这里不覆盖失效选择，实际发送模型可能与
    // DB 中冻结的模型不一致。
    target.lx.chat.selection = Some(selection.clone());
    target.lx.chat.generation_counter = generation;
    target.lx.chat.main_stopped = false;
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
            attachments: prepared.attachments,
            model: ModelSnapshot {
                provider: frozen.provider.clone(),
                model: frozen.model.clone(),
                variant: frozen.variant.clone(),
            },
        });
    // 实际发送过的模型记入全局最近使用（只改 recentModels，不改 defaultModel）。
    remember_workspace_model(state, pane, &selection);
    remember_model_used(state, &selection);
    // 立即进入流中态：loading 行（转轮 + Esc 待命提示）从按下 Enter 起就可见，
    // 不必等首个 delta。
    refresh_lx_status(state, pane);
    true
}

/// DB begin_turn 成功回执：绑定 session/message、清冻结草稿、启动网络流。
/// 若期间用户继续编辑，新草稿不被清掉（只清与冻结草稿相同的输入）。
pub fn on_turn_committed(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    committed: &crate::db::chat::TurnCommitted,
) -> bool {
    let models = state.models.clone();
    // 先只读取出冻结草稿与提升条数：历史记录要在拿窗格可变借用之前完成，
    // 避免同时借用 `state` 两处。
    let (frozen_draft, promoting_steers) = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        match &target.lx.chat.request {
            RequestState::Committing {
                generation: g,
                draft,
            } if *g == generation => (draft.clone(), target.lx.chat.promoting_steers),
            _ => return false,
        }
    };
    // 新轮开始：取消在途 recap（用户已继续对话，旧回顾不再贴当前上下文）。
    // 必须放在 generation 校验之后：迟到的旧回执不能取消新一次 recap。
    let _ = recap::cancel_pending_recap(state, pane);
    // 记录到全局 prompt 历史：DB ack 已确认这一轮真正入库，才值得留在历史里。
    // 只有无附件的纯文本草稿才记录，带附件轮次不向纯文本历史记录（避免 ↑ 召回缺图草稿）。
    // 内存先记录（`↑` 立即可用），持久化走独立效应；两者互不阻塞。
    if committed.attachments.is_empty() && state.prompt_history.record(&frozen_draft) {
        state
            .effects
            .push(crate::app::effects::Effect::RecordPromptHistory {
                text: frozen_draft.trim().to_string(),
            });
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let is_first_turn = target.lx.chat.session_id.is_none() && committed.turn_index == 0;
    let draft_for_title = frozen_draft.clone();
    if promoting_steers > 0 {
        // pending steer 提升为普通下一轮：消费已提交的队首批次，但不清输入框
        //（input 里可能是用户在收尾期间新写的草稿）。
        let take = promoting_steers.min(target.lx.chat.pending_steers.len());
        target.lx.chat.pending_steers.drain(..take);
        target.lx.chat.promoting_steers = 0;
    } else if target.lx.input.trim() == frozen_draft.trim() {
        // 只清仍与冻结草稿相同的输入（期间用户编辑则保留新草稿）。
        target.lx.clear();
    }
    // 发送成功：退出历史浏览态，否则下一次 `↑` 会从旧游标继续而非最新一条。
    target.lx.end_history_browse();
    // 展示：user 消息 + streaming assistant 占位。
    target.lx.chat.session_id = Some(committed.session_id.clone());
    target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
    target.lx.chat.main_stopped = false;
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
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: committed.turn_index,
        shell: None,
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
        reports: Vec::new(),
        attachments: committed.attachments.clone(),
    });
    target.lx.chat.messages.push(ChatMessageView {
        sequence: committed.assistant_sequence,
        id: committed.assistant_message_id.clone(),
        kind: "chat".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: committed.turn_index,
        shell: None,
        steps: Vec::new(),
        model_label,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    target
        .lx
        .chat
        .set_message_time(&committed.user_message_id, committed.created_at_ms);
    target
        .lx
        .chat
        .set_message_time(&committed.assistant_message_id, committed.created_at_ms);
    target.lx.chat.follow_bottom = true;
    // 新轮从空聚合槽开始：上一轮终态若被丢弃，不能把旧 usage 带进第一次采样。
    target.lx.chat.reset_step_usage();
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
    if is_first_turn {
        title::maybe_schedule_session_title(state, pane, committed, &draft_for_title);
    }
    // 同一会话可能同时开在多个窗格：发送成功后把实际使用的模型同步给其它窗格，
    // 避免 A 已换模型、B 仍显示旧模型。
    if let Some(selection) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.selection.clone())
        && let Some(selection) = state.models.validated_selection(Some(&selection))
    {
        sync_session_selection(state, pane, &committed.session_id, &selection);
    }
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
    let promoting_steers = target.lx.chat.promoting_steers;
    target.lx.chat.request = RequestState::Idle;
    target.lx.chat.promoting_steers = 0;
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Error,
        text::LX_CHAT_COMMIT_FAILED,
    );
    if promoting_steers > 0 {
        // 提升失败不自动重试：pending 退回输入框，避免停在 Idle + pending 的悬空状态。
        steer::restore_pending_to_input(state, pane);
    }
    refresh_lx_status(state, pane);
    true
}

/// 流 delta 回投：只更新匹配 `(pane, session, generation)` 的视图。
/// 整轮终态由 `on_stream_finished` 处理；旧 generation / 其它窗格事件在此丢弃。
pub fn on_stream_delta(state: &mut AppState, key: &RequestKey, deltas: &[ChatDelta]) -> bool {
    let project_root = state.project_root_for_pane(key.pane);
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if project_root.is_some() {
        target.lx.chat.project_root.clone_from(&project_root);
    }
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
                // 上下文占用只认单步 input + output：同一次响应内分帧到达的字段先在
                // step_usage 里合并，绝不跨工具步累加 input（多步循环不再撑爆 100%）。
                target.lx.chat.absorb_usage_delta(usage);
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
        subagent::request_subagent_cards_refresh(state, key.pane);
    }
    changed
}

/// 流终态回投（完整 deltas 先经 on_stream_delta 消费；本函数只处理结束）。
/// 取消/失败/完成均先转 Finishing，再入队最终 DB 写入。
pub fn on_stream_finished(
    state: &mut AppState,
    key: &RequestKey,
    reason: &StopReason,
    usage: Usage,
    billing: Usage,
) -> bool {
    // 失焦期间完成的轮次把自动 recap 计时起点推到本轮结束（对齐 codex 30min）。
    let recap_unfocused = state.recap_due_at.is_some();
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
    let completed_turn_index = target
        .lx
        .chat
        .messages
        .iter()
        .find(|message| message.id == assistant_id)
        .map(|message| message.turn_index);
    if let Some(message) = target
        .lx
        .chat
        .messages
        .iter_mut()
        .find(|message| message.id == assistant_id)
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
    // 终态 usage 是最后一次响应的权威单步值：同步上下文占用（未知时保留流中已知值）。
    if let Some(context) = usage.context_tokens() {
        target.lx.chat.context_tokens = Some(context);
    }
    if matches!(reason, StopReason::Cancelled) {
        target.lx.chat.main_stopped = true;
    }
    if matches!(reason, StopReason::Complete)
        && !web::is_ai_web_session(&key.session)
        && let Some(turn_index) = completed_turn_index
    {
        target.lx.recap.on_turn_completed(turn_index);
    }
    // AI Web 是纯内存会话：消息已在上面结算，直接回 Idle，不入队 DB 终态写入。
    if web::is_ai_web_session(&key.session) {
        target.lx.chat.request = RequestState::Idle;
        refresh_lx_status(state, key.pane);
        return true;
    }
    target.lx.chat.request = RequestState::Finishing {
        key: key.clone(),
        reason: reason.clone(),
    };
    if recap_unfocused {
        state.recap_due_at = Some(Instant::now() + super::recap::RECAP_IDLE);
    }
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
            billing,
        });
    true
}

/// 空闲且没有用户排队提交时，请求后台报告 drain。
///
/// 同一窗格只挂一个 effect；用户输入优先的判定与事件层 `parent_idle` 保持一致。
pub(crate) fn request_report_drain_if_idle(state: &mut AppState, pane: PaneId) -> bool {
    let ready = state.pane_anywhere(pane).is_some_and(|target| {
        matches!(target.lx.chat.request, RequestState::Idle) && target.lx.chat.queue.is_empty()
    });
    if !ready
        || state.effects.iter().any(|effect| {
            matches!(
                effect,
                crate::app::effects::Effect::DrainSubagentReports { pane: queued }
                    if *queued == pane
            )
        })
    {
        return false;
    }
    state
        .effects
        .push(crate::app::effects::Effect::DrainSubagentReports { pane });
    true
}

/// Esc 停止的待命窗口：窗口内再按一次才真正停止（与撤销待命统一为 3 秒）。
pub const STOP_ARM_WINDOW: Duration = Duration::from_secs(3);

/// 有可停止工作时按 `Esc`：若存在待生效的 pending steer，单次 `Esc` 优先按 LIFO
/// 弹出最近一条并回显输入框；否则首次只**待命**并显示统一提示，窗口内再按一次
/// **打开停止面板**（不立即执行停止）。
pub fn escape_stop(state: &mut AppState, pane: PaneId) -> bool {
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.ask.is_none())
        && steer::recall_last(state, pane)
    {
        return true;
    }
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let availability = crate::app::state::StopAvailability::from_lx(&target.lx);
    if !availability.any() {
        return false;
    }
    let now = Instant::now();
    let armed = target
        .lx
        .chat
        .stop_armed_at
        .is_some_and(|at| now.saturating_duration_since(at) <= STOP_ARM_WINDOW);
    if !armed {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        target.lx.chat.stop_armed_at = Some(now);
        target.lx.rewind.armed_at = None;
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_STOP_ARMED);
        return true;
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.stop_armed_at = None;
    target.lx.slash = Some(crate::app::state::SlashPanel::Stop(Box::new(
        crate::app::state::StopPanel::new(availability),
    )));
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
            target.lx.chat.main_stopped = true;
            state
                .effects
                .push(crate::app::effects::Effect::CancelChatStream { key });
            true
        }
        RequestState::Committing { generation, .. } => {
            // 等 DB ack 后直接落 interrupted（不发 HTTP）：标记待取消。
            target.lx.chat.main_stopped = true;
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
    let project_root = state.project_root_for_pane(key.pane);
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if project_root.is_some() {
        target.lx.chat.project_root.clone_from(&project_root);
    }
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
    let project_root = state.project_root_for_pane(key.pane);
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if project_root.is_some() {
        target.lx.chat.project_root.clone_from(&project_root);
    }
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
        if target.lx.entry == crate::app::state::LxEntry::AiWeb
            || target
                .lx
                .chat
                .session_id
                .as_deref()
                .is_some_and(web::is_ai_web_session)
        {
            web::apply_ai_web_status(&mut target.lx);
        }
    }
}

/// 启动/配置加载后刷新所有窗格状态行。
pub fn refresh_all_lx_status(state: &mut AppState) {
    for pane in state.all_pane_ids() {
        refresh_lx_status(state, pane);
    }
}

/// 记录某窗格所属工作区最近使用的模型；新建 lx 窗格/标签继承。
pub fn remember_workspace_model(
    state: &mut AppState,
    pane: PaneId,
    selection: &crate::config::ai::ModelSelection,
) {
    let Some(index) = state
        .workspaces
        .iter()
        .position(|workspace| workspace.tabs.iter().any(|tab| tab.pane(pane).is_some()))
    else {
        return;
    };
    let workspace = &mut state.workspaces[index];
    if workspace.lx_model.as_ref() != Some(selection) {
        workspace.lx_model = Some(selection.clone());
    }
}

/// 同一会话被多个窗格打开时，把模型选择同步到其它窗格并刷新状态行。
///
/// 只改绑定同一 `session_id` 的窗格；源窗格已由调用方更新。
pub fn sync_session_selection(
    state: &mut AppState,
    source: PaneId,
    session_id: &str,
    selection: &crate::config::ai::ModelSelection,
) {
    let panes = state.all_pane_ids();
    for pane in panes {
        if pane == source {
            continue;
        }
        let needs = state.pane_anywhere(pane).is_some_and(|target| {
            target.lx.chat.session_id.as_deref() == Some(session_id)
                && target.lx.chat.selection.as_ref() != Some(selection)
        });
        if !needs {
            continue;
        }
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.chat.selection = Some(selection.clone());
        }
        remember_workspace_model(state, pane, selection);
        refresh_lx_status(state, pane);
    }
}

/// 刷新当前工作区**当前标签**的 lx 状态行（切/建工作区、切标签后调用）。
///
/// 不遍历后台标签，避免把尚未展示的一次性 `omitted_turns_notice` 刷新掉。
pub fn refresh_active_tab_lx_status(state: &mut AppState) {
    let panes = state.active_tab().layout.pane_ids();
    for pane in panes {
        refresh_lx_status(state, pane);
    }
}

/// 模型偏好（default/recent）变化后，刷新依赖全局兜底的空白 lx 窗格。
///
/// 只刷无消息窗格：它们不可能持有一次性省略提示；有历史的窗格等下一次自身事件刷新。
pub fn refresh_fallback_lx_status(state: &mut AppState) {
    let panes = state.all_pane_ids();
    for pane in panes {
        let needs = state.pane_anywhere(pane).is_some_and(|target| {
            target.view == crate::app::state::PaneView::Lx
                && target.lx.chat.selection.is_none()
                && target.lx.chat.messages.is_empty()
        });
        if needs {
            refresh_lx_status(state, pane);
        }
    }
}

/// 窗格内下一个 generation（单调递增；从窗格计数器取）。
pub(super) fn next_generation(state: &AppState, pane: PaneId) -> u64 {
    state
        .pane_anywhere(pane)
        .map(|target| target.lx.chat.generation_counter)
        .map(|counter| counter.wrapping_add(1))
        .unwrap_or(1)
        .max(1)
}

/// 测试钩子：直接完成模型选择（绕过两级面板交互）。
#[cfg(test)]
pub(crate) mod test_hooks {
    use crate::app::state::AppState;
    use crate::config::ai::ModelSelection;
    use crate::tiling::PaneId;

    pub(crate) fn finish_model_selection(
        state: &mut AppState,
        pane: PaneId,
        selection: ModelSelection,
    ) {
        super::slash::finish_model_selection(state, pane, selection)
    }
}

/// 退出「跟随底部」前把**当前视口顶部行**固化进状态（锚定视口）。
///
/// `follow_bottom = true` 时 `history_scroll` 是过期值（顶部行由 `total - height` 现算）；
/// 切换展开状态前必须用 [`crate::app::state::LxChat::viewport_top`]（上次实际绘制行号）
/// 锚定，否则下一帧按 0 渲染会跳回历史顶部。已在阅读态时无需锚定。
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

/// 推进流中窗格的 loading 转轮帧、刷新压缩耗时；返回是否需要重绘。
///
/// 流中/本地命令在跑推进扫描帧与本轮耗时；`/compact` 摘要无流式输出，同样按帧刷新 `elapsed_ms`。
pub fn advance_lx_loading(state: &mut AppState) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        if target.view != crate::app::state::PaneView::Lx {
            continue;
        }
        let streaming = target.lx.chat.status.streaming
            || target.lx.chat.request.is_busy()
            || target.lx.chat.has_running_shell();
        let compacting = target.lx.compact.is_running();
        let recapping = target.lx.recap.pending.is_some();
        // 仍有子 agent 在跑：推进同一套 loading 帧（Agents 详情动画与等待行）。
        let agents_running = target.lx.agents.runs.iter().any(|run| run.is_running());
        // 详情会话（子 agent / BTW 线程）的手动压缩同样没有流式输出，不能依赖
        // 子 agent 自身在跑；否则压缩期间整段界面冻结。
        let detail_animating = target.lx.any_detail_compacting();
        // BTW 面板可见时，生成中或线程历史加载中都需要推进帧；关掉面板不空转重绘。
        let btw_running = target.lx.is_btw_tab()
            && target
                .lx
                .btw
                .active_thread()
                .is_some_and(|thread| thread.is_loading() || !thread.loaded);
        if !streaming
            && !compacting
            && !recapping
            && !agents_running
            && !btw_running
            && !detail_animating
        {
            continue;
        }
        // ask 等待用户作答：主会话本轮耗时与扫描帧冻结，子 agent 动画不受影响。
        let paused = target.lx.ask.is_some();
        let mut advanced = false;
        // 压缩耗时：摘要请求没有任何流式输出，这一行只有计时能证明它还在跑。
        if compacting {
            target.lx.compact.elapsed_ms = target
                .lx
                .compact
                .started_at_ms
                .map(|started| crate::db::now_ms().saturating_sub(started).max(0));
            advanced = true;
        }
        if detail_animating {
            target.lx.tick_detail_compacting(crate::db::now_ms());
            advanced = true;
        }
        if streaming {
            // 待命超时：提示消失（重新回到普通 `Working`）。
            if target
                .lx
                .chat
                .stop_armed_at
                .is_some_and(|at| at.elapsed() > STOP_ARM_WINDOW)
            {
                target.lx.chat.stop_armed_at = None;
                advanced = true;
            }
            if !paused {
                // 本轮耗时：时钟只在这里读（渲染层纯只读），随同一帧刷新。
                target.lx.chat.turn_elapsed_ms = target
                    .lx
                    .chat
                    .turn_started_at_ms
                    .map(|started| crate::db::now_ms().saturating_sub(started).max(0));
                advanced = true;
            }
        }
        // 扫描帧两种场景共用；ask 暂停期间只有压缩或子 agent 在跑才继续推进。
        if !paused || compacting || recapping || agents_running {
            target.lx.chat.loading_frame = target.lx.chat.loading_frame.wrapping_add(1);
            if agents_running {
                let frame = target.lx.chat.loading_frame;
                for run in &mut target.lx.agents.runs {
                    if run.is_running() {
                        run.chat.loading_frame = frame;
                    }
                }
            }
            advanced = true;
        }
        // BTW 的帧独立推进：主轮次空闲时工具卡片/loading 扫描仍要动。
        if btw_running {
            target.lx.btw.advance_loading_frames();
            advanced = true;
        }
        changed |= advanced;
    }
    changed
}

#[cfg(test)]
mod tests;
