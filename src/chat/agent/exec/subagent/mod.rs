//! `exec` 无头子 agent 池：spawn / send_input / resume / wait / close / list / cancel_input。
//!
//! 子轮仍是独立子会话：工具握手与报告结算直接写 Database，流增量经 unbounded
//! channel 回到无头事件循环聚合为 `ChatPart`。与 [`super::runner`] 同边界：不导入
//! `app` / `ui` / `tui` / `event`。
//!
//! 池控制入口在本文件；子轮上下文装配见 [`context`]，输入归档与宿主事件见 [`turn`]，
//! 等待与结算见 [`settle`]。

mod context;
mod settle;
mod turn;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::sync::{mpsc, oneshot, watch};

use crate::chat::agent::{
    AgentListItemView, AgentRole, INPUT_NOT_FOUND_CODE, LIMIT_REACHED_CODE, NOT_ACTIVE_CODE,
    NOT_FOUND_CODE, SubagentInputItem, SubagentOp, SubagentOutcome, WaitMode, path_depth,
};
use crate::chat::agent_loop::{
    AgentHost, AgentHostRequest, HostError, TurnSummary, run_agent_turn,
};
use crate::chat::compact::AutoCompactNotice;
use crate::chat::events::ChatDelta;
use crate::chat::process::ProcessManager;
use crate::db::chat::{ChatPart, SubagentQueueState, SubagentRecord, TurnOutcome, Usage};
use crate::db::{Database, new_external_id};

use super::cli::ExecModelTarget;
use super::runner::{ExecLoopEvent, ExecStreamFactory};

const STATUS_RUNNING: &str = "running";
const STATUS_IDLE: &str = "idle";
const STATUS_INTERRUPTED: &str = "interrupted";
const CHECKPOINT_BYTES: usize = 8 * 1024;

/// 子轮内部事件：由子任务回投无头事件循环，或由等待 watcher 回投。
pub(crate) enum ExecSubagentEvent {
    Delta {
        agent_id: String,
        child_session_id: String,
        child_assistant_id: String,
        deltas: Vec<ChatDelta>,
    },
    Host {
        agent_id: String,
        child_session_id: String,
        child_assistant_id: String,
        request: AgentHostRequest,
    },
    AutoCompact {
        child_session_id: String,
        notice: AutoCompactNotice,
        reply: oneshot::Sender<Result<(), HostError>>,
    },
    Finished {
        agent_id: String,
        child_session_id: String,
        child_assistant_id: String,
        child_user_message_id: String,
        summary: TurnSummary,
    },
    WaitReady {
        wait_id: u64,
        timed_out: bool,
    },
}

struct ChildHandle {
    agent_id: String,
    /// 子 agent 的真实父会话（孙 agent 的队列与续跑按它归属，而不是池的根会话）。
    parent_session_id: String,
    child_session_id: String,
    child_assistant_id: String,
    child_user_message_id: String,
    /// 规范层级路径（如 `/root/turing`）。
    agent_path: String,
    role: AgentRole,
    title: String,
    started_at_ms: i64,
    parts: Vec<ChatPart>,
    next_position: i64,
    open_part_index: Option<usize>,
    cancel_tx: watch::Sender<bool>,
    done_tx: watch::Sender<bool>,
    processes: Arc<ProcessManager>,
    join_handle: Option<tokio::task::JoinHandle<()>>,
    uncheckpointed_bytes: usize,
    /// `close_agent` / 退出关闭：结算时不生成半程汇报。
    closed_without_report: bool,
}

struct PendingWait {
    targets: Vec<String>,
    all: bool,
    mode: WaitMode,
    timeout_ms: u64,
    auto_continue: bool,
    /// 发起等待的会话：只有它名下的报告可在结算时领取。
    caller_session: String,
    parent_cancel: watch::Receiver<bool>,
    reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
    cancelled: Arc<AtomicBool>,
}

