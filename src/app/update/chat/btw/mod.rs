//! 旁路问答（`/btw`）的应用层更新：打开页签、提交、事件回投与输入路由。
//!
//! 只改内存状态并登记 [`Effect::Btw`]；数据库与网络由事件层执行。
//!
//! 文件按职责拆分：本模块管生命周期（打开/提交/停止/归属切换），`input` 管键盘
//! 与 composer 浮层，`events` 管事件回投；测试在 `tests.rs`。

use crate::app::effects::{BtwEffect, BtwSubmission, BtwTargetChoice, Effect};
use crate::app::state::lx::btw::{BtwFocus, BtwTargetView};
use crate::app::state::{AppState, InputHintLevel, Lx, TaskTab};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::{show_lx_hint, show_lx_toast};
use crate::chat::events::PartKind;
use crate::tiling::PaneId;
use crate::ui::text;

mod events;
mod input;

pub use events::{
    AcceptedBtw, BtwToolSettlement, BtwToolUpdate, btw_message_parts, on_accepted, on_delta,
    on_failed, on_finished, on_qa_list_loaded, on_recipient_started, on_thread_loaded,
    on_tool_state,
};
pub use input::{apply_input, select_btw_slash};

/// 打开 BTW 页签并请求 QA 列表。
pub fn open_btw_tab(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx {
        return false;
    }
    if target.lx.entry != crate::app::state::LxEntry::Native {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_BTW_ENTRY_UNAVAILABLE,
        );
        return false;
    }
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_BTW_NO_SESSION,
        );
        return false;
    };
    let page = target.lx.task.get_or_insert_with(Default::default);
    page.tab = TaskTab::Btw;
    page.agent_reply = None;
    page.agent_reply_target = None;
    state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
        pane,
        parent_session_id: session_id.clone(),
    }));
    // 同时刷新 subagent 实例列表：BTW 的 `@` 需要运行中与已结束实例都能解析。
    state.effects.push(Effect::ListSubagents {
        pane,
        parent_session: session_id,
    });
    true
}

pub(super) fn is_btw_list_command(text: &str) -> bool {
    let line = text.trim();
    line.eq_ignore_ascii_case("/btwlist") || line.eq_ignore_ascii_case("/btw-list")
}

/// 打开 QA 选择器：页签已开时直接就地打开（必要时补拉列表），否则先打开页签。
pub fn open_btw_list(state: &mut AppState, pane: PaneId) -> bool {
    let already = state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.is_btw_tab());
    if already {
        let Some(parent_session_id) = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.chat.session_id.clone())
        else {
            return false;
        };
        state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
            pane,
            parent_session_id,
        }));
    } else if !open_btw_tab(state, pane) {
        return false;
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.btw.picker_loading = true;
    if target.lx.btw.qa.is_empty() {
        // 列表还没到：先立起二级面板的 loading 占位，拿到列表后再换成候选。
        target.lx.btw.pending_pick = true;
    } else {
        target.lx.btw.open_picker();
    }
    true
}

/// 选择器确认/点击：切到目标 QA 并加载它的线程历史。
pub fn select_btw_qa(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let Some(qa_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.btw.qa.get(index))
        .map(|summary| summary.qa_id.clone())
    else {
        return false;
    };
    switch_qa(state, pane, &qa_id)
}

/// 滚轮滚动当前 BTW 线程的对话区（与 Agents 详情同一套贴尾/脱离跟随语义）。
pub fn scroll_btw_detail(state: &mut AppState, pane: PaneId, delta: isize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !target.lx.is_btw_tab() {
        return false;
    }
    let Some(thread) = target.lx.btw.active_thread_mut() else {
        return false;
    };
    let visible = thread.visible.max(1);
    let max = thread.total_rows.saturating_sub(visible);
    let current = if thread.follow {
        max
    } else {
        thread.scroll.min(max)
    };
    let mut next = (current as isize)
        .saturating_add(delta)
        .clamp(0, max as isize) as usize;
    // 向下滚到最后一屏附近直接贴尾，避免差一行永远回不到 follow。
    if delta > 0 && next.saturating_add(1) >= max {
        next = max;
    }
    let next_follow = next >= max;
    let moved = next != current || thread.follow != next_follow;
    if !moved {
        return false;
    }
    thread.scroll = next;
    thread.follow = next_follow;
    true
}

/// 直接设置 BTW 对话区顶部行（滚动条点击/拖拽）；语义与 Agents 详情一致。
pub fn set_task_btw_scroll(state: &mut AppState, pane: PaneId, offset: usize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !target.lx.is_btw_tab() {
        return false;
    }
    let Some(thread) = target.lx.btw.active_thread_mut() else {
        return false;
    };
    let max = thread.total_rows.saturating_sub(thread.visible.max(1));
    let clamped = offset.min(max);
    let before = thread.scroll;
    let was_following = thread.follow;
    thread.scroll = clamped;
    thread.follow = clamped >= max;
    before != clamped || was_following != thread.follow
}

