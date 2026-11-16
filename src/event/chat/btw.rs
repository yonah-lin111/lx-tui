//! 旁路问答（`/btw`）的事件层：提交、排队、只读回答与结算。
//!
//! 与子 agent 的关键差异：
//! - 独立 `source='btw'` child session，不写 `agent_subagent`，不产生父报告；
//! - 每条回答只结算自己的 assistant 消息，不调用 `finish_subagent_turn`；
//! - 目标实例只读引用，不 resume、不发送输入、不打断原任务；
//! - 关闭面板/切换主会话不取消：只有显式 Stop、删除归属或退出应用才停止。

mod maintenance;
mod run;

pub(crate) use run::handle_net_event;
use run::{cancel_threads, start_recipient, stop_thread};

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use super::*;
use crate::app::effects::{BtwEffect, BtwSubmission};
use crate::app::state::lx::btw::BtwQaSummary;
use crate::app::state::{FrozenModel, RequestKey};
use crate::chat::btw::{BtwAgentReference, BtwContextInput, assemble_btw_context};
use crate::chat::events::{ChatRequest, RequestPurpose, StopReason};
use crate::db::chat::{BtwTargetSpec, BtwThreadRecord, ChatMessage, ModelSnapshot, TurnOutcome};
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::tiling::PaneId;

/// 全局同时运行的旁路回答数（独立于主子 agent 配额）。
const BTW_CONCURRENCY: usize = 2;

/// 事件层回投给应用层的 BTW 更新。
#[derive(Debug)]
pub enum BtwEvent {
    /// QA 列表已加载。
    QaListLoaded {
        pane: PaneId,
        parent_session_id: String,
        qa: Vec<BtwQaSummary>,
    },
    /// 某主 QA 的线程历史已加载。
    ThreadLoaded {
        pane: PaneId,
        parent_session_id: String,
        qa_id: String,
        view: Box<BtwLoadedView>,
    },
    /// 线程已被删除（`/new`）：视图回到空白，QA 列表随后刷新。
    ThreadDeleted {
        pane: PaneId,
        parent_session_id: String,
        qa_id: String,
    },
    /// 提交已被接受（user 消息与批次已落库）。
    Accepted {
        pane: PaneId,
        qa_id: String,
        submit_id: String,
        thread_id: String,
        child_session_id: String,
        user_message_id: String,
        text: String,
    },
    /// 提交失败（保留草稿）。
    Failed {
        pane: PaneId,
        qa_id: Option<String>,
        message: String,
    },
    /// 回答者开始（assistant 占位已落库）。
    RecipientStarted {
        child_session_id: String,
        assistant_message_id: String,
        label: String,
    },
}

/// 线程加载结果。
#[derive(Debug)]
pub struct BtwLoadedView {
    pub thread_id: Option<String>,
    pub child_session_id: Option<String>,
    pub title: String,
    pub messages: Vec<ChatMessage>,
    pub targets: Vec<crate::app::state::lx::BtwTargetView>,
    /// 该线程的 compact 检查点（与消息一起回填，卡片才能显示）。
    pub checkpoints: Vec<crate::db::chat::CompactCheckpoint>,
    pub loaded: bool,
}

/// 一个在途旁路回答。
pub(crate) struct BtwRun {
    pub thread_id: String,
    pub batch_id: String,
    pub recipient_id: String,
    pub assistant_message_id: String,
    pub child_session_id: String,
    pub parent_session_id: String,
    /// 独立于 pane 投影的流缓冲：切主会话/关 pane 后仍能结算完整正文。
    pub buffer: crate::app::state::LxChat,
    /// 本 recipient 的跨进程 session guard；保持到 finish，避免启动恢复误伤。
    _guard: crate::platform::FileGuard,
    pub started_at_ms: i64,
    /// 上一次流中 checkpoint 的时刻（面板关闭后仍能在库里留下已生成正文）。
    last_checkpoint_ms: i64,
    /// 面板显示用的模型标签。
    pub label: String,
    /// 报告分节用的回答者名字（默认助手固定为 `BTW`）。
    pub answerer_label: String,
}

/// 一条等待启动的回答者。
struct BtwPending {
    thread_id: String,
    batch_id: String,
    recipient_id: String,
    qa_id: String,
    child_session_id: String,
    /// 报告分节用的回答者名字（默认助手固定为 `BTW`）。
    answerer_label: String,
    /// 提交时冻结的 DB 工作区与执行 cwd：关闭源窗格/切换工作区后仍按原归属运行。
    workspace: PathBuf,
    cwd: PathBuf,
    pane: PaneId,
    parent_session_id: String,
}

