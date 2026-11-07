//! 会话生命周期：`/new`、`/resume`、重命名与删除。
//!
//! 只改纯状态、只入队效应；DB/HTTP 由事件层执行。展示历史一律由
//! [`super::view::view_message_of`] 把 DB 的 parts 与 tools 按 position 合并成唯一步骤时间线。

use std::time::Instant;

use crate::app::state::lx::slash::SlashPanel;
use crate::app::state::{AppState, InputHintLevel};
use crate::app::toast::{Toast, ToastKind};
use crate::config::ai::ModelSelection;
use crate::db::chat::ChatMessage;
use crate::layout::PaneId;
use crate::ui::text;

use super::{refresh_lx_status, stop_stream, view_message_of};

#[cfg(test)]
pub(crate) use super::slash::finish_model_selection;

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
    // AI Web 会话只存在内存：`/new` 同时要求网页端开启新对话。
    let reset_ai_web = target.lx.entry == crate::app::state::LxEntry::AiWeb
        || target
            .lx
            .chat
            .session_id
            .as_deref()
            .is_some_and(super::ai_web::is_ai_web_session);
    let ai_web_site = target.lx.ai_web_site;
    target.lx.ai_web_edit_user_index = None;
    target.lx.chat.reset_session_view();
    target.lx.clear_agents();
    target.lx.compact.reset();
    target.lx.model_switch.reset();
    target.lx.clear_rewind();
    target.lx.text_paste_cancel();
    target.lx.clear_expanded_pastes();
    crate::app::update::clear_lx_selection_for(state, pane);
    if reset_ai_web {
        state
            .effects
            .push(crate::app::effects::Effect::ResetAiWebSession { site: ai_web_site });
    }
    refresh_lx_status(state, pane);
    true
}

/// `/subagents`（别名 `/agents`）：打开后台任务页并直接落在 `Agents` 页签。
///
/// 与 [`open_agent_page`](super::open_agent_page) 同一数据流：先建页面骨架，
/// 再请求事件层回投本会话的子 agent 列表（历史会话也能看到）。
pub fn open_subagents_page(state: &mut AppState, pane: PaneId) -> bool {
    if !super::open_task_page(state, pane) {
        return false;
    }
    let session = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        if let Some(page) = target.lx.task.as_mut() {
            page.select_tab(crate::app::state::TaskTab::Agents);
        }
        target.lx.chat.session_id.clone()
    };
    if let Some(parent_session) = session {
        state
            .effects
            .push(crate::app::effects::Effect::ListSubagents {
                pane,
                parent_session,
            });
    }
    true
}

/// 历史列表回投：填充 `/resume` 面板。
///
/// 当前窗格已绑定的会话打上 `❯` 标记，并作为高亮与窗口定位目标
/// （`PanelWindow::new(active)` 的最小滚动语义：它在视口外时会被拉进视口）。
pub fn on_history_listed(
    state: &mut AppState,
    pane: PaneId,
    entries: Vec<crate::app::state::lx::slash::HistoryEntry>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let bound = target.lx.chat.session_id.clone();
    let Some(SlashPanel::History(panel)) = target.lx.slash.as_mut() else {
        return false;
    };
    panel.loading = false;
    panel.entries = entries;
    panel.current = bound.as_deref().and_then(|session_id| {
        panel
            .entries
            .iter()
            .position(|entry| entry.session_id == session_id)
    });
    panel.now_ms = crate::db::now_ms();
    // 打开面板时不过滤：恢复全量视图，再按当前过滤词重算（打开时为空的恒等视图）。
    panel.filtered = (0..panel.entries.len()).collect();
    panel.clamp_window();
    let active = panel
        .current
        .and_then(|index| panel.filtered.iter().position(|item| *item == index))
        .unwrap_or(0);
    panel.window = crate::app::state::lx::slash::PanelWindow::new(active);
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
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_SESSION_BUSY,
        );
        return true;
    }
    state
        .effects
        .push(crate::app::effects::Effect::LoadChatSession {
            pane,
            session_id: session_id.to_string(),
        });
    // 面板先行关闭并进入历史加载态；列表数据到达后填充。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
        target.lx.chat.loading_session = true;
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
    on_session_loaded_full(state, pane, session_id, messages, &[], 0, selection)
}