pub(super) fn switch_qa(state: &mut AppState, pane: PaneId, qa_id: &str) -> bool {
    let Some(parent_session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return false;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.btw.select_qa(qa_id);
    target.lx.btw.close_picker();
    target.lx.btw.focus = BtwFocus::Composer;
    state.effects.push(Effect::Btw(BtwEffect::LoadThread {
        pane,
        parent_session_id,
        qa_id: qa_id.to_string(),
    }));
    crate::app::update::sync_lx_mention_root(state, pane);
    true
}

/// `/btw [prompt]` 命令入口：打开页签，把 prompt 放入当前 QA 的 composer 并提交。
///
/// 裸 `/btw` 只回显 `"/btw "` 与灰色参数提示，不发送。
pub fn try_submit_btw_command(state: &mut AppState, pane: PaneId) -> bool {
    let Some((input, session_id)) = state.pane_anywhere(pane).and_then(|target| {
        if target.view != crate::app::state::PaneView::Lx || target.lx.shell_mode() {
            return None;
        }
        Some((target.lx.input.clone(), target.lx.chat.session_id.clone()))
    }) else {
        return false;
    };
    let Some((prompt, bare)) = crate::app::state::lx::slash::parse_btw_command(&input) else {
        return false;
    };
    if bare {
        if let Some(target) = state.pane_mut_anywhere(pane)
            && let Some(prefix) = crate::app::state::SlashCommand::Btw.echo_prefix()
        {
            if target.lx.input != prefix {
                target.lx.replace_input(prefix);
            } else {
                target.lx.cursor = prefix.len();
                target.lx.clear_text_selection();
            }
            target.lx.slash = None;
        }
        open_btw_tab(state, pane);
        return true;
    }
    let Some(session_id) = session_id else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_BTW_NO_SESSION,
        );
        return true;
    };
    if !open_btw_tab(state, pane) {
        return true;
    }
    let prompt = prompt.to_string();
    // 命令本身就是一次提交：主输入框内容不保留。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.clear();
    }
    if let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    {
        thread.composer.editor_mut().replace_input(&prompt);
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.btw.pending_prompt = true;
    }
    // QA 列表由事件回投后才有；先登记一次延迟提交，列表到达时立即发出。
    state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
        pane,
        parent_session_id: session_id,
    }));
    true
}

/// 进入 BTW 页签后按需加载 QA 列表（重复调用幂等）。
pub fn ensure_loaded(state: &mut AppState, pane: PaneId) -> bool {
    let Some((is_btw, session_id)) = state
        .pane_anywhere(pane)
        .map(|target| (target.lx.is_btw_tab(), target.lx.chat.session_id.clone()))
    else {
        return false;
    };
    if !is_btw {
        return false;
    }
    let Some(parent_session_id) = session_id else {
        return false;
    };
    state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
        pane,
        parent_session_id,
    }));
    true
}

/// 提交当前 QA 的草稿；返回是否登记了提交。
pub fn submit_current(state: &mut AppState, pane: PaneId) -> bool {
    let Some(session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return false;
    };
    let Some((workspace, cwd)) = state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.workspace_cwd_for_pane(pane))
        .map(|workspace| {
            let cwd = state
                .workspace_cwd_for_pane(pane)
                .unwrap_or_else(|| workspace.clone());
            (workspace, cwd)
        })
    else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_BTW_NO_SESSION);
        return false;
    };
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(qa_id) = target.lx.btw.active.clone() else {
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_BTW_NO_QA);
        return false;
    };
    let Some(thread) = target.lx.btw.active_thread() else {
        return false;
    };
    let text_value = thread.composer.text().trim().to_string();
    if text_value.is_empty() {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_BTW_EMPTY_PROMPT,
        );
        return false;
    }
    let Some(selection) = target.lx.chat.effective_selection(&state.models) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return false;
    };
    let Some(frozen) = crate::app::state::FrozenModel::resolve(&state.models, &selection) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return false;
    };
    let targets = match resolve_targets(&target.lx, &text_value) {
        Ok(targets) => targets,
        Err(message) => {
            show_lx_hint(state, pane, InputHintLevel::Warning, message);
            return false;
        }
    };
    let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    else {
        return false;
    };
    if !targets.is_empty() {
        thread.targets = targets
            .iter()
            .map(|choice| BtwTargetView {
                key: choice.agent_id.clone(),
                label: format!("{} [{}]", choice.display_nickname, choice.role_name),
                agent_id: Some(choice.agent_id.clone()),
            })
            .collect();
    }
    thread.submit_seq = thread.submit_seq.saturating_add(1);
    let submit_id = format!(
        "btw-{}-{}-{}",
        crate::db::now_ms(),
        pane.raw(),
        thread.submit_seq
    );
    let revision = thread.composer.revision();
    thread.pending_submit = Some((submit_id.clone(), revision));
    thread.pending_recipients = targets.len().max(1);
    let thread_id = thread.thread_id.clone();
    state
        .effects
        .push(Effect::Btw(BtwEffect::Submit(Box::new(BtwSubmission {
            pane,
            parent_session_id: session_id,
            qa_id,
            workspace,
            cwd,
            thread_id,
            submit_id,
            draft_revision: revision,
            text: text_value,
            targets,
            default_model: crate::db::chat::ModelSnapshot {
                provider: frozen.provider.clone(),
                model: frozen.model.clone(),
                variant: frozen.variant.clone(),
            },
            default_variant_params: frozen.variant_params.clone(),
            idle_timeout_ms: frozen.idle_timeout_ms,
        }))));
    true
}

