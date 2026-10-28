//! 本地命令的提交、回执与队列状态机（纯状态 + 效应）。
//!
//! shell 不占用 `RequestState`：模型流与命令可并行。DB 预登记与真实进程都在事件层，
//! 这里只负责冻结草稿、维护待提交/运行卡片和 FIFO 队列。

use std::path::PathBuf;

use crate::app::state::{
    AppState, ChatMessageView, ChatPartView, InputHintLevel, PendingShell, QueuedSubmission,
    UserShellView,
};
use crate::chat::events::PartKind;
use crate::layout::PaneId;
use crate::ui::text;

use super::{next_generation, show_lx_hint};

/// 提交当前 shell 草稿；非 shell 模式或空命令返回 false（空命令给提示并返回 true）。
///
/// 返回是否产生状态变化。DB 成功与 spawn 成功分别由事件回执确认；草稿在此之前保留。
pub fn try_submit_user_shell(state: &mut AppState, pane: PaneId) -> bool {
    submit(state, pane, None)
}

/// 提交时使用指定执行目录（排队恢复的冻结 cwd）；普通提交传 None。
fn submit(state: &mut AppState, pane: PaneId, frozen_cwd: Option<PathBuf>) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || !target.lx.shell_mode() {
        return false;
    }
    // 只去掉模式前缀与首尾空白；正文中的换行/引号/管道原样交给 shell。
    let command = target.lx.input.trim().to_string();
    if command.is_empty() {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_SHELL_EMPTY_COMMAND,
        );
        return true;
    }
    let revision = target.lx.revision();
    let session_id = target.lx.chat.session_id.clone();
    let Some(workspace) = state.chat_session_key_for_pane(pane).or_else(|| {
        state
            .active_workspace()
            .repo_key()
            .map(std::path::Path::to_path_buf)
    }) else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_NO_WORKSPACE,
        );
        return true;
    };
    let Some(cwd) = frozen_cwd.or_else(|| state.workspace_cwd_for_pane(pane)) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_SHELL_NO_CWD);
        return true;
    };
    let generation = next_generation(state, pane);
    let shell = crate::platform::default_shell();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.generation_counter = generation;
    target.lx.chat.pending_shells.push(PendingShell {
        generation,
        revision,
        command: command.clone(),
        session: session_id.clone(),
    });
    state
        .effects
        .push(crate::app::effects::Effect::BeginUserShell {
            pane,
            generation,
            workspace,
            cwd,
            session_id,
            command,
            shell,
            draft_revision: revision,
        });
    // 命令冻结的是修订号；草稿在 spawn 成功后才按它判定是否清除。
    true
}