/// 带独立撤销提示与会话 revision 的会话加载（生产入口）。
pub fn on_session_loaded_full(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    messages: &[ChatMessage],
    notices: &[crate::db::chat::NoticeRecord],
    revision: i64,
    selection: Option<ModelSelection>,
) -> bool {
    let models = state.models.clone();
    let project_root = state.project_root_for_pane(pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.loading_session = false;
    let session_changed = target.lx.chat.session_id.as_deref() != Some(session_id);
    if session_changed {
        // Agents 面板属于会话：不清理会把上一会话的 run、计数和 ^d 目标带过来。
        target.lx.clear_agents();
        target.lx.chat.main_stopped = false;
        target.lx.chat.stop_armed_at = None;
        target.lx.chat.session_title = None;
        target.lx.chat.session_title_palette = 0;
        if let Some(page) = target.lx.task.as_mut() {
            page.agents_window = Default::default();
            page.agents_focus = None;
        }
    }
    target.lx.chat.session_id = Some(session_id.to_string());
    // 切会话：先清空 compact 与 model_switch 状态（列表随后由加载回执回填）。
    target.lx.compact.reset();
    target.lx.model_switch.reset();
    target.lx.clear_rewind();
    target.lx.text_paste_cancel();
    target.lx.clear_expanded_pastes();
    // 切换会话：未执行的命令与排队项不得漂移到新会话。
    target.lx.chat.pending_shells.clear();
    target.lx.chat.queue.clear();
    if let Some(selection) = selection {
        target.lx.chat.selection = Some(selection);
    } else if session_changed {
        // `/resume` 到一个没有模型列的会话（如纯本地命令会话）：必须清掉上一会话残留的
        // 选择，否则状态行显示的模型与历史实际对应的模型脱节。同会话重载保留现有选择。
        target.lx.chat.selection = None;
    }
    target.lx.chat.messages = messages
        .iter()
        .map(|message| view_message_of(message, &models))
        .collect();
    target.lx.chat.message_times = messages
        .iter()
        .map(|message| (message.external_id.clone(), message.created_at_ms))
        .collect();
    target.lx.set_project_root(project_root);
    target.lx.chat.notices = notices
        .iter()
        .map(crate::app::state::RewindNoticeView::from_record)
        .collect();
    target.lx.chat.revision = u64::try_from(revision).unwrap_or(0);
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
    // 压缩边界由随后的 `set_checkpoints` 回填后再次重算。
    target.lx.chat.recompute_context_tokens(&target.lx.compact);
    target.lx.chat.follow_bottom = true;
    target.lx.chat.thinking_toggled.clear();
    target.lx.chat.turn_started_at_ms = None;
    target.lx.chat.turn_elapsed_ms = None;
    // 展开状态与键盘选择按稳定 ID 定位；切会话后默认折叠、不选中任何卡片。
    target.lx.chat.detail_expanded.clear();
    target.lx.chat.report_expand_steps.clear();
    target.lx.chat.selected_tool = None;
    target.lx.chat.selected_compact = None;
    target.lx.chat.plan = None;
    // 计划：从本会话最后一条**成功**的 update_plan 结果恢复（失败/中断不覆盖）。
    // 先克隆视图，避免在可变借用期间再借 `state`。
    let restored_messages = target.lx.chat.messages.clone();
    refresh_lx_status(state, pane);
    super::restore_plan(state, pane, &restored_messages);
    // 历史会话：spawn 卡片的状态可从已加载的汇报行回填。
    super::subagent::refresh_subagent_cards(state, pane);
    crate::app::update::clear_lx_selection_for(state, pane);
    true
}

/// 重命名会话。
pub fn rename_session(state: &mut AppState, pane: PaneId, session_id: &str, title: &str) -> bool {
    super::title::cancel_session_title(state, session_id);
    if super::ai_web::is_ai_web_session(session_id) {
        let workspace = state.chat_session_key_for_pane(pane).unwrap_or_default();
        state
            .effects
            .push(crate::app::effects::Effect::RenameChatSession {
                workspace,
                session_id: session_id.to_string(),
                title: title.to_string(),
            });
        let panes = state.all_pane_ids();
        for p in panes {
            crate::app::update::on_session_renamed(state, p, session_id, title);
        }
        return true;
    }
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
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
    super::title::cancel_session_title(state, session_id);
    // 在途请求先取消（事件层等 finish 后再删除）。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && target.lx.chat.session_id.as_deref() == Some(session_id)
    {
        stop_stream(state, pane);
        // 本地命令不占用 RequestState，必须单独停止；删除后迟到回执不得写回。
        super::user_shell::cancel_user_shells(state, pane);
    }
    let workspace = if super::ai_web::is_ai_web_session(session_id) {
        state.chat_session_key_for_pane(pane).unwrap_or_default()
    } else {
        let Some(workspace) = state.chat_session_key_for_pane(pane) else {
            return false;
        };
        workspace
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
        target.lx.chat.reset_session_view();
        target.lx.clear_agents();
        target.lx.compact.reset();
        target.lx.model_switch.reset();
        target.lx.clear_rewind();
        crate::app::update::clear_lx_selection_for(state, pane);
        return true;
    }
    false
}

/// 在**指定窗格**内展示一条 lx 反馈（错误红 / 警告黄）。
///
/// lx 页的所有反馈都走这里，落在**该窗格自己的 toast 槽**里，2 秒后自动消失。
///
/// 用户要求「lx 页的 toast 都要在 lx 中显示，不要用全局的」：全局 toast 只有一个槽，
/// 锚点容器放不下时还会降级到屏幕顶部居中、横跨整个窗口；lx 的反馈只属于某个 lx 窗格，
/// 越界就失去指代（也会被别的页的提示互相覆盖）。
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
    crate::app::update::show_lx_toast(state, pane, toast);
}
