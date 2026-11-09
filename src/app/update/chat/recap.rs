//! recap 的状态调度：自动失焦计时、手动 `/recap`、请求入队、回执回填与取消。
//!
//! 纯状态更新（零 IO）：网络请求与 DB 写入由事件层按 `Effect` / `AppEvent` 执行。

use std::time::{Duration, Instant};

use crate::app::effects::Effect;
use crate::app::state::lx::{MessageRecapView, RecapPending};
use crate::app::state::{
    AppState, AssistantStepView, ChatMessageView, InputHintLevel, Lx, LxEntry, PaneView,
};
use crate::chat::events::PartKind;
use crate::chat::recap::RecapExchange;
use crate::db::chat::MessageRecap;
use crate::layout::PaneId;
use crate::ui::text;

use super::{refresh_lx_status, show_lx_hint};

/// 自动 recap 的失焦等待时长（对齐 codex 30 分钟）。
pub const RECAP_IDLE: Duration = Duration::from_secs(30 * 60);
/// 忙碌时重新检查的间隔（对齐 codex 30 秒）。
pub const RECAP_RETRY: Duration = Duration::from_secs(30);

/// 自动触发的检查结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Eligibility {
    Ready,
    /// 当前忙（流式/压缩/子 agent/队列/同会话其它窗格），稍后重试。
    Retry,
    /// 轮次不足、无会话、非 Native 等，不重试。
    Ineligible,
}

/// 调度失败原因；手动命令映射为文案，自动触发静默丢弃。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScheduleError {
    Pending,
    Busy,
    NoSession,
    NoWorkspace,
    NoModel,
    NoHistory,
    CompactAtBottom,
}

impl ScheduleError {
    fn message(self) -> &'static str {
        match self {
            Self::Pending => text::LX_RECAP_PENDING,
            Self::Busy => text::LX_RECAP_BUSY,
            Self::NoSession | Self::NoHistory => text::LX_RECAP_EMPTY,
            Self::NoWorkspace => text::LX_CHAT_NO_WORKSPACE,
            Self::NoModel => text::LX_RECAP_NO_MODEL,
            Self::CompactAtBottom => text::LX_RECAP_AFTER_COMPACT,
        }
    }
}

/// 终端失焦：开始 30 分钟计时。
pub fn on_terminal_focus_lost(state: &mut AppState, now: Instant) {
    state.recap_due_at = Some(now + RECAP_IDLE);
}

/// 终端重新聚焦：取消计时与所有在途自动请求；迟到回执按 generation 丢弃。
pub fn on_terminal_focus_gained(state: &mut AppState) -> bool {
    let had_deadline = state.recap_due_at.take().is_some();
    let mut changed = false;
    for pane in state.all_pane_ids() {
        if let Some(target) = state.pane_mut_anywhere(pane)
            && target
                .lx
                .recap
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.manual)
        {
            changed |= target.lx.recap.cancel_pending();
        }
    }
    had_deadline || changed
}

/// 定时 tick：刷新在途耗时；到期后检查并调度自动 recap。
pub fn tick_recap(state: &mut AppState, now: Instant) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        if let Some(target) = state.pane_mut_anywhere(pane) {
            changed |= target.lx.recap.tick_pending();
        }
    }
    let Some(due) = state.recap_due_at else {
        return changed;
    };
    if now < due {
        return changed;
    }
    // 先清空，避免 next_deadline 持续返回过去时刻导致事件循环忙转。
    state.recap_due_at = None;
    let pane = state.active_tab().layout.focus();
    match auto_eligibility(state, pane) {
        Eligibility::Ready => {
            if schedule_recap(state, pane, false).is_ok() {
                changed = true;
            }
        }
        Eligibility::Retry => {
            state.recap_due_at = Some(now + RECAP_RETRY);
            changed = true;
        }
        Eligibility::Ineligible => {}
    }
    changed
}

/// 手动 `/recap`：校验、提示并调度一次请求。
pub fn recap_command(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if target.view != PaneView::Lx || target.lx.entry == LxEntry::AiWeb {
        return false;
    }
    match schedule_recap(state, pane, true) {
        Ok(()) => true,
        Err(error) => {
            show_lx_hint(state, pane, InputHintLevel::Warning, error.message());
            true
        }
    }
}

/// 取消指定窗格及同会话其它窗格的在途 recap（新消息 / `/rewind` / 切会话）。
pub fn cancel_pending_recap(state: &mut AppState, pane: PaneId) -> bool {
    let session_id = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone());
    let mut changed = false;
    for other in state.all_pane_ids() {
        if let Some(target) = state.pane_mut_anywhere(other) {
            let same_session = match session_id.as_deref() {
                Some(session) => target.lx.chat.session_id.as_deref() == Some(session),
                None => other == pane,
            };
            if same_session {
                changed |= target.lx.recap.cancel_pending();
            }
        }
    }
    changed
}

