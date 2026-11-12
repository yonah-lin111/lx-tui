//! BTW 事件回投：提交接受、回答者生命周期、工具状态与线程恢复。

use crate::app::effects::{BtwEffect, Effect};
use crate::app::state::lx::btw::BtwQaSummary;
use crate::app::state::{AppState, ChatMessageView, InputHintLevel};
use crate::app::update::show_lx_hint;
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::tiling::PaneId;
use crate::ui::text;

use super::submit_current;

/// 事件层回投：QA 列表已加载。
pub fn on_qa_list_loaded(
    state: &mut AppState,
    pane: PaneId,
    parent_session_id: &str,
    qa: Vec<BtwQaSummary>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() != Some(parent_session_id) {
        return false;
    }
    target.lx.btw.qa = qa;
    target.lx.btw.picker_loading = false;
    // 列表到达前可能已经显示 loading 占位面板；空列表也应保留面板让用户看到空态。
    let picker_open = target.lx.btw.picker_open();
    if target.lx.btw.active.is_none()
        && let Some(last) = target.lx.btw.qa.last()
    {
        let qa_id = last.qa_id.clone();
        target.lx.btw.select_qa(&qa_id);
    }
    if target.lx.btw.qa.is_empty() {
        // 没有正式 QA 时不能把 `/btw prompt` 的正文丢掉：还回主输入，用户可先提问。
        if let Some(prompt) = target.lx.btw.pending_prompt.take() {
            target.lx.replace_input(&format!("/btw {prompt}"));
        }
        target.lx.btw.picking = None;
        target.lx.btw.pending_pick = picker_open;
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_BTW_NO_QA);
        return true;
    }
    if target.lx.btw.pending_pick {
        target.lx.btw.open_picker();
    } else if let Some(index) = target.lx.btw.picking {
        // 列表刷新后可能变短：把高亮钳回有效范围。
        target.lx.btw.picking = Some(index.min(target.lx.btw.qa.len() - 1));
    }
    let qa_id = target
        .lx
        .btw
        .active
        .clone()
        .filter(|_| !target.lx.btw.qa.is_empty())
        .unwrap_or_default();
    let has_qa = !qa_id.is_empty();
    let pending_prompt = target.lx.btw.pending_prompt.take();
    // composer 是懒创建的：拿到 QA、建立线程之后才存在，此时再同步 `@` 根与实例。
    crate::app::update::sync_lx_mention_root(state, pane);
    if has_qa {
        state.effects.push(Effect::Btw(BtwEffect::LoadThread {
            pane,
            parent_session_id: parent_session_id.to_string(),
            qa_id,
        }));
        // `/btw prompt` 在 QA 列表到达后才真正提交（只认命令暂存，不误发普通草稿）。
        // 首次打开时 `try_submit_btw_command` 还没有活动线程可写，必须在这里把
        // 暂存正文填进刚建立的 composer，再走正常提交路径。
        if let Some(prompt) = pending_prompt {
            if let Some(thread) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.btw.active_thread_mut())
            {
                thread.composer.editor_mut().replace_input(&prompt);
            }
            submit_current(state, pane);
        }
    }
    true
}

