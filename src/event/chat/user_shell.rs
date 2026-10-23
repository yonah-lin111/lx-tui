//! 用户本地命令的执行编排：DB 预登记 → spawn → 输出/终态落库 → 队列分派。
//!
//! 与模型轮次完全解耦：不接受 `RequestState`，不创建 assistant 占位，不产生 HTTP。
//! 复用 `ProcessManager` 的 spawn、双流 drain、输出上限与停止能力。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::app::effects::Effect;
use crate::app::state::{AppState, UserShellView};
use crate::app::update;
use crate::chat::process::{
    ExecArgs, MAX_OUTPUT_TOKENS, ProcessManager, ProcessSnapshot, ProcessState,
};
use crate::db::chat::UserShellRecord;
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::layout::PaneId;

use super::ChatStreams;

/// 本地命令总超时：1 小时（与 Codex 默认一致）。
pub const USER_SHELL_TIMEOUT_MS: u64 = 60 * 60 * 1000;

/// 一条在途本地命令的归属与执行句柄（只存在于事件层）。
struct ShellJob {
    pane: PaneId,
    /// 会话数据库键（workspace）。
    db_workspace: PathBuf,
    /// 聊天会话 external id（进程记录归属）。
    session_id: String,
    epoch: String,
    manager: Arc<ProcessManager>,
    /// 用户原始命令（卡片与模型上下文的事实；不是 rtk 包装后的脚本）。
    command: String,
    /// 是否需要用内置过滤器处理输出（外部 rtk 未包装时）。
    filter_builtin: bool,
    cwd: String,
    shell: String,
    process_id: Option<u64>,
    stop_requested: bool,
    /// 本地快照修订号（单调；迟到的 Running 不能覆盖终态）。
    revision: u64,
}

/// 全部在途本地命令（按消息 ID 索引，进程事件以 `origin_call_id` 回投）。
#[derive(Default)]
pub struct UserShellJobs {
    jobs: HashMap<String, ShellJob>,
}

/// 一轮模型请求的命令注入记忆：已见终态 ID（即使之后被预算裁掉也不再复活）。
#[derive(Default)]
pub struct ShellInjectionState {
    pub seen: std::collections::HashSet<String>,
    /// 本轮已注入的命令文本字节数（防止多轮采样无限堆叠）。
    pub bytes: usize,
}

impl UserShellJobs {
    /// 是否为本地命令来源（进程事件分流用）。
    pub fn is_origin(&self, message_id: &str) -> bool {
        self.jobs.contains_key(message_id)
    }

    /// 该命令所属进程管理器（读快照/停止用）。
    pub(crate) fn manager_of(&self, message_id: &str) -> Option<Arc<ProcessManager>> {
        self.jobs.get(message_id).map(|job| job.manager.clone())
    }

    /// 标记停止意图并返回所有已知进程 ID（未 spawn 的命令等 on_spawn 后处理）。
    fn request_cancel(&mut self, pane: PaneId) -> Vec<u64> {
        let mut targets = Vec::new();
        for job in self.jobs.values_mut().filter(|job| job.pane == pane) {
            job.stop_requested = true;
            if let Some(process_id) = job.process_id {
                targets.push(process_id);
            }
        }
        targets
    }

    /// 进程终态后移除（迟到事件不再改卡片）。
    fn finish(&mut self, message_id: &str) {
        self.jobs.remove(message_id);
    }
}

impl ChatStreams {
    /// 注册一条在途命令（spawn 之前调用；早到的进程事件据此分流）。
    #[expect(clippy::too_many_arguments)]
    fn register_user_shell(
        &mut self,
        message_id: String,
        pane: PaneId,
        db_workspace: PathBuf,
        session_id: String,
        epoch: String,
        manager: Arc<ProcessManager>,
        command: String,
        filter_builtin: bool,
        cwd: String,
        shell: String,
    ) {
        self.user_shell.jobs.insert(
            message_id,
            ShellJob {
                pane,
                db_workspace,
                session_id,
                epoch,
                manager,
                command,
                filter_builtin,
                cwd,
                shell,
                process_id: None,
                stop_requested: false,
                revision: 0,
            },
        );
    }
}

