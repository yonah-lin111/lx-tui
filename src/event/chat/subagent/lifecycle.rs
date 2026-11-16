//! 子 agent 生命周期：对外状态投影、级联取消与用户主动停止。

use super::*;

/// 投影子 agent 对外可见状态（工具结果与 UI 列表同源）。
pub(crate) fn projected_agent_status(
    streams: &ChatStreams,
    record: &SubagentRecord,
    queue: Option<&SubagentQueueState>,
) -> String {
    let running = streams
        .subagents
        .values()
        .any(|handle| handle.agent_id == record.agent_id && handle.status == STATUS_RUNNING);
    let resumed_or_paused = streams.resumed_subagents.contains(&record.agent_id)
        || streams.paused_by_interrupt.contains(&record.agent_id)
        || (record.status != STATUS_FAILED
            && queue.is_some_and(|q| q.queued_count > 0 && q.head_error_code.is_none()));
    let has_blocked_head = queue.and_then(|q| q.head_error_code.as_ref()).is_some();
    let projected = crate::chat::agent::projected_agent_status(
        running,
        resumed_or_paused,
        has_blocked_head,
        &record.status,
    );
    // 崩溃恢复/插队中断留下的 `cancelled + interrupted` 组合投影为中断态。
    if projected == crate::chat::agent::AgentStatus::Cancelled
        && queue.and_then(|q| q.last_input_status.as_deref()) == Some(STATUS_INTERRUPTED)
    {
        return STATUS_INTERRUPTED.to_string();
    }
    projected.as_str().to_string()
}

/// 会话被删除或用户手动停止：取消该根会话树下所有在册子 turn（级联后代，不产报告）。
pub(crate) fn cancel_for_session(
    streams: &mut ChatStreams,
    session_id: &str,
    agent_id: Option<&str>,
    agent_path: Option<&str>,
) -> bool {
    // 种子：显式 agent 自身，或该会话直接相关的句柄；再沿 `agent_path` 级联后代。
    // `agent_path` 来自 DB 记录：目标句柄已结算时仍能按路径找到在跑的后代。
    let seeds: Vec<String> = streams
        .subagents
        .values()
        .filter(|handle| match agent_id {
            Some(id) => handle.agent_id == id,
            None => handle.parent.session == session_id || handle.child_session_id == session_id,
        })
        .map(|handle| handle.agent_path.clone())
        .collect();
    let selected = |path: &str| {
        agent_path.is_some_and(|target| {
            path.eq_ignore_ascii_case(target) || is_strict_descendant_path(target, path)
        }) || seeds
            .iter()
            .any(|seed| path.eq_ignore_ascii_case(seed) || is_strict_descendant_path(seed, path))
    };

    let mut cancelled = false;
    let mut cleared_agents = Vec::new();
    let mut dead = Vec::new();
    for (child, handle) in &mut streams.subagents {
        if !selected(&handle.agent_path) {
            continue;
        }
        cleared_agents.push(handle.agent_id.clone());
        // 用户手动停止 / 会话删除 / 退出：不生成半程汇报。
        handle.closed_without_report = true;
        if handle.cancel.send(true).is_ok() {
            cancelled = true;
        } else {
            dead.push(child.clone());
        }
    }
    for id in cleared_agents {
        streams.resumed_subagents.remove(&id);
        streams.interrupted_subagents.remove(&id);
        streams.paused_by_interrupt.remove(&id);
    }
    if let Some(target) = agent_id {
        streams.resumed_subagents.remove(target);
        streams.interrupted_subagents.remove(target);
        streams.paused_by_interrupt.remove(target);
    }
    for child in dead {
        streams.subagents.remove(&child);
        streams.frozen.remove(&child);
    }
    cancelled
}

/// 用户主动停止：标记目标子树 `stopped_by_user`、取消在途轮次与排队输入。
///
/// 与 [`cancel_for_session`] 的区别：
/// - 只停目标（`Some`）或整棵当前会话树（`None`），不涉及会话删除；
/// - 不设置 `closed_without_report`，结算时按 `stopped_by_user` 抑制报告；
/// - 队列输入全部取消，恢复后只执行新输入；
/// - 标记保留在 `ChatStreams.user_stopped`，直到用户显式恢复。
///
/// 返回被标记的 agent 数（用于 Toast）。
pub(crate) fn stop_subagents_by_user(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    agent_id: Option<&str>,
) -> usize {
    let Some(workspace) = workspace_for_pane(state, pane) else {
        return 0;
    };
    let Some(database) = persistence.database() else {
        return 0;
    };
    let tree = runner::load_subagent_tree(database, &workspace, session_id);
    let selected_paths: Vec<String> = match agent_id {
        Some(target) => match runner::resolve_tree_target(&tree, target) {
            Some(record) => runner::subtree_paths(&tree.records, &record.agent_path),
            None => return 0,
        },
        None => tree
            .records
            .iter()
            .map(|record| record.agent_path.clone())
            .collect(),
    };
    let selected = |path: &str| {
        selected_paths
            .iter()
            .any(|target| path.eq_ignore_ascii_case(target))
    };
    let selected_records: Vec<&SubagentRecord> = tree
        .records
        .iter()
        .filter(|record| selected(&record.agent_path))
        .collect();
    if selected_records.is_empty() {
        return 0;
    }

    // 1. 先写内存标记：即使 handle 在本帧结算，后续状态投影也不会丢失用户意图。
    let mut stopped = 0usize;
    for record in &selected_records {
        streams.user_stopped.insert(record.agent_id.clone());
        stopped += 1;
        // 2. 取消该 agent 自己的排队输入（孙 agent 各自有独立父会话）。
        if let Err(error) = database.cancel_queued_inputs_for_agent(
            &workspace,
            &record.parent_session_id,
            &record.agent_id,
        ) {
            tracing::warn!(
                %error,
                agent_id = %record.agent_id,
                "cancelling queued subagent inputs failed"
            );
        }
    }

    // 3. 取消在途 handle，并清掉“自动恢复”标记。
    let selected_agent_ids: std::collections::HashSet<&str> = selected_records
        .iter()
        .map(|record| record.agent_id.as_str())
        .collect();
    let mut dead = Vec::new();
    for (child, handle) in &mut streams.subagents {
        if !selected_agent_ids.contains(handle.agent_id.as_str()) && !selected(&handle.agent_path) {
            continue;
        }
        handle.stopped_by_user = true;
        if handle.cancel.send(true).is_err() {
            dead.push(child.clone());
        }
    }
    for id in selected_agent_ids {
        streams.resumed_subagents.remove(id);
        streams.interrupted_subagents.remove(id);
        streams.paused_by_interrupt.remove(id);
    }
    for child in dead {
        streams.subagents.remove(&child);
        streams.frozen.remove(&child);
    }

    // 4. 立刻把 UI 标成 stopped、队列清零；结算回执随后只更新正文与终态。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        for record in &selected_records {
            if let Some(run) = target.lx.agents.by_agent_mut(&record.agent_id) {
                run.stopped_by_user = true;
                run.queued_count = 0;
                run.completed_at_ms.get_or_insert_with(crate::db::now_ms);
                run.rows_dirty = true;
            }
        }
        target.lx.sync_mention_subagent_runs();
    }
    update::request_subagent_cards_refresh(state, pane);
    stopped
}
