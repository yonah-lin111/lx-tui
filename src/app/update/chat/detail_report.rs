//! `/report`：人类在 Agents 详情里显式要求子 agent 生成一份汇报。
//!
//! 与普通私聊追问共用 `SendSubagentUserInput`，只用 `notify_parent=true` 区分：
//! 报告正文仍由本轮 assistant 输出结算，闲聊原文不会进入主 agent。

use crate::app::effects::Effect;
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::show_lx_toast;
use crate::tiling::PaneId;
use crate::ui::text;

/// 提交一轮显式汇报；忙碌时拒绝且不产生效应。
pub(crate) fn submit(state: &mut AppState, pane: PaneId, note: &str) -> bool {
    let Some((agent_id, running, session_id)) = state.pane_anywhere(pane).and_then(|target| {
        let agent_id = target.lx.agent_edit_id()?.to_string();
        let run = target.lx.agents.by_agent(&agent_id)?;
        Some((
            agent_id,
            run.is_running(),
            target.lx.chat.session_id.clone(),
        ))
    }) else {
        return false;
    };
    if running {
        super::composer::clear(state, pane);
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_DETAIL_REPORT_BUSY,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return true;
    }
    let Some(session_id) = session_id else {
        return false;
    };
    let message = crate::chat::agent::report_request_message(note);
    if message.len() > crate::db::chat::MAX_MESSAGE_BYTES {
        super::composer::clear(state, pane);
        show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Warning,
                text::detail::LX_REPORT_NOTE_LIMIT,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return true;
    }
    state.effects.push(Effect::SendSubagentUserInput {
        pane,
        session_id,
        agent_id,
        text: message,
        notify_parent: true,
    });
    // 先清命令文本，避免残留；面板同时关闭。
    super::composer::clear(state, pane);
    true
}