/// 执行全部 `BeginUserShell`/`CancelUserShells` 效应；返回是否产生状态变化。
pub fn pump_user_shells(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::BeginUserShell { .. } | Effect::CancelUserShells { .. }
            )
        })
        .cloned()
        .collect();
    if effects.is_empty() {
        return false;
    }
    state.effects.retain(|effect| {
        !matches!(
            effect,
            Effect::BeginUserShell { .. } | Effect::CancelUserShells { .. }
        )
    });
    let mut changed = false;
    for effect in effects {
        match effect {
            Effect::BeginUserShell {
                pane,
                generation,
                workspace,
                cwd,
                session_id,
                command,
                shell,
                draft_revision,
            } => {
                changed |= begin(
                    state,
                    streams,
                    sender,
                    persistence,
                    BeginShell {
                        pane,
                        generation,
                        workspace,
                        cwd,
                        session_id,
                        command,
                        shell,
                        draft_revision,
                    },
                );
            }
            Effect::CancelUserShells { pane } => {
                changed |= cancel(state, streams, pane);
            }
            _ => {}
        }
    }
    changed
}

/// 一次提交的纯数据。
struct BeginShell {
    pane: PaneId,
    generation: u64,
    workspace: PathBuf,
    cwd: PathBuf,
    session_id: Option<String>,
    command: String,
    shell: String,
    draft_revision: u64,
}

fn begin(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    request: BeginShell,
) -> bool {
    let _ = request.draft_revision;
    // 取消、切会话、会话复位都会先清空 pending：效应可能已在队列里，必须以 pending
    // 为唯一凭据，否则用户已经按停/离开的命令仍会落库并产生不可撤销的副作用。
    let pending = state.pane_anywhere(request.pane).is_some_and(|target| {
        target
            .lx
            .chat
            .pending_shells
            .iter()
            .any(|pending| pending.generation == request.generation)
    });
    if !pending {
        return false;
    }
    let cwd_text = request.cwd.display().to_string();
    let record = UserShellRecord::starting(&request.command, &cwd_text, &request.shell);
    // 先落库预登记：失败绝不 spawn（不产生不可追踪副作用），草稿保留。
    let committed = match persistence.begin_user_shell(
        &request.workspace,
        request.session_id.as_deref(),
        &record,
    ) {
        Ok(committed) => committed,
        Err(error) => {
            tracing::warn!(target: "lx::shell", code = error.code(), "user shell begin failed");
            update::on_user_shell_rejected(state, request.pane, request.generation);
            return true;
        }
    };
    // 卡片先于 spawn 出现：早到的输出/退出事件不会因“没有消息”被丢弃。
    update::on_user_shell_begun(
        state,
        request.pane,
        request.generation,
        &committed.session_id,
        &committed.message_id,
        committed.turn_index,
        &request.command,
        &cwd_text,
        &request.shell,
    );
    let Some(runtime) = streams.runtime_for(request.cwd.clone(), &committed.session_id) else {
        // 无进程通道（测试环境）：保留记录并标记失败，不执行命令。
        let failed = UserShellRecord {
            state: ProcessState::Failed {
                code: "process runtime unavailable".into(),
            }
            .as_str()
            .to_string(),
            revision: 1,
            ..record
        };
        let _ = persistence.update_user_shell(&request.workspace, &committed.message_id, &failed);
        update::on_user_shell_spawn_failed(
            state,
            request.pane,
            request.generation,
            &committed.message_id,
            "process runtime unavailable",
        );
        return true;
    };
    let manager = runtime.manager().clone();
    let epoch = runtime.epoch.clone();
    // rtk：外部安装优先；未包装的命令由事件层用内置过滤器处理输出。
    let rtk = crate::chat::process::Rtk::detect();
    let prepared = rtk.prepare(&request.command);
    let filter_builtin = rtk.should_filter_builtin(&prepared);
    streams.register_user_shell(
        committed.message_id.clone(),
        request.pane,
        request.workspace.clone(),
        committed.session_id.clone(),
        epoch,
        manager.clone(),
        request.command.clone(),
        filter_builtin,
        cwd_text,
        request.shell.clone(),
    );
    let args = ExecArgs {
        cmd: prepared.command,
        workdir: None,
        shell: Some(request.shell),
        login: true,
        tty: false,
        yield_time_ms: 0,
        timeout_ms: Some(USER_SHELL_TIMEOUT_MS),
        max_output_tokens: MAX_OUTPUT_TOKENS,
    };
    let sender = sender.clone();
    let pane = request.pane;
    let generation = request.generation;
    let message_id = committed.message_id.clone();
    let workspace = request.cwd.clone();
    tokio::spawn(async move {
        let spawned_sender = sender.clone();
        let spawned_message = message_id.clone();
        let on_spawn = move |process_id: u64| {
            let _ = spawned_sender.send(AppEvent::UserShellSpawned {
                pane,
                generation,
                message_id: spawned_message.clone(),
                process_id,
            });
        };
        if let Err(error) = manager
            .exec_observed(args, &workspace, &message_id, &on_spawn)
            .await
        {
            let _ = sender.send(AppEvent::UserShellSpawnFailed {
                pane,
                generation,
                message_id,
                code: error.code.to_string(),
            });
        }
        // 成功时 `on_spawn` 已把进程句柄交给事件层；等待窗口结束不结束进程。
    });
    true
}

