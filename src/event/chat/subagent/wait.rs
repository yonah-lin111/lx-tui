//! `wait_agent` 的等待/续等/结算：从生命周期模块拆出，保持单文件职责与行数上限。

use std::time::Duration;

use tokio::sync::oneshot;

use super::*;
use crate::chat::agent::{
    AgentStatusView, MAX_WAIT_TARGETS, NOT_FOUND_CODE, SubagentOutcome, WaitMode,
};

/// wait：等目标结束或超时；结果由事件层读库结算。
///
/// `targets` 为空且 `all` 为真时表示“本会话所有子 agent”。`wait_for=all` 由事件层
/// 在超时后自动续等（见 [`handle_subagent_wait_ready`]）。
// 等待参数包含目标、模式、超时、续等开关与回执；拆成一个结构只会增加搬运层。
#[allow(clippy::too_many_arguments)]
pub(super) fn wait(
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    requested: Vec<String>,
    all: bool,
    mode: WaitMode,
    timeout_ms: u64,
    auto_continue: bool,
    reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
) {
    let Some(sender) = streams.event_sender.clone() else {
        let _ = reply.send(Err(HostError::Failed {
            code: "subagent event channel unavailable".into(),
        }));
        return;
    };
    let parent = key.session.clone();
    let tree = persistence
        .database()
        .and_then(|database| {
            streams
                .db_workspaces
                .get(&key)
                .map(|workspace| super::runner::load_subagent_tree(database, workspace, &parent))
        })
        .unwrap_or_else(|| super::runner::SubagentTree {
            root_session: parent.clone(),
            caller_depth: 0,
            caller_path: crate::chat::agent::ROOT_AGENT_PATH.to_string(),
            records: Vec::new(),
        });
    let root_session = tree.root_session.clone();
    let records = tree.records.clone();
    let unique: Vec<String> = if all && requested.is_empty() {
        // `wait_for=all` 只等调用者的严格后代：调用者自身的记录也在整树里，
        // 若不排除，嵌套 Executor 等待时会把自己算成目标而永远等不到结束。
        records
            .iter()
            .filter(|record| {
                record.child_session_id != key.session
                    && crate::chat::agent::is_strict_descendant_path(
                        &tree.caller_path,
                        &record.agent_path,
                    )
            })
            .map(|record| record.agent_id.clone())
            .collect()
    } else {
        let mut unique = Vec::new();
        for target in requested {
            let trimmed = target.trim();
            let resolved = super::runner::resolve_tree_target(&tree, trimmed);
            if let Some(record) = resolved
                && (record.child_session_id == key.session
                    || record.agent_path.eq_ignore_ascii_case(&tree.caller_path))
            {
                let _ = reply.send(Err(invalid_target(&format!(
                    "subagent target `{target}` cannot wait on itself"
                ))));
                return;
            }
            let resolved = resolved
                .map(|record| record.agent_id.clone())
                .or_else(|| {
                    streams
                        .subagents
                        .values()
                        .find(|handle| {
                            handle.root_session == root_session
                                && crate::chat::agent::matches_agent_target(
                                    &handle.agent_id,
                                    handle.nickname.as_deref(),
                                    Some(&handle.agent_path),
                                    &tree.caller_path,
                                    trimmed,
                                )
                        })
                        .map(|handle| handle.agent_id.clone())
                })
                .unwrap_or(target);
            if !unique.contains(&resolved) {
                unique.push(resolved);
            }
        }
        unique
    };
    if !all && unique.len() > MAX_WAIT_TARGETS {
        let _ = reply.send(Err(HostError::Failed {
            code: "invalid wait targets".into(),
        }));
        return;
    }
    if !all {
        // 显式目标必须属于本根会话树（在册句柄或 DB 记录）。
        let known =
            |agent_id: &str| {
                streams.subagents.values().any(|handle| {
                    handle.agent_id == agent_id && handle.root_session == root_session
                }) || records.iter().any(|record| record.agent_id == agent_id)
            };
        if unique.iter().any(|target| !known(target)) {
            let _ = reply.send(Err(HostError::Failed {
                code: NOT_FOUND_CODE.to_string(),
            }));
            return;
        }
    }
    // any：调用时已经有一个目标到达终态（结束或已被用户停止），立即结算；
    // 不能再等其它 running 目标。
    if mode == WaitMode::Any
        && unique.iter().any(|target| {
            if streams.user_stopped.contains(target) {
                return true;
            }
            let handle_running = streams
                .subagents
                .values()
                .any(|handle| handle.agent_id == *target && handle.status == STATUS_RUNNING);
            let record_running = records
                .iter()
                .find(|record| &record.agent_id == target)
                .is_some_and(|record| record.status == STATUS_RUNNING);
            !handle_running && !record_running
        })
    {
        let outcome = settle_wait(streams, persistence, &key, &unique, false, all, mode);
        let _ = reply.send(outcome);
        return;
    }
    // 收集运行中句柄的完成接收端；全部已结束或已被用户停止则立刻结算。
    // 停止是终态：等待绝不能停在已被用户停掉的目标上。
    let mut receivers: Vec<(String, watch::Receiver<bool>)> = Vec::new();
    for agent_id in &unique {
        if streams.user_stopped.contains(agent_id) {
            continue;
        }
        if let Some(handle) = streams
            .subagents
            .values()
            .find(|handle| handle.agent_id == *agent_id && handle.status == STATUS_RUNNING)
        {
            receivers.push((agent_id.clone(), handle.done.subscribe()));
        }
    }
    if receivers.is_empty() {
        let outcome = settle_wait(streams, persistence, &key, &unique, false, all, mode);
        let _ = reply.send(outcome);
        return;
    }
    let timeout = Duration::from_millis(timeout_ms.min(crate::chat::agent::MAX_WAIT_TIMEOUT_MS));
    // 主轮 Esc 只取消等待本身，不取消被等待的子 agent：子 agent 继续跑，
    // 结果留在 Agents 面板；用户可在面板 `^d` 精确停止选中的那个。
    let mut parent_cancel = streams.cancels.get(&key).map(|sender| sender.subscribe());
    // 用户主动停止目标时把目标从等待集合剔除并**立即结算**：停止是终态，
    // 不能让主轮永远停在一个已经不会结束的目标上。订阅必须在 spawn 之前完成，
    // 避免停止事件落在订阅窗口之外。
    let mut user_stops = streams.subscribe_user_stop();
    let watched = unique.clone();
    tokio::spawn(async move {
        // 仍参与等待的目标（有运行句柄、且未被用户停止）。
        let mut active: std::collections::HashSet<String> =
            receivers.iter().map(|(target, _)| target.clone()).collect();
        let mut watchers = tokio::task::JoinSet::new();
        for (target, mut receiver) in receivers {
            watchers.spawn(async move {
                loop {
                    if *receiver.borrow() {
                        return target;
                    }
                    if receiver.changed().await.is_err() {
                        return target;
                    }
                }
            });
        }
        let sleep = tokio::time::sleep(timeout);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                biased;
                agent_id = next_user_stop(&mut user_stops, &watched) => {
                    // 停止即终态：Any 立即结算；All 继续等剩余目标，
                    // 剩余目标全部停止时同样立即结算，绝不挂起主轮。
                    if !active.remove(&agent_id) {
                        continue;
                    }
                    if mode == WaitMode::Any || active.is_empty() {
                        let _ = sender.send(AppEvent::SubagentWaitReady {
                            key,
                            targets: watched.clone(),
                            timed_out: false,
                            all,
                            mode,
                            timeout_ms,
                            auto_continue,
                            reply,
                        });
                        return;
                    }
                }
                _ = wait_parent_cancel(&mut parent_cancel) => {
                    let _ = reply.send(Err(HostError::Cancelled));
                    return;
                }
                _ = &mut sleep => {
                    // 超时：auto wait 由事件层重派。目标集合保持完整，
                    // 已停止目标在下一轮 `wait` 中按 `user_stopped` 过滤，
                    // 最终结算结果仍会带上它们，模型能明确知道谁被停了。
                    let _ = sender.send(AppEvent::SubagentWaitReady {
                        key,
                        targets: watched.clone(),
                        timed_out: true,
                        all,
                        mode,
                        timeout_ms,
                        auto_continue,
                        reply,
                    });
                    return;
                }
                joined = watchers.join_next() => {
                    let Some(Ok(target)) = joined else { continue; };
                    // 已被用户停止的目标：完成事件不参与结算，继续等其它目标。
                    if !active.remove(&target) {
                        continue;
                    }
                    // Any：第一个真实完成即可结算；All：全部真实目标完成才结算。
                    if mode == WaitMode::Any || active.is_empty() {
                        let _ = sender.send(AppEvent::SubagentWaitReady {
                            key,
                            targets: watched.clone(),
                            timed_out: false,
                            all,
                            mode,
                            timeout_ms,
                            auto_continue,
                            reply,
                        });
                        return;
                    }
                }
            }
        }
    });
}

