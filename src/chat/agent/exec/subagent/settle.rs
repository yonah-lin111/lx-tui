//! 子轮等待、自动续等与终态结算。
//!
//! 池控制入口（[`super::ExecSubagentPool`]）在超时或子轮结束时把事件交回本模块：
//! 等待只读池内状态，结算与报告领取直接写 Database。

use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::{oneshot, watch};

use crate::chat::agent::{
    AgentStatusView, MAX_WAIT_TARGETS, SubagentReportView, synthesize_partial_report_body,
};
use crate::chat::agent_loop::TurnOutcome as LoopOutcome;
use crate::db::chat::{CompactCommit, ReportPayload};
use crate::db::{Database, new_external_id, now_ms};

use super::*;

impl ExecSubagentPool {
    /// 取消并收割全部在途子 agent，提交 `interrupted` 终态。
    pub(crate) async fn shutdown_all(&mut self, db: &Database) {
        let handles: Vec<ChildHandle> = self.running.drain().map(|(_, handle)| handle).collect();
        for handle in &handles {
            let _ = handle.cancel_tx.send(true);
        }
        for mut handle in handles {
            if let Some(join) = handle.join_handle.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), join).await;
            }
            close_open_part(&mut handle.parts, &mut handle.open_part_index);
            if let Err(error) = db.finish_subagent_turn(
                &self.workspace,
                &handle.agent_id,
                &handle.child_assistant_id,
                &handle.parts,
                &TurnOutcome::Interrupted,
                Usage::default(),
                Usage::default(),
                None,
            ) {
                tracing::warn!(target: "lx::exec", code = error.code(), "subagent shutdown finish failed");
            }
            handle.processes.shutdown().await;
            crate::chat::tools::drop_reads(&handle.child_session_id);
        }
        for (_, pending) in self.waits.drain() {
            let _ = pending.reply.send(Err(HostError::Cancelled));
        }
        self.resumed.clear();
        self.interrupted.clear();
        self.paused_by_interrupt.clear();
        self.closed.clear();
    }

    pub(super) fn finish(
        &mut self,
        db: &Database,
        agent_id: &str,
        child_session_id: &str,
        child_assistant_id: &str,
        child_user_message_id: &str,
        summary: TurnSummary,
    ) {
        let Some(mut handle) = self.running.remove(agent_id) else {
            return;
        };
        let was_interrupted_by_input = self.interrupted.remove(agent_id);
        let was_paused_by_interrupt = self.paused_by_interrupt.remove(agent_id);
        let was_closed = self.closed.remove(agent_id) || handle.closed_without_report;
        close_open_part(&mut handle.parts, &mut handle.open_part_index);
        tracing::debug!(
            target: "lx::exec", agent_id, role = handle.role.as_str(),
            child_user_message_id = %handle.child_user_message_id, "subagent turn finished",
        );
        let outcome = db_outcome(&summary.outcome);
        let report = if was_closed {
            None
        } else {
            let (files_changed, changed_paths) = db
                .list_changed_paths(&self.workspace, child_session_id, child_user_message_id)
                .unwrap_or_default();
            let mut body = last_text_body(&handle.parts);
            // 非正常收尾：即使已有正文也补上半程汇报头，让父 agent 明确这是截断结果。
            if !matches!(summary.outcome, LoopOutcome::Complete)
                && !body.starts_with("[Partial report")
            {
                body = synthesize_partial_report_body(
                    &body,
                    completed_tools(db, &self.workspace, child_session_id, child_assistant_id),
                    &changed_paths,
                    partial_reason_label(&summary),
                );
            }
            Some(ReportPayload {
                title: handle.title.clone(),
                body,
                files_changed,
                changed_paths,
                duration_ms: Some(now_ms().saturating_sub(handle.started_at_ms).max(0)),
            })
        };
        if let Err(error) = db.finish_subagent_turn(
            &self.workspace,
            agent_id,
            child_assistant_id,
            &handle.parts,
            &outcome,
            summary.usage,
            summary.billing,
            report.as_ref(),
        ) {
            tracing::warn!(target: "lx::exec", code = error.code(), "subagent finish failed");
        }
        let mut continued = false;
        if was_closed {
            self.resumed.remove(agent_id);
        } else {
            match &summary.outcome {
                LoopOutcome::Complete => {
                    if was_paused_by_interrupt {
                        // interrupt 与自然完成竞争：保持 idle，队列仍暂停等显式 resume/send_input。
                        self.paused_by_interrupt.insert(agent_id.to_string());
                        self.resumed.insert(agent_id.to_string());
                    } else if self.resumed.contains(agent_id) {
                        let has_triggerable = db
                            .has_triggerable_queued_input(
                                &self.workspace,
                                &handle.parent_session_id,
                                agent_id,
                            )
                            .unwrap_or(false);
                        if has_triggerable {
                            match self.launch_next_queued(
                                db,
                                &handle.parent_session_id,
                                agent_id,
                                Some(handle.done_tx.clone()),
                            ) {
                                Ok(true) => continued = true,
                                Ok(false) => {
                                    self.resumed.remove(agent_id);
                                }
                                // 队首输入 blocked（如 image_missing）：保留队列，停止继续消费。
                                Err(_) => {}
                            }
                        } else {
                            let has_queued = db
                                .next_queued_input(
                                    &self.workspace,
                                    &handle.parent_session_id,
                                    agent_id,
                                )
                                .ok()
                                .flatten()
                                .is_some();
                            if has_queued {
                                self.resumed.insert(agent_id.to_string());
                            } else {
                                self.resumed.remove(agent_id);
                            }
                        }
                    }
                }
                LoopOutcome::Interrupted => {
                    if was_interrupted_by_input {
                        self.resumed.insert(agent_id.to_string());
                        match self.launch_next_queued(
                            db,
                            &handle.parent_session_id,
                            agent_id,
                            Some(handle.done_tx.clone()),
                        ) {
                            Ok(true) => continued = true,
                            Ok(false) => {
                                self.resumed.remove(agent_id);
                            }
                            Err(_) => {}
                        }
                    } else if was_paused_by_interrupt || summary.budget_exceeded {
                        // 保留暂停标记：`cancel_input` 不得越过 interrupt / 预算熔断自动续跑队列。
                        self.paused_by_interrupt.insert(agent_id.to_string());
                        self.resumed.insert(agent_id.to_string());
                    } else {
                        self.resumed.remove(agent_id);
                        let _ = db.close_idle_subagent(
                            &self.workspace,
                            &handle.parent_session_id,
                            agent_id,
                        );
                    }
                }
                LoopOutcome::Failed { .. } => {
                    self.resumed.remove(agent_id);
                }
            }
        }
        let processes = Arc::clone(&handle.processes);
        let session_id = handle.child_session_id.clone();
        tokio::spawn(async move {
            processes.shutdown().await;
            crate::chat::tools::drop_reads(&session_id);
        });
        if !continued {
            let _ = handle.done_tx.send(true);
        }
    }

    /// 自动压缩：把摘要写成子会话检查点后回执（失败不吞错）。
    pub(super) fn commit_compact(
        &self,
        db: &Database,
        child_session_id: &str,
        notice: &AutoCompactNotice,
    ) -> Result<(), HostError> {
        let messages = db
            .load_chat_session(&self.workspace, child_session_id)
            .map_err(db_failed)?
            .unwrap_or_default();
        let end_sequence = messages
            .iter()
            .map(|message| message.sequence)
            .max()
            .unwrap_or(0);
        let base_checkpoint_id = db
            .active_checkpoint(&self.workspace, child_session_id)
            .map_err(db_failed)?
            .map(|checkpoint| checkpoint.id);
        let expected_revision = db.session_revision(child_session_id).map_err(db_failed)?;
        db.commit_compact(
            &self.workspace,
            &CompactCommit {
                session_id: child_session_id.to_string(),
                expected_revision,
                base_checkpoint_id,
                start_sequence: 0,
                end_sequence,
                summary_text: notice.summary.clone(),
                model_label: notice.model_label.clone(),
                operation_id: new_external_id("autocompact"),
            },
        )
        .map_err(db_failed)?;
        Ok(())
    }

    pub(super) fn wait(
        &mut self,
        db: &Database,
        requested: Vec<String>,
        mode: WaitMode,
        timeout_ms: u64,
        parent_cancel: &watch::Receiver<bool>,
        reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
    ) {
        let all = requested.is_empty();
        let (root_session, _, caller_path) = db
            .resolve_session_tree_root(&self.workspace, &self.parent_session_id)
            .unwrap_or_else(|_| {
                (
                    self.parent_session_id.clone(),
                    0,
                    crate::chat::agent::ROOT_AGENT_PATH.to_string(),
                )
            });
        let records = db
            .list_subagent_tree(&self.workspace, &root_session)
            .unwrap_or_else(|_| {
                db.list_subagents(&self.workspace, &self.parent_session_id)
                    .unwrap_or_default()
            });
        let mut unique: Vec<String> = if all {
            // 空 targets 等「我的后代」：只看严格后代，绝不等待调用者自己
            // （嵌套 executor 会因此等自己的 done 信号而永久死锁）。
            records
                .iter()
                .filter(|record| {
                    crate::chat::agent::is_strict_descendant_path(&caller_path, &record.agent_path)
                        && record.child_session_id != self.parent_session_id
                })
                .map(|record| record.agent_id.clone())
                .collect()
        } else {
            let mut resolved = Vec::with_capacity(requested.len());
            for target in requested {
                let trimmed = target.trim();
                let record = records.iter().find(|r| {
                    crate::chat::agent::matches_agent_target(
                        &r.agent_id,
                        r.nickname.as_deref(),
                        Some(&r.agent_path),
                        &caller_path,
                        trimmed,
                    )
                });
                if record.is_some_and(|r| {
                    r.child_session_id == self.parent_session_id
                        || r.agent_path.eq_ignore_ascii_case(&caller_path)
                }) {
                    let _ = reply.send(Err(failed("subagent cannot wait on itself")));
                    return;
                }
                resolved.push(record.map_or_else(|| target.clone(), |r| r.agent_id.clone()));
            }
            resolved
        };
        if !all {
            let mut seen = HashSet::new();
            unique.retain(|target| seen.insert(target.clone()));
        }
        if !all && unique.len() > MAX_WAIT_TARGETS {
            let _ = reply.send(Err(failed("invalid wait targets")));
            return;
        }
        if !all
            && unique.iter().any(|target| {
                !records.iter().any(|record| &record.agent_id == target)
                    && !self.running.contains_key(target)
            })
        {
            let _ = reply.send(Err(failed(NOT_FOUND_CODE)));
            return;
        }
        let any_done = unique
            .iter()
            .any(|target| !self.running.contains_key(target));
        let none_running = unique
            .iter()
            .all(|target| !self.running.contains_key(target));
        if (mode == WaitMode::Any && any_done) || none_running {
            let outcome = self.settle_wait(db, &unique, false, all, mode, &self.parent_session_id);
            let _ = reply.send(outcome);
            return;
        }
        let receivers: Vec<watch::Receiver<bool>> = unique
            .iter()
            .filter_map(|target| {
                self.running
                    .get(target)
                    .map(|handle| handle.done_tx.subscribe())
            })
            .collect();
        if receivers.is_empty() {
            let outcome = self.settle_wait(db, &unique, false, all, mode, &self.parent_session_id);
            let _ = reply.send(outcome);
            return;
        }
        if *parent_cancel.borrow() {
            let _ = reply.send(Err(HostError::Cancelled));
            return;
        }
        let timeout_ms = timeout_ms.min(crate::chat::agent::MAX_WAIT_TIMEOUT_MS);
        // 显式 targets + `wait_for=all` 同样把 timeout 当作内部重检周期。
        let auto_continue = mode == WaitMode::All;
        self.enqueue_wait(
            unique,
            all,
            mode,
            timeout_ms,
            auto_continue,
            self.parent_session_id.clone(),
            parent_cancel.clone(),
            receivers,
            reply,
        );
    }

    /// 登记一次等待并派生 watcher；超时/取消经事件循环回到 [`Self::handle_wait_ready`]。
    #[allow(clippy::too_many_arguments)]
    fn enqueue_wait(
        &mut self,
        targets: Vec<String>,
        all: bool,
        mode: WaitMode,
        timeout_ms: u64,
        auto_continue: bool,
        caller_session: String,
        parent_cancel: watch::Receiver<bool>,
        receivers: Vec<watch::Receiver<bool>>,
        reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
    ) {
        let wait_id = self.next_wait_id;
        self.next_wait_id = self.next_wait_id.saturating_add(1);
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.waits.insert(
            wait_id,
            PendingWait {
                targets,
                all,
                mode,
                timeout_ms,
                auto_continue,
                caller_session,
                parent_cancel: parent_cancel.clone(),
                reply,
                cancelled: Arc::clone(&cancelled),
            },
        );
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let cancelled_task = async {
                let mut parent_cancel = parent_cancel;
                loop {
                    if *parent_cancel.borrow() {
                        return;
                    }
                    if parent_cancel.changed().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                }
            };
            tokio::select! {
                _ = cancelled_task => {
                    cancelled.store(true, Ordering::Relaxed);
                    let _ = sender.send(ExecLoopEvent::Subagent(ExecSubagentEvent::WaitReady { wait_id, timed_out: true }));
                }
                timed_out = wait_until(mode, receivers, Duration::from_millis(timeout_ms)) => {
                    let _ = sender.send(ExecLoopEvent::Subagent(ExecSubagentEvent::WaitReady { wait_id, timed_out }));
                }
            }
        });
    }

    pub(super) fn handle_wait_ready(&mut self, db: &Database, wait_id: u64, timed_out: bool) {
        let Some(pending) = self.waits.remove(&wait_id) else {
            return;
        };
        if pending.cancelled.load(Ordering::Relaxed) {
            let _ = pending.reply.send(Err(HostError::Cancelled));
            return;
        }
        if *pending.parent_cancel.borrow() {
            let _ = pending.reply.send(Err(HostError::Cancelled));
            return;
        }
        // `wait_for=all` 把 timeout 当作内部重检周期：仍有目标在跑就续等。
        if timed_out && pending.auto_continue {
            let receivers: Vec<watch::Receiver<bool>> = pending
                .targets
                .iter()
                .filter_map(|target| {
                    self.running
                        .get(target)
                        .map(|handle| handle.done_tx.subscribe())
                })
                .collect();
            if !receivers.is_empty() {
                let timeout_ms = pending.timeout_ms.max(500);
                self.enqueue_wait(
                    pending.targets,
                    pending.all,
                    pending.mode,
                    timeout_ms,
                    true,
                    pending.caller_session,
                    pending.parent_cancel,
                    receivers,
                    pending.reply,
                );
                return;
            }
        }
        // 所有目标恰好在超时后结束：不把 timed_out 交给模型。
        let still_running = pending
            .targets
            .iter()
            .any(|target| self.running.contains_key(target));
        let timed_out = timed_out && still_running;
        let outcome = self.settle_wait(
            db,
            &pending.targets,
            timed_out,
            pending.all,
            pending.mode,
            &pending.caller_session,
        );
        let _ = pending.reply.send(outcome);
    }

    fn settle_wait(
        &self,
        db: &Database,
        targets: &[String],
        timed_out: bool,
        all: bool,
        mode: WaitMode,
        caller_session: &str,
    ) -> Result<SubagentOutcome, HostError> {
        let (root_session, _, _) = db
            .resolve_session_tree_root(&self.workspace, &self.parent_session_id)
            .unwrap_or_else(|_| {
                (
                    self.parent_session_id.clone(),
                    0,
                    crate::chat::agent::ROOT_AGENT_PATH.to_string(),
                )
            });
        let records = db
            .list_subagent_tree(&self.workspace, &root_session)
            .unwrap_or_default();
        let queues = self.tree_queue_states(db, &records);
        let mut agents = Vec::new();
        for target in targets {
            let Some(record) = records.iter().find(|record| &record.agent_id == target) else {
                continue;
            };
            let mut bodies = Vec::new();
            // 报告只归真实父会话所有：根等待不得偷走孙 agent 交给中间 agent 的报告。
            if !self.running.contains_key(target) && record.parent_session_id == caller_session {
                let pending = db
                    .pending_reports(&self.workspace, &record.parent_session_id)
                    .unwrap_or_default();
                let mut claimed = Vec::new();
                for item in pending.iter().filter(|item| &item.agent_id == target) {
                    if let Some(view) = SubagentReportView::from_json(&item.report_json) {
                        bodies.push(view.body_with_changed_paths());
                        claimed.push(item.report_id.clone());
                    }
                }
                if !claimed.is_empty() {
                    let _ = db.mark_reports_delivered_in_batch(
                        &self.workspace,
                        &record.parent_session_id,
                        &claimed,
                    );
                }
            }
            agents.push(AgentStatusView {
                agent_id: record.agent_id.clone(),
                nickname: record.nickname.clone(),
                agent_path: Some(record.agent_path.clone()),
                title: record.title.clone(),
                role: record.role,
                status: self.projected_status(record, queues.get(target)),
                report: (!bodies.is_empty()).then(|| bodies.join("\n\n")),
            });
        }
        Ok(SubagentOutcome::Waited {
            timed_out,
            agents,
            all,
            mode,
        })
    }
}