fn cancel(state: &mut AppState, streams: &mut ChatStreams, pane: PaneId) -> bool {
    let targets = streams.user_shell.request_cancel(pane);
    if !targets.is_empty() {
        // 同一窗格的命令可能分属不同会话实例：各自按已知进程 ID 停止，未知 ID 被忽略。
        let managers: Vec<Arc<ProcessManager>> = streams
            .user_shell
            .jobs
            .values()
            .filter(|job| job.pane == pane)
            .map(|job| job.manager.clone())
            .collect();
        let mut unique: Vec<Arc<ProcessManager>> = Vec::new();
        for manager in managers {
            if !unique.iter().any(|seen| Arc::ptr_eq(seen, &manager)) {
                unique.push(manager);
            }
        }
        for manager in unique {
            let ids = targets.clone();
            tokio::spawn(async move {
                manager.stop_many(ids).await;
            });
        }
    }
    update::on_user_shell_cancelled(state, pane)
}

/// 处理进程 UI 事件中的本地命令分支；返回 None 表示不是本地命令来源。
pub fn handle_process_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    origin_call_id: &str,
    snapshot: &ProcessSnapshot,
    terminal: bool,
) -> Option<bool> {
    let (pane, db_workspace, revision) = {
        let job = streams.user_shell.jobs.get_mut(origin_call_id)?;
        job.revision = job.revision.saturating_add(1);
        if job.process_id.is_none() {
            job.process_id = Some(snapshot.session_id);
        }
        (job.pane, job.db_workspace.clone(), job.revision)
    };
    let Some(job) = streams.user_shell.jobs.get(origin_call_id) else {
        return Some(false);
    };
    let (command, cwd, shell, epoch, filter_builtin) = (
        job.command.clone(),
        job.cwd.clone(),
        job.shell.clone(),
        job.epoch.clone(),
        job.filter_builtin,
    );
    let duration_ms = snapshot
        .ended_at_ms
        .map(|ended| ended.saturating_sub(snapshot.started_at_ms).max(0) as u64);
    let mut view = view_of(
        snapshot,
        command.clone(),
        cwd,
        shell,
        epoch.clone(),
        revision,
        duration_ms,
    );
    // 外部 rtk 已过滤时不再二次处理；内置过滤只在终态执行，避免运行中的
    // 半截输出（例如只有 Compiling 行）被误判成“ok”。
    if filter_builtin && terminal {
        view.output = crate::chat::process::rtk::filter_output(&command, &view.output);
    }
    let changed = update::apply_user_shell_snapshot(state, pane, origin_call_id, view.clone());
    let record = record_of(&view);
    // 终态立即写；运行态按进程 ID 限流合并（每进程最多一个待合并快照）。
    let due = terminal
        || streams.process_snapshot_due(snapshot.session_id, std::time::Duration::from_secs(1));
    let mut saved = true;
    if due {
        if let Err(error) = persistence.update_user_shell(&db_workspace, origin_call_id, &record) {
            saved = false;
            tracing::warn!(target: "lx::shell", code = error.code(), "user shell snapshot write failed");
            crate::app::update::show_toast(
                state,
                crate::app::toast::Toast::new(
                    crate::app::toast::ToastKind::Danger,
                    crate::ui::text::LX_CHAT_SAVE_FAILED,
                    Some(pane),
                    std::time::Instant::now(),
                ),
            );
        }
        // 进程元数据与命令记录同源：后台任务页走这一份，重载不出现两套状态。
        super::process_events::persist_snapshot(
            persistence,
            &db_workspace,
            &epoch,
            snapshot.session_id,
            snapshot,
            terminal,
        );
    }
    if terminal {
        streams.user_shell.finish(origin_call_id);
        // 结果未落库时不注入、不继续分派队列（只允许幂等重试保存，绝不重跑命令）。
        if saved {
            update::on_user_shell_terminal(state, pane);
        }
    }
    Some(changed)
}

