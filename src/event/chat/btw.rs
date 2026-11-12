//! 旁路问答（`/btw`）的事件层：提交、排队、只读回答与结算。
//!
//! 与子 agent 的关键差异：
//! - 独立 `source='btw'` child session，不写 `agent_subagent`，不产生父报告；
//! - 每条回答只结算自己的 assistant 消息，不调用 `finish_subagent_turn`；
//! - 目标实例只读引用，不 resume、不发送输入、不打断原任务；
//! - 关闭面板/切换主会话不取消：只有显式 Stop、删除归属或退出应用才停止。

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
        qa_id: String,
        view: Box<BtwLoadedView>,
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
    pub started_at_ms: i64,
    /// 上一次流中 checkpoint 的时刻（面板关闭后仍能在库里留下已生成正文）。
    last_checkpoint_ms: i64,
    pub label: String,
}

/// 一条等待启动的回答者。
struct BtwPending {
    thread_id: String,
    batch_id: String,
    recipient_id: String,
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
            qa_id: qa_id.to_string(),
            view: Box::new(BtwLoadedView {
                thread_id: None,
                child_session_id: None,
                title: String::new(),
                messages: Vec::new(),
                targets: Vec::new(),
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
    let targets = resolve_target_views(persistence, &workspace, parent_session_id, &thread);
    let _ = sender.send(AppEvent::Btw(BtwEvent::ThreadLoaded {
        pane,
        qa_id: qa_id.to_string(),
        view: Box::new(BtwLoadedView {
            thread_id: Some(thread.id),
            child_session_id: Some(thread.child_session_id),
            title: thread.title,
            messages,
            targets,
            loaded: true,
        }),
    }));
    true
}

/// 线程保存的稳定 agent ID → 展示标签（找不到记录时只显示 ID）。
fn resolve_target_views(
    persistence: &Persistence,
    workspace: &Path,
    parent_session_id: &str,
    thread: &BtwThreadRecord,
) -> Vec<crate::app::state::lx::BtwTargetView> {
    if thread.answerers.is_empty() {
        return Vec::new();
    }
    let records = persistence
        .database()
        .and_then(|db| db.list_subagents(workspace, parent_session_id).ok())
        .unwrap_or_default();
    thread
        .answerers
        .iter()
        .map(|id| {
            let record = records.iter().find(|record| &record.agent_id == id);
            crate::app::state::lx::BtwTargetView {
                key: id.clone(),
                label: record
                    .map(|record| {
                        let name = record.nickname.clone().unwrap_or_else(|| id.clone());
                        format!("{name} [{}]", record.role.as_str())
                    })
                    .unwrap_or_else(|| id.clone()),
                agent_id: Some(id.clone()),
            }
        })
        .collect()
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
        streams.btw.pending.push_back(BtwPending {
            thread_id: accepted.thread.id.clone(),
            batch_id: accepted.batch_id.clone(),
            recipient_id: recipient.id.clone(),
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
        if start_recipient(state, streams, persistence, sender, pending) {
            changed = true;
        }
    }
    changed
}

/// 启动一个回答者（DB 认领 + 上下文装配 + 网络任务）。
fn start_recipient(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    pending: BtwPending,
) -> bool {
    let Some(database) = persistence.database() else {
        return false;
    };
    // 工作区/cwd 取提交时冻结值：源窗格关闭或工作区切换后仍按原归属运行。
    let workspace = pending.workspace.clone();
    let Some(start) = database
        .start_btw_recipient(&workspace, &pending.recipient_id)
        .ok()
        .flatten()
    else {
        // 已被恢复流程/其它路径结算：收敛批次状态，避免永久停在 queued/running。
        let _ = database.sync_btw_batch_status(&pending.batch_id);
        return false;
    };
    let pane = pending.pane;
    let cwd = pending.cwd.clone();
    let selection = state
        .models
        .normalize_selection(crate::config::ai::ModelSelection {
            provider: start.model.provider.clone(),
            model: start.model.model.clone(),
            variant: start.model.variant.clone(),
        });
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        let _ = database.finish_btw_recipient(
            &workspace,
            &pending.recipient_id,
            &start.assistant_message_id,
            None,
            &TurnOutcome::Failed {
                error_code: "model unavailable".to_string(),
            },
            Default::default(),
            Default::default(),
        );
        return false;
    };
    let mcp_snapshot = super::mcp::snapshot_for_turn_with_disabled(
        persistence,
        state
            .project_root_for_pane(pane)
            .as_deref()
            .or(Some(workspace.as_path())),
        &std::collections::BTreeSet::new(),
    );
    let references = agent_references(persistence, &workspace, &start, &pending.thread_id);
    let context = match assemble_btw_context(BtwContextInput {
        database,
        workspace: &workspace,
        parent_session_id: &start.parent_session_id,
        qa_anchor_id: &start.qa_message_id,
        child_session_id: &start.child_session_id,
        current_user_message_id: &start.user_message_id,
        question: &start.question,
        answerer_label: &start.display_nickname,
        references: &references,
        model: &start.model,
        variant_params: frozen.variant_params.clone(),
        cwd: &cwd,
        context_limit: frozen.context_limit,
        mcp: &mcp_snapshot,
    }) {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(%error, "btw context assembly failed");
            let _ = database.finish_btw_recipient(
                &workspace,
                &pending.recipient_id,
                &start.assistant_message_id,
                None,
                &TurnOutcome::Failed {
                    error_code: "context unavailable".to_string(),
                },
                Default::default(),
                Default::default(),
            );
            return false;
        }
    };
    let Some(sender) = Some(sender.clone()) else {
        return false;
    };
    let (manager, turn) = match streams.runtime_for(cwd.clone(), &start.child_session_id) {
        Some(runtime) => (runtime.manager.clone(), runtime.turn(&pending.recipient_id)),
        None => return false,
    };
    streams.btw.generation = streams.btw.generation.saturating_add(1);
    let key = RequestKey {
        pane,
        session: start.child_session_id.clone(),
        generation: streams.btw.generation,
    };
    streams
        .btw
        .active_batches
        .insert(pending.thread_id.clone(), pending.batch_id.clone());
    let (cancel_sender, cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender.clone());
    streams.db_workspaces.insert(key.clone(), workspace.clone());
    let run = BtwRun {
        thread_id: pending.thread_id.clone(),
        batch_id: pending.batch_id.clone(),
        recipient_id: pending.recipient_id.clone(),
        assistant_message_id: start.assistant_message_id.clone(),
        child_session_id: start.child_session_id.clone(),
        parent_session_id: start.parent_session_id.clone(),
        started_at_ms: crate::db::now_ms(),
        last_checkpoint_ms: crate::db::now_ms(),
        label: format!(
            "{} [{}]",
            start.display_nickname,
            if start.target_kind == "default" {
                "lx".to_string()
            } else {
                start.target_kind.clone()
            }
        ),
    };
    let label = run.label.clone();
    streams.btw.runs.insert(key.clone(), run);

    let reads = crate::chat::tools::read_registry(&start.child_session_id);
    let recorder = super::file_snapshots::HostRecorderFactory::new(
        sender.clone(),
        workspace.clone(),
        start.child_session_id.clone(),
        std::sync::Arc::new(std::sync::RwLock::new(start.user_message_id.clone())),
    );
    let mut executor = AgentTools::new(manager, turn)
        .with_recorder(std::sync::Arc::new(recorder))
        .with_reads(reads)
        .with_image_support(frozen.supports_image_input)
        .with_mcp(
            crate::mcp::global_manager().clone(),
            mcp_snapshot,
            start.child_session_id.clone(),
        );
    let _ = &mut executor;
    let transport = frozen.transport.clone();
    let base_url = frozen.base_url.clone();
    let api_key = frozen.api_key.clone();
    let idle_timeout = Duration::from_millis(frozen.idle_timeout_ms);
    let stream_key = key.clone();
    let stream_log = crate::chat::client::StreamLog {
        kind: "btw",
        pane: Some(pane.raw()),
        session_id: Some(key.session.clone()),
        generation: Some(key.generation),
        parent_session: Some(start.parent_session_id.clone()),
        ..crate::chat::client::StreamLog::default()
    };
    let _ = sender.send(AppEvent::Btw(BtwEvent::RecipientStarted {
        child_session_id: start.child_session_id.clone(),
        assistant_message_id: start.assistant_message_id.clone(),
        label,
    }));
    let finish_sender = sender.clone();
    tokio::spawn(async move {
        let task_sender = sender;
        let task_key = stream_key.clone();
        // 内层任务捕获 panic：否则 panic 后既不会回投 Finished，也不会结算回答者，
        // 批次会永久停在 running、后续追问永久排队（表现为「AI 无法输出」）。
        let handle = tokio::spawn(async move {
            let live_sender = task_sender.clone();
            let live_key = task_key.clone();
            let stream_one = move |request: ChatRequest, cancel: watch::Receiver<bool>| {
                let sender = live_sender.clone();
                let key = live_key.clone();
                let transport = transport.clone();
                let base_url = base_url.clone();
                let api_key = api_key.clone();
                let stream_log = stream_log.clone();
                let emit = request.purpose != RequestPurpose::CompactSummary;
                async move {
                    crate::chat::client::run_stream_with_log(
                        &transport,
                        &base_url,
                        Some(api_key.as_str()),
                        idle_timeout,
                        request,
                        &stream_log,
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
            super::host::drive_agent_turn(
                task_key,
                context,
                task_sender,
                executor,
                stream_one,
                cancel,
            )
            .await
        });
        let summary = match handle.await {
            Ok(summary) => summary,
            Err(error) => {
                tracing::error!(%error, "btw turn task failed");
                crate::chat::agent_loop::TurnSummary {
                    outcome: crate::chat::agent_loop::TurnOutcome::Failed {
                        code: "btw task aborted".to_string(),
                    },
                    usage: Default::default(),
                    billing: Default::default(),
                    budget_exceeded: false,
                }
            }
        };
        let _ = finish_sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
            key: stream_key,
            reason: summary.stop_reason(),
            budget_exceeded: summary.budget_exceeded,
            usage: summary.usage,
            billing: summary.billing,
        }));
    });
    true
}

fn agent_references(
    persistence: &Persistence,
    workspace: &Path,
    start: &crate::db::chat::BtwRecipientStart,
    _thread_id: &str,
) -> Vec<BtwAgentReference> {
    let Some(database) = persistence.database() else {
        return Vec::new();
    };
    let Some(agent_id) = start.agent_id.as_deref() else {
        return Vec::new();
    };
    let records = database
        .list_subagents(workspace, &start.parent_session_id)
        .unwrap_or_default();
    records
        .iter()
        .filter(|record| record.agent_id == agent_id)
        .map(|record| BtwAgentReference {
            agent_id: record.agent_id.clone(),
            nickname: record
                .nickname
                .clone()
                .unwrap_or_else(|| record.agent_id.clone()),
            role: record.role.as_str().to_string(),
            status: record.status.clone(),
            summary: format!(
                "title: {}\nstatus: {}\nchild_session: {}",
                record.title, record.status, record.child_session_id
            ),
        })
        .collect()
}

/// 停止某线程的全部活动与排队回答；不影响原任务。
fn stop_thread(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    thread_id: Option<&str>,
) -> bool {
    let Some(thread_id) = thread_id else {
        return false;
    };
    let keys: Vec<RequestKey> = streams
        .btw
        .runs
        .iter()
        .filter(|(_, run)| run.thread_id == thread_id)
        .map(|(key, _)| key.clone())
        .collect();
    let pending_batches: Vec<String> = streams
        .btw
        .pending
        .iter()
        .filter(|pending| pending.thread_id == thread_id)
        .map(|pending| pending.batch_id.clone())
        .collect();
    let changed = cancel_threads(streams, &[thread_id.to_string()]);
    if let Some(database) = persistence.database() {
        let mut batches = pending_batches;
        batches.sort();
        batches.dedup();
        for batch in batches {
            let _ = database.cancel_btw_queued(&batch);
        }
    }
    let _ = state;
    changed || !keys.is_empty()
}

/// 取消若干线程的在途回答与排队项（不写库）。
fn cancel_threads(streams: &mut ChatStreams, thread_ids: &[String]) -> bool {
    let keys: Vec<RequestKey> = streams
        .btw
        .runs
        .iter()
        .filter(|(_, run)| thread_ids.iter().any(|id| id == &run.thread_id))
        .map(|(key, _)| key.clone())
        .collect();
    let mut changed = false;
    for key in keys {
        changed |= streams.cancel(&key);
    }
    let before = streams.btw.pending.len();
    streams
        .btw
        .pending
        .retain(|pending| !thread_ids.iter().any(|id| id == &pending.thread_id));
    for thread in thread_ids {
        streams.btw.active_batches.remove(thread);
    }
    changed || streams.btw.pending.len() != before
}

/// 旁路网络事件分流（Delta / Finished）。
pub(crate) fn handle_net_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    event: ChatNetEvent,
) -> bool {
    let key = match &event {
        ChatNetEvent::Delta { key, .. } | ChatNetEvent::Finished { key, .. } => key.clone(),
    };
    let Some(run) = streams.btw.runs.get(&key) else {
        // 已结束或已删除的 BTW 事件：丢弃，绝不回落主路径。
        return false;
    };
    let assistant_id = run.assistant_message_id.clone();
    let child_session = run.child_session_id.clone();
    match event {
        ChatNetEvent::Delta { deltas, .. } => {
            let changed =
                crate::app::update::on_btw_delta(state, &child_session, &assistant_id, &deltas);
            // 流中 checkpoint：面板关闭后仍可在库里留下已生成正文；节流 500ms。
            let now = crate::db::now_ms();
            if let Some(run) = streams.btw.runs.get_mut(&key)
                && now.saturating_sub(run.last_checkpoint_ms) >= 500
            {
                run.last_checkpoint_ms = now;
                let parts = crate::app::update::btw_parts(state, &child_session, &assistant_id);
                if !parts.is_empty()
                    && let Some(workspace) = streams.db_workspaces.get(&key).cloned()
                {
                    state
                        .effects
                        .push(crate::app::effects::Effect::CheckpointChatTurn {
                            workspace,
                            assistant_id: assistant_id.clone(),
                            parts,
                        });
                }
            }
            changed
        }
        ChatNetEvent::Finished {
            reason,
            usage,
            billing,
            ..
        } => finish_run(
            state,
            streams,
            persistence,
            &key,
            &child_session,
            &assistant_id,
            reason,
            usage,
            billing,
        ),
    }
}

