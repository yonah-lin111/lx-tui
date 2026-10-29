//! 子 agent 生命周期：spawn 落库与子 turn、增量分流、工具握手、wait/close、报告投递。
//!
//! 报告恰好一次：`agent_subagent.report_delivered` 是唯一判据，后台自动轮与
//! `wait_agent` 共享同一闸门。子 turn 的消息模型由事件层持有（`SubagentHandle.chat`），
//! 窗格关闭后仍能继续落库；UI 的 `SubagentRun.chat` 只是同一批增量的镜像。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, watch};

use super::*;

mod tools;
mod wait;

pub(crate) use wait::handle_subagent_wait_ready;
#[cfg(test)]
pub(crate) use wait::settle_wait;

use crate::app::state::lx::agent::{SubagentRun, seed_chat, seed_chat_model};
use crate::app::state::{RequestKey, RequestState, SubagentReportView};
use crate::chat::agent::{
    AgentRole, FINISHED_CODE, LIMIT_REACHED_CODE, MAX_CONCURRENT_SUBAGENTS, NOT_FOUND_CODE,
    SubagentOp, SubagentOutcome, WaitMode, compose_subagent_prompt, format_report_envelope,
    reports_to_json,
};
use crate::chat::agent_loop::ModelContext;
use crate::chat::events::{ChatRequest, ChatTurn, ModelItem, RequestPurpose, StopReason};
use crate::db::chat::{ModelSnapshot, ReportPayload, SubagentRecord};
use crate::event::AppEvent;
use crate::layout::PaneId;
pub(crate) use tools::{begin_child_tool, finish_child_tool};

/// 运行中状态值。
pub const STATUS_RUNNING: &str = "running";
/// 完成状态值。
pub const STATUS_COMPLETED: &str = "completed";
/// 失败状态值。
pub const STATUS_FAILED: &str = "failed";
/// 取消状态值。
pub const STATUS_CANCELLED: &str = "cancelled";

/// 一个在跑的子 agent 句柄（只存在于事件循环）。
pub(crate) struct SubagentHandle {
    /// 父请求键：报告投递路由与 UI 归属。
    pub parent: RequestKey,
    /// 稳定 agent id。
    pub agent_id: String,
    /// 子会话 external id。
    pub child_session_id: String,
    /// spawn 落库的 assistant 行（子 turn 工具归属校验的锚点）。
    pub child_assistant_id: String,
    /// 子 user 消息 id（文件 mutation 归属）。
    pub child_user_message_id: String,
    /// 角色。
    pub role: AgentRole,
    /// 面向用户的短标题。
    pub title: String,
    /// 任务原文。
    pub task: String,
    /// 取消通道；close / 会话删除 / 退出时发送。
    pub cancel: watch::Sender<bool>,
    /// 完成通知：`wait_agent` 用。
    pub done: watch::Sender<bool>,
    /// running / completed / failed / cancelled。
    pub status: &'static str,
    /// 启动时刻（UTC 毫秒）。
    pub started_at_ms: i64,
    /// 子会话消息模型（持久化来源；窗格关闭后仍存在）。
    pub chat: crate::app::state::LxChat,
    /// 子会话 DB 工作区键。
    pub workspace: PathBuf,
}

/// 处理控制工具请求；返回是否产生状态变化。
pub(crate) fn handle_subagent_op(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    op: SubagentOp,
    reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
) -> bool {
    match op {
        SubagentOp::Spawn { role, title, task } => {
            let changed = spawn(state, streams, persistence, &key, role, &title, &task);
            match changed {
                Ok(outcome) => {
                    let _ = reply.send(Ok(outcome));
                }
                Err(code) => {
                    let _ = reply.send(Err(HostError::Failed { code }));
                }
            }
            true
        }
        SubagentOp::Close { target } => {
            let result = close(streams, &key, &target);
            let changed = result.is_ok();
            let _ = reply.send(result);
            changed
        }
        SubagentOp::Wait {
            targets,
            mode,
            timeout_ms,
        } => {
            let all = targets.is_empty();
            // `wait_for=all` 一次调用就是“等到目标全部结束”：超时只作为内部
            // 重检周期，绝不把 timeout 交回模型提前结束本轮。`any` 仍按超时返回。
            let auto_continue = mode == WaitMode::All;
            wait::wait(
                streams,
                persistence,
                key,
                targets,
                all,
                mode,
                timeout_ms,
                auto_continue,
                reply,
            );
            false
        }
    }
}