/// 空闲时 Tab 保留输入；忙时（模型或命令在跑）把 shell 草稿加入 FIFO。
pub fn queue_shell(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || !target.lx.shell_mode() {
        return false;
    }
    let busy = !target.lx.chat.request.can_send() || target.lx.chat.has_running_shell();
    if !busy {
        return false;
    }
    if target.lx.input.trim().is_empty() {
        return false;
    }
    let full_text = target.lx.serialized_input();
    let Some(cwd) = state.workspace_cwd_for_pane(pane) else {
        return false;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target
        .lx
        .chat
        .queue
        .push(QueuedSubmission::Shell { full_text, cwd });
    target.lx.clear();
    true
}

/// 独立命令运行期间，普通聊天 Enter 冻结入队；返回是否已入队。
pub fn queue_chat_while_shell_runs(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx
        || !target.lx.chat.has_running_shell()
        || !target.lx.chat.request.can_send()
        || target.lx.input_mode() == crate::app::state::LxInputMode::Shell
    {
        return false;
    }
    let draft = target.lx.input.trim().to_string();
    if draft.is_empty() {
        return false;
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.queue.push(QueuedSubmission::Chat { draft });
    target.lx.clear();
    true
}

/// DB 预登记成功：插入运行卡片（在 spawn 之前调用，避免早到事件丢失）。
#[expect(clippy::too_many_arguments)]
pub fn on_user_shell_begun(
    state: &mut AppState,
    pane: PaneId,
    _generation: u64,
    session_id: &str,
    message_id: &str,
    turn_index: i64,
    command: &str,
    cwd: &str,
    shell: &str,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 待提交项保留到 spawn 回执：清草稿与记历史都依赖冻结的修订号/命令。
    if target
        .lx
        .chat
        .messages
        .iter()
        .any(|message| message.id == message_id)
    {
        return false;
    }
    target.lx.chat.session_id = Some(session_id.to_string());
    // 乐观插入的显示序号：DB 事务分配的正式序号在下次加载时回填。
    let sequence = target
        .lx
        .chat
        .messages
        .last()
        .map_or(0, |message| message.sequence + 1);
    target.lx.chat.messages.push(ChatMessageView {
        id: message_id.to_string(),
        sequence,
        kind: "user_shell".into(),
        role: "user".into(),
        status: "streaming".into(),
        turn_index,
        shell: Some(UserShellView {
            command: command.to_string(),
            cwd: cwd.to_string(),
            shell: shell.to_string(),
            state: crate::app::state::ProcessStateView::Starting,
            exit_code: None,
            duration_ms: None,
            output: String::new(),
            output_start: 0,
            output_end: 0,
            dropped_bytes: 0,
            truncated: false,
            runtime_epoch: None,
            process_id: None,
            revision: 0,
        }),
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: command.to_string(),
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
    });
    target.lx.chat.follow_bottom = true;
    true
}

/// spawn 成功：登记进程 ID；只有草稿修订仍匹配时才清空并回到 Chat。
///
/// 同时把 `!command` 写入全局输入历史（spawn 失败、排队、空命令都不写）。
pub fn on_user_shell_spawned(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    message_id: &str,
    process_id: u64,
    runtime_epoch: Option<&str>,
) -> bool {
    let mut changed = false;
    if let Some(shell) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.chat.shell_message_mut(message_id))
        && shell.process_id.is_none()
    {
        shell.process_id = Some(process_id);
        // job 已因终态移除时没有 epoch 可写；已有快照（含 epoch）不得被空值覆盖。
        if let Some(runtime_epoch) = runtime_epoch {
            shell.runtime_epoch = Some(runtime_epoch.to_string());
        }
        changed = true;
    }
    // 取出待提交项后再改其它字段，避免同时借用 `state` 的两处。
    let pending = state.pane_mut_anywhere(pane).and_then(|target| {
        let index = target
            .lx
            .chat
            .pending_shells
            .iter()
            .position(|pending| pending.generation == generation)?;
        Some(target.lx.chat.pending_shells.remove(index))
    });
    let Some(pending) = pending else {
        // 已被取消/清理：迟到回执不得清新草稿或重复记历史。
        return changed;
    };
    let serialized = format!("!{}", pending.command);
    if state.prompt_history.record(&serialized) {
        state
            .effects
            .push(crate::app::effects::Effect::RecordPromptHistory { text: serialized });
    }
    // 只按修订号判定：执行期间改过又改回时修订号也已前进，旧 ACK 不得清空。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && target.lx.revision() == pending.revision
    {
        target.lx.clear();
        target.lx.end_history_browse();
    }
    let _ = changed;
    true
}

/// spawn 失败：卡片记失败、保留草稿；不写输入历史。
pub fn on_user_shell_spawn_failed(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    message_id: &str,
    code: &str,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target
        .lx
        .chat
        .pending_shells
        .retain(|pending| pending.generation != generation);
    if let Some(shell) = target.lx.chat.shell_message_mut(message_id) {
        shell.state = crate::app::state::ProcessStateView::Failed;
        shell.revision = shell.revision.saturating_add(1);
    }
    if let Some(status) = target.lx.chat.shell_status_mut(message_id) {
        *status = "failed".into();
    }
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Error,
        format!("{}: {code}", text::LX_SHELL_SPAWN_FAILED),
    );
    true
}

/// 应用一次受控快照：终态不回退，revision 较小的事件丢弃。
pub fn apply_user_shell_snapshot(
    state: &mut AppState,
    pane: PaneId,
    message_id: &str,
    incoming: UserShellView,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(shell) = target.lx.chat.shell_message_mut(message_id) else {
        // 已切会话/已删除：迟到事件不复活卡片、不清草稿。
        return false;
    };
    if shell.state.is_terminal() && incoming.state.is_running() {
        return false;
    }
    if incoming.revision < shell.revision {
        return false;
    }
    *shell = incoming;
    let status = shell.message_status();
    if let Some(current) = target.lx.chat.shell_status_mut(message_id) {
        *current = status.to_string();
    }
    true
}