/// BTW 事件层状态（挂在 [`ChatStreams`] 上，与子 agent 完全独立）。
#[derive(Default)]
pub(crate) struct BtwStreams {
    /// `(pane, child_session, generation)` → 在途回答。
    pub runs: HashMap<RequestKey, BtwRun>,
    /// 待启动回答者（按批次接受顺序）。
    pending: VecDeque<BtwPending>,
    /// 线程 → 当前活动批次（同线程批次串行）。
    active_batches: HashMap<String, String>,
    /// 已见过的 BTW child session：迟到事件必须落在这里并被丢弃，不能回落主路径。
    pub sessions: HashSet<String>,
    /// 回答者代次计数器。
    generation: u64,
    /// 线程 → 最近的完成状态（停止/失败反馈用）。
    pub last_status: HashMap<String, String>,
    /// `/report` 批次 → 待聚合的合并报告（按 batch_id）。
    pub report_batches: HashMap<String, BtwReportBatch>,
}

/// 一次 `/report` 多目标批次的聚合状态。
pub(crate) struct BtwReportBatch {
    pub pane: PaneId,
    pub parent_session_id: String,
    pub thread_id: String,
    pub title: String,
    pub expected: usize,
    pub done: usize,
    pub sections: Vec<(String, String)>,
    pub failed: Vec<String>,
}

impl BtwStreams {
    /// 记录一个报告目标的结算结果；全部收口时返回待填入主输入框的合并报告。
    ///
    /// `section` 为 `None` 或空白表示该目标失败，写入失败占位。
    pub(super) fn record_report_outcome(
        &mut self,
        batch_id: &str,
        label: &str,
        section: Option<String>,
    ) -> Option<crate::app::update::BtwReportReady> {
        let batch = self.report_batches.get_mut(batch_id)?;
        match section {
            Some(body) if !body.trim().is_empty() => {
                batch.sections.push((label.to_string(), body));
            }
            _ => batch.failed.push(label.to_string()),
        }
        batch.done = batch.done.saturating_add(1);
        if batch.done < batch.expected {
            return None;
        }
        let batch = self.report_batches.remove(batch_id)?;
        Some(crate::app::update::BtwReportReady {
            pane: batch.pane,
            parent_session: batch.parent_session_id,
            thread_id: batch.thread_id,
            title: batch.title,
            sections: batch.sections,
            failed: batch.failed,
        })
    }
}

/// 处理一条 BTW 效应；返回状态是否变化。
pub(crate) fn apply_effect(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    effect: BtwEffect,
) -> bool {
    match effect {
        BtwEffect::LoadQaList {
            pane,
            parent_session_id,
        } => load_qa_list(state, persistence, sender, pane, &parent_session_id),
        BtwEffect::LoadThread {
            pane,
            parent_session_id,
            qa_id,
        } => load_thread(state, persistence, sender, pane, &parent_session_id, &qa_id),
        BtwEffect::Submit(submission) => submit(state, streams, persistence, sender, *submission),
        BtwEffect::DeleteThread {
            pane,
            parent_session_id,
            qa_id,
            thread_id,
        } => maintenance::delete_thread(
            state,
            persistence,
            sender,
            pane,
            &parent_session_id,
            &qa_id,
            &thread_id,
        ),
        BtwEffect::Stop {
            pane,
            parent_session_id,
            qa_id,
            thread_id,
        } => {
            let _ = (pane, parent_session_id, qa_id);
            stop_thread(state, streams, persistence, thread_id.as_deref())
        }
        BtwEffect::CancelOwner { parent_session_id } => {
            let mut thread_ids: Vec<String> = streams
                .btw
                .runs
                .values()
                .filter(|run| run.parent_session_id == parent_session_id)
                .map(|run| run.thread_id.clone())
                .collect();
            thread_ids.extend(
                streams
                    .btw
                    .pending
                    .iter()
                    .filter(|pending| pending.parent_session_id == parent_session_id)
                    .map(|pending| pending.thread_id.clone()),
            );
            thread_ids.sort();
            thread_ids.dedup();
            let _ = persistence;
            let _ = state;
            cancel_threads(streams, &thread_ids)
        }
    }
}