/// spawn：并发检查 → 落库 → 注册句柄 → 启动子 turn → 立即回执。
fn spawn(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    role: AgentRole,
    title: &str,
    task: &str,
) -> Result<SubagentOutcome, String> {
    let Some(sender) = streams.event_sender.clone() else {
        return Err("subagent event channel unavailable".to_string());
    };
    let sender = &sender;
    let parent_session = key.session.clone();
    let running = streams
        .subagents
        .values()
        .filter(|handle| handle.parent.session == parent_session && handle.status == STATUS_RUNNING)
        .count();
    if running >= MAX_CONCURRENT_SUBAGENTS {
        return Err(LIMIT_REACHED_CODE.to_string());
    }
    let Some(frozen) = streams.frozen.get(&parent_session).cloned() else {
        return Err("subagent model snapshot unavailable".to_string());
    };
    let Some(workspace) = streams.db_workspaces.get(key).cloned() else {
        return Err("subagent workspace unavailable".to_string());
    };
    let Some(database) = persistence.database() else {
        return Err("subagent persistence unavailable".to_string());
    };
    let spawn_message_id = active_assistant_id(state, key);
    let model = ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let committed = database
        .begin_subagent_turn(
            &workspace,
            &parent_session,
            role,
            title,
            task,
            &model,
            spawn_message_id.as_deref(),
        )
        .map_err(|error| error.code().to_string())?;
    let record = SubagentRecord {
        agent_id: committed.agent_id.clone(),
        child_session_id: committed.child_session_id.clone(),
        parent_session_id: parent_session.clone(),
        spawn_message_id,
        role,
        status: STATUS_RUNNING.to_string(),
        title: title.to_string(),
        created_at_ms: committed.created_at_ms,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    };
    let mut run = SubagentRun::from_record(&record);
    // 列表用短标题；子会话种子消息必须保存完整 task（历史重载时由 DB 提供）。
    run.task = task.to_string();
    let child_key = RequestKey {
        pane: key.pane,
        session: committed.child_session_id.clone(),
        generation: key.generation,
    };
    seed_chat(&mut run, child_key.clone(), &committed.child_assistant_id);
    update::on_subagent_started(state, key.pane, run);
    let mut child_chat = crate::app::state::LxChat::new();
    seed_chat_model(
        &mut child_chat,
        &committed.child_session_id,
        task,
        child_key.clone(),
        &committed.child_assistant_id,
    );

    let (cancel_sender, cancel) = watch::channel(false);
    let (done_sender, _done) = watch::channel(false);
    let Some(cwd) = state
        .workspace_cwd_for_pane(key.pane)
        .or_else(|| Some(workspace.clone()))
    else {
        return Err("subagent workspace unavailable".to_string());
    };
    let (manager, turn) = match streams.runtime_for(cwd.clone(), &committed.child_session_id) {
        Some(runtime) => (
            runtime.manager.clone(),
            runtime.turn(&key.generation.to_string()),
        ),
        None => return Err("process runtime unavailable".to_string()),
    };
    let environment = crate::chat::prompt::environment_snapshot(
        &crate::platform::EnvironmentFacts::capture(),
        Some(&cwd),
        Some(&cwd),
        &[],
    );
    let handle = SubagentHandle {
        parent: key.clone(),
        agent_id: committed.agent_id.clone(),
        child_session_id: committed.child_session_id.clone(),
        child_assistant_id: committed.child_assistant_id.clone(),
        child_user_message_id: committed.child_user_message_id.clone(),
        role,
        title: title.to_string(),
        task: task.to_string(),
        cancel: cancel_sender.clone(),
        done: done_sender.clone(),
        status: STATUS_RUNNING,
        started_at_ms: committed.created_at_ms,
        chat: child_chat,
        workspace: workspace.clone(),
    };
    streams
        .cancels
        .insert(child_key.clone(), cancel_sender.clone());
    streams
        .db_workspaces
        .insert(child_key.clone(), workspace.clone());
    streams
        .subagents
        .insert(committed.child_session_id.clone(), handle);
    streams
        .frozen
        .insert(committed.child_session_id.clone(), frozen.clone());

    // 子 turn 后台执行；终态由 `ChatNetEvent::Finished` 回投事件层结算。
    let net_sender = sender.clone();
    let stream_key = child_key.clone();
    let system_prompt = compose_subagent_prompt(role, Some(&environment));
    let request = ChatRequest {
        turns: vec![ChatTurn {
            role: "user".into(),
            text: task.to_string(),
            reasoning_metadata: None,
        }],
        items: vec![ModelItem::UserText(task.to_string())],
        model,
        variant_params: frozen.variant_params.clone(),
        purpose: RequestPurpose::Subagent(role),
        system_prompt: Some(system_prompt),
    };
    let context = ModelContext::new(request, cwd).with_context_limit(frozen.context_limit);
    let reads = crate::chat::tools::read_registry(&committed.child_session_id);
    let recorder = super::file_snapshots::HostRecorderFactory::new(
        net_sender.clone(),
        workspace,
        committed.child_session_id.clone(),
        committed.child_user_message_id,
    );
    let executor = AgentTools::new(manager, turn)
        .with_recorder(std::sync::Arc::new(recorder))
        .with_reads(reads)
        .with_image_support(frozen.supports_image_input);
    let transport = frozen.transport.clone();
    let base_url = frozen.base_url.clone();
    let api_key = frozen.api_key.clone();
    let idle_timeout = Duration::from_millis(frozen.idle_timeout_ms);
    let guard_session = committed.child_session_id.clone();
    tokio::spawn(async move {
        // 会话 guard 失败：直接按失败结束（不半途死，不重试）。
        let _guard = match crate::platform::try_lock_session(&guard_session) {
            Ok(Some(guard)) => guard,
            Ok(None) | Err(_) => {
                let _ = net_sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
                    key: stream_key,
                    reason: StopReason::Failed {
                        code: "subagent session busy".into(),
                    },
                    usage: Usage::default(),
                }));
                return;
            }
        };
        let live_sender = net_sender.clone();
        let live_key = stream_key.clone();
        let stream_one = move |request: ChatRequest, cancel: watch::Receiver<bool>| {
            let sender = live_sender.clone();
            let key = live_key.clone();
            let transport = transport.clone();
            let base_url = base_url.clone();
            let api_key = api_key.clone();
            // 自动压缩的摘要请求不能把摘要正文当成普通增量回投：否则摘要会
            // 被画进子 agent 时间线并参与 checkpoint。普通采样照旧实时回投。
            let emit = request.purpose != RequestPurpose::CompactSummary;
            async move {
                crate::chat::client::run_stream(
                    &transport,
                    &base_url,
                    Some(api_key.as_str()),
                    idle_timeout,
                    request,
                    cancel,
                    move |deltas| {
                        if emit {
                            let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Delta {
                                key: key.clone(),
                                deltas: deltas.to_vec(),
                            }));
                        }
                    },
                )
                .await
            }
        };
        let summary = host::drive_agent_turn(
            stream_key.clone(),
            context,
            net_sender.clone(),
            executor,
            stream_one,
            cancel,
        )
        .await;
        let _ = net_sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
            key: stream_key,
            reason: summary.stop_reason(),
            usage: summary.usage,
        }));
    });

    Ok(SubagentOutcome::Spawned {
        agent_id: committed.agent_id,
        role,
        title: title.to_string(),
    })
}