/// 子轮宿主：把 loop 的请求转发到 exec 事件循环。
struct ChildHost {
    sender: mpsc::UnboundedSender<ExecLoopEvent>,
    agent_id: String,
    child_session_id: String,
    child_assistant_id: String,
}

impl AgentHost for ChildHost {
    fn send(&mut self, request: AgentHostRequest) {
        let event = ExecSubagentEvent::Host {
            agent_id: self.agent_id.clone(),
            child_session_id: self.child_session_id.clone(),
            child_assistant_id: self.child_assistant_id.clone(),
            request,
        };
        let _ = self.sender.send(ExecLoopEvent::Subagent(event));
    }

    fn notify_auto_compact(
        &mut self,
        notice: &AutoCompactNotice,
    ) -> impl Future<Output = Result<(), HostError>> + Send {
        let (reply, receiver) = oneshot::channel();
        let sent = self
            .sender
            .send(ExecLoopEvent::Subagent(ExecSubagentEvent::AutoCompact {
                child_session_id: self.child_session_id.clone(),
                notice: notice.clone(),
                reply,
            }))
            .is_ok();
        async move {
            if !sent {
                return Err(HostError::Cancelled);
            }
            receiver.await.unwrap_or(Err(HostError::Cancelled))
        }
    }
}

/// exec 模式下的子 agent 池：串行处理全部控制请求与子轮事件。
pub(crate) struct ExecSubagentPool {
    workspace: PathBuf,
    cwd: PathBuf,
    parent_session_id: String,
    model: ExecModelTarget,
    exa_api_key: String,
    tavily_api_key: String,
    mcp_snapshot: Arc<crate::mcp::McpToolSnapshot>,
    sender: mpsc::UnboundedSender<ExecLoopEvent>,
    stream_factory: Arc<dyn ExecStreamFactory>,
    running: HashMap<String, ChildHandle>,
    resumed: HashSet<String>,
    interrupted: HashSet<String>,
    paused_by_interrupt: HashSet<String>,
    /// 由 `close_agent` / 退出关闭的 agent：结算时不生成半程汇报。
    closed: HashSet<String>,
    child_models: HashMap<String, ExecModelTarget>,
    /// 本次派发生效的单轮 Token 预算（`agent_id -> limit`；队列轮复用）。
    agent_budgets: HashMap<String, Option<u64>>,
    subagents_config: crate::config::subagents::SubagentsSection,
    waits: HashMap<u64, PendingWait>,
    next_wait_id: u64,
}