/// 等待父轮取消信号（Esc / 关窗格 / 切换会话）。
async fn wait_parent_cancel(cancel: &mut Option<watch::Receiver<bool>>) {
    match cancel {
        Some(receiver) => loop {
            if *receiver.borrow() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        },
        None => std::future::pending::<()>().await,
    }
}

/// 取下一条命中等待目标的用户停止通知；无关通知与滞后直接跳过。
async fn next_user_stop(
    receiver: &mut tokio::sync::broadcast::Receiver<String>,
    watched: &[String],
) -> String {
    loop {
        match receiver.recv().await {
            Ok(agent_id) if watched.contains(&agent_id) => return agent_id,
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                std::future::pending::<()>().await;
            }
        }
    }
}

/// 等待目标本身（调用者自己）不是合法目标：稳定错误、且不进入等待。
fn invalid_target(message: &str) -> HostError {
    HostError::Failed {
        code: message.to_string(),
    }
}

/// 等待到期后的结算：读 DB 状态与未投递报告，领取正文（恰好一次）。
pub(crate) fn settle_wait(
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    targets: &[String],
    timed_out: bool,
    all: bool,
    mode: WaitMode,
) -> Result<SubagentOutcome, HostError> {
    let Some(database) = persistence.database() else {
        return Err(HostError::Failed {
            code: "subagent persistence unavailable".into(),
        });
    };
    let Some(workspace) = streams.db_workspaces.get(key) else {
        return Err(HostError::Failed {
            code: NOT_FOUND_CODE.to_string(),
        });
    };
    let tree = super::runner::load_subagent_tree(database, workspace, &key.session);
    let queue_states = super::runner::tree_queue_states(database, workspace, &tree);
    let records = tree.records;
    let mut agents = Vec::new();
    for target in targets {
        let Some(record) = records.iter().find(|record| &record.agent_id == target) else {
            continue;
        };
        let is_running = streams
            .subagents
            .values()
            .any(|h| h.agent_id == *target && h.status == STATUS_RUNNING);
        let mut report_bodies = Vec::new();
        // 只领取调用者自己的直接子报告：祖辈 `wait_agent` 不能把孙 agent 的报告
        // 从它的 depth-1 父 Executor 手里截走（否则父级再也收不到逐轮报告）。
        // 用户主动停止的目标不领报告：停止是终态，设计承诺 `report=null`，
        // 旧轮次遗留的未投递报告留给以后非停止的等待。
        if !is_running
            && !streams.user_stopped.contains(target)
            && record.parent_session_id == key.session
        {
            let pending = database
                .pending_reports(workspace, &record.parent_session_id)
                .unwrap_or_default();
            let mut claimed_report_ids = Vec::new();
            for item in pending.iter().filter(|r| &r.agent_id == target) {
                if let Some(view) =
                    crate::app::state::SubagentReportView::from_json(&item.report_json)
                {
                    report_bodies.push(view.body_with_changed_paths());
                    claimed_report_ids.push(item.report_id.clone());
                }
            }
            if !claimed_report_ids.is_empty() {
                let _ = database.mark_reports_delivered_in_batch(
                    workspace,
                    &record.parent_session_id,
                    &claimed_report_ids,
                );
            }
        }
        let report = if report_bodies.is_empty() {
            None
        } else {
            Some(report_bodies.join("\n\n"))
        };
        let status = super::projected_agent_status(streams, record, queue_states.get(target));
        agents.push(AgentStatusView {
            agent_id: record.agent_id.clone(),
            nickname: record.nickname.clone(),
            agent_path: Some(record.agent_path.clone()),
            title: record.title.clone(),
            role: record.role,
            status,
            stopped_by_user: streams.user_stopped.contains(target),
            report,
        });
    }
    Ok(SubagentOutcome::Waited {
        timed_out,
        agents,
        all,
        mode,
    })
}