/// 解析工作区键：优先聊天会话键（`/resume` 共享键），回退窗格 cwd。
fn workspace_for_pane(state: &AppState, pane: PaneId) -> Option<PathBuf> {
    state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.workspace_cwd_for_pane(pane))
}

/// QA 标题：首条正文的首行（最多 60 字符）。
fn qa_title(messages: &[ChatMessage], qa_id: &str) -> String {
    let text = messages
        .iter()
        .find(|message| message.external_id == qa_id)
        .and_then(|message| {
            message
                .parts
                .iter()
                .find(|part| part.kind == "text")
                .map(|part| part.content.as_str())
        })
        .unwrap_or("");
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(59).collect::<String>())
    } else {
        line.to_string()
    }
}

fn load_qa_list(
    state: &mut AppState,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    pane: PaneId,
    parent_session_id: &str,
) -> bool {
    let Some(database) = persistence.database() else {
        return false;
    };
    let Some(workspace) = workspace_for_pane(state, pane) else {
        return false;
    };
    let anchors = database
        .qa_anchors(&workspace, parent_session_id)
        .unwrap_or_default();
    let messages = database
        .load_chat_session(&workspace, parent_session_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    let threads = database
        .list_btw_threads(&workspace, parent_session_id)
        .unwrap_or_default();
    let qa: Vec<BtwQaSummary> = anchors
        .iter()
        .map(|anchor| {
            let thread = threads
                .iter()
                .find(|summary| summary.thread.parent_user_message_id == anchor.message_id);
            BtwQaSummary {
                qa_id: anchor.message_id.clone(),
                title: qa_title(&messages, &anchor.message_id),
                has_thread: thread.is_some(),
                status: thread
                    .map(|summary| summary.last_status.clone())
                    .unwrap_or_else(|| "idle".to_string()),
            }
        })
        .collect();
    let _ = sender.send(AppEvent::Btw(BtwEvent::QaListLoaded {
        pane,
        parent_session_id: parent_session_id.to_string(),
        qa,
    }));
    true
}

fn load_thread(
    state: &mut AppState,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    pane: PaneId,
    parent_session_id: &str,
    qa_id: &str,
) -> bool {
    let Some(database) = persistence.database() else {
        return false;
    };
    let Some(workspace) = workspace_for_pane(state, pane) else {
        return false;
    };
    let thread = database
        .btw_thread_for_qa(&workspace, parent_session_id, qa_id)
        .ok()
        .flatten();
    let Some(thread) = thread else {
        let _ = sender.send(AppEvent::Btw(BtwEvent::ThreadLoaded {
            pane,
            parent_session_id: parent_session_id.to_string(),
            qa_id: qa_id.to_string(),
            view: Box::new(BtwLoadedView {
                thread_id: None,
                child_session_id: None,
                title: String::new(),
                messages: Vec::new(),
                targets: Vec::new(),
                checkpoints: Vec::new(),
                loaded: true,
            }),
        }));
        return true;
    };
    let messages = database
        .load_chat_session(&workspace, &thread.child_session_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    let targets = resolve_target_views(&thread);
    let checkpoints = database
        .compact_checkpoints(&workspace, &thread.child_session_id)
        .unwrap_or_default();
    let _ = sender.send(AppEvent::Btw(BtwEvent::ThreadLoaded {
        pane,
        parent_session_id: parent_session_id.to_string(),
        qa_id: qa_id.to_string(),
        view: Box::new(BtwLoadedView {
            thread_id: Some(thread.id),
            child_session_id: Some(thread.child_session_id),
            title: thread.title,
            messages,
            targets,
            checkpoints,
            loaded: true,
        }),
    }));
    true
}

/// 线程保存的稳定目标键 → 回查实例用的视图；`default` 是默认助手哨兵。
fn resolve_target_views(thread: &BtwThreadRecord) -> Vec<crate::app::state::lx::BtwTargetView> {
    thread
        .answerers
        .iter()
        .map(|id| target_view_for(id))
        .collect()
}

fn target_view_for(id: &str) -> crate::app::state::lx::BtwTargetView {
    if id == "default" {
        return crate::app::state::lx::BtwTargetView::default_assistant();
    }
    crate::app::state::lx::BtwTargetView {
        agent_id: Some(id.to_string()),
    }
}

/// 提交一次旁路问题：落库、回投 Accepted、入队回答者。
fn submit(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    submission: BtwSubmission,
) -> bool {
    let Some(database) = persistence.database() else {
        let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
            pane: submission.pane,
            qa_id: Some(submission.qa_id.clone()),
            message: crate::ui::text::LX_BTW_SUBMIT_FAILED.to_string(),
        }));
        return false;
    };
    // 解析目标模型：显式实例优先其持久化模型，失败回退默认助手模型。
    let targets = build_targets(state, persistence, &submission);
    let title = submission
        .text
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(60)
        .collect::<String>();
    let accepted = match database.accept_btw_batch(
        &submission.workspace,
        &submission.parent_session_id,
        &submission.qa_id,
        &title,
        &submission.submit_id,
        &submission.text,
        &targets,
    ) {
        Ok(accepted) => accepted,
        Err(error) => {
            tracing::warn!(code = error.code(), "btw submit failed");
            let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
                pane: submission.pane,
                qa_id: Some(submission.qa_id.clone()),
                message: crate::ui::text::LX_BTW_SUBMIT_FAILED.to_string(),
            }));
            return false;
        }
    };
    streams
        .btw
        .sessions
        .insert(accepted.thread.child_session_id.clone());
    if submission.report_to_main_input {
        streams.btw.report_batches.insert(
            accepted.batch_id.clone(),
            BtwReportBatch {
                pane: submission.pane,
                parent_session_id: submission.parent_session_id.clone(),
                thread_id: accepted.thread.id.clone(),
                title: accepted.thread.title.clone(),
                expected: accepted.recipients.len(),
                done: 0,
                sections: Vec::new(),
                failed: Vec::new(),
            },
        );
    }
    let _ = sender.send(AppEvent::Btw(BtwEvent::Accepted {
        pane: submission.pane,
        qa_id: submission.qa_id.clone(),
        submit_id: submission.submit_id.clone(),
        thread_id: accepted.thread.id.clone(),
        child_session_id: accepted.thread.child_session_id.clone(),
        user_message_id: accepted.user_message_id.clone(),
        text: submission.text.clone(),
    }));
    for recipient in &accepted.recipients {
        let answerer_label = if recipient.target_kind == "default" {
            "BTW".to_string()
        } else {
            recipient.display_nickname.clone()
        };
        streams.btw.pending.push_back(BtwPending {
            thread_id: accepted.thread.id.clone(),
            batch_id: accepted.batch_id.clone(),
            recipient_id: recipient.id.clone(),
            qa_id: submission.qa_id.clone(),
            child_session_id: accepted.thread.child_session_id.clone(),
            answerer_label,
            workspace: submission.workspace.clone(),
            cwd: submission.cwd.clone(),
            pane: submission.pane,
            parent_session_id: submission.parent_session_id.clone(),
        });
    }
    pump(state, streams, persistence, sender);
    true
}