/// close：发取消信号；子 turn 在下一检查点收尾，不产生报告。
fn close(
    streams: &mut ChatStreams,
    key: &RequestKey,
    target: &str,
) -> Result<SubagentOutcome, HostError> {
    let Some(handle) = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == target && handle.parent.session == key.session)
    else {
        return Err(HostError::Failed {
            code: NOT_FOUND_CODE.to_string(),
        });
    };
    let previous = handle.status.to_string();
    if handle.status != STATUS_RUNNING {
        return Err(HostError::Failed {
            code: FINISHED_CODE.to_string(),
        });
    }
    // 发送失败说明子 turn 已不在（句柄即将被清理）：同样是「已结束」。
    if handle.cancel.send(true).is_err() {
        return Err(HostError::Failed {
            code: FINISHED_CODE.to_string(),
        });
    }
    Ok(SubagentOutcome::Closed {
        agent_id: target.to_string(),
        previous_status: previous,
    })
}

/// 子 turn 的网络事件分流（`Delta` / `Finished`）。
pub(crate) fn handle_subagent_net_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    event: ChatNetEvent,
) -> bool {
    match event {
        ChatNetEvent::Delta { key, deltas } => {
            let bytes: usize = deltas.iter().map(ChatDelta::payload_bytes).sum();
            let entry = streams
                .checkpoints
                .entry(key.clone())
                .or_insert_with(|| (Instant::now(), 0));
            entry.1 += bytes;
            let due = entry.1 >= CHECKPOINT_BYTES || entry.0.elapsed() >= CHECKPOINT_INTERVAL;
            let mut changed = false;
            if let Some(handle) = streams.subagents.get_mut(&key.session) {
                changed |= update::apply_subagent_deltas(&mut handle.chat, &deltas);
                if due {
                    *entry = (Instant::now(), 0);
                    if let Some(parts) = assistant_parts(&handle.chat, &handle.child_assistant_id) {
                        state.effects.push(Effect::CheckpointChatTurn {
                            workspace: handle.workspace.clone(),
                            assistant_id: handle.child_assistant_id.clone(),
                            parts,
                        });
                    }
                }
            }
            changed |= update::on_subagent_delta(state, &key.session, &deltas);
            changed
        }
        ChatNetEvent::Finished { key, reason, usage } => {
            // 终态结算需要 DB；回投事件循环（本函数有意不持有 `Persistence`）。
            if let Some(sender) = streams.event_sender.clone() {
                let _ = sender.send(AppEvent::SubagentFinished { key, reason, usage });
            }
            false
        }
    }
}

