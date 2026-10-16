//! lx 聊天网络流编排：取走网络效应、装配上下文、启动 Tokio 任务并回投结果。
//!
//! 事件循环每轮调用 `pump_chat_streams`；DB 效应由 Persistence 同步泵执行，
//! 网络效应在此转后台任务。delta 按 `(pane, session, generation)` 回投过滤，
//! 网络模块不直接修改 `AppState`。

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use crate::app::effects::Effect;
use crate::app::state::{AppState, RequestKey};
use crate::app::update;
use crate::chat::client::StreamOutcome;
use crate::chat::context::{self, AssemblyInput};
use crate::chat::events::{ChatDelta, ChatRequest, StopReason};
use crate::db::chat::{Usage, chat_part_views};
use crate::event::AppEvent;

/// 在途流的取消句柄（按 request key 存放）。
#[derive(Default)]
pub struct ChatStreams {
    /// 取消信号发送端。
    cancels: HashMap<RequestKey, watch::Sender<bool>>,
    /// checkpoint 合并状态：最近一次落盘时刻 + 累计字节。
    checkpoints: HashMap<RequestKey, (std::time::Instant, usize)>,
}

/// checkpoint 落盘间隔与累计字节阈值（设计 5.3）。
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);
const CHECKPOINT_BYTES: usize = 4 * 1024;

/// 网络回投事件。
#[derive(Debug)]
pub enum ChatNetEvent {
    /// 一批流增量（可合并；避免逐 token 无界堆积）。
    Delta {
        key: RequestKey,
        deltas: Vec<ChatDelta>,
    },
    /// 流结束（含终态原因与 usage）。
    Finished {
        key: RequestKey,
        reason: StopReason,
        usage: Usage,
    },
}

/// 取走网络效应并启动后台流任务；返回是否有变化。
///
/// `sender` 把结果投回唯一事件循环；本函数只做装配与 spawn。
pub fn pump_chat_streams(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
) -> bool {
    let mut changed = false;
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::StartChatStream { .. } | Effect::CancelChatStream { .. }
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
            Effect::StartChatStream { .. } | Effect::CancelChatStream { .. }
        )
    });
    for effect in effects {
        match effect {
            Effect::StartChatStream { key, assistant_id } => {
                changed |= start_stream(state, streams, sender, key, assistant_id);
            }
            Effect::CancelChatStream { key } => {
                if let Some(cancel) = streams.cancels.remove(&key) {
                    // 关闭取消信号；流任务随后回投 Finished(Cancelled)。
                    let _ = cancel.send(true);
                }
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// 启动一条流：装配上下文、冻结模型快照并 spawn。
fn start_stream(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    _assistant_id: String,
) -> bool {
    // 已有同 key 在途流：不重复启动。
    if streams.cancels.contains_key(&key) {
        return false;
    }
    let Some(pane) = state.pane_anywhere(key.pane) else {
        return false;
    };
    // 冻结模型快照：从窗格当前消息流状态读取该轮的选择（发送时冻结语义：
    // Committing 已把模型写入 DB；流启动时按窗格 selection 重解析——
    // 在途请求持有启动时快照，设置页中途保存不影响本轮）。
    let selection = pane
        .lx
        .chat
        .selection
        .clone()
        .or_else(|| state.models.default_selection());
    let Some(selection) = selection else {
        // 无有效模型：直接按失败收尾。
        let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
            key,
            reason: StopReason::Failed {
                code: "no model available".into(),
            },
            usage: Usage::default(),
        }));
        return true;
    };
    let Some(frozen) = crate::app::state::FrozenModel::resolve(&state.models, &selection) else {
        let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
            key,
            reason: StopReason::Failed {
                code: "model configuration invalid".into(),
            },
            usage: Usage::default(),
        }));
        return true;
    };
    // 上下文：从窗格内存中的完整轮次装配（DB 历史与内存一致；partial 排除）。
    let draft = pane
        .lx
        .chat
        .messages
        .last()
        .filter(|message| message.role == "user")
        .map(|message| {
            message
                .parts
                .iter()
                .map(|part| part.content.as_str())
                .collect::<String>()
        })
        .unwrap_or_default();
    let db_messages: Vec<crate::db::chat::ChatMessage> =
        chat_part_views_to_db(&pane.lx.chat.messages);
    let assembly = context::assemble(&AssemblyInput {
        messages: &db_messages,
        draft: &draft,
        model: &crate::db::chat::ModelSnapshot {
            provider: frozen.provider.clone(),
            model: frozen.model.clone(),
            variant: frozen.variant.clone(),
        },
        context_limit: frozen.context_limit,
        output_limit: frozen.output_limit,
        variant_params: frozen.variant_params.clone(),
    });
    let request = ChatRequest {
        turns: assembly.request.turns.clone(),
        model: crate::db::chat::ModelSnapshot {
            provider: frozen.provider.clone(),
            model: frozen.model.clone(),
            variant: frozen.variant.clone(),
        },
        variant_params: frozen.variant_params.clone(),
    };
    // 省略提示（一次性显示）。
    if assembly.omitted_turns > 0
        && let Some(pane) = state.pane_mut_anywhere(key.pane)
    {
        pane.lx.chat.omitted_turns_notice = Some(assembly.omitted_turns);
    }
    // 取消通道；sender 由 ChatStreams 持有。
    let (cancel_sender, cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender);
    let transport = frozen.transport.clone();
    let base_url = frozen.base_url.clone();
    let api_key = frozen.api_key.clone();
    let idle_timeout = Duration::from_millis(frozen.idle_timeout_ms);
    let net_sender = sender.clone();
    let stream_key = key.clone();
    tokio::spawn(async move {
        let outcome = crate::chat::client::run_stream(
            &transport,
            &base_url,
            Some(api_key.as_str()),
            idle_timeout,
            request,
            cancel,
        )
        .await;
        report_outcome(net_sender, stream_key, outcome);
    });
    true
}