/// 组装回答者快照；模型在提交时冻结。
fn build_targets(
    _state: &AppState,
    persistence: &Persistence,
    submission: &BtwSubmission,
) -> Vec<BtwTargetSpec> {
    let default_model = submission.default_model.clone();
    if submission.targets.is_empty() {
        return vec![BtwTargetSpec::default_assistant(default_model)];
    }
    let records = persistence
        .database()
        .and_then(|db| {
            db.list_subagents(&submission.workspace, &submission.parent_session_id)
                .ok()
        })
        .unwrap_or_default();
    submission
        .targets
        .iter()
        .map(|choice| {
            let record = records
                .iter()
                .find(|record| record.agent_id == choice.agent_id);
            let model = record
                .filter(|record| {
                    !record.model_provider.trim().is_empty() && !record.model_id.trim().is_empty()
                })
                .map(|record| ModelSnapshot {
                    provider: record.model_provider.clone(),
                    model: record.model_id.clone(),
                    variant: record.model_variant.clone(),
                })
                .unwrap_or_else(|| default_model.clone());
            BtwTargetSpec {
                target_kind: "subagent".to_string(),
                target_key: choice.agent_id.clone(),
                agent_id: Some(choice.agent_id.clone()),
                source_child_session_id: Some(choice.source_child_session_id.clone()),
                display_nickname: choice.display_nickname.clone(),
                role_name: Some(choice.role_name.clone()),
                model,
            }
        })
        .collect()
}