/// 事件层回投：线程历史已加载。
#[allow(clippy::too_many_arguments)]
pub fn on_thread_loaded(
    state: &mut AppState,
    pane: PaneId,
    parent_session_id: &str,
    qa_id: &str,
    thread_id: Option<String>,
    child_session_id: Option<String>,
    title: String,
    messages: Vec<crate::db::chat::ChatMessage>,
    targets: Vec<crate::app::state::lx::BtwTargetView>,
    loaded: bool,
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 迟到回投必须校验父会话与当前 QA：否则切会话/切 QA 后旧加载结果会
    // 覆盖新视图，甚至把 active 改回旧 QA。
    if target.lx.chat.session_id.as_deref() != Some(parent_session_id)
        || target.lx.btw.active.as_deref() != Some(qa_id)
    {
        return false;
    }
    let thread = target.lx.btw.ensure_thread(qa_id);
    if thread.loaded {
        return false;
    }
    thread.chat.session_id = child_session_id.clone();
    thread.chat.messages = messages
        .iter()
        .map(|message| crate::app::update::view_message_of(message, &models))
        .collect();
    thread.thread_id = thread_id;
    thread.child_session_id = child_session_id;
    thread.title = title;
    // 线程加载与首次提交可能竞态：加载结果是提交前的快照，不能把刚清除的
    // 过期目标再覆盖回来；以本地已提交的目标集合（submit_seq > 0）为准。
    if !targets.is_empty() && thread.submit_seq == 0 {
        thread.targets = targets;
    }
    thread.loaded = loaded;
    thread.rows_dirty = true;
    thread.follow = true;
    // 线程建立后 composer 才存在：补一次 `@` 扫描根与实例同步。
    crate::app::update::sync_lx_mention_root(state, pane);
    true
}

/// 提交被接受后的回投字段（事件层 → 应用层）。
#[derive(Debug, Clone)]
pub struct AcceptedBtw {
    pub submit_id: String,
    pub thread_id: String,
    pub child_session_id: String,
    pub user_message_id: String,
    pub text: String,
}

/// 事件层回投：提交已被接受。
pub fn on_accepted(state: &mut AppState, pane: PaneId, event: AcceptedBtw) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(thread) = target.lx.btw.active_thread_mut() else {
        return false;
    };
    if thread.pending_submit.as_ref().map(|(id, _)| id.as_str()) != Some(event.submit_id.as_str()) {
        return false;
    }
    let revision = thread.pending_submit.as_ref().map(|(_, rev)| *rev);
    thread.pending_submit = None;
    thread.thread_id = Some(event.thread_id.clone());
    thread.child_session_id = Some(event.child_session_id.clone());
    if revision == Some(thread.composer.revision()) {
        thread.composer.clear();
    }
    thread.chat.messages.push(ChatMessageView {
        id: event.user_message_id.clone(),
        sequence: 0,
        kind: "chat".to_string(),
        role: "user".to_string(),
        status: "complete".to_string(),
        turn_index: 0,
        shell: None,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: PartKind::Text,
                content: event.text.clone(),
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
        attachments: Vec::new(),
    });
    thread.rows_dirty = true;
    thread.follow = true;
    true
}

/// 新建一个 streaming assistant 投影；BTW 的事件层缓冲与 UI 共用同一形状。
pub fn new_assistant_message(id: &str, label: &str) -> ChatMessageView {
    ChatMessageView {
        id: id.to_string(),
        sequence: 0,
        kind: "chat".to_string(),
        role: "assistant".to_string(),
        status: "streaming".to_string(),
        turn_index: 0,
        shell: None,
        steps: Vec::new(),
        model_label: Some(label.to_string()),
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

/// 事件层回投：某个回答者开始运行。
pub fn on_recipient_started(
    state: &mut AppState,
    child_session_id: &str,
    assistant_message_id: &str,
    label: &str,
) -> bool {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session_mut(child_session_id) else {
            continue;
        };
        if thread
            .chat
            .messages
            .iter()
            .any(|message| message.id == assistant_message_id)
        {
            return false;
        }
        thread.pending_recipients = thread.pending_recipients.saturating_sub(1);
        thread
            .chat
            .messages
            .push(new_assistant_message(assistant_message_id, label));
        thread.rows_dirty = true;
        return true;
    }
    false
}

/// 事件层回投：旁路流增量。
pub fn on_delta(
    state: &mut AppState,
    child_session_id: &str,
    assistant_message_id: &str,
    deltas: &[ChatDelta],
) -> bool {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session_mut(child_session_id) else {
            continue;
        };
        thread.chat.stream_assistant = Some(assistant_message_id.to_string());
        let changed = super::super::subagent::apply_deltas(&mut thread.chat, deltas);
        thread.chat.stream_assistant = None;
        if changed {
            thread.rows_dirty = true;
            if thread.follow {
                thread.scroll = thread.total_rows;
            }
        }
        return changed;
    }
    false
}