impl ExecSubagentPool {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        workspace: PathBuf,
        cwd: PathBuf,
        parent_session_id: String,
        model: ExecModelTarget,
        exa_api_key: String,
        tavily_api_key: String,
        mcp_snapshot: Arc<crate::mcp::McpToolSnapshot>,
        sender: mpsc::UnboundedSender<ExecLoopEvent>,
        stream_factory: Arc<dyn ExecStreamFactory>,
    ) -> Self {
        Self {
            workspace,
            cwd,
            parent_session_id,
            model,
            exa_api_key,
            tavily_api_key,
            mcp_snapshot,
            sender,
            stream_factory,
            running: HashMap::new(),
            resumed: HashSet::new(),
            interrupted: HashSet::new(),
            paused_by_interrupt: HashSet::new(),
            closed: HashSet::new(),
            child_models: HashMap::new(),
            agent_budgets: HashMap::new(),
            subagents_config: crate::config::subagents::load_subagents_or_default(),
            waits: HashMap::new(),
            next_wait_id: 1,
        }
    }

    pub(crate) fn handle_op(
        &mut self,
        db: &Database,
        parent_assistant_id: &str,
        call_id: Option<&str>,
        op: SubagentOp,
        parent_cancel: &watch::Receiver<bool>,
        reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
    ) {
        match op {
            SubagentOp::Spawn {
                role,
                title,
                task_name,
                task,
                items,
                fork_context,
                fork_turns,
                max_tokens,
                model,
                reasoning_effort,
            } => {
                let out = self.spawn(
                    db,
                    parent_assistant_id,
                    call_id,
                    role,
                    &title,
                    task_name.as_deref(),
                    &task,
                    &items,
                    fork_context,
                    fork_turns,
                    max_tokens,
                    model.as_deref(),
                    reasoning_effort.as_deref(),
                );
                let _ = reply.send(out);
            }
            SubagentOp::Wait {
                targets,
                mode,
                timeout_ms,
            } => self.wait(db, targets, mode, timeout_ms, parent_cancel, reply),
            SubagentOp::SendInput {
                target,
                items,
                interrupt,
                trigger_turn,
            } => {
                let out = self.send_input(
                    db,
                    parent_assistant_id,
                    call_id,
                    &target,
                    &items,
                    interrupt,
                    trigger_turn,
                );
                let _ = reply.send(out);
            }
            SubagentOp::Interrupt { target } => {
                let out = self.interrupt(db, &target);
                let _ = reply.send(out);
            }
            SubagentOp::Resume { id } => {
                let out = self.resume(db, &id);
                let _ = reply.send(out);
            }
            SubagentOp::Close { target } => {
                let out = self.close(db, &target);
                let _ = reply.send(out);
            }
            SubagentOp::List { path_prefix } => {
                let out = self.list(db, path_prefix.as_deref());
                let _ = reply.send(out);
            }
            SubagentOp::CancelInput { submission_id } => {
                let out = self.cancel_input(db, &submission_id);
                let _ = reply.send(out);
            }
        }
    }

    pub(crate) fn handle_event(&mut self, db: &Database, event: ExecSubagentEvent) {
        match event {
            ExecSubagentEvent::Delta {
                agent_id,
                child_session_id,
                child_assistant_id,
                deltas,
            } => self.handle_delta(
                db,
                &agent_id,
                &child_session_id,
                &child_assistant_id,
                &deltas,
            ),
            ExecSubagentEvent::Host {
                agent_id,
                child_session_id,
                child_assistant_id,
                request,
            } => self.handle_host(
                db,
                &agent_id,
                &child_session_id,
                &child_assistant_id,
                request,
            ),
            ExecSubagentEvent::AutoCompact {
                child_session_id,
                notice,
                reply,
            } => {
                let result = self.commit_compact(db, &child_session_id, &notice);
                let _ = reply.send(result);
            }
            ExecSubagentEvent::Finished {
                agent_id,
                child_session_id,
                child_assistant_id,
                child_user_message_id,
                summary,
            } => self.finish(
                db,
                &agent_id,
                &child_session_id,
                &child_assistant_id,
                &child_user_message_id,
                summary,
            ),
            ExecSubagentEvent::WaitReady { wait_id, timed_out } => {
                self.handle_wait_ready(db, wait_id, timed_out)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn send_input(
        &mut self,
        db: &Database,
        parent_assistant_id: &str,
        call_id: Option<&str>,
        target: &str,
        items: &[SubagentInputItem],
        interrupt: bool,
        trigger_turn: bool,
    ) -> Result<SubagentOutcome, HostError> {
        let record = db
            .resolve_owned_subagent(&self.workspace, &self.parent_session_id, target)
            .map_err(db_failed)?
            .ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let target = record.agent_id.as_str();
        // 队列与续跑必须归属目标真正的父会话：目标可能是孙 agent。
        let owner_session = record.parent_session_id.clone();
        self.closed.remove(target);
        if let Some(handle) = self.running.get(target)
            && handle.closed_without_report
        {
            return Err(failed(NOT_ACTIVE_CODE));
        }
        let is_running = self.running.contains_key(target);
        if !is_running && !self.resumed.contains(target) {
            return Err(failed(NOT_ACTIVE_CODE));
        }
        // `trigger_turn=false` 只入队：不唤醒 idle 子 agent。
        let interrupt = interrupt && trigger_turn;
        let was_cancelling_from_interrupt = self.paused_by_interrupt.remove(target);
        let supports_image_input = self.child_model_for(&record).supports_image_input;
        let persisted =
            self.prepare_input_items(&record.child_session_id, items, supports_image_input)?;
        let dedupe = match call_id.filter(|cid| !cid.trim().is_empty()) {
            Some(cid) => format!(
                "send:{}:{}:{cid}",
                self.parent_session_id, parent_assistant_id
            ),
            None => format!(
                "send:{}:{}",
                self.parent_session_id,
                new_external_id("dedupe")
            ),
        };
        let enqueued = db
            .enqueue_input(
                &self.workspace,
                &owner_session,
                target,
                &dedupe,
                call_id,
                Some(parent_assistant_id),
                &persisted,
                interrupt,
            )
            .map_err(db_failed)?;
        if !trigger_turn {
            let queues = db
                .subagent_queue_states(&self.workspace, &owner_session)
                .unwrap_or_default();
            let status = self.projected_status(&record, queues.get(target));
            return Ok(SubagentOutcome::InputAccepted {
                submission_id: enqueued.submission_id,
                agent_id: target.to_string(),
                status,
                queued_count: enqueued.queued_count,
                interrupt: false,
                trigger_turn: false,
            });
        }
        if is_running {
            if (interrupt || was_cancelling_from_interrupt)
                && enqueued.created
                && self.running.contains_key(target)
            {
                self.interrupted.insert(target.to_string());
                if let Some(handle) = self.running.get(target) {
                    let _ = handle.cancel_tx.send(true);
                }
            }
            return Ok(SubagentOutcome::InputAccepted {
                submission_id: enqueued.submission_id,
                agent_id: target.to_string(),
                status: enqueued.status,
                queued_count: enqueued.queued_count,
                interrupt,
                trigger_turn: true,
            });
        }
        let (mut status, mut queued_count) = (enqueued.status, enqueued.queued_count);
        if let Some(claimed) = self.claim_and_launch(db, &owner_session, target, None)? {
            if claimed.submission_id == enqueued.submission_id {
                status = STATUS_RUNNING.to_string();
            }
            queued_count = claimed.queued_remaining;
        }
        Ok(SubagentOutcome::InputAccepted {
            submission_id: enqueued.submission_id,
            agent_id: target.to_string(),
            status,
            queued_count,
            interrupt,
            trigger_turn: true,
        })
    }

    fn interrupt(&mut self, db: &Database, target: &str) -> Result<SubagentOutcome, HostError> {
        let record = db
            .resolve_owned_subagent(&self.workspace, &self.parent_session_id, target)
            .map_err(db_failed)?
            .ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let canonical_id = record.agent_id.clone();
        let records = self.load_tree_records(db);
        let queued_count = self
            .tree_queue_states(db, &records)
            .get(&canonical_id)
            .map_or(0, |state| state.queued_count);
        let subtree = subtree_paths(&records, &record.agent_path);
        let mut previous_status = None;
        for handle in self.running.values_mut() {
            if !subtree.contains(&handle.agent_path) {
                continue;
            }
            self.paused_by_interrupt.insert(handle.agent_id.clone());
            self.resumed.insert(handle.agent_id.clone());
            self.interrupted.remove(&handle.agent_id);
            if handle.agent_id == canonical_id {
                previous_status = Some(STATUS_RUNNING.to_string());
            }
            let _ = handle.cancel_tx.send(true);
        }
        if let Some(previous) = previous_status {
            return Ok(SubagentOutcome::Interrupted {
                agent_id: canonical_id,
                nickname: record.nickname,
                previous_status: previous,
                queued_count,
            });
        }
        if self.resumed.contains(&canonical_id) {
            self.paused_by_interrupt.insert(canonical_id.clone());
            return Ok(SubagentOutcome::Interrupted {
                agent_id: canonical_id,
                nickname: record.nickname,
                previous_status: STATUS_IDLE.to_string(),
                queued_count,
            });
        }
        Err(failed(NOT_ACTIVE_CODE))
    }

    fn resume(&mut self, db: &Database, agent_id: &str) -> Result<SubagentOutcome, HostError> {
        let record = db
            .resolve_owned_subagent(&self.workspace, &self.parent_session_id, agent_id)
            .map_err(db_failed)?
            .ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let agent_id = record.agent_id.as_str();
        let owner_session = record.parent_session_id.clone();
        // close 过再 resume：清掉结算抑制标记，新一轮报告必须正常投递。
        self.closed.remove(agent_id);
        // 先判运行态：interrupt 取消未落定时，resume 只是把「恢复」意图交给结算路径。
        let is_running = self.running.contains_key(agent_id);
        if self.paused_by_interrupt.remove(agent_id) && is_running {
            self.interrupted.insert(agent_id.to_string());
        }
        let queued_count = db
            .subagent_queue_states(&self.workspace, &owner_session)
            .unwrap_or_default()
            .get(agent_id)
            .map_or(0, |state| state.queued_count);
        if is_running {
            self.resumed.insert(agent_id.to_string());
            return Ok(SubagentOutcome::Resumed {
                agent_id: agent_id.to_string(),
                status: STATUS_RUNNING.to_string(),
                queued_count,
            });
        }
        let max_concurrent = self
            .subagents_config
            .max_concurrent
            .clamp(1, crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT);
        if queued_count > 0 && self.running.len() >= max_concurrent {
            return Err(failed(LIMIT_REACHED_CODE));
        }
        db.recover_subagent_for_resume(&self.workspace, &owner_session, agent_id)
            .map_err(db_failed)?;
        self.resumed.insert(agent_id.to_string());
        match self.claim_and_launch(db, &owner_session, agent_id, None)? {
            Some(claimed) => Ok(SubagentOutcome::Resumed {
                agent_id: agent_id.to_string(),
                status: STATUS_RUNNING.to_string(),
                queued_count: claimed.queued_remaining,
            }),
            None => Ok(SubagentOutcome::Resumed {
                agent_id: agent_id.to_string(),
                status: STATUS_IDLE.to_string(),
                queued_count: 0,
            }),
        }
    }

    fn close(&mut self, db: &Database, target: &str) -> Result<SubagentOutcome, HostError> {
        let record = db
            .resolve_owned_subagent(&self.workspace, &self.parent_session_id, target)
            .map_err(db_failed)?
            .ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let target = record.agent_id.as_str();
        let is_running = self.running.contains_key(target);
        let previous_status = if is_running {
            STATUS_RUNNING.to_string()
        } else if self.resumed.contains(target) {
            STATUS_IDLE.to_string()
        } else {
            record.status
        };
        let tree = self.load_tree_records(db);
        let subtree = subtree_paths(&tree, &record.agent_path);
        // 级联：目标及其所有存活后代标记「关闭不产报告」并取消在途轮次。
        for handle in self.running.values_mut() {
            if !subtree.contains(&handle.agent_path) {
                continue;
            }
            handle.closed_without_report = true;
            let _ = handle.cancel_tx.send(true);
            self.closed.insert(handle.agent_id.clone());
            self.interrupted.remove(&handle.agent_id);
            self.paused_by_interrupt.remove(&handle.agent_id);
            self.resumed.remove(&handle.agent_id);
        }
        for agent in tree
            .iter()
            .filter(|agent| subtree.contains(&agent.agent_path))
        {
            // 只有仍在运行的 agent 才需要抑制报告；给 idle agent 打标记会毒化后续 resume。
            if self.running.contains_key(&agent.agent_id) {
                self.closed.insert(agent.agent_id.clone());
            }
            self.interrupted.remove(&agent.agent_id);
            self.paused_by_interrupt.remove(&agent.agent_id);
            self.resumed.remove(&agent.agent_id);
            // 目标及其后代的排队输入一并取消，避免留下可被误恢复的孤儿队列。
            let _ =
                db.close_idle_subagent(&self.workspace, &agent.parent_session_id, &agent.agent_id);
        }
        Ok(SubagentOutcome::Closed {
            agent_id: target.to_string(),
            previous_status,
        })
    }

    fn list(&self, db: &Database, path_prefix: Option<&str>) -> Result<SubagentOutcome, HostError> {
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
            .unwrap_or_default();
        let queues = self.tree_queue_states(db, &records);
        let agents = records
            .iter()
            .filter(|record| {
                path_prefix.is_none_or(|prefix| {
                    crate::chat::agent::path_matches_caller_prefix(
                        &record.agent_path,
                        &caller_path,
                        prefix,
                    )
                })
            })
            .map(|record| AgentListItemView {
                agent_id: record.agent_id.clone(),
                nickname: record.nickname.clone(),
                agent_path: Some(record.agent_path.clone()),
                role: record.role,
                title: record.title.clone(),
                status: self.projected_status(record, queues.get(&record.agent_id)),
                queued_count: queues.get(&record.agent_id).map_or(0, |q| q.queued_count),
            })
            .collect();
        Ok(SubagentOutcome::Listed { agents })
    }

    /// 整棵根会话树的记录；查询失败时回退到当前会话的直接子 agent。
    fn load_tree_records(&self, db: &Database) -> Vec<SubagentRecord> {
        let (root_session, _, _) = db
            .resolve_session_tree_root(&self.workspace, &self.parent_session_id)
            .unwrap_or_else(|_| {
                (
                    self.parent_session_id.clone(),
                    0,
                    crate::chat::agent::ROOT_AGENT_PATH.to_string(),
                )
            });
        db.list_subagent_tree(&self.workspace, &root_session)
            .or_else(|_| db.list_subagents(&self.workspace, &self.parent_session_id))
            .unwrap_or_default()
    }

    /// 整树排队状态：按每个子 agent 的真实父会话分别读取后合并（孙 agent 的队列也可见）。
    fn tree_queue_states(
        &self,
        db: &Database,
        records: &[SubagentRecord],
    ) -> HashMap<String, SubagentQueueState> {
        let mut parents = vec![self.parent_session_id.clone()];
        for record in records {
            if !parents.contains(&record.parent_session_id) {
                parents.push(record.parent_session_id.clone());
            }
        }
        let mut states = HashMap::new();
        for parent in parents {
            for (agent_id, state) in db
                .subagent_queue_states(&self.workspace, &parent)
                .unwrap_or_default()
            {
                states.insert(agent_id, state);
            }
        }
        states
    }

    fn cancel_input(
        &self,
        db: &Database,
        submission_id: &str,
    ) -> Result<SubagentOutcome, HostError> {
        // 输入行按目标真实父会话归属：整树寻址时逐个候选父会话尝试。
        let records = self.load_tree_records(db);
        let mut parents = vec![self.parent_session_id.clone()];
        for record in &records {
            if !parents.contains(&record.parent_session_id) {
                parents.push(record.parent_session_id.clone());
            }
        }
        for parent in parents {
            if let Some((agent_id, status, queued_count)) = db
                .cancel_input(&self.workspace, &parent, submission_id)
                .map_err(db_failed)?
            {
                return Ok(SubagentOutcome::InputCancelled {
                    submission_id: submission_id.to_string(),
                    agent_id,
                    status,
                    queued_count,
                });
            }
        }
        Err(failed(INPUT_NOT_FOUND_CODE))
    }

    /// 认领队首输入并启动下一轮；没有可执行输入返回 `Ok(None)`。
    fn claim_and_launch(
        &mut self,
        db: &Database,
        owner_session_id: &str,
        agent_id: &str,
        reused_done: Option<watch::Sender<bool>>,
    ) -> Result<Option<crate::db::chat::ClaimedSubagentTurn>, HostError> {
        if self.running.contains_key(agent_id) {
            return Ok(None);
        }
        let record = db
            .owned_subagent(&self.workspace, owner_session_id, agent_id)
            .map_err(db_failed)?
            .ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let Some(next) = db
            .next_queued_input(&self.workspace, owner_session_id, agent_id)
            .map_err(db_failed)?
        else {
            return Ok(None);
        };
        if let Some(code) = &next.error_code {
            return Err(failed(&format!("subagent input blocked: {code}")));
        }
        let max_concurrent = self
            .subagents_config
            .max_concurrent
            .clamp(1, crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT);
        if self.running.len() >= max_concurrent {
            return Err(failed(LIMIT_REACHED_CODE));
        }
        let child_snapshot = self.child_model_for(&record).snapshot();
        let claimed = db
            .claim_input(
                &self.workspace,
                owner_session_id,
                agent_id,
                &next.submission_id,
                &child_snapshot,
            )
            .map_err(db_failed)?;
        let max_depth = self
            .subagents_config
            .max_depth
            .clamp(1, crate::config::subagents::MAX_AGENT_DEPTH_LIMIT);
        let max_tokens = self
            .agent_budgets
            .get(agent_id)
            .copied()
            .unwrap_or(self.subagents_config.default_max_tokens);
        self.start_child(
            db,
            agent_id,
            &record.parent_session_id,
            &claimed.child_session_id,
            &claimed.child_assistant_id,
            &claimed.child_user_message_id,
            &record.agent_path,
            path_depth(&record.agent_path),
            max_depth,
            max_tokens,
            record.role,
            &record.title,
            claimed.started_at_ms,
            &claimed.items,
            reused_done.unwrap_or_else(|| watch::channel(false).0),
        )?;
        Ok(Some(claimed))
    }

    /// 启动队首输入；用于结算后的续跑判断（`false` 表示队列已空）。
    fn launch_next_queued(
        &mut self,
        db: &Database,
        owner_session_id: &str,
        agent_id: &str,
        reused_done: Option<watch::Sender<bool>>,
    ) -> Result<bool, HostError> {
        Ok(self
            .claim_and_launch(db, owner_session_id, agent_id, reused_done)?
            .is_some())
    }

    fn projected_status(
        &self,
        record: &SubagentRecord,
        queue: Option<&SubagentQueueState>,
    ) -> String {
        let running = self.running.contains_key(&record.agent_id);
        let resumed_or_paused = self.resumed.contains(&record.agent_id)
            || self.paused_by_interrupt.contains(&record.agent_id);
        let has_blocked_head = queue
            .and_then(|state| state.head_error_code.as_ref())
            .is_some();
        let projected = crate::chat::agent::projected_agent_status(
            running,
            resumed_or_paused,
            has_blocked_head,
            &record.status,
        );
        if projected == crate::chat::agent::AgentStatus::Cancelled
            && queue.and_then(|state| state.last_input_status.as_deref())
                == Some(STATUS_INTERRUPTED)
        {
            return STATUS_INTERRUPTED.to_string();
        }
        projected.as_str().to_string()
    }
}

/// 目标自身与其所有后代路径（按根会话树的记录集合展开）。
fn subtree_paths(records: &[SubagentRecord], target_path: &str) -> Vec<String> {
    let prefix = format!(
        "{}/",
        target_path.trim_end_matches('/').to_ascii_lowercase()
    );
    let mut paths = vec![target_path.to_string()];
    for record in records {
        let candidate = record.agent_path.to_ascii_lowercase();
        if candidate != target_path.to_ascii_lowercase() && candidate.starts_with(&prefix) {
            paths.push(record.agent_path.clone());
        }
    }
    paths
}

fn failed(code: &str) -> HostError {
    HostError::Failed {
        code: code.to_string(),
    }
}

fn db_failed(error: crate::db::DbError) -> HostError {
    failed(error.code())
}

fn tool_failed(prefix: &str, code: &str) -> HostError {
    failed(&format!("{prefix} · {code}"))
}

fn first_line_title(task: &str) -> String {
    task.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("subagent")
        .chars()
        .take(80)
        .collect()
}

fn close_open_part(parts: &mut [ChatPart], open_part_index: &mut Option<usize>) {
    if let Some(index) = open_part_index.take()
        && let Some(part) = parts.get_mut(index)
    {
        part.completed_at_ms = Some(crate::db::now_ms());
    }
}