/// 停止当前线程的活动与排队回答。
pub fn stop_current(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(qa_id) = target.lx.btw.active.clone() else {
        return false;
    };
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        return false;
    };
    let thread_id = target
        .lx
        .btw
        .active_thread()
        .and_then(|thread| thread.thread_id.clone());
    if let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    {
        thread.pending_submit = None;
        thread.pending_recipients = 0;
    }
    state.effects.push(Effect::Btw(BtwEffect::Stop {
        pane,
        parent_session_id: session_id,
        qa_id,
        thread_id,
    }));
    show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Default,
            text::LX_BTW_STOPPED,
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

pub(super) fn resolve_targets(
    target: &Lx,
    draft: &str,
) -> std::result::Result<Vec<BtwTargetChoice>, &'static str> {
    let masked = crate::app::paste::mask_pasted_text(draft);
    let mut chosen: Vec<BtwTargetChoice> = Vec::new();
    for reference in crate::app::markdown::parse_references(&masked) {
        if reference.kind != crate::app::markdown::ReferKind::Subagent {
            continue;
        }
        let raw = reference.path.trim();
        if raw.is_empty() {
            continue;
        }
        let (head, role) = match raw.rsplit_once(':') {
            Some((head, role)) => (head.trim(), Some(role.trim())),
            None => (raw, None),
        };
        let run = find_instance(target, head, role)?;
        if chosen.iter().any(|item| item.agent_id == run.agent_id) {
            continue;
        }
        chosen.push(BtwTargetChoice {
            agent_id: run.agent_id.clone(),
            display_nickname: run.nickname.clone().unwrap_or_else(|| run.agent_id.clone()),
            role_name: run.role.as_str().to_string(),
            source_child_session_id: run.child_session_id.clone(),
        });
    }
    Ok(chosen)
}

fn find_instance<'a>(
    target: &'a Lx,
    head: &str,
    role: Option<&str>,
) -> std::result::Result<&'a crate::app::state::SubagentRun, &'static str> {
    let matches = |run: &&crate::app::state::SubagentRun| {
        run.agent_id == head
            || run.nickname.as_deref() == Some(head)
            || (role.is_none_or(|role| role == run.role.as_str()) && run.role.as_str() == head)
    };
    let candidates: Vec<&crate::app::state::SubagentRun> = target
        .agents
        .runs
        .iter()
        .filter(|run| matches(run))
        .collect();
    match candidates.as_slice() {
        [run] => Ok(run),
        [] => Err(text::LX_BTW_NO_MATCHING_AGENT),
        _ => Err(text::LX_BTW_AMBIGUOUS_AGENT),
    }
}

/// 切换旁路对话里的折叠目标（思考块 / 工具卡片 / 用户消息 / 粘贴芯片）。
///
/// 与主历史一致：展开前先把贴底视口锚定到当前位置，避免展开后整段跳回顶部。
pub fn toggle_btw_card(
    state: &mut AppState,
    pane: PaneId,
    target: crate::app::state::CardTarget,
) -> bool {
    let Some(target_pane) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(thread) = target_pane.lx.btw.active_thread() else {
        return false;
    };
    let tool_present = match &target {
        crate::app::state::CardTarget::Tool {
            message_id,
            call_id,
            ..
        } => thread
            .chat
            .messages
            .iter()
            .any(|message| message.id == *message_id && message.tool(call_id).is_some()),
        _ => true,
    };
    let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    else {
        return false;
    };
    if thread.follow {
        thread.scroll = thread.total_rows.saturating_sub(thread.visible);
        thread.follow = false;
    }
    let changed = match target {
        crate::app::state::CardTarget::Tool {
            message_id,
            call_id,
            section,
        } => tool_present && thread.chat.toggle_section(&message_id, &call_id, section),
        crate::app::state::CardTarget::Thinking {
            message_index,
            step_index,
        } => {
            let is_reasoning = thread
                .chat
                .messages
                .get(message_index)
                .and_then(|message| message.step_at(step_index))
                .is_some_and(|step| {
                    matches!(
                        step,
                        crate::app::state::AssistantStepView::Part { part, .. }
                            if part.kind == PartKind::Reasoning
                    )
                });
            if is_reasoning {
                thread.chat.toggle_reasoning(message_index, step_index);
            }
            is_reasoning
        }
        crate::app::state::CardTarget::UserMessage { message_id } => {
            thread.chat.toggle_user_message_expanded(&message_id)
        }
        crate::app::state::CardTarget::UserPaste {
            message_id,
            paste_id,
            occurrence,
        } => {
            thread
                .chat
                .toggle_user_paste(&message_id, paste_id, occurrence);
            true
        }
        _ => false,
    };
    if changed {
        thread.rows_dirty = true;
    }
    changed
}

#[cfg(test)]
mod tests;