/// 事件层回投：旁路工具状态。
pub fn on_tool_state(
    state: &mut AppState,
    child_session_id: &str,
    assistant_message_id: &str,
    update: BtwToolUpdate<'_>,
) -> bool {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session_mut(child_session_id) else {
            continue;
        };
        thread.chat.stream_assistant = Some(assistant_message_id.to_string());
        let changed = match update {
            BtwToolUpdate::Pending(call_id) => thread.chat.mark_tool_pending(call_id),
            BtwToolUpdate::Running(call_id) => thread.chat.mark_tool_running(call_id),
            BtwToolUpdate::Settled(settlement) => thread.chat.settle_tool_result(
                settlement.call_id,
                settlement.status,
                settlement.summary,
                settlement.result_json,
                settlement.error,
            ),
        };
        thread.chat.stream_assistant = None;
        if changed {
            thread.rows_dirty = true;
        }
        return changed;
    }
    false
}

/// 工具状态回投的三种形态。
pub enum BtwToolUpdate<'a> {
    Pending(&'a str),
    Running(&'a str),
    Settled(Box<BtwToolSettlement<'a>>),
}

/// 工具结算是最大变体：装箱避免整个事件枚举被撑大。
pub struct BtwToolSettlement<'a> {
    pub call_id: &'a str,
    pub status: crate::app::state::ToolStatus,
    pub summary: Option<crate::app::state::ToolSummary>,
    pub result_json: Option<String>,
    pub error: Option<String>,
}

/// 事件层回投：某个回答者结束。
pub fn on_finished(
    state: &mut AppState,
    child_session_id: &str,
    assistant_message_id: &str,
    reason: &StopReason,
    usage: crate::db::chat::Usage,
    duration_ms: i64,
) -> bool {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session_mut(child_session_id) else {
            continue;
        };
        thread.chat.stream_assistant = Some(assistant_message_id.to_string());
        thread.chat.close_stream(reason);
        if let Some(message) = thread
            .chat
            .messages
            .iter_mut()
            .find(|message| message.id == assistant_message_id)
        {
            message.status = match reason {
                StopReason::Complete => "complete".to_string(),
                StopReason::Cancelled => "interrupted".to_string(),
                StopReason::Failed { .. } => "failed".to_string(),
            };
            message.error_code = match reason {
                StopReason::Failed { code } => Some(code.clone()),
                _ => None,
            };
            message.input_tokens = usage.input;
            message.output_tokens = usage.output;
            message.cache_read_tokens = usage.cache_read;
            message.duration_ms = Some(duration_ms);
        }
        thread.chat.stream_assistant = None;
        thread.rows_dirty = true;
        return true;
    }
    false
}

/// 事件层回投：提交/回答者启动失败（保留草稿）。
pub fn on_failed(state: &mut AppState, pane: PaneId, qa_id: Option<&str>, message: &str) -> bool {
    // 失败不会再有回答者开始：清掉等待标记，loading 必须立即消失、草稿保留可重试。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        let qa = qa_id
            .map(str::to_owned)
            .or_else(|| target.lx.btw.active.clone());
        if let Some(thread) = qa.as_deref().and_then(|qa| target.lx.btw.thread_mut(qa)) {
            thread.pending_submit = None;
            thread.pending_recipients = 0;
        }
    }
    show_lx_hint(state, pane, InputHintLevel::Error, message);
    true
}

/// 读取某回答者 assistant 消息的完整 parts（结算前快照）。
pub fn btw_message_parts(
    state: &AppState,
    child_session_id: &str,
    assistant_message_id: &str,
) -> Vec<crate::db::chat::ChatPart> {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session(child_session_id) else {
            continue;
        };
        if let Some(message) = thread
            .chat
            .messages
            .iter()
            .find(|message| message.id == assistant_message_id)
        {
            return crate::db::chat::chat_message_part_views(message);
        }
    }
    Vec::new()
}
