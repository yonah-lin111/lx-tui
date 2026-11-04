//! `exec` 无头子 agent 池：spawn / send_input / resume / wait / close / list / cancel_input。
//!
//! 子轮仍是独立子会话：工具握手与报告结算直接写 Database，流增量经 unbounded
//! channel 回到无头事件循环聚合为 `ChatPart`。与 [`super::runner`] 同边界：不导入
//! `app` / `ui` / `tui` / `event`。
//!
//! 池控制入口在本文件；子轮上下文装配见 [`turn`]，等待与结算见 [`settle`]。

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
    NOT_FOUND_CODE, SubagentInputItem, SubagentOp, SubagentOutcome, WaitMode, joined_input_text,
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
    timeout_ms: u64,
    auto_continue: bool,
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
    child_models: HashMap<String, ExecModelTarget>,
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
            child_models: HashMap::new(),
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
                task,
                items,
                fork_context,
                model,
                reasoning_effort,
            } => {
                let out = self.spawn(
                    db,
                    parent_assistant_id,
                    call_id,
                    role,
                    &title,
                    &task,
                    &items,
                    fork_context,
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
            } => {
                let out =
                    self.send_input(db, parent_assistant_id, call_id, &target, &items, interrupt);
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
            SubagentOp::List => {
                let out = self.list(db);
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
    fn spawn(
        &mut self,
        db: &Database,
        parent_assistant_id: &str,
        call_id: Option<&str>,
        role: AgentRole,
        title: &str,
        task: &str,
        items: &[SubagentInputItem],
        fork_context: bool,
        model_override: Option<&str>,
        reasoning_effort_override: Option<&str>,
    ) -> Result<SubagentOutcome, HostError> {
        let role_spec = self
            .subagents_config
            .resolve_role(role.as_str())
            .ok_or_else(|| failed(&format!("unknown subagent role '{}'", role.as_str())))?;
        let role = role.with_base(role_spec.base_role);
        let max_concurrent = self
            .subagents_config
            .max_concurrent
            .clamp(1, crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT);
        if self.running.len() >= max_concurrent {
            return Err(failed(LIMIT_REACHED_CODE));
        }
        let child_target = self.resolve_child_model(
            role_spec.default_model.as_ref(),
            model_override,
            reasoning_effort_override,
        )?;
        let existing_records = db
            .list_subagents(&self.workspace, &self.parent_session_id)
            .unwrap_or_default();
        let used_nicknames: Vec<String> = existing_records
            .into_iter()
            .filter_map(|r| r.nickname)
            .collect();
        let nickname =
            crate::chat::agent::allocate_nickname(&used_nicknames, &role_spec.nickname_candidates);
        let child_session_id = new_external_id("sess");
        let image_rewrite = if fork_context {
            crate::db::chat::copy_session_images(&self.parent_session_id, &child_session_id)
        } else {
            None
        };
        let source: Vec<SubagentInputItem> = if items.is_empty() {
            vec![SubagentInputItem::Text {
                text: task.to_string(),
            }]
        } else {
            items.to_vec()
        };
        let persisted = match self.prepare_input_items(
            &child_session_id,
            &source,
            child_target.supports_image_input,
        ) {
            Ok(persisted) => persisted,
            Err(error) => {
                crate::platform::remove_session_images(&child_session_id);
                return Err(error);
            }
        };
        let task_text = if task.trim().is_empty() {
            joined_input_text(&source)
        } else {
            task.to_string()
        };
        if task_text.trim().is_empty() {
            crate::platform::remove_session_images(&child_session_id);
            return Err(failed("subagent task must not be empty"));
        }
        let title = if title.trim().is_empty() {
            first_line_title(&task_text)
        } else {
            title.to_string()
        };
        let dedupe = call_id.map(|cid| {
            format!(
                "spawn:{}:{}:{cid}",
                self.parent_session_id, parent_assistant_id
            )
        });
        let committed = match db.begin_subagent_turn_with_options(
            &self.workspace,
            &self.parent_session_id,
            Some(&child_session_id),
            Some(&nickname),
            role,
            &title,
            &task_text,
            &persisted,
            &child_target.snapshot(),
            Some(parent_assistant_id),
            dedupe.as_deref(),
            call_id,
            fork_context,
            &image_rewrite,
        ) {
            Ok(committed) => committed,
            Err(error) => {
                crate::platform::remove_session_images(&child_session_id);
                return Err(db_failed(error));
            }
        };
        self.child_models
            .insert(committed.child_session_id.clone(), child_target.clone());
        self.resumed.insert(committed.agent_id.clone());
        let started = self.start_child(
            db,
            &committed.agent_id,
            &committed.child_session_id,
            &committed.child_assistant_id,
            &committed.child_user_message_id,
            role,
            &title,
            committed.created_at_ms,
            &persisted,
            watch::channel(false).0,
        );
        if let Err(error) = started {
            self.resumed.remove(&committed.agent_id);
            return Err(error);
        }
        let model_label = if model_override.is_some()
            || child_target.provider != self.model.provider
            || child_target.model != self.model.model
        {
            Some(format!("{}/{}", child_target.provider, child_target.model))
        } else {
            None
        };
        let effort_label = if reasoning_effort_override.is_some()
            || (role_spec.default_model.is_some() && child_target.variant != self.model.variant)
        {
            child_target.variant.clone()
        } else {
            None
        };
        Ok(SubagentOutcome::Spawned {
            agent_id: committed.agent_id,
            nickname: Some(nickname),
            role,
            title,
            model: model_label,
            reasoning_effort: effort_label,
            fork_context,
        })
    }

    fn resolve_child_model(
        &self,
        role_default: Option<&crate::config::ai::ModelSelection>,
        model_override: Option<&str>,
        effort_override: Option<&str>,
    ) -> Result<ExecModelTarget, HostError> {
        let explicit_model = model_override.map(str::trim).filter(|s| !s.is_empty());
        let explicit_effort = effort_override.map(str::trim).filter(|s| !s.is_empty());
        if explicit_model.is_none() && explicit_effort.is_none() && role_default.is_none() {
            return Ok(self.model.clone());
        }
        let settings = crate::config::ai::ai_config_path()
            .ok()
            .and_then(|path| crate::config::ai::load_ai(&path).ok().flatten())
            .unwrap_or_else(crate::config::ai::AiSettings::empty);
        if settings.providers.is_empty() {
            // 只有单元测试环境（无 ai.json）才允许用父模型做兜底；真实运行中配置缺失
            // 必须显式失败，不能把 `provider/model` 静默塞进 model 字段。
            if !cfg!(test) {
                return Err(failed("model configuration unavailable"));
            }
            let mut fallback = self.model.clone();
            if let Some(m) = explicit_model {
                apply_reference_to_target(&mut fallback, m);
            } else if let Some(sel) = role_default {
                fallback.provider = sel.provider.clone();
                fallback.model = sel.model.clone();
                if sel.variant.is_some() {
                    fallback.variant = sel.variant.clone();
                }
            }
            if let Some(e) = explicit_effort {
                fallback.variant = Some(e.to_string());
            }
            return Ok(fallback);
        }
        let raw_spec = if let Some(m) = explicit_model {
            match explicit_effort {
                Some(e) => format!("{m}#{e}"),
                None => m.to_string(),
            }
        } else if let Some(sel) = role_default {
            let eff = explicit_effort.or(sel.variant.as_deref());
            match eff {
                Some(e) => format!("{}/{}#{e}", sel.provider, sel.model),
                None => format!("{}/{}", sel.provider, sel.model),
            }
        } else {
            let e = explicit_effort.unwrap_or_default();
            format!("{}/{}#{e}", self.model.provider, self.model.model)
        };
        crate::chat::agent::exec::resolve_model_target(&settings, Some(&raw_spec), None)
            .map_err(|err| failed(&err))
    }

    /// 子 agent 的模型目标：内存缓存 > 数据库持久化快照（重启后恢复）> 父模型。
    fn child_model_for(&mut self, record: &SubagentRecord) -> &ExecModelTarget {
        let differs = !record.model_provider.trim().is_empty()
            && !record.model_id.trim().is_empty()
            && (record.model_provider != self.model.provider
                || record.model_id != self.model.model
                || record.model_variant != self.model.variant);
        if differs && !self.child_models.contains_key(&record.child_session_id) {
            let settings = crate::config::ai::ai_config_path()
                .ok()
                .and_then(|path| crate::config::ai::load_ai(&path).ok().flatten())
                .unwrap_or_else(crate::config::ai::AiSettings::empty);
            let reference = match record
                .model_variant
                .as_deref()
                .map(str::trim)
                .filter(|variant| !variant.is_empty())
            {
                Some(variant) => format!("{}/{}#{variant}", record.model_provider, record.model_id),
                None => format!("{}/{}", record.model_provider, record.model_id),
            };
            if let Ok(target) =
                crate::chat::agent::exec::resolve_model_target(&settings, Some(&reference), None)
            {
                self.child_models
                    .insert(record.child_session_id.clone(), target);
            }
        }
        self.child_models
            .get(&record.child_session_id)
            .unwrap_or(&self.model)
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
    ) -> Result<SubagentOutcome, HostError> {
        let record = db
            .resolve_owned_subagent(&self.workspace, &self.parent_session_id, target)
            .map_err(db_failed)?
            .ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let target = record.agent_id.as_str();
        let is_running = self.running.contains_key(target);
        if !is_running && !self.resumed.contains(target) {
            return Err(failed(NOT_ACTIVE_CODE));
        }
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
                &self.parent_session_id,
                target,
                &dedupe,
                call_id,
                Some(parent_assistant_id),
                &persisted,
                interrupt,
            )
            .map_err(db_failed)?;
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
            });
        }
        let (mut status, mut queued_count) = (enqueued.status, enqueued.queued_count);
        if let Some(claimed) = self.claim_and_launch(db, target, None)? {
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
        })
    }

    fn interrupt(&mut self, db: &Database, target: &str) -> Result<SubagentOutcome, HostError> {
        let record = db
            .resolve_owned_subagent(&self.workspace, &self.parent_session_id, target)
            .map_err(db_failed)?
            .ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let canonical_id = record.agent_id;
        let queued_count = db
            .subagent_queue_states(&self.workspace, &self.parent_session_id)
            .unwrap_or_default()
            .get(&canonical_id)
            .map_or(0, |state| state.queued_count);
        if let Some(handle) = self.running.get(&canonical_id) {
            self.paused_by_interrupt.insert(canonical_id.clone());
            self.resumed.insert(canonical_id.clone());
            self.interrupted.remove(&canonical_id);
            let _ = handle.cancel_tx.send(true);
            return Ok(SubagentOutcome::Interrupted {
                agent_id: canonical_id,
                nickname: record.nickname,
                previous_status: STATUS_RUNNING.to_string(),
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
        // 先判运行态：interrupt 取消未落定时，resume 只是把「恢复」意图交给结算路径。
        let is_running = self.running.contains_key(agent_id);
        if self.paused_by_interrupt.remove(agent_id) && is_running {
            self.interrupted.insert(agent_id.to_string());
        }
        let queued_count = db
            .subagent_queue_states(&self.workspace, &self.parent_session_id)
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
        db.recover_subagent_for_resume(&self.workspace, &self.parent_session_id, agent_id)
            .map_err(db_failed)?;
        self.resumed.insert(agent_id.to_string());
        match self.claim_and_launch(db, agent_id, None)? {
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
        if let Some(handle) = self.running.get(target) {
            let _ = handle.cancel_tx.send(true);
        }
        self.interrupted.remove(target);
        self.paused_by_interrupt.remove(target);
        self.resumed.remove(target);
        if !is_running {
            // idle 关闭：DB 记录与排队输入一并置 cancelled，避免留下可被误恢复的孤儿队列。
            let _ = db.close_idle_subagent(&self.workspace, &self.parent_session_id, target);
        }
        Ok(SubagentOutcome::Closed {
            agent_id: target.to_string(),
            previous_status,
        })
    }

    fn list(&self, db: &Database) -> Result<SubagentOutcome, HostError> {
        let records = db
            .list_subagents(&self.workspace, &self.parent_session_id)
            .map_err(db_failed)?;
        let queues = db
            .subagent_queue_states(&self.workspace, &self.parent_session_id)
            .unwrap_or_default();
        let agents = records
            .iter()
            .map(|record| AgentListItemView {
                agent_id: record.agent_id.clone(),
                nickname: record.nickname.clone(),
                role: record.role,
                title: record.title.clone(),
                status: self.projected_status(record, queues.get(&record.agent_id)),
                queued_count: queues.get(&record.agent_id).map_or(0, |q| q.queued_count),
            })
            .collect();
        Ok(SubagentOutcome::Listed { agents })
    }

    fn cancel_input(
        &self,
        db: &Database,
        submission_id: &str,
    ) -> Result<SubagentOutcome, HostError> {
        match db
            .cancel_input(&self.workspace, &self.parent_session_id, submission_id)
            .map_err(db_failed)?
        {
            Some((agent_id, status, queued_count)) => Ok(SubagentOutcome::InputCancelled {
                submission_id: submission_id.to_string(),
                agent_id,
                status,
                queued_count,
            }),
            None => Err(failed(INPUT_NOT_FOUND_CODE)),
        }
    }

    /// 认领队首输入并启动下一轮；没有可执行输入返回 `Ok(None)`。
    fn claim_and_launch(
        &mut self,
        db: &Database,
        agent_id: &str,
        reused_done: Option<watch::Sender<bool>>,
    ) -> Result<Option<crate::db::chat::ClaimedSubagentTurn>, HostError> {
        if self.running.contains_key(agent_id) {
            return Ok(None);
        }
        let record = db
            .owned_subagent(&self.workspace, &self.parent_session_id, agent_id)
            .map_err(db_failed)?
            .ok_or_else(|| failed(NOT_FOUND_CODE))?;
        let Some(next) = db
            .next_queued_input(&self.workspace, &self.parent_session_id, agent_id)
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
                &self.parent_session_id,
                agent_id,
                &next.submission_id,
                &child_snapshot,
            )
            .map_err(db_failed)?;
        self.start_child(
            db,
            agent_id,
            &claimed.child_session_id,
            &claimed.child_assistant_id,
            &claimed.child_user_message_id,
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
        agent_id: &str,
        reused_done: Option<watch::Sender<bool>>,
    ) -> Result<bool, HostError> {
        Ok(self.claim_and_launch(db, agent_id, reused_done)?.is_some())
    }

    fn projected_status(
        &self,
        record: &SubagentRecord,
        queue: Option<&SubagentQueueState>,
    ) -> String {
        if self.running.contains_key(&record.agent_id) {
            return STATUS_RUNNING.to_string();
        }
        if queue
            .and_then(|state| state.head_error_code.as_ref())
            .is_some()
        {
            return STATUS_BLOCKED.to_string();
        }
        if self.resumed.contains(&record.agent_id) {
            return STATUS_IDLE.to_string();
        }
        if record.status == "cancelled"
            && queue.and_then(|state| state.last_input_status.as_deref())
                == Some(STATUS_INTERRUPTED)
        {
            return STATUS_INTERRUPTED.to_string();
        }
        record.status.clone()
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

fn first_line_title(task: &str) -> String {
    task.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("subagent")
        .chars()
        .take(80)
        .collect()
}

/// 测试兜底模型解析：`<provider>/<model>` 拆分写入 provider/model，纯模型 ID 只改 model。
fn apply_reference_to_target(target: &mut ExecModelTarget, reference: &str) {
    match reference.split_once('/') {
        Some((provider, model)) if !provider.trim().is_empty() && !model.trim().is_empty() => {
            target.provider = provider.trim().to_string();
            target.model = model.trim().to_string();
        }
        _ => target.model = reference.trim().to_string(),
    }
}

fn close_open_part(parts: &mut [ChatPart], open_part_index: &mut Option<usize>) {
    if let Some(index) = open_part_index.take()
        && let Some(part) = parts.get_mut(index)
    {
        part.completed_at_ms = Some(crate::db::now_ms());
    }
}