/// 子 turn 终态：落库、UI 更新、空闲时自动报告批次与句柄清理。
pub(crate) fn finish_subagent(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    reason: StopReason,
    usage: Usage,
) -> bool {
    let Some(mut handle) = streams.subagents.remove(&key.session) else {
        return false;
    };
    handle.status = match reason {
        StopReason::Complete => STATUS_COMPLETED,
        StopReason::Cancelled => STATUS_CANCELLED,
        StopReason::Failed { .. } => STATUS_FAILED,
    };
    let reason_for_chat = reason.clone();
    handle.chat.close_stream(&reason_for_chat);
    let parts = assistant_parts(&handle.chat, &handle.child_assistant_id).unwrap_or_default();
    let outcome = match &reason {
        StopReason::Complete => crate::db::chat::TurnOutcome::Complete {
            stop_reason: Some("complete".into()),
        },
        StopReason::Cancelled => crate::db::chat::TurnOutcome::Interrupted,
        StopReason::Failed { code } => crate::db::chat::TurnOutcome::Failed {
            error_code: code.clone(),
        },
    };
    let duration_ms = Some((crate::db::now_ms() - handle.started_at_ms).max(0));
    let mut report_view = None;
    if !matches!(reason, StopReason::Cancelled)
        && let Some(database) = persistence.database()
    {
        let body = report_body(&handle);
        let files_changed = database
            .count_changed_paths(
                &handle.workspace,
                &key.session,
                &handle.child_user_message_id,
            )
            .unwrap_or_default();
        let payload = ReportPayload {
            title: handle.title.clone(),
            body: body.clone(),
            files_changed,
            duration_ms,
        };
        if let Err(error) = database.finish_subagent_turn(
            &handle.workspace,
            &handle.agent_id,
            &handle.child_assistant_id,
            &parts,
            &outcome,
            usage,
            Some(&payload),
        ) {
            tracing::warn!(%error, agent_id = %handle.agent_id, "subagent finish failed");
        }
        let view = SubagentReportView {
            agent_id: handle.agent_id.clone(),
            title: handle.title.clone(),
            role: handle.role,
            status: handle.status.to_string(),
            files_changed,
            duration_ms,
            body,
            delivered_at_ms: crate::db::now_ms(),
        };
        report_view = Some(view);
    } else if let Some(database) = persistence.database() {
        // 取消/关闭：不生成报告，只封口子 assistant 与 agent 行。
        if let Err(error) = database.finish_subagent_turn(
            &handle.workspace,
            &handle.agent_id,
            &handle.child_assistant_id,
            &parts,
            &outcome,
            usage,
            None,
        ) {
            tracing::warn!(%error, agent_id = %handle.agent_id, "subagent cancel finish failed");
        }
    }
    // UI：终态 + 报告（窗格已关时忽略）。
    let _ = update::on_subagent_finished(state, &key.session, handle.status, report_view, usage);
    // 运行中的等待卡片同步刷新目标状态（黄/绿方块）。
    update::refresh_subagent_cards(state, handle.parent.pane);
    // 完成通知先于句柄清理：`wait_agent` 的接收端由 handle 的 done 发出。
    let _ = handle.done.send(true);
    let cancel_key = RequestKey {
        pane: handle.parent.pane,
        session: handle.child_session_id.clone(),
        generation: handle.parent.generation,
    };
    streams.finish(&cancel_key);
    // 父窗格空闲时立即起报告批次；忙时留到本轮结束后的 idle 再 drain。
    let _ = drain_reports(state, persistence, handle.parent.pane);
    true
}