/// 命令进入终态（真实进程退出）：更新卡片后按 FIFO 分派下一项。
pub fn on_user_shell_terminal(state: &mut AppState, pane: PaneId) -> bool {
    // 只在模型也已结束时清除停止待命：模型仍在流式时，第一次 Esc 的待命目标是
    // 模型轮次，不能被一条无关命令的结束顺手取消。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && !target.lx.chat.request.is_busy()
    {
        target.lx.chat.stop_armed_at = None;
    }
    let drained = drain_queue(state, pane);
    let requested = super::request_report_drain_if_idle(state, pane);
    drained || requested
}

/// 分派队首提交；模型忙或仍有命令在跑时停止。
fn drain_queue(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if !target.lx.chat.request.can_send() || target.lx.chat.has_running_shell() {
        return false;
    }
    // 用户正在输入时不得用队列文本覆盖草稿：队首留到下一次结算（或用户提交后）再派发。
    if !target.lx.input.trim().is_empty() {
        return false;
    }
    let Some(item) = target.lx.chat.queue.first().cloned() else {
        return false;
    };
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.queue.remove(0);
    }
    match item {
        QueuedSubmission::Shell { full_text, cwd } => {
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.import_full_text(&full_text);
            }
            submit(state, pane, Some(cwd));
        }
        QueuedSubmission::Chat { draft } => {
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.import_full_text(&draft);
            }
            super::try_send(state, pane);
        }
    }
    true
}

/// DB 预登记失败：移除待提交项、保留草稿并提示。
pub fn on_user_shell_rejected(state: &mut AppState, pane: PaneId, generation: u64) -> bool {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target
            .lx
            .chat
            .pending_shells
            .retain(|pending| pending.generation != generation);
    }
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Error,
        text::LX_SHELL_COMMIT_FAILED,
    );
    true
}

/// 停止意图已下发：清除待命提示，保留已产生的部分输出。
pub fn on_user_shell_cancelled(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.stop_armed_at = None;
    true
}

/// 点击本地命令卡片：**行为随进程状态切换**（与 agent 命令卡片同一口径）。
///
/// - 命令仍在跑（长任务）：打开后台任务面板并定位该进程——卡片只给一行引导；
/// - 终态：切换**被点中的那个区域**的折叠（参数与输出各自独立）。
pub fn activate_shell_card(
    state: &mut AppState,
    pane: PaneId,
    message_id: &str,
    section: crate::app::state::CardSection,
) -> bool {
    let running = state
        .pane_anywhere(pane)
        .and_then(|target| {
            target
                .lx
                .chat
                .messages
                .iter()
                .find(|message| message.id == message_id)
                .and_then(|message| message.shell.as_ref())
        })
        .filter(|shell| shell.is_running())
        .map(|shell| shell.process_id);
    let Some(process_id) = running else {
        return toggle_shell_section(state, pane, message_id, section);
    };
    super::open_task_page_at(state, pane, process_id)
}

/// 切换命令卡片某区域的展开状态：先锚定视口，避免跟随底部时跳回顶部。
pub fn toggle_shell_section(
    state: &mut AppState,
    pane: PaneId,
    message_id: &str,
    section: crate::app::state::CardSection,
) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if !target
        .lx
        .chat
        .messages
        .iter()
        .any(|message| message.id == message_id && message.shell.is_some())
    {
        return false;
    }
    super::pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target
        .lx
        .chat
        .toggle_section(message_id, crate::app::state::SHELL_SLOT, section);
    true
}

/// 双 Esc：停止该窗格在途的本地命令并清空队列。
pub fn cancel_user_shells(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let had_pending = !target.lx.chat.pending_shells.is_empty();
    let running = target.lx.chat.has_running_shell();
    target.lx.chat.pending_shells.clear();
    target.lx.chat.queue.clear();
    if !had_pending && !running {
        return false;
    }
    state
        .effects
        .push(crate::app::effects::Effect::CancelUserShells { pane });
    true
}
