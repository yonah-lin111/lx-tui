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
    let records = persistence
        .database()
        .and_then(|database| {
            streams
                .db_workspaces
                .get(&key)
                .and_then(|workspace| database.list_subagents(workspace, &parent).ok())
        })
        .unwrap_or_default();
    let unique: Vec<String> = if all {
        records
            .iter()
            .map(|record| record.agent_id.clone())
            .collect()
    } else {
        let mut unique = Vec::new();
        for target in requested {
            if !unique.contains(&target) {
                unique.push(target);
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
        // 显式目标必须属于本父会话（在册句柄或 DB 记录）。
        let known = |agent_id: &str| {
            streams
                .subagents
                .values()
                .any(|handle| handle.agent_id == agent_id && handle.parent.session == parent)
                || records.iter().any(|record| record.agent_id == agent_id)
        };
        if unique.iter().any(|target| !known(target)) {
            let _ = reply.send(Err(HostError::Failed {
                code: NOT_FOUND_CODE.to_string(),
            }));
            return;
        }
    }
    // any：调用时已经有一个目标结束（没有 running 句柄），立即结算；
    // 不能再等其它 running 目标。
    if mode == WaitMode::Any
        && unique.iter().any(|target| {
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
    // 收集运行中句柄的完成接收端；全部已结束则立刻结算。
    let mut receivers = Vec::new();
    for agent_id in &unique {
        if let Some(handle) = streams
            .subagents
            .values()
            .find(|handle| handle.agent_id == *agent_id && handle.status == STATUS_RUNNING)
        {
            receivers.push(handle.done.subscribe());
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
    tokio::spawn(async move {
        let wait_done = async {
            match mode {
                WaitMode::Any => {
                    // 任意一个 receiver 完成即可。用 JoinSet 持有子任务，超时或
                    // 父轮取消时统一 abort，避免每次 any 等待都遗留 N 个 detached
                    // watcher 直到子 agent 自己结束。
                    let mut set = tokio::task::JoinSet::new();
                    for mut receiver in receivers {
                        set.spawn(async move {
                            loop {
                                if *receiver.borrow() {
                                    return;
                                }
                                if receiver.changed().await.is_err() {
                                    return;
                                }
                            }
                        });
                    }
                    let timed_out = tokio::time::timeout(timeout, async {
                        let _ = set.join_next().await;
                    })
                    .await
                    .is_err();
                    set.abort_all();
                    timed_out
                }
                WaitMode::All => tokio::time::timeout(timeout, async {
                    for mut receiver in receivers {
                        if *receiver.borrow() {
                            continue;
                        }
                        while receiver.changed().await.is_ok() {
                            if *receiver.borrow() {
                                break;
                            }
                        }
                    }
                })
                .await
                .is_err(),
            }
        };
        let cancelled = async {
            match parent_cancel.as_mut() {
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
        };
        tokio::select! {
            _ = cancelled => {
                let _ = reply.send(Err(HostError::Cancelled));
            }
            timed_out = wait_done => {
                let _ = sender.send(AppEvent::SubagentWaitReady {
                    key,
                    targets: unique,
                    timed_out,
                    all,
                    mode,
                    timeout_ms,
                    auto_continue,
                    reply,
                });
            }
        }
    });
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
    let records = database
        .list_subagents(workspace, &key.session)
        .map_err(|error| HostError::Failed {
            code: error.code().to_string(),
        })?;
    let mut agents = Vec::new();
    for target in targets {
        let Some(record) = records.iter().find(|record| &record.agent_id == target) else {
            continue;
        };
        let mut report = None;
        if matches!(record.status.as_str(), STATUS_COMPLETED | STATUS_FAILED)
            && !record.report_delivered
            && let Some(view) = record
                .report_json
                .as_deref()
                .and_then(SubagentReportView::from_json)
            && database
                .mark_report_delivered(workspace, target)
                .unwrap_or(false)
        {
            report = Some(view.body);
        }
        agents.push(AgentStatusView {
            agent_id: record.agent_id.clone(),
            title: record.title.clone(),
            role: record.role,
            status: record.status.clone(),
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
    let still_running = streams.subagents.values().any(|handle| {
        handle.parent.session == parent
            && handle.status == STATUS_RUNNING
            && (all || targets.iter().any(|target| target == &handle.agent_id))
    });
    if auto_continue && still_running && !reply.is_closed() {
        // all 模式把 timeout 当作内部重检周期；0/1ms 会把事件循环打成忙轮询。
        // 保留最小 500ms 间隔，any 模式的上限语义不受影响。
        wait(
            streams,
            persistence,
            key,
            if all { Vec::new() } else { targets },
            all,
            mode,
            timeout_ms.max(500),
            true,
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