/// 校验回执代次并消费 pending；不匹配时保留原状态。
pub fn take_pending_recap(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    message_id: &str,
    generation: u64,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() != Some(session_id) {
        return false;
    }
    let Some(pending) = target.lx.recap.pending.as_ref() else {
        return false;
    };
    if pending.generation != generation || pending.message_id != message_id {
        return false;
    }
    target.lx.recap.pending = None;
    true
}

/// 回执失败：清 pending；手动提示，自动静默。
pub fn recap_failed(state: &mut AppState, pane: PaneId, manual: bool) -> bool {
    let had = state
        .pane_mut_anywhere(pane)
        .is_some_and(|target| target.lx.recap.pending.take().is_some());
    if manual {
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_RECAP_FAILED);
    }
    had || manual
}

/// 回执成功：写入同会话全部窗格的内存视图并更新上次 recap 进度。
pub fn apply_recap(state: &mut AppState, pane: PaneId, record: &MessageRecap) -> bool {
    let Some(session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return false;
    };
    let mut changed = false;
    for other in state.all_pane_ids() {
        if let Some(target) = state.pane_mut_anywhere(other)
            && target.lx.chat.session_id.as_deref() == Some(session_id.as_str())
        {
            target.lx.recap.entries.insert(
                record.message_id.clone(),
                MessageRecapView::from_record(record),
            );
            target.lx.recap.pending = None;
            target.lx.recap.last_recap_turn = Some(record.end_turn);
            changed = true;
        }
    }
    changed
}

/// 会话加载：回填 recap 映射与自动触发进度。
pub fn set_message_recaps(state: &mut AppState, pane: PaneId, recaps: &[MessageRecap]) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let had_entries = !target.lx.recap.entries.is_empty();
    let completed: std::collections::BTreeSet<i64> = target
        .lx
        .chat
        .messages
        .iter()
        .filter(|message| message.role == "assistant" && message.status == "complete")
        .map(|message| message.turn_index)
        .collect();
    let last_turn = recaps.iter().map(|recap| recap.end_turn).max();
    target.lx.recap.reset_for_session(completed, last_turn);
    for record in recaps {
        target.lx.recap.entries.insert(
            record.message_id.clone(),
            MessageRecapView::from_record(record),
        );
    }
    had_entries || !recaps.is_empty()
}

fn auto_eligibility(state: &AppState, pane: PaneId) -> Eligibility {
    if !state.models.settings.recap_enabled {
        return Eligibility::Ineligible;
    }
    let Some(target) = state.pane_anywhere(pane) else {
        return Eligibility::Ineligible;
    };
    if target.view != PaneView::Lx || target.lx.entry != LxEntry::Native {
        return Eligibility::Ineligible;
    }
    if target.lx.chat.session_id.is_none() || target.lx.recap.pending.is_some() {
        return Eligibility::Ineligible;
    }
    // 在途运行先判：计时到期时轮次/compact 条件可能暂时不满足，30 秒后需复检，
    // 不能因为暂时不足 3 轮或底部是 compact 就永久丢掉本次自动触发。
    if target_busy(&target.lx) || session_busy(state, pane) {
        return Eligibility::Retry;
    }
    if !target.lx.recap.auto_eligible() || compact_at_bottom(&target.lx) {
        return Eligibility::Ineligible;
    }
    let fallback = target.lx.chat.effective_selection(&state.models);
    if state
        .models
        .resolve_recap_model(fallback.as_ref())
        .is_none()
    {
        return Eligibility::Ineligible;
    }
    Eligibility::Ready
}

fn target_busy(lx: &Lx) -> bool {
    lx.chat.request.is_busy()
        || lx.compact.is_running()
        || lx.chat.has_running_shell()
        || lx.agents.running_count() > 0
        || !lx.chat.queue.is_empty()
}

/// 时间线底部已是上下文压缩条目（晚于最后一条完成态 assistant 回复）时禁止 recap：
/// recap 挂在最后一条 assistant 消息尾部，若底部有更新的 compact，回顾会落到压缩块上方。
fn compact_at_bottom(lx: &Lx) -> bool {
    let Some(checkpoint) = lx.compact.checkpoints.iter().rev().find(|c| c.active) else {
        return false;
    };
    let last_assistant_sequence = lx
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant" && message.status == "complete")
        .map(|message| message.sequence)
        .unwrap_or(i64::MIN);
    checkpoint.sequence > last_assistant_sequence
}