/// 把流结果拆分回投：内容 delta 先行，Finished 最后。
fn report_outcome(
    sender: mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    outcome: StreamOutcome,
) {
    let StreamOutcome { deltas, stop } = outcome;
    // 内容增量与 usage 合并为一批（移除非终态）。
    let content: Vec<ChatDelta> = deltas
        .into_iter()
        .filter(|delta| !matches!(delta, ChatDelta::Finished { .. }))
        .collect();
    let usage = content
        .iter()
        .rev()
        .find_map(|delta| match delta {
            ChatDelta::Usage(usage) => Some(*usage),
            _ => None,
        })
        .unwrap_or_default();
    if !content.is_empty() {
        let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Delta {
            key: key.clone(),
            deltas: content,
        }));
    }
    let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
        key,
        reason: stop,
        usage,
    }));
}

/// 网络事件回投处理：按 key 过滤后转 update。
pub fn handle_net_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    event: ChatNetEvent,
) -> bool {
    match event {
        ChatNetEvent::Delta { key, deltas } => {
            // 累计字节：超过间隔或阈值时入队一次 checkpoint（合并写盘）。
            let bytes: usize = deltas
                .iter()
                .filter_map(|delta| match delta {
                    ChatDelta::Part(part) => Some(part.text.len()),
                    _ => None,
                })
                .sum();
            let entry = streams
                .checkpoints
                .entry(key.clone())
                .or_insert_with(|| (std::time::Instant::now(), 0));
            entry.1 += bytes;
            let due = entry.1 >= CHECKPOINT_BYTES || entry.0.elapsed() >= CHECKPOINT_INTERVAL;
            if due {
                *entry = (std::time::Instant::now(), 0);
                schedule_checkpoint(state, &key);
            }
            update::on_stream_delta(state, &key, &deltas)
        }
        ChatNetEvent::Finished { key, reason, usage } => {
            // 清除在途取消句柄与 checkpoint 状态。
            streams.cancels.remove(&key);
            streams.checkpoints.remove(&key);
            update::on_stream_finished(state, &key, &reason, usage)
        }
    }
}

/// 入队一次 checkpoint：把窗格当前内存 parts 快照交给持久化层。
fn schedule_checkpoint(state: &mut AppState, key: &RequestKey) {
    let Some(target) = state.pane_anywhere(key.pane) else {
        return;
    };
    let assistant_id = match &target.lx.chat.request {
        crate::app::state::RequestState::Streaming {
            key: active,
            assistant_id,
        } if active == key => assistant_id.clone(),
        _ => return,
    };
    let parts = target
        .lx
        .chat
        .messages
        .last()
        .filter(|message| message.id == assistant_id)
        .map(|message| crate::db::chat::chat_part_views(&message.parts))
        .unwrap_or_default();
    let Some(workspace) = state.workspace_cwd_for_pane(key.pane) else {
        return;
    };
    state
        .effects
        .push(crate::app::effects::Effect::CheckpointChatTurn {
            workspace,
            assistant_id,
            parts,
        });
}

/// 窗格聊天视图 → DB 消息记录（上下文装配输入）。
fn chat_part_views_to_db(
    messages: &[crate::app::state::ChatMessageView],
) -> Vec<crate::db::chat::ChatMessage> {
    messages
        .iter()
        .map(|message| crate::db::chat::ChatMessage {
            external_id: message.id.clone(),
            sequence: 0,
            turn_index: message.turn_index,
            role: message.role.clone(),
            status: message.status.clone(),
            model_provider: None,
            model_id: None,
            variant: None,
            stop_reason: None,
            error_code: message.error_code.clone(),
            input_tokens: None,
            output_tokens: None,
            created_at_ms: 0,
            parts: chat_part_views(&message.parts),
        })
        .collect()
}