/// 等任一 / 全部完成；超时返回 true。
async fn wait_until(
    mode: WaitMode,
    receivers: Vec<watch::Receiver<bool>>,
    timeout: Duration,
) -> bool {
    match mode {
        WaitMode::Any => {
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
}

fn db_outcome(outcome: &LoopOutcome) -> TurnOutcome {
    match outcome {
        LoopOutcome::Complete => TurnOutcome::Complete {
            stop_reason: Some("complete".into()),
        },
        LoopOutcome::Interrupted => TurnOutcome::Interrupted,
        LoopOutcome::Failed { code } => TurnOutcome::Failed {
            error_code: code.clone(),
        },
    }
}

fn last_text_body(parts: &[ChatPart]) -> String {
    parts
        .iter()
        .rev()
        .find(|part| part.kind == "text" && !part.content.trim().is_empty())
        .map(|part| part.content.clone())
        .unwrap_or_default()
}

/// 本轮已结算的工具调用数（半程汇报正文用）。
fn completed_tools(
    db: &Database,
    workspace: &std::path::Path,
    child_session: &str,
    child_assistant_id: &str,
) -> usize {
    db.load_chat_session(workspace, child_session)
        .ok()
        .flatten()
        .and_then(|messages| {
            messages
                .into_iter()
                .find(|message| message.external_id == child_assistant_id)
        })
        .map(|message| message.tools.len())
        .unwrap_or_default()
}

/// 半程汇报的停止原因标签（英文稳定值，写进正文）。
fn partial_reason_label(summary: &TurnSummary) -> &'static str {
    if summary.budget_exceeded {
        return "budget_exceeded";
    }
    match summary.outcome {
        LoopOutcome::Interrupted => "interrupted",
        LoopOutcome::Failed { .. } => "failed",
        LoopOutcome::Complete => "complete",
    }
}