/// 处理 spawn 回执（成功/失败）；迟到回执不清新草稿。
pub fn handle_spawned(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    generation: u64,
    message_id: &str,
    process_id: u64,
) -> bool {
    // 进程可能在 spawn 回执**之前**就已退出（终态事件先到，job 已被移除）。
    // 回执仍必须结算 pending：否则 `pending_shells` 永远非空，聊天 Enter 会一直入队、
    // Esc 一直待命停止，直到切会话才能恢复。
    let mut epoch = None;
    let job = streams.user_shell.jobs.get_mut(message_id).map(|job| {
        job.process_id = Some(process_id);
        epoch = Some(job.epoch.clone());
        (
            job.epoch.clone(),
            job.manager.clone(),
            job.db_workspace.clone(),
            job.session_id.clone(),
            job.stop_requested,
        )
    });
    if let Some((job_epoch, manager, db_workspace, session_id, stop_requested)) = job {
        if stop_requested {
            tokio::spawn(async move {
                manager.stop_many([process_id]).await;
            });
        }
        // 进程元数据落库：后台任务页/历史卡片与运行实例同一份事实。
        if let Some(snapshot) = streams
            .user_shell
            .manager_of(message_id)
            .and_then(|manager| manager.snapshot(process_id))
        {
            super::process_events::register_process(
                persistence,
                &db_workspace,
                &job_epoch,
                &session_id,
                message_id,
                &snapshot,
            );
        }
    }
    let changed = update::on_user_shell_spawned(
        state,
        pane,
        generation,
        message_id,
        process_id,
        epoch.as_deref(),
    );
    // 命令可能在 spawn 回执前就已退出：此刻 pending 已移除，队列可继续。
    update::on_user_shell_terminal(state, pane) | changed
}

pub fn handle_spawn_failed(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    generation: u64,
    message_id: &str,
    code: &str,
) -> bool {
    let job = streams.user_shell.jobs.remove(message_id);
    if let Some(job) = job {
        let failed = UserShellRecord {
            command: job.command,
            cwd: job.cwd,
            shell: job.shell,
            state: ProcessState::Failed {
                code: code.to_string(),
            }
            .as_str()
            .to_string(),
            exit_code: None,
            duration_ms: None,
            output: String::new(),
            output_start: 0,
            output_end: 0,
            dropped_bytes: 0,
            truncated: false,
            runtime_epoch: Some(job.epoch),
            process_id: None,
            revision: job.revision.saturating_add(1),
        };
        let _ = persistence.update_user_shell(&job.db_workspace, message_id, &failed);
    }
    let changed = update::on_user_shell_spawn_failed(state, pane, generation, message_id, code);
    // spawn 失败同样结束本次命令：按 FIFO 分派下一项。
    update::on_user_shell_terminal(state, pane) | changed
}

/// 会话运行实例被回收后，把本地命令的最终快照写入命令记录。
///
/// `ProcessesReaped` 只带进程快照，命令的 shell/cwd 与数据库归属保存在 job 里；
/// 不写这一份，库中的命令会永远停在 streaming（重启时才被改成无输出的 interrupted）。
pub fn persist_reaped(
    persistence: &Persistence,
    jobs: &mut UserShellJobs,
    snapshots: &[ProcessSnapshot],
) {
    for snapshot in snapshots {
        let Some(job) = jobs.jobs.remove(&snapshot.origin_call_id) else {
            continue;
        };
        let terminal = snapshot.state.is_terminal();
        let state = if terminal {
            snapshot.state.clone()
        } else {
            ProcessState::Interrupted
        };
        // 非终态快照是被强制记成 interrupted 的半截输出：内置过滤会把它压成
        // “ok”，形成 interrupted/ok 的自相矛盾记录，这里保留原始尾部。
        let output = if terminal && job.filter_builtin {
            crate::chat::process::rtk::filter_output(&job.command, &snapshot.tail)
        } else {
            snapshot.tail.clone()
        };
        let record = UserShellRecord {
            command: job.command,
            cwd: job.cwd,
            shell: job.shell,
            state: state.as_str().to_string(),
            exit_code: snapshot.exit_code,
            duration_ms: snapshot
                .ended_at_ms
                .map(|ended| ended.saturating_sub(snapshot.started_at_ms).max(0) as u64),
            output,
            output_start: snapshot.output_start,
            output_end: snapshot.output_end,
            dropped_bytes: snapshot.dropped_bytes,
            truncated: snapshot.dropped_bytes > 0,
            runtime_epoch: Some(job.epoch),
            process_id: Some(snapshot.session_id),
            revision: job.revision.saturating_add(1),
        };
        if let Err(error) =
            persistence.update_user_shell(&job.db_workspace, &snapshot.origin_call_id, &record)
        {
            tracing::warn!(target: "lx::shell", code = error.code(), "user shell reap write failed");
        }
    }
}

