//! `ask` 工具的事件层接线：请求落状态、取消唤醒 loop、作答回执。
//!
//! 边界（design §5.1 / §5.4 / §5.5）：
//! - 请求只写 `pane.lx.ask` 与 [`super::ChatStreams::asks`]，**不碰 DB**：
//!   ask 的行状态由前后两个工具门槛维护；
//! - 回执通道的**唯一**持有者是 `ChatStreams`；UI 提交只推 `Effect::AnswerAsk`，
//!   由效应泵比对 `call_id` 后唤醒 loop；
//! - 任何取消路径（Esc / 关窗格 / 切会话 / 流结束）都必须唤回 `Err(Cancelled)` 并
//!   清 UI，绝不让 `ask_user` 永久挂起，也绝不用空答案冒充「用户全不选」。

use tokio::sync::oneshot;

use crate::app::state::{AppState, AskState, RequestKey};
use crate::chat::agent_loop::HostError;
use crate::chat::tools::ask::{AskAnswers, AskQuestion};

use super::{ChatStreams, PendingAsk, active_assistant_id};

/// 该窗格已有挂起提问时的稳定失败码（模型侧按工具失败处理）。
pub const ASK_BUSY_CODE: &str = "another ask is already pending";

/// 作答载荷缺题（空答案数组）时的稳定失败码。
pub const ASK_EMPTY_ANSWER_CODE: &str = "ask answer is empty";

/// 挂起一次提问；返回是否写入了 UI 状态。
///
/// 归属校验先于一切写入：旧 generation、窗格已删除的迟到请求直接回 `Cancelled`，
/// 不产生任何可见状态。
pub(crate) fn request(
    state: &mut AppState,
    streams: &mut ChatStreams,
    key: RequestKey,
    call_id: String,
    questions: Vec<AskQuestion>,
    reply: oneshot::Sender<Result<AskAnswers, HostError>>,
) -> bool {
    // 取消可能先于本事件到达：`CancelChatStream` 只置位取消信号并清面板，请求态
    // 仍是 `Streaming`。这里不补判就会重新装上面板，而 loop 已经收不到唤醒信号了。
    if active_assistant_id(state, &key).is_none()
        || streams
            .cancels
            .get(&key)
            .is_some_and(|cancel| *cancel.borrow())
    {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    }
    let Some(pane) = state.pane_mut_anywhere(key.pane) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    // 每轮最多一个挂起提问（loop 串行）。已有面板时**不覆盖**用户正在看的题。
    if pane.lx.ask.is_some() {
        let _ = reply.send(Err(HostError::Failed {
            code: ASK_BUSY_CODE.to_string(),
        }));
        return false;
    }
    let mut ask = AskState::new(key.clone(), call_id.clone(), questions);
    // 手写缓冲的软换行宽度与输入框同源；窗格 resize 时由 `resize_panes` 再精确同步。
    ask.resize_editor(
        pane.lx
            .input_width()
            .saturating_sub(crate::layout::LX_ASK_EDITOR_INSET),
        crate::layout::LX_ASK_EDITOR_ROWS,
    );
    pane.lx.ask = Some(ask);
    streams.asks.insert(key, PendingAsk { call_id, reply });
    true
}

/// 取消挂起提问：取走回执通道并回 `Err(Cancelled)`，同时清该窗格面板。
///
/// 返回 UI 是否发生变化。
pub(crate) fn cancel(state: &mut AppState, streams: &mut ChatStreams, key: &RequestKey) -> bool {
    let changed = clear(state, key);
    streams.wake_ask(key);
    changed
}

/// 只清该窗格的 ask 面板（必须属于同一个 request key）。
pub(crate) fn clear(state: &mut AppState, key: &RequestKey) -> bool {
    let Some(pane) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    let mine = pane.lx.ask.as_ref().is_some_and(|ask| ask.key == *key);
    if !mine {
        return false;
    }
    pane.lx.ask = None;
    true
}

/// 作答回执：`call_id` 与当前挂起项一致才发送；不一致视为旧面板的迟到提交，
/// **静默丢弃且不动**当前挂起项（否则会把正在等待的提问一并取消）。
///
/// 空答案数组一律拒收回 `Failed`：`answered_result` 会把缺题补成 `[]`，那正是
/// 「用户全不选」的形状，绝不能让这种半成品冒充用户作答。
pub(crate) fn answer(
    state: &mut AppState,
    streams: &mut ChatStreams,
    key: &RequestKey,
    call_id: &str,
    answers: AskAnswers,
) -> bool {
    let matched = streams
        .asks
        .get(key)
        .is_some_and(|pending| pending.call_id == call_id);
    if !matched {
        tracing::debug!(
            target: "lx::chat",
            pane = key.pane.raw(),
            call_id,
            "stale ask answer dropped"
        );
        return false;
    }
    if answers.values().any(Vec::is_empty) {
        let Some(pending) = streams.asks.remove(key) else {
            return false;
        };
        let _ = pending.reply.send(Err(HostError::Failed {
            code: ASK_EMPTY_ANSWER_CODE.to_string(),
        }));
        clear(state, key);
        return true;
    }
    let Some(pending) = streams.asks.remove(key) else {
        return false;
    };
    if pending.reply.send(Ok(answers)).is_err() {
        // loop 已不在（本轮被上游取消）：答案无处可去，但不是错误路径。
        tracing::debug!(target: "lx::chat", pane = key.pane.raw(), "ask answer receiver gone");
    }
    // 双保险：UI 侧提交时已经清过，这里兜住任何遗漏路径。
    clear(state, key);
    true
}

/// 防御 sweep（design §6.1）：面板还在但该轮已不在途时清除。
///
/// 留下悬挂面板就是留下一个没人接收的 sender：用户还能作答，答案却无处可去。
pub(crate) fn sweep_orphans(state: &mut AppState, streams: &mut ChatStreams) -> bool {
    let orphaned: Vec<RequestKey> = state
        .all_pane_ids()
        .into_iter()
        .filter_map(|pane| {
            let target = state.pane_anywhere(pane)?;
            let ask = target.lx.ask.as_ref()?;
            (!target.lx.chat.request.is_busy()).then(|| ask.key.clone())
        })
        .collect();
    let mut changed = false;
    for key in orphaned {
        changed |= cancel(state, streams, &key);
    }
    changed
}
