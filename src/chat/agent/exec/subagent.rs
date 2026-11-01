//! `exec` 无头子 agent 池：spawn / send_input / resume / wait / close / list / cancel_input。
//!
//! 子轮仍是独立子会话：工具握手与报告结算直接写 Database，流增量经 unbounded
//! channel 回到无头事件循环聚合为 `ChatPart`。与 [`super::runner`] 同边界：不导入
//! `app` / `ui` / `tui` / `event`。

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, mpsc, oneshot, watch};

use crate::chat::agent::{
    AgentListItemView, AgentRole, AgentStatusView, IMAGE_MISSING_CODE, INPUT_NOT_FOUND_CODE,
    LIMIT_REACHED_CODE, MAX_CONCURRENT_SUBAGENTS, MAX_WAIT_TARGETS, NOT_ACTIVE_CODE,
    NOT_FOUND_CODE, SubagentInputItem, SubagentOp, SubagentOutcome, SubagentReportView,
    ToolSurface, WaitMode, joined_input_text,
};
use crate::chat::agent_loop::{
    AgentHost, AgentHostRequest, AgentTools, HostError, ModelContext, TurnExecution, TurnSummary,
    run_agent_turn,
};
use crate::chat::compact::AutoCompactNotice;
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, ModelImage, ModelItem, RequestPurpose,
};
use crate::chat::process::{ProcessManager, ProcessOwner};
use crate::chat::prompt::PromptParts;
use crate::db::chat::{
    ChatPart, CompactCommit, PersistedInputItem, ReportPayload, SubagentQueueState, SubagentRecord,
    TurnOutcome, Usage,
};
use crate::db::{Database, new_external_id, now_ms};

use super::cli::ExecModelTarget;
use super::runner::{
    ExecLoopEvent, ExecStreamFactory, append_deltas_to_parts, db_result_of,
    new_mutation_recorder_factory,
};

const STATUS_RUNNING: &str = "running";
const STATUS_IDLE: &str = "idle";
const STATUS_BLOCKED: &str = "blocked";
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
    child_session_id: String,
    child_assistant_id: String,
    child_user_message_id: String,
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
}

struct PendingWait {
    targets: Vec<String>,
    all: bool,
    mode: WaitMode,
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
    waits: HashMap<u64, PendingWait>,
    next_wait_id: u64,
}