fn session_busy(state: &AppState, pane: PaneId) -> bool {
    let Some(session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.as_deref())
    else {
        return false;
    };
    state.all_pane_ids().into_iter().any(|other| {
        state.pane_anywhere(other).is_some_and(|target| {
            target.lx.chat.session_id.as_deref() == Some(session_id)
                && (target.lx.chat.request.is_busy()
                    || target.lx.compact.is_running()
                    || target.lx.recap.pending.is_some())
        })
    })
}

/// 构造并登记一次 recap 请求。
fn schedule_recap(state: &mut AppState, pane: PaneId, manual: bool) -> Result<(), ScheduleError> {
    let (workspace, session_id, message_id, end_turn, end_sequence, prompt, model, model_label) = {
        let Some(target) = state.pane_anywhere(pane) else {
            return Err(ScheduleError::NoSession);
        };
        if target.lx.recap.pending.is_some() {
            return Err(ScheduleError::Pending);
        }
        if target_busy(&target.lx) || session_busy(state, pane) {
            return Err(ScheduleError::Busy);
        }
        let Some(session_id) = target.lx.chat.session_id.clone() else {
            return Err(ScheduleError::NoSession);
        };
        let Some(workspace) = state.chat_session_key_for_pane(pane) else {
            return Err(ScheduleError::NoWorkspace);
        };
        let Some(last) = target
            .lx
            .chat
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "assistant" && message.status == "complete")
        else {
            return Err(ScheduleError::NoHistory);
        };
        if compact_at_bottom(&target.lx) {
            return Err(ScheduleError::CompactAtBottom);
        }
        let fallback = target.lx.chat.effective_selection(&state.models);
        let Some(frozen) = state.models.resolve_recap_model(fallback.as_ref()) else {
            return Err(ScheduleError::NoModel);
        };
        let compact_summary = target
            .lx
            .compact
            .checkpoints
            .iter()
            .rev()
            .find(|checkpoint| checkpoint.active)
            .map(|checkpoint| checkpoint.summary.clone());
        let exchanges = recap_exchanges(&target.lx.chat.messages);
        let Some(prompt) =
            crate::chat::recap::build_recap_prompt(compact_summary.as_deref(), &exchanges)
        else {
            return Err(ScheduleError::NoHistory);
        };
        let model_label = state
            .models
            .model_display_name(&frozen.provider, &frozen.model);
        (
            workspace,
            session_id,
            last.id.clone(),
            last.turn_index,
            last.sequence,
            prompt,
            frozen,
            model_label,
        )
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return Err(ScheduleError::NoSession);
    };
    let generation = target.lx.recap.allocate_generation();
    target.lx.recap.pending = Some(RecapPending {
        generation,
        manual,
        message_id: message_id.clone(),
        started_at: Instant::now(),
        elapsed_ms: 0,
    });
    state.effects.push(Effect::GenerateRecap {
        pane,
        workspace,
        session_id,
        message_id,
        generation,
        manual,
        end_turn,
        end_sequence,
        prompt,
        model,
        model_label,
    });
    refresh_lx_status(state, pane);
    Ok(())
}

/// 从消息时间线提取最近 8 组用户/助手可见正文（不读思考、工具与子 agent 报告）。
fn recap_exchanges(messages: &[ChatMessageView]) -> Vec<RecapExchange> {
    let mut groups: Vec<(i64, String, String)> = Vec::new();
    for message in messages {
        if message.kind != "chat" || message.status != "complete" {
            continue;
        }
        if message.role != "user" && message.role != "assistant" {
            continue;
        }
        // 子 agent 报告自动轮以 user 消息落库，但正文是报告信封，不属于用户输入。
        if !message.reports.is_empty() {
            continue;
        }
        let text = message_text(message);
        if text.trim().is_empty() {
            continue;
        }
        if let Some(last) = groups
            .last_mut()
            .filter(|group| group.0 == message.turn_index)
        {
            if message.role == "user" {
                last.1.push_str(&text);
            } else {
                last.2.push_str(&text);
            }
        } else {
            let user = if message.role == "user" {
                text.clone()
            } else {
                String::new()
            };
            let assistant = if message.role == "assistant" {
                text
            } else {
                String::new()
            };
            groups.push((message.turn_index, user, assistant));
        }
    }
    let mut exchanges: Vec<RecapExchange> = groups
        .into_iter()
        .filter(|(_, user, assistant)| !user.trim().is_empty() && !assistant.trim().is_empty())
        .rev()
        .take(8)
        .map(|(_, user, assistant)| RecapExchange { user, assistant })
        .collect();
    exchanges.reverse();
    exchanges
}

fn message_text(message: &ChatMessageView) -> String {
    message
        .steps
        .iter()
        .filter_map(|step| match step {
            AssistantStepView::Part { part, .. }
                if part.kind == PartKind::Text && !part.content.is_empty() =>
            {
                Some(part.content.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