/// 父窗格是否空闲且没有排队提交（自动投递的唯一前置条件）。
fn parent_idle(state: &AppState, pane: PaneId) -> bool {
    state.pane_anywhere(pane).is_some_and(|target| {
        matches!(target.lx.chat.request, RequestState::Idle) && target.lx.chat.queue.is_empty()
    })
}

/// 把当前所有未投递报告合并成一条报告消息 + 一个自动 assistant 回合。
///
/// 用户输入优先：父窗格非 Idle、有排队提交时不启动；用户显式停止主轮后也不
/// 自动起轮（避免后台完成把主 agent 叫醒），报告留在 `agent_subagent` 里，等
/// 下一次用户轮或重新进入 idle 后再 drain。
pub(crate) fn drain_reports(state: &mut AppState, persistence: &Persistence, pane: PaneId) -> bool {
    // 用户显式停止主轮后不自动起报告轮：子 agent 可以继续跑，报告留在库里，
    // 等下一次用户消息（或用户主动操作）再 drain，避免被后台完成“叫醒”。
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.chat.main_stopped)
    {
        return false;
    }
    if !parent_idle(state, pane) {
        return false;
    }
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        return false;
    };
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
        return false;
    };
    let Some(selection) = target
        .lx
        .chat
        .effective_selection(&state.models)
        .or_else(|| state.models.default_selection())
    else {
        return false;
    };
    let Some(database) = persistence.database() else {
        return false;
    };
    let pending = match database.pending_subagent_reports(&workspace, &session_id) {
        Ok(pending) => pending,
        Err(error) => {
            tracing::warn!(%error, "pending subagent reports unavailable");
            return false;
        }
    };
    let mut agent_ids = Vec::with_capacity(pending.len());
    let mut reports = Vec::with_capacity(pending.len());
    for (agent_id, report_json) in pending {
        let Some(report) = SubagentReportView::from_json(&report_json) else {
            tracing::warn!(%agent_id, "subagent report JSON invalid");
            let _ = database.mark_report_delivered(&workspace, &agent_id);
            continue;
        };
        agent_ids.push(agent_id);
        reports.push(report);
    }
    if reports.is_empty() {
        return false;
    }
    let envelope = format!(
        "[local runtime] subagent reports:\n{}",
        reports
            .iter()
            .map(format_report_envelope)
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let reports_json = reports_to_json(&reports);
    let model = ModelSnapshot {
        provider: selection.provider,
        model: selection.model,
        variant: selection.variant,
    };
    let committed = match database.begin_report_batch(
        &workspace,
        &session_id,
        &envelope,
        &reports_json,
        &agent_ids,
        &model,
    ) {
        Ok(committed) => committed,
        Err(error) => {
            tracing::warn!(%error, "subagent report batch not started");
            return false;
        }
    };
    update::on_report_batch_committed(state, pane, &committed, reports)
}

/// 报告正文：子会话最后一条 assistant 的正文，超限截断。
fn report_body(handle: &SubagentHandle) -> String {
    let text = handle
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map(|message| message.text())
        .unwrap_or_default();
    crate::chat::agent::truncate_report_body(&text)
}

/// 从子会话消息模型取 assistant 的 parts（位置来自同一时间线）。
fn assistant_parts(
    chat: &crate::app::state::LxChat,
    assistant_id: &str,
) -> Option<Vec<crate::db::chat::ChatPart>> {
    chat.messages
        .iter()
        .find(|message| message.id == assistant_id)
        .map(crate::db::chat::chat_message_part_views)
}