#[rustfmt::skip]
impl ExecSubagentPool {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        workspace: PathBuf, cwd: PathBuf, parent_session_id: String, model: ExecModelTarget,
        exa_api_key: String, tavily_api_key: String, mcp_snapshot: Arc<crate::mcp::McpToolSnapshot>,
        sender: mpsc::UnboundedSender<ExecLoopEvent>, stream_factory: Arc<dyn ExecStreamFactory>,
    ) -> Self {
        Self {
            workspace, cwd, parent_session_id, model, exa_api_key, tavily_api_key, mcp_snapshot,
            sender, stream_factory, running: HashMap::new(), resumed: HashSet::new(),
            waits: HashMap::new(), next_wait_id: 1,
        }
    }

    pub(crate) fn handle_op(
        &mut self, db: &Database, parent_assistant_id: &str, call_id: Option<&str>, op: SubagentOp,
        parent_cancel: &watch::Receiver<bool>,
        reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
    ) {
        match op {
            SubagentOp::Spawn { role, title, task, items } => {
                let out = self.spawn(db, parent_assistant_id, call_id, role, &title, &task, &items);
                let _ = reply.send(out);
            }
            SubagentOp::Wait { targets, mode, timeout_ms } => {
                self.wait(db, targets, mode, timeout_ms, parent_cancel, reply)
            }
            SubagentOp::SendInput { target, items, interrupt } => {
                let out = self.send_input(db, parent_assistant_id, call_id, &target, &items, interrupt);
                let _ = reply.send(out);
            }
            SubagentOp::Resume { id } => { let out = self.resume(db, &id); let _ = reply.send(out); }
            SubagentOp::Close { target } => { let out = self.close(db, &target); let _ = reply.send(out); }
            SubagentOp::List => { let out = self.list(db); let _ = reply.send(out); }
            SubagentOp::CancelInput { submission_id } => {
                let out = self.cancel_input(db, &submission_id);
                let _ = reply.send(out);
            }
        }
    }

    pub(crate) fn handle_event(&mut self, db: &Database, event: ExecSubagentEvent) {
        match event {
            ExecSubagentEvent::Delta { agent_id, child_session_id, child_assistant_id, deltas, .. } => {
                self.handle_delta(db, &agent_id, &child_session_id, &child_assistant_id, &deltas)
            }
            ExecSubagentEvent::Host { agent_id, child_session_id, child_assistant_id, request } => {
                self.handle_host(db, &agent_id, &child_session_id, &child_assistant_id, request)
            }
            ExecSubagentEvent::AutoCompact { child_session_id, notice, reply } => {
                let result = self.commit_compact(db, &child_session_id, &notice);
                let _ = reply.send(result);
            }
            ExecSubagentEvent::Finished { agent_id, child_session_id, child_assistant_id, child_user_message_id, summary } => {
                self.finish(db, &agent_id, &child_session_id, &child_assistant_id, &child_user_message_id, summary)
            }
            ExecSubagentEvent::WaitReady { wait_id, timed_out } => self.handle_wait_ready(db, wait_id, timed_out),
        }
    }

    /// 取消并收割全部在途子 agent，提交 `interrupted` 终态。
    pub(crate) async fn shutdown_all(&mut self, db: &Database) {
        let handles: Vec<ChildHandle> = self.running.drain().map(|(_, handle)| handle).collect();
        for handle in &handles { let _ = handle.cancel_tx.send(true); }
        for mut handle in handles {
            if let Some(join) = handle.join_handle.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), join).await;
            }
            close_open_part(&mut handle.parts, &mut handle.open_part_index);
            if let Err(error) = db.finish_subagent_turn(
                &self.workspace, &handle.agent_id, &handle.child_assistant_id, &handle.parts,
                &TurnOutcome::Interrupted, Usage::default(), None,
            ) {
                tracing::warn!(target: "lx::exec", code = error.code(), "subagent shutdown finish failed");
            }
            handle.processes.shutdown().await;
            crate::chat::tools::drop_reads(&handle.child_session_id);
        }
        for (_, pending) in self.waits.drain() { let _ = pending.reply.send(Err(HostError::Cancelled)); }
        self.resumed.clear();
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &mut self, db: &Database, parent_assistant_id: &str, call_id: Option<&str>, role: AgentRole,
        title: &str, task: &str, items: &[SubagentInputItem],
    ) -> Result<SubagentOutcome, HostError> {
        if self.running.len() >= MAX_CONCURRENT_SUBAGENTS { return Err(failed(LIMIT_REACHED_CODE)); }
        let child_session_id = new_external_id("sess");
        let source: Vec<SubagentInputItem> = if items.is_empty() {
            vec![SubagentInputItem::Text { text: task.to_string() }]
        } else { items.to_vec() };
        let persisted = match self.prepare_input_items(&child_session_id, &source) {
            Ok(persisted) => persisted,
            Err(error) => { crate::platform::remove_session_images(&child_session_id); return Err(error); }
        };
        let task_text = if task.trim().is_empty() { joined_input_text(&source) } else { task.to_string() };
        if task_text.trim().is_empty() {
            crate::platform::remove_session_images(&child_session_id);
            return Err(failed("subagent task must not be empty"));
        }
        let title = if title.trim().is_empty() { first_line_title(&task_text) } else { title.to_string() };
        let dedupe = call_id.map(|cid| format!("spawn:{}:{}:{cid}", self.parent_session_id, parent_assistant_id));
        let committed = match db.begin_subagent_turn_with_items(
            &self.workspace, &self.parent_session_id, Some(&child_session_id), role, &title,
            &task_text, &persisted, &self.model.snapshot(), Some(parent_assistant_id),
            dedupe.as_deref(), call_id,
        ) {
            Ok(committed) => committed,
            Err(error) => {
                crate::platform::remove_session_images(&child_session_id);
                return Err(db_failed(error));
            }
        };
        self.resumed.insert(committed.agent_id.clone());
        let started = self.start_child(
            db, &committed.agent_id, &committed.child_session_id, &committed.child_assistant_id,
            &committed.child_user_message_id, role, &title, committed.created_at_ms, &persisted,
            watch::channel(false).0,
        );
        if let Err(error) = started { self.resumed.remove(&committed.agent_id); return Err(error); }
        Ok(SubagentOutcome::Spawned { agent_id: committed.agent_id, role, title })
    }

    #[allow(clippy::too_many_arguments)]
    fn send_input(
        &mut self, db: &Database, parent_assistant_id: &str, call_id: Option<&str>, target: &str,
        items: &[SubagentInputItem], interrupt: bool,
    ) -> Result<SubagentOutcome, HostError> {
        let record = db.owned_subagent(&self.workspace, &self.parent_session_id, target)
            .map_err(db_failed)?.ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let is_running = self.running.contains_key(target);
        if !is_running && !self.resumed.contains(target) { return Err(failed(NOT_ACTIVE_CODE)); }
        let persisted = self.prepare_input_items(&record.child_session_id, items)?;
        let dedupe = match call_id.filter(|cid| !cid.trim().is_empty()) {
            Some(cid) => format!("send:{}:{}:{cid}", self.parent_session_id, parent_assistant_id),
            None => format!("send:{}:{}", self.parent_session_id, new_external_id("dedupe")),
        };
        let enqueued = db.enqueue_input(
            &self.workspace, &self.parent_session_id, target, &dedupe, call_id,
            Some(parent_assistant_id), &persisted, interrupt,
        ).map_err(db_failed)?;
        if is_running {
            if interrupt && enqueued.created && let Some(handle) = self.running.get(target) {
                let _ = handle.cancel_tx.send(true);
            }
            return Ok(SubagentOutcome::InputAccepted {
                submission_id: enqueued.submission_id, agent_id: target.to_string(),
                status: enqueued.status, queued_count: enqueued.queued_count, interrupt,
            });
        }
        let (mut status, mut queued_count) = (enqueued.status, enqueued.queued_count);
        if let Some(claimed) = self.claim_and_launch(db, target, None)? {
            if claimed.submission_id == enqueued.submission_id { status = STATUS_RUNNING.to_string(); }
            queued_count = claimed.queued_remaining;
        }
        Ok(SubagentOutcome::InputAccepted {
            submission_id: enqueued.submission_id, agent_id: target.to_string(),
            status, queued_count, interrupt,
        })
    }

    fn resume(&mut self, db: &Database, agent_id: &str) -> Result<SubagentOutcome, HostError> {
        if db.owned_subagent(&self.workspace, &self.parent_session_id, agent_id)
            .map_err(db_failed)?.is_none() { return Err(failed(NOT_FOUND_CODE)); }
        let queued_count = db.subagent_queue_states(&self.workspace, &self.parent_session_id)
            .unwrap_or_default().get(agent_id).map_or(0, |state| state.queued_count);
        self.resumed.insert(agent_id.to_string());
        if self.running.contains_key(agent_id) {
            return Ok(SubagentOutcome::Resumed {
                agent_id: agent_id.to_string(), status: STATUS_RUNNING.to_string(), queued_count,
            });
        }
        if queued_count > 0 && self.running.len() >= MAX_CONCURRENT_SUBAGENTS {
            return Err(failed(LIMIT_REACHED_CODE));
        }
        db.recover_subagent_for_resume(&self.workspace, &self.parent_session_id, agent_id).map_err(db_failed)?;
        match self.claim_and_launch(db, agent_id, None)? {
            Some(claimed) => Ok(SubagentOutcome::Resumed {
                agent_id: agent_id.to_string(), status: STATUS_RUNNING.to_string(),
                queued_count: claimed.queued_remaining,
            }),
            None => Ok(SubagentOutcome::Resumed {
                agent_id: agent_id.to_string(), status: STATUS_IDLE.to_string(), queued_count: 0,
            }),
        }
    }

    fn close(&mut self, db: &Database, target: &str) -> Result<SubagentOutcome, HostError> {
        let record = db.owned_subagent(&self.workspace, &self.parent_session_id, target)
            .map_err(db_failed)?.ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let previous_status = if self.running.contains_key(target) {
            STATUS_RUNNING.to_string()
        } else { record.status };
        if let Some(handle) = self.running.get(target) { let _ = handle.cancel_tx.send(true); }
        self.resumed.remove(target);
        Ok(SubagentOutcome::Closed { agent_id: target.to_string(), previous_status })
    }

    fn list(&self, db: &Database) -> Result<SubagentOutcome, HostError> {
        let records = db.list_subagents(&self.workspace, &self.parent_session_id).map_err(db_failed)?;
        let queues = db.subagent_queue_states(&self.workspace, &self.parent_session_id).unwrap_or_default();
        let agents = records.iter().map(|record| AgentListItemView {
            agent_id: record.agent_id.clone(), role: record.role, title: record.title.clone(),
            status: self.projected_status(record, queues.get(&record.agent_id)),
            queued_count: queues.get(&record.agent_id).map_or(0, |q| q.queued_count),
        }).collect();
        Ok(SubagentOutcome::Listed { agents })
    }

    fn cancel_input(&self, db: &Database, submission_id: &str) -> Result<SubagentOutcome, HostError> {
        match db.cancel_input(&self.workspace, &self.parent_session_id, submission_id).map_err(db_failed)? {
            Some((agent_id, status, queued_count)) => Ok(SubagentOutcome::InputCancelled {
                submission_id: submission_id.to_string(), agent_id, status, queued_count,
            }),
            None => Err(failed(INPUT_NOT_FOUND_CODE)),
        }
    }

    /// 认领队首输入并启动下一轮；没有可执行输入返回 `Ok(None)`。
    fn claim_and_launch(
        &mut self, db: &Database, agent_id: &str, reused_done: Option<watch::Sender<bool>>,
    ) -> Result<Option<crate::db::chat::ClaimedSubagentTurn>, HostError> {
        if self.running.contains_key(agent_id) { return Ok(None); }
        let record = db.owned_subagent(&self.workspace, &self.parent_session_id, agent_id)
            .map_err(db_failed)?.ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let Some(next) = db.next_queued_input(&self.workspace, &self.parent_session_id, agent_id)
            .map_err(db_failed)? else { return Ok(None); };
        if let Some(code) = &next.error_code { return Err(failed(&format!("subagent input blocked: {code}"))); }
        if self.running.len() >= MAX_CONCURRENT_SUBAGENTS { return Err(failed(LIMIT_REACHED_CODE)); }
        let claimed = db.claim_input(
            &self.workspace, &self.parent_session_id, agent_id, &next.submission_id, &self.model.snapshot(),
        ).map_err(db_failed)?;
        self.start_child(
            db, agent_id, &claimed.child_session_id, &claimed.child_assistant_id,
            &claimed.child_user_message_id, record.role, &record.title, claimed.started_at_ms,
            &claimed.items, reused_done.unwrap_or_else(|| watch::channel(false).0),
        )?;
        Ok(Some(claimed))
    }

    #[allow(clippy::too_many_arguments)]
    fn start_child(
        &mut self, db: &Database, agent_id: &str, child_session_id: &str, child_assistant_id: &str,
        child_user_message_id: &str, role: AgentRole, title: &str, started_at_ms: i64,
        current_items: &[PersistedInputItem], done_tx: watch::Sender<bool>,
    ) -> Result<(), HostError> {
        let context = self.child_context(db, child_session_id, role, current_items)
            .map_err(|error| failed(&error))?;
        let processes = Arc::new(ProcessManager::new(ProcessOwner {
            workspace: self.cwd.clone(), chat_session_id: child_session_id.to_string(),
            runtime_epoch: new_external_id("epoch"),
        }, None));
        let executor = AgentTools::new(Arc::clone(&processes), Arc::new(Mutex::new(TurnExecution::default())))
            .with_recorder(new_mutation_recorder_factory(
                self.sender.clone(), self.workspace.clone(), child_session_id.to_string(),
                child_user_message_id.to_string(),
            ))
            .with_reads(crate::chat::tools::read_registry(child_session_id))
            .with_image_support(self.model.supports_image_input)
            .with_web_search_keys(self.exa_api_key.clone(), self.tavily_api_key.clone())
            .with_mcp(crate::mcp::global_manager().clone(), Arc::clone(&self.mcp_snapshot), child_session_id.to_string());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let stream = {
            let factory = Arc::clone(&self.stream_factory);
            let model = self.model.clone();
            let sender = self.sender.clone();
            let ids = (agent_id.to_string(), child_session_id.to_string(), child_assistant_id.to_string());
            move |request: ChatRequest, cancel: watch::Receiver<bool>| {
                let factory = Arc::clone(&factory);
                let model = model.clone();
                let sender = sender.clone();
                let (agent_id, child_session_id, child_assistant_id) = ids.clone();
                async move {
                    let on_delta = Box::new(move |deltas: &[ChatDelta]| {
                        let _ = sender.send(ExecLoopEvent::Subagent(ExecSubagentEvent::Delta {
                            agent_id: agent_id.clone(), child_session_id: child_session_id.clone(),
                            child_assistant_id: child_assistant_id.clone(), deltas: deltas.to_vec(),
                        }));
                    });
                    factory.stream(&model, request, cancel, on_delta).await
                }
            }
        };
        let host = ChildHost {
            sender: self.sender.clone(), agent_id: agent_id.to_string(),
            child_session_id: child_session_id.to_string(),
            child_assistant_id: child_assistant_id.to_string(),
        };
        let finished = {
            let sender = self.sender.clone();
            let ids = (agent_id.to_string(), child_session_id.to_string(), child_assistant_id.to_string(), child_user_message_id.to_string());
            move |summary: TurnSummary| {
                let (agent_id, child_session_id, child_assistant_id, child_user_message_id) = ids.clone();
                let _ = sender.send(ExecLoopEvent::Subagent(ExecSubagentEvent::Finished {
                    agent_id, child_session_id, child_assistant_id, child_user_message_id, summary,
                }));
            }
        };
        let join_handle = tokio::spawn(async move {
            let summary = run_agent_turn(context, host, executor, stream, cancel_rx).await;
            finished(summary);
        });
        self.running.insert(agent_id.to_string(), ChildHandle {
            agent_id: agent_id.to_string(), child_session_id: child_session_id.to_string(),
            child_assistant_id: child_assistant_id.to_string(),
            child_user_message_id: child_user_message_id.to_string(), role, title: title.to_string(),
            started_at_ms, parts: Vec::new(), next_position: 0, open_part_index: None,
            cancel_tx, done_tx, processes, join_handle: Some(join_handle),
            uncheckpointed_bytes: 0,
        });
        Ok(())
    }

    /// 子轮请求：历史按已完成轮次回放（工具步骤保留），末尾接当前输入。
    fn child_context(
        &self, db: &Database, child_session_id: &str, role: AgentRole,
        current_items: &[PersistedInputItem],
    ) -> Result<ModelContext, String> {
        let (mut turns, mut items) = (Vec::new(), Vec::new());
        let checkpoint = db.active_checkpoint(&self.workspace, child_session_id)
            .map_err(|error| error.code().to_string())?;
        let start_after = checkpoint.as_ref().map_or(i64::MIN, |cp| cp.end_sequence);
        let messages: Vec<_> = db.load_chat_session(&self.workspace, child_session_id)
            .map_err(|error| error.code().to_string())?.unwrap_or_default()
            .into_iter().filter(|message| message.sequence > start_after).collect();
        let mut index = 0;
        while index < messages.len() {
            let turn_index = messages[index].turn_index;
            let (mut user, mut assistant) = (None, None);
            while index < messages.len() && messages[index].turn_index == turn_index {
                let message = &messages[index];
                match message.role.as_str() {
                    "user" if user.is_none() => user = Some(message),
                    "assistant" if assistant.is_none() => assistant = Some(message),
                    _ => {}
                }
                index += 1;
            }
            let (Some(user), Some(assistant)) = (user, assistant) else { continue };
            if user.status != "complete" || assistant.status != "complete" { continue; }
            let user_text = message_text(user);
            turns.push(ChatTurn { role: "user".into(), text: user_text.clone(), reasoning_metadata: None });
            turns.push(ChatTurn { role: "assistant".into(), text: message_text(assistant), reasoning_metadata: None });
            items.push(ModelItem::UserText(user_text));
            items.extend(crate::chat::context::assistant_items(assistant));
        }
        let current_text = PersistedInputItem::joined_text(current_items);
        turns.push(ChatTurn { role: "user".into(), text: current_text, reasoning_metadata: None });
        items.extend(self.current_model_items(child_session_id, current_items)?);
        let mcp_prompt = self.mcp_snapshot.instructions_prompt();
        let system_prompt = crate::chat::agent::compose_subagent_prompt(role, PromptParts {
            model_identity: None, environment: None, instructions: None, skills: None,
            mcp: mcp_prompt.as_deref(), compact: None,
        });
        let request = ChatRequest {
            turns, items, model: self.model.snapshot(), variant_params: self.model.variant_params.clone(),
            purpose: RequestPurpose::Subagent(role), system_prompt: Some(system_prompt),
            tools: Arc::new(crate::chat::tools::turn_tools(ToolSurface::from_role(role), &self.mcp_snapshot)),
        };
        let context = ModelContext::new(request, self.cwd.clone()).with_context_limit(self.model.context_limit);
        Ok(match checkpoint {
            Some(checkpoint) => context.with_compact_summary(Some(checkpoint.summary_text)),
            None => context,
        })
    }

    fn handle_delta(
        &mut self, db: &Database, agent_id: &str, child_session_id: &str,
        child_assistant_id: &str, deltas: &[ChatDelta],
    ) {
        let Some(handle) = self.running.get_mut(agent_id) else { return };
        if handle.child_session_id != child_session_id || handle.child_assistant_id != child_assistant_id {
            return;
        }
        append_deltas_to_parts(&mut handle.parts, &mut handle.next_position, &mut handle.open_part_index, deltas, |_, _| {});
        handle.uncheckpointed_bytes = handle.uncheckpointed_bytes
            .saturating_add(deltas.iter().map(ChatDelta::payload_bytes).sum::<usize>());
        if handle.uncheckpointed_bytes < CHECKPOINT_BYTES { return; }
        handle.uncheckpointed_bytes = 0;
        let _ = db.checkpoint(&self.workspace, child_assistant_id, &handle.parts);
    }

    fn handle_host(
        &mut self, db: &Database, agent_id: &str, child_session_id: &str,
        child_assistant_id: &str, request: AgentHostRequest,
    ) {
        match request {
            AgentHostRequest::BeginTool { call, reply } => {
                let Some(handle) = self.running.get_mut(agent_id) else {
                    let _ = reply.send(Err(HostError::Cancelled));
                    return;
                };
                if handle.child_session_id != child_session_id || handle.child_assistant_id != child_assistant_id {
                    let _ = reply.send(Err(HostError::Cancelled));
                    return;
                }
                close_open_part(&mut handle.parts, &mut handle.open_part_index);
                let result = db.checkpoint(&self.workspace, &handle.child_assistant_id, &handle.parts)
                    .map_err(|error| tool_failed("tool call pending record failed", error.code()))
                    .and_then(|()| db.begin_tool_call(
                        &self.workspace, child_session_id, child_assistant_id, &call.call_id,
                        &call.name, &call.arguments_json,
                    ).map_err(|error| tool_failed("tool call pending record failed", error.code())))
                    .and_then(|ticket| {
                        handle.next_position = handle.next_position.max(ticket.position.saturating_add(1));
                        db.mark_tool_running(&self.workspace, &ticket)
                            .map_err(|error| tool_failed("tool call running record failed", error.code()))
                            .map(|()| ticket)
                    });
                let _ = reply.send(result);
            }
            AgentHostRequest::FinishTool { ticket, outcome, reply } => {
                let result = db_result_of(&outcome);
                let sent = match db.finish_tool_call(&self.workspace, &ticket, &result) {
                    Ok(()) => Ok(()),
                    Err(error) => Err(tool_failed("tool result persistence failed", error.code())),
                };
                let _ = reply.send(sent);
            }
            AgentHostRequest::AskUser { reply, .. } => {
                let _ = reply.send(Err(failed("unsupported in subagent")));
            }
            AgentHostRequest::Subagent { reply, .. } => {
                let _ = reply.send(Err(failed("unsupported in subagent")));
            }
            AgentHostRequest::UserShellUpdates { reply } => { let _ = reply.send(Vec::new()); }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &mut self, db: &Database, agent_id: &str, child_session_id: &str, child_assistant_id: &str,
        child_user_message_id: &str, summary: TurnSummary,
    ) {
        let Some(mut handle) = self.running.remove(agent_id) else { return };
        close_open_part(&mut handle.parts, &mut handle.open_part_index);
        tracing::debug!(
            target: "lx::exec", agent_id, role = handle.role.as_str(),
            child_user_message_id = %handle.child_user_message_id, "subagent turn finished",
        );
        let outcome = db_outcome(&summary.outcome);
        let report = match &summary.outcome {
            crate::chat::agent_loop::TurnOutcome::Interrupted => None,
            _ => Some(ReportPayload {
                title: handle.title.clone(),
                body: crate::chat::agent::truncate_report_body(&last_text_body(&handle.parts)),
                files_changed: db.count_changed_paths(&self.workspace, child_session_id, child_user_message_id).unwrap_or_default(),
                duration_ms: Some(now_ms().saturating_sub(handle.started_at_ms).max(0)),
            }),
        };
        if let Err(error) = db.finish_subagent_turn(
            &self.workspace, agent_id, child_assistant_id, &handle.parts, &outcome, summary.usage, report.as_ref(),
        ) {
            tracing::warn!(target: "lx::exec", code = error.code(), "subagent finish failed");
        }
        let mut continued = false;
        if !matches!(summary.outcome, crate::chat::agent_loop::TurnOutcome::Failed { .. })
            && self.resumed.contains(agent_id)
        {
            continued = matches!(self.claim_and_launch(db, agent_id, Some(handle.done_tx.clone())), Ok(Some(_)));
        }
        if !continued { self.resumed.remove(agent_id); }
        let processes = Arc::clone(&handle.processes);
        let session_id = handle.child_session_id.clone();
        tokio::spawn(async move {
            processes.shutdown().await;
            crate::chat::tools::drop_reads(&session_id);
        });
        if !continued { let _ = handle.done_tx.send(true); }
    }

    /// 自动压缩：把摘要写成子会话检查点后回执（失败不吞错）。
    fn commit_compact(
        &self, db: &Database, child_session_id: &str, notice: &AutoCompactNotice,
    ) -> Result<(), HostError> {
        let messages = db.load_chat_session(&self.workspace, child_session_id)
            .map_err(db_failed)?.unwrap_or_default();
        let end_sequence = messages.iter().map(|message| message.sequence).max().unwrap_or(0);
        let base_checkpoint_id = db.active_checkpoint(&self.workspace, child_session_id)
            .map_err(db_failed)?.map(|checkpoint| checkpoint.id);
        let expected_revision = db.session_revision(child_session_id).map_err(db_failed)?;
        db.commit_compact(&self.workspace, &CompactCommit {
            session_id: child_session_id.to_string(), expected_revision, base_checkpoint_id,
            start_sequence: 0, end_sequence, summary_text: notice.summary.clone(),
            model_label: notice.model_label.clone(), operation_id: new_external_id("autocompact"),
        }).map_err(db_failed)?;
        Ok(())
    }

    fn wait(
        &mut self, db: &Database, requested: Vec<String>, mode: WaitMode, timeout_ms: u64,
        parent_cancel: &watch::Receiver<bool>,
        reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
    ) {
        let all = requested.is_empty();
        let records = db.list_subagents(&self.workspace, &self.parent_session_id).unwrap_or_default();
        let mut unique: Vec<String> = if all {
            records.iter().map(|record| record.agent_id.clone()).collect()
        } else { requested };
        if !all {
            let mut seen = HashSet::new();
            unique.retain(|target| seen.insert(target.clone()));
        }
        if !all && unique.len() > MAX_WAIT_TARGETS { let _ = reply.send(Err(failed("invalid wait targets"))); return; }
        if !all && unique.iter().any(|target| {
            !records.iter().any(|record| &record.agent_id == target) && !self.running.contains_key(target)
        }) { let _ = reply.send(Err(failed(NOT_FOUND_CODE))); return; }
        let any_done = unique.iter().any(|target| !self.running.contains_key(target));
        let none_running = unique.iter().all(|target| !self.running.contains_key(target));
        if (mode == WaitMode::Any && any_done) || none_running {
            let outcome = self.settle_wait(db, &unique, false, all, mode);
            let _ = reply.send(outcome);
            return;
        }
        let receivers: Vec<watch::Receiver<bool>> = unique.iter()
            .filter_map(|target| self.running.get(target).map(|handle| handle.done_tx.subscribe()))
            .collect();
        if receivers.is_empty() {
            let outcome = self.settle_wait(db, &unique, false, all, mode);
            let _ = reply.send(outcome);
            return;
        }
        if *parent_cancel.borrow() { let _ = reply.send(Err(HostError::Cancelled)); return; }
        let wait_id = self.next_wait_id;
        self.next_wait_id = self.next_wait_id.saturating_add(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.waits.insert(wait_id, PendingWait {
            targets: unique, all, mode, reply, cancelled: Arc::clone(&cancelled),
        });
        let sender = self.sender.clone();
        let mut parent_cancel = parent_cancel.clone();
        let timeout = Duration::from_millis(timeout_ms.min(crate::chat::agent::MAX_WAIT_TIMEOUT_MS));
        tokio::spawn(async move {
            let cancelled_task = async {
                loop {
                    if *parent_cancel.borrow() { return; }
                    if parent_cancel.changed().await.is_err() { std::future::pending::<()>().await; }
                }
            };
            tokio::select! {
                _ = cancelled_task => {
                    cancelled.store(true, Ordering::Relaxed);
                    let _ = sender.send(ExecLoopEvent::Subagent(ExecSubagentEvent::WaitReady { wait_id, timed_out: true }));
                }
                timed_out = wait_until(mode, receivers, timeout) => {
                    let _ = sender.send(ExecLoopEvent::Subagent(ExecSubagentEvent::WaitReady { wait_id, timed_out }));
                }
            }
        });
    }

    fn handle_wait_ready(&mut self, db: &Database, wait_id: u64, timed_out: bool) {
        let Some(pending) = self.waits.remove(&wait_id) else { return };
        if pending.cancelled.load(Ordering::Relaxed) {
            let _ = pending.reply.send(Err(HostError::Cancelled));
            return;
        }
        let outcome = self.settle_wait(db, &pending.targets, timed_out, pending.all, pending.mode);
        let _ = pending.reply.send(outcome);
    }

    fn settle_wait(
        &self, db: &Database, targets: &[String], timed_out: bool, all: bool, mode: WaitMode,
    ) -> Result<SubagentOutcome, HostError> {
        let records = db.list_subagents(&self.workspace, &self.parent_session_id).map_err(db_failed)?;
        let pending = db.pending_reports(&self.workspace, &self.parent_session_id).unwrap_or_default();
        let queues = db.subagent_queue_states(&self.workspace, &self.parent_session_id).unwrap_or_default();
        let (mut agents, mut claimed) = (Vec::new(), Vec::new());
        for target in targets {
            let Some(record) = records.iter().find(|record| &record.agent_id == target) else { continue };
            let mut bodies = Vec::new();
            if !self.running.contains_key(target) {
                for item in pending.iter().filter(|item| &item.agent_id == target) {
                    if let Some(view) = SubagentReportView::from_json(&item.report_json) {
                        bodies.push(view.body);
                        claimed.push(item.report_id.clone());
                    }
                }
            }
            agents.push(AgentStatusView {
                agent_id: record.agent_id.clone(), title: record.title.clone(), role: record.role,
                status: self.projected_status(record, queues.get(target)),
                report: (!bodies.is_empty()).then(|| bodies.join("\n\n")),
            });
        }
        if !claimed.is_empty() {
            let _ = db.mark_reports_delivered_in_batch(&self.workspace, &self.parent_session_id, &claimed);
        }
        Ok(SubagentOutcome::Waited { timed_out, agents, all, mode })
    }

    fn projected_status(&self, record: &SubagentRecord, queue: Option<&SubagentQueueState>) -> String {
        if self.running.contains_key(&record.agent_id) { return STATUS_RUNNING.to_string(); }
        if queue.and_then(|state| state.head_error_code.as_ref()).is_some() { return STATUS_BLOCKED.to_string(); }
        if self.resumed.contains(&record.agent_id) { return STATUS_IDLE.to_string(); }
        if record.status == "cancelled"
            && queue.and_then(|state| state.last_input_status.as_deref()) == Some(STATUS_INTERRUPTED)
        { return STATUS_INTERRUPTED.to_string(); }
        record.status.clone()
    }

    /// 预校验并归档一次输入的全部图片；失败时由调用方决定是否清理会话图片目录。
    fn prepare_input_items(
        &self, child_session_id: &str, items: &[SubagentInputItem],
    ) -> Result<Vec<PersistedInputItem>, HostError> {
        if items.is_empty() { return Err(failed("subagent input items must not be empty")); }
        let has_images = items.iter().any(|item| !matches!(item, SubagentInputItem::Text { .. }));
        if has_images && !self.model.supports_image_input {
            return Err(failed("current model does not support image input"));
        }
        let mut out = Vec::with_capacity(items.len());
        let (mut count, mut total) = (0usize, 0usize);
        for item in items {
            let processed = match item {
                SubagentInputItem::Text { text } => {
                    out.push(PersistedInputItem::Text { text: text.clone() });
                    continue;
                }
                SubagentInputItem::LocalImage { path } => {
                    count += 1;
                    ensure_image(count, total)?;
                    let resolved = resolve_image_path(&self.cwd, path)?;
                    crate::platform::load_and_process_image_file(&resolved)
                        .map_err(|error| failed(&format!("invalid local_image: {}", error.code())))?
                }
                SubagentInputItem::Image { image_url } => {
                    count += 1;
                    ensure_image(count, total)?;
                    crate::platform::process_data_url_image(image_url, count.saturating_sub(1))
                        .map_err(|error| failed(&format!("invalid inline image: {}", error.code())))?
                }
            };
            total = total.saturating_add(processed.original_byte_count);
            ensure_image(count, total)?;
            out.push(archive_image(child_session_id, &processed)?);
        }
        Ok(out)
    }

    /// 当前输入 → 模型项；图片严格校验归档（缺失即 `image_missing`）。
    fn current_model_items(
        &self, child_session_id: &str, items: &[PersistedInputItem],
    ) -> Result<Vec<ModelItem>, String> {
        let has_images = items.iter().any(|item| matches!(item, PersistedInputItem::Image { .. }));
        if has_images && !self.model.supports_image_input {
            return Err("current model does not support image input".to_string());
        }
        if !has_images { return Ok(vec![ModelItem::UserText(PersistedInputItem::joined_text(items))]); }
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            match item {
                PersistedInputItem::Text { text } => out.push(ModelItem::UserContent {
                    text: text.clone(), images: Vec::new(),
                }),
                PersistedInputItem::Image { archive_path, mime, content_hash, width, height, .. } => {
                    let bytes = crate::platform::verify_archived_image(
                        child_session_id, Path::new(archive_path), mime, content_hash, *width, *height,
                    ).map_err(|_| IMAGE_MISSING_CODE.to_string())?;
                    out.push(ModelItem::UserContent {
                        text: String::new(),
                        images: vec![ModelImage { mime: mime.clone(), bytes: Arc::new(bytes) }],
                    });
                }
            }
        }
        Ok(out)
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

fn db_outcome(outcome: &crate::chat::agent_loop::TurnOutcome) -> TurnOutcome {
    match outcome {
        crate::chat::agent_loop::TurnOutcome::Complete => TurnOutcome::Complete {
            stop_reason: Some("complete".into()),
        },
        crate::chat::agent_loop::TurnOutcome::Interrupted => TurnOutcome::Interrupted,
        crate::chat::agent_loop::TurnOutcome::Failed { code } => TurnOutcome::Failed {
            error_code: code.clone(),
        },
    }
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

fn resolve_image_path(cwd: &Path, raw: &str) -> Result<PathBuf, HostError> {
    let path = Path::new(raw.trim());
    if path.as_os_str().is_empty() {
        return Err(failed("image path must not be empty"));
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if cwd.as_os_str().is_empty() || !cwd.is_dir() {
        return Err(failed("workspace cwd unavailable for relative image path"));
    }
    Ok(cwd.join(path))
}

fn ensure_image(count: usize, total: usize) -> Result<(), HostError> {
    if count > crate::platform::MAX_ATTACHED_IMAGES {
        return Err(failed(&format!(
            "too many images: at most {} images per input",
            crate::platform::MAX_ATTACHED_IMAGES
        )));
    }
    if total > crate::platform::MAX_TOTAL_IMAGE_RAW_BYTES {
        return Err(failed("total image bytes exceed the 40 MiB limit"));
    }
    Ok(())
}

fn archive_image(
    child_session_id: &str,
    processed: &crate::platform::ProcessedImage,
) -> Result<PersistedInputItem, HostError> {
    let archived = crate::platform::archive_processed_image(child_session_id, processed)
        .map_err(|error| failed(&format!("image archive failed: {}", error.code())))?;
    Ok(PersistedInputItem::Image {
        label: archived.label,
        archive_path: archived.archive_path.to_string_lossy().into_owned(),
        mime: archived.mime,
        content_hash: archived.content_hash,
        width: archived.width,
        height: archived.height,
    })
}

fn close_open_part(parts: &mut [ChatPart], open_part_index: &mut Option<usize>) {
    if let Some(index) = open_part_index.take()
        && let Some(part) = parts.get_mut(index)
    {
        part.completed_at_ms = Some(now_ms());
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

fn message_text(message: &crate::db::chat::ChatMessage) -> String {
    message
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| part.content.as_str())
        .collect::<Vec<_>>()
        .join("")
}