/// 快照 → 展示视图（事件层唯一一次映射）。
fn view_of(
    snapshot: &ProcessSnapshot,
    command: String,
    cwd: String,
    shell: String,
    epoch: String,
    revision: u64,
    duration_ms: Option<u64>,
) -> UserShellView {
    UserShellView {
        command,
        cwd,
        shell,
        state: snapshot.state.clone().into(),
        exit_code: snapshot.exit_code,
        duration_ms,
        output: snapshot.tail.clone(),
        output_start: snapshot.output_start,
        output_end: snapshot.output_end,
        dropped_bytes: snapshot.dropped_bytes,
        truncated: snapshot.dropped_bytes > 0,
        runtime_epoch: Some(epoch),
        process_id: Some(snapshot.session_id),
        revision,
    }
}

/// 展示视图 → 持久化记录。
fn record_of(view: &UserShellView) -> UserShellRecord {
    UserShellRecord {
        command: view.command.clone(),
        cwd: view.cwd.clone(),
        shell: view.shell.clone(),
        state: view.state.as_str().to_string(),
        exit_code: view.exit_code,
        duration_ms: view.duration_ms,
        output: view.output.clone(),
        output_start: view.output_start,
        output_end: view.output_end,
        dropped_bytes: view.dropped_bytes,
        truncated: view.truncated,
        runtime_epoch: view.runtime_epoch.clone(),
        process_id: view.process_id,
        revision: view.revision,
    }
}

/// 命令的本轮上下文增量注入：只返回**连续终态前缀**中尚未注入的记录。
///
/// 运行中的命令会阻塞其后命令，但不阻塞模型任务本身；已注入 ID 永久保留在本轮
/// 请求状态里，即使之后被预算裁掉也不会再次复活。
pub fn poll_updates(
    state: &AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &crate::app::state::RequestKey,
) -> Vec<String> {
    let Some(pane) = state.pane_anywhere(key.pane) else {
        return Vec::new();
    };
    let Some(workspace) = state.chat_session_key_for_pane(key.pane) else {
        return Vec::new();
    };
    let _ = pane;
    // -1：包含 sequence 0 的第一条命令（`sequence > after` 的闭下界）。
    let Ok(messages) = persistence.load_user_shells_since(&workspace, &key.session, -1) else {
        return Vec::new();
    };
    let state_entry = streams.user_shell_injected.entry(key.clone()).or_default();
    let mut out = Vec::new();
    let mut bytes = state_entry.bytes;
    for message in &messages {
        if state_entry.seen.contains(&message.external_id) {
            continue;
        }
        let Some(record) = message.user_shell.as_ref() else {
            continue;
        };
        // 顺序屏障：前面的命令仍在运行时，后面的结果本次不注入。
        if !record.is_terminal() || message.status == "streaming" {
            break;
        }
        let text = crate::chat::context::format_user_shell(record);
        if bytes.saturating_add(text.len()) > MAX_INJECTED_BYTES {
            break;
        }
        bytes = bytes.saturating_add(text.len());
        state_entry.seen.insert(message.external_id.clone());
        out.push(text);
    }
    state_entry.bytes = bytes;
    out
}

/// 单轮增量注入的文本预算（防止多条 64 KiB 输出无限堆叠）。
///
/// 与历史装配的预留同源：装配按 [`MAX_USER_SHELL_INJECTION_BYTES`] 扣掉等量 token，
/// 两边任一改动都必须同步。
pub const MAX_INJECTED_BYTES: usize = crate::chat::context::MAX_USER_SHELL_INJECTION_BYTES;

/// 一轮开始时把**已进入初始上下文的终态命令**标记为已见。
///
/// 以本轮装配的来源（窗格消息视图）为准，而不是重新查库：装配之后才落库完成的命令
/// 不会被误标，随后由下一次采样前的增量握手吸收。
pub fn mark_initial_seen(
    state: &AppState,
    streams: &mut ChatStreams,
    key: &crate::app::state::RequestKey,
) {
    let Some(pane) = state.pane_anywhere(key.pane) else {
        return;
    };
    let entry = streams.user_shell_injected.entry(key.clone()).or_default();
    for message in &pane.lx.chat.messages {
        // 初始仍在运行的命令不标记：它完成后的结果应可被本轮吸收。
        if message.is_user_shell() && message.status != "streaming" {
            entry.seen.insert(message.id.clone());
        }
    }
}