/// 结算一个旁路回答并推进同线程下一批次。
#[allow(clippy::too_many_arguments)]
fn finish_run(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    child_session: &str,
    assistant_id: &str,
    reason: StopReason,
    usage: crate::db::chat::Usage,
    billing: crate::db::chat::Usage,
) -> bool {
    let Some(run) = streams.btw.runs.remove(key) else {
        return false;
    };
    let workspace = streams.db_workspaces.get(key).cloned().unwrap_or_default();
    let duration_ms = crate::db::now_ms().saturating_sub(run.started_at_ms).max(0);
    // 先在内存收尾（关闭思考块/在途工具），再取 parts 落库；顺序反过来会丢最后一次收尾。
    let changed = crate::app::update::on_btw_finished(
        state,
        child_session,
        assistant_id,
        &reason,
        usage,
        duration_ms,
    );
    let parts = crate::app::update::btw_parts(state, child_session, assistant_id);
    let outcome = match &reason {
        StopReason::Complete => TurnOutcome::Complete {
            stop_reason: Some("complete".to_string()),
        },
        StopReason::Cancelled => TurnOutcome::Interrupted,
        StopReason::Failed { code } => TurnOutcome::Failed {
            error_code: code.clone(),
        },
    };
    let mut batch_terminal = false;
    if let Some(database) = persistence.database() {
        match database.finish_btw_recipient(
            &workspace,
            &run.recipient_id,
            assistant_id,
            if parts.is_empty() {
                None
            } else {
                Some(parts.as_slice())
            },
            &outcome,
            usage,
            billing,
        ) {
            Ok(completion) => {
                batch_terminal = completion.batch_terminal;
                streams
                    .btw
                    .last_status
                    .insert(run.thread_id.clone(), completion.recipient_status);
            }
            Err(error) => tracing::warn!(code = error.code(), "btw settle failed"),
        }
    }
    // 只清本 run 的执行资源；不按 session 清理其他目标的模型。
    streams.cancels.remove(key);
    streams.db_workspaces.remove(key);
    streams.btw.runs.remove(key);
    if batch_terminal
        && streams
            .btw
            .active_batches
            .get(&run.thread_id)
            .is_some_and(|batch| batch == &run.batch_id)
    {
        streams.btw.active_batches.remove(&run.thread_id);
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
        BtwEvent::ThreadLoaded { pane, qa_id, view } => {
            let view = *view;
            crate::app::update::on_btw_thread_loaded(
                state,
                pane,
                &qa_id,
                view.thread_id,
                view.child_session_id,
                view.title,
                view.messages,
                view.targets,
                view.loaded,
            );
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
            let _ = qa_id;
            crate::app::update::on_btw_accepted(
                state,
                pane,
                crate::app::update::AcceptedBtw {
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
            qa_id: _,
            message,
        } => {
            crate::app::update::on_btw_failed(state, pane, &message);
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

/// 启动恢复：把未完成的 BTW 批次标为中断（不自动重发）。
pub(crate) fn recover_startup(database: &crate::db::Database) {
    if let Err(error) = database.recover_btw_startup() {
        tracing::warn!(code = error.code(), "btw startup recovery failed");
    }
}