/// 启动排队回答者，直到全局并发上限。
pub(crate) fn pump(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
) -> bool {
    let mut changed = false;
    while streams.btw.runs.len() < BTW_CONCURRENCY {
        let position = streams.btw.pending.iter().position(|pending| {
            streams
                .btw
                .active_batches
                .get(&pending.thread_id)
                .is_none_or(|batch| batch == &pending.batch_id)
        });
        let Some(position) = position else {
            break;
        };
        let Some(pending) = streams.btw.pending.remove(position) else {
            break;
        };
        let batch_id = pending.batch_id.clone();
        let answerer_label = pending.answerer_label.clone();
        if start_recipient(state, streams, persistence, sender, pending) {
            changed = true;
        } else if let Some(ready) =
            streams
                .btw
                .record_report_outcome(&batch_id, &answerer_label, None)
        {
            // 启动失败也计入批次，避免报告永远等不到最后一个目标。
            changed |= crate::app::update::on_btw_report_ready(state, ready);
        }
    }
    changed
}

/// BTW 工具握手：BeginTool。
pub(crate) fn begin_tool(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    call: ReadyToolCall,
    reply: oneshot::Sender<Result<crate::db::chat::ToolCallTicket, HostError>>,
) -> bool {
    let Some(run) = streams.btw.runs.get(&key) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let assistant_id = run.assistant_message_id.clone();
    let child_session = run.child_session_id.clone();
    let recipient_id = run.recipient_id.clone();
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    // 同一 child session 内多目标可能拿到相同 provider call ID：入库 id 加目标命名空间，
    // 模型上下文仍使用供应商原始 id（结果回传按 loop 的原始 id 配对）。
    let storage_call_id = format!("btw:{recipient_id}:{}", call.call_id);
    // 与主轮/子 agent 同一条时间线门槛：先提交工具之前的 parts，DB 分配的
    // position 才与 app 时间线一致，长工具循环不会在后续 checkpoint/finish 撞位置。
    if let Some(chat) = state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.btw.thread_by_child_session(&child_session))
        .map(|thread| &thread.chat)
        && let Err(error) = super::timeline::checkpoint_before_tool(
            chat,
            &workspace,
            &assistant_id,
            &call.call_id,
            persistence,
        )
    {
        tracing::warn!(
            target: "lx::agent::tool",
            pane = key.pane.raw(),
            session = %child_session,
            assistant = %assistant_id,
            call_id = %call.call_id,
            code = error.code(),
            error = %error,
            "btw tool timeline sync failed"
        );
        let _ = reply.send(Err(HostError::Failed {
            code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
        }));
        return true;
    }
    let ticket = match persistence.begin_tool_call(
        &workspace,
        &child_session,
        &assistant_id,
        &storage_call_id,
        &call.name,
        &call.arguments_json,
    ) {
        Ok(ticket) => ticket,
        Err(error) => {
            tracing::warn!(
                target: "lx::agent::tool",
                pane = key.pane.raw(),
                session = %child_session,
                assistant = %assistant_id,
                call_id = %call.call_id,
                name = %call.name,
                code = error.code(),
                error = %error,
                "btw tool call pending record failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            return true;
        }
    };
    let pending = PendingBegin {
        key,
        ticket,
        reply: Some(reply),
    };
    if streams.pane_visible(pending.key.pane) {
        streams.pending_begins.push(pending);
        true
    } else {
        settle_begin(state, streams, persistence, pending)
    }
}