/// 会话被删除：取消该会话下所有在册子 turn（DB 外键会级联清行）。
///
/// 内存句柄不清理的话，子 turn 会继续跑并把结果写进已删除的子会话（写入失败），
/// 面板里也会留下永远 running 的幽灵行。
pub(crate) fn cancel_for_session(
    streams: &mut ChatStreams,
    session_id: &str,
    agent_id: Option<&str>,
) -> bool {
    let mut cancelled = false;
    let mut dead = Vec::new();
    for (child, handle) in &streams.subagents {
        let selected = agent_id.is_none_or(|target| handle.agent_id == target);
        let belongs = match agent_id {
            Some(_) => handle.parent.session == session_id,
            None => handle.parent.session == session_id || handle.child_session_id == session_id,
        };
        if selected && belongs {
            // `send` 在没有接收者时失败：子 turn 已经结束、只是终态还没回投，
            // 句柄是幽灵，直接移除（否则面板里永远显示 running）。
            if handle.cancel.send(true).is_ok() {
                cancelled = true;
            } else {
                dead.push(child.clone());
            }
        }
    }
    for child in dead {
        streams.subagents.remove(&child);
        streams.frozen.remove(&child);
    }
    cancelled
}

/// 应用子 agent 效应（列表、懒加载、停止）。
pub fn apply_subagent_effects(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::ListSubagents { .. }
                    | Effect::LoadSubagentSession { .. }
                    | Effect::CancelSubagents { .. }
                    | Effect::DrainSubagentReports { .. }
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
            Effect::ListSubagents { .. }
                | Effect::LoadSubagentSession { .. }
                | Effect::CancelSubagents { .. }
                | Effect::DrainSubagentReports { .. }
        )
    });
    let mut changed = false;
    for effect in effects {
        match effect {
            Effect::ListSubagents {
                pane,
                parent_session,
            } => {
                let Some(database) = persistence.database() else {
                    continue;
                };
                let Some(workspace) = workspace_for_pane(state, pane) else {
                    continue;
                };
                match database.list_subagents(&workspace, &parent_session) {
                    Ok(records) => {
                        changed |=
                            update::set_subagent_list(state, pane, &parent_session, &records);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "listing subagents failed");
                    }
                }
            }
            Effect::LoadSubagentSession {
                pane,
                agent_id,
                child_session,
            } => {
                let Some(database) = persistence.database() else {
                    continue;
                };
                let Some(workspace) = workspace_for_pane(state, pane) else {
                    continue;
                };
                match database.load_chat_session(&workspace, &child_session) {
                    Ok(Some(messages)) => {
                        changed |= update::on_subagent_loaded(state, pane, &agent_id, &messages);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "loading subagent session failed");
                    }
                }
            }
            Effect::CancelSubagents {
                session_id,
                agent_id,
            } => {
                changed |= cancel_for_session(streams, &session_id, agent_id.as_deref());
            }
            Effect::DrainSubagentReports { pane } => {
                changed |= drain_reports(state, persistence, pane);
            }
            _ => {}
        }
    }
    changed
}

/// 窗格的会话 DB 工作区键（面板打开时没有在途轮次，不能读 `db_workspaces`）。
fn workspace_for_pane(state: &AppState, pane: PaneId) -> Option<PathBuf> {
    state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.workspace_cwd_for_pane(pane))
}

/// 运行中的子 agent 摘要（`runtime_context_summary` 追加段）。
pub(crate) fn running_subagent_lines(streams: &ChatStreams, parent_session: &str) -> Vec<String> {
    let running: Vec<&SubagentHandle> = streams
        .subagents
        .values()
        .filter(|handle| handle.parent.session == parent_session && handle.status == STATUS_RUNNING)
        .collect();
    if running.is_empty() {
        return Vec::new();
    }
    let mut lines =
        vec!["[local runtime] running subagents (use wait_agent / close_agent):".to_string()];
    for handle in running.iter().take(MAX_CONCURRENT_SUBAGENTS) {
        let head: String = handle
            .task
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect();
        lines.push(format!(
            "- agent_id={} role={} · {head}",
            handle.agent_id,
            handle.role.as_str()
        ));
    }
    lines
}