/// 等待到期事件：在事件循环内读库结算并回执。
///
/// `auto_continue` 只用于 `wait_for=all`：一次 wait 超时后仍有子 agent 在跑时，
/// **不把 timeout 交回模型**，而是重新派发同一份等待，直到全部结束或用户取消本轮。
// 与 wait 共用同一组等待参数；拆结构只会让事件字段与函数参数来回搬运。
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_subagent_wait_ready(
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    targets: Vec<String>,
    timed_out: bool,
    all: bool,
    mode: WaitMode,
    timeout_ms: u64,
    auto_continue: bool,
    reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
) -> bool {
    let parent = key.session.clone();
    let root_session = persistence
        .database()
        .zip(streams.db_workspaces.get(&key))
        .and_then(|(database, workspace)| {
            database
                .resolve_session_tree_root(workspace, &parent)
                .ok()
                .map(|(root, _, _)| root)
        })
        .unwrap_or(parent);
    // 已停止的目标不再算“仍在跑”：停止是终态，续等/结算都必须跳过它。
    let still_running = streams.subagents.values().any(|handle| {
        handle.status == STATUS_RUNNING
            && targets.contains(&handle.agent_id)
            && !streams.user_stopped.contains(&handle.agent_id)
    });
    let any_target_finished = !targets.is_empty()
        && targets.iter().any(|target| {
            streams.user_stopped.contains(target)
                || !streams.subagents.values().any(|handle| {
                    handle.root_session == root_session
                        && handle.agent_id == *target
                        && handle.status == STATUS_RUNNING
                })
        });
    if !reply.is_closed()
        && ((auto_continue && still_running)
            || (mode == WaitMode::Any && !timed_out && still_running && !any_target_finished))
    {
        // all 模式把 timeout 当作内部重检周期；0/1ms 会把事件循环打成忙轮询。
        // 保留最小 500ms 间隔，any 模式的上限语义不受影响。
        let next_timeout = if auto_continue {
            timeout_ms.max(500)
        } else {
            timeout_ms
        };
        wait(
            streams,
            persistence,
            key,
            targets,
            all,
            mode,
            next_timeout,
            auto_continue,
            reply,
        );
        return false;
    }
    if reply.is_closed() {
        return false;
    }
    // 所有目标恰好在超时后结束：不把 timed_out 交给模型。
    let timed_out = timed_out && still_running;
    let outcome = settle_wait(streams, persistence, &key, &targets, timed_out, all, mode);
    let _ = reply.send(outcome);
    false
}