/// BTW 工具握手：FinishTool。
pub(crate) fn finish_tool(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    ticket: &crate::db::chat::ToolCallTicket,
    outcome: ToolOutcome,
    reply: oneshot::Sender<Result<(), HostError>>,
) -> bool {
    let Some(run) = streams.btw.runs.get(&key) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let assistant_id = run.assistant_message_id.clone();
    let child_session = run.child_session_id.clone();
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let result = super::tool_calls::db_result_of(&outcome);
    match persistence.finish_tool_call(&workspace, ticket, &result) {
        Ok(()) => {
            let (status, summary, error) = super::tool_calls::view_settlement(&outcome);
            let provider_call_id = ticket
                .call_id
                .splitn(3, ':')
                .nth(2)
                .unwrap_or(&ticket.call_id)
                .to_string();
            let changed = crate::app::update::on_btw_tool_state(
                state,
                &child_session,
                &assistant_id,
                crate::app::update::BtwToolUpdate::Settled(Box::new(
                    crate::app::update::BtwToolSettlement {
                        call_id: &provider_call_id,
                        status,
                        summary,
                        result_json: result.result_json.clone(),
                        error,
                    },
                )),
            );
            let _ = reply.send(Ok(()));
            changed
        }
        Err(error) => {
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{FINISH_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            true
        }
    }
}

/// 待回执 BeginTool 的推进（与主路径共用 pending 队列）。
pub(crate) use super::tool_calls::settle_begin;

/// 迟到的 BTW 工具/steer 请求：一律按取消/空结果回执，不执行。
pub(crate) fn reject_host_request(event: AgentHostEvent) -> bool {
    match event {
        AgentHostEvent::BeginTool { reply, .. } => {
            let _ = reply.send(Err(HostError::Cancelled));
            true
        }
        AgentHostEvent::FinishTool { reply, .. } => {
            let _ = reply.send(Err(HostError::Cancelled));
            true
        }
        AgentHostEvent::AskUser { reply, .. } => {
            let _ = reply.send(Err(HostError::Cancelled));
            true
        }
        AgentHostEvent::Subagent { reply, .. } => {
            let _ = reply.send(Err(HostError::Failed {
                code: crate::chat::agent::UNAVAILABLE_CODE.to_string(),
            }));
            true
        }
        AgentHostEvent::UserShellUpdates { reply, .. } => {
            let _ = reply.send(Vec::new());
            true
        }
        AgentHostEvent::SteerInputs { reply, .. } => {
            let _ = reply.send(Ok(Vec::new()));
            true
        }
        AgentHostEvent::HasPendingSteers { reply, .. } => {
            let _ = reply.send(false);
            true
        }
        AgentHostEvent::Mutation(_) => false,
    }
}

/// 把 BTW 回投事件写入应用层状态。
pub(crate) fn apply_event_to_state(state: &mut AppState, event: BtwEvent) {
    match event {
        BtwEvent::QaListLoaded {
            pane,
            parent_session_id,
            qa,
        } => {
            crate::app::update::on_qa_list_loaded(state, pane, &parent_session_id, qa);
        }
        BtwEvent::ThreadLoaded {
            pane,
            parent_session_id,
            qa_id,
            view,
        } => {
            let view = *view;
            crate::app::update::on_btw_thread_loaded(
                state,
                pane,
                &parent_session_id,
                &qa_id,
                view.thread_id,
                view.child_session_id,
                view.title,
                view.messages,
                view.targets,
                view.checkpoints,
                view.loaded,
            );
        }
        BtwEvent::ThreadDeleted {
            pane,
            parent_session_id,
            qa_id,
        } => {
            crate::app::update::on_btw_thread_deleted(state, pane, &parent_session_id, &qa_id);
        }
        BtwEvent::Accepted {
            pane,
            qa_id,
            submit_id,
            thread_id,
            child_session_id,
            user_message_id,
            text,
        } => {
            crate::app::update::on_btw_accepted(
                state,
                pane,
                crate::app::update::AcceptedBtw {
                    qa_id,
                    submit_id,
                    thread_id,
                    child_session_id,
                    user_message_id,
                    text,
                },
            );
        }
        BtwEvent::Failed {
            pane,
            qa_id,
            message,
        } => {
            crate::app::update::on_btw_failed(state, pane, qa_id.as_deref(), &message);
        }
        BtwEvent::RecipientStarted {
            child_session_id,
            assistant_message_id,
            label,
        } => {
            crate::app::update::on_btw_recipient_started(
                state,
                &child_session_id,
                &assistant_message_id,
                &label,
            );
        }
    }
}

/// 启动恢复：只处理能取得 child session guard 的 BTW 批次（不自动重发）。
pub(crate) fn recover_startup(database: &crate::db::Database) {
    let sessions = match database.btw_sessions_needing_recovery() {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                "scanning btw sessions for recovery failed"
            );
            return;
        }
    };
    let mut guards = Vec::new();
    let mut guarded = Vec::new();
    for session in sessions {
        match crate::platform::try_lock_session(&session) {
            Ok(Some(guard)) => {
                guards.push(guard);
                guarded.push(session);
            }
            Ok(None) => {
                tracing::info!(
                    %session,
                    "skipping btw recovery: session is owned by another instance"
                );
            }
            Err(error) => {
                tracing::warn!(%error, %session, "btw session guard unavailable during recovery");
            }
        }
    }
    if let Err(error) = database.recover_btw_startup_for(&guarded) {
        tracing::warn!(code = error.code(), "btw startup recovery failed");
    }
    drop(guards);
}

#[cfg(test)]
mod tests;
