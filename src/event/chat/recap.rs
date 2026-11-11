//! 后台 recap 请求：取走效应、执行一次性无工具流请求并回投结果。

use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::app::effects::Effect;
use crate::app::state::{AppState, FrozenModel};
use crate::app::update;
use crate::chat::client;
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::chat::recap::{
    RECAP_MAX_OUTPUT_BYTES, RECAP_TIMEOUT_SECS, RecapResult, build_recap_request,
    parse_generated_recap,
};
use crate::db::chat::{ModelSnapshot, RecapCommit};
use crate::db::service::Persistence;
use crate::event::AppEvent;

/// recap 生成完成事件载荷。
#[derive(Debug, Clone)]
pub struct RecapGeneratedEvent {
    pub pane: crate::tiling::PaneId,
    pub workspace: PathBuf,
    pub session_id: String,
    pub message_id: String,
    pub generation: u64,
    pub manual: bool,
    pub end_turn: i64,
    pub end_sequence: i64,
    pub model: ModelSnapshot,
    pub model_label: String,
    pub result: Result<RecapResult, String>,
}

/// 抽取并后台执行 recap 效应（不阻塞主对话流或事件循环）。
pub fn apply_recap_effects(state: &mut AppState, sender: &mpsc::UnboundedSender<AppEvent>) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::GenerateRecap { .. }))
        .cloned()
        .collect();
    if effects.is_empty() {
        return false;
    }
    state
        .effects
        .retain(|effect| !matches!(effect, Effect::GenerateRecap { .. }));

    for effect in effects {
        let Effect::GenerateRecap {
            pane,
            workspace,
            session_id,
            message_id,
            generation,
            manual,
            end_turn,
            end_sequence,
            prompt,
            model,
            model_label,
        } = effect
        else {
            continue;
        };
        let sender = sender.clone();
        let recap_session = Some(session_id.clone());
        tokio::spawn(async move {
            let result = execute_recap_stream(&prompt, &model, recap_session).await;
            let _ = sender.send(AppEvent::RecapGenerated(RecapGeneratedEvent {
                pane,
                workspace,
                session_id,
                message_id,
                generation,
                manual,
                end_turn,
                end_sequence,
                model: ModelSnapshot {
                    provider: model.provider.clone(),
                    model: model.model.clone(),
                    variant: model.variant.clone(),
                },
                model_label,
                result,
            }));
        });
    }
    true
}

/// 处理 recap 完成回执：校验代次、写库并回填内存视图。
pub fn handle_recap_generated(
    state: &mut AppState,
    persistence: &Persistence,
    event: RecapGeneratedEvent,
) -> bool {
    let RecapGeneratedEvent {
        pane,
        workspace,
        session_id,
        message_id,
        generation,
        manual,
        end_turn,
        end_sequence,
        model,
        model_label,
        result,
    } = event;
    if !update::take_pending_recap(state, pane, &session_id, &message_id, generation) {
        return false;
    }
    match result {
        Ok(recap) => {
            let commit = RecapCommit {
                session_id,
                message_id,
                summary: recap.summary,
                next_action: recap.next_action.unwrap_or_default(),
                model,
                model_label,
                end_turn,
                end_sequence,
            };
            match persistence.commit_recap(&workspace, &commit) {
                Ok(record) => update::apply_recap(state, pane, &record),
                Err(error) => {
                    tracing::warn!(%error, "failed to persist recap");
                    update::recap_failed(state, pane, manual)
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "failed to generate recap");
            update::recap_failed(state, pane, manual)
        }
    }
}

async fn execute_recap_stream(
    prompt: &str,
    model: &FrozenModel,
    session_id: Option<String>,
) -> Result<RecapResult, String> {
    let snapshot = ModelSnapshot {
        provider: model.provider.clone(),
        model: model.model.clone(),
        variant: model.variant.clone(),
    };
    let chat_request = build_recap_request(prompt, &snapshot);
    let (_cancel_sender, cancel) = client::uncancellable();
    let stream_log = client::StreamLog {
        kind: "recap",
        session_id,
        ..client::StreamLog::default()
    };
    let idle_timeout =
        Duration::from_millis(model.idle_timeout_ms).min(Duration::from_secs(RECAP_TIMEOUT_SECS));
    let stream_fut = client::run_stream_with_log(
        &model.transport,
        &model.base_url,
        Some(model.api_key.as_str()),
        idle_timeout,
        chat_request,
        &stream_log,
        cancel,
        |_| {},
    );
    let outcome =
        match tokio::time::timeout(Duration::from_secs(RECAP_TIMEOUT_SECS), stream_fut).await {
            Ok(outcome) => outcome,
            Err(_) => {
                tracing::debug!(model = %model.model, "recap generation timed out");
                return Err("timeout".to_string());
            }
        };
    if outcome.stop != StopReason::Complete {
        tracing::debug!(model = %model.model, stop = ?outcome.stop, "recap generation stopped early");
        return Err(format!("stop {:?}", outcome.stop));
    }
    let mut raw_text = String::new();
    for delta in outcome.deltas {
        if let ChatDelta::Part(part) = delta
            && part.kind == PartKind::Text
        {
            raw_text.push_str(&part.text);
            if raw_text.len() >= RECAP_MAX_OUTPUT_BYTES {
                break;
            }
        }
    }
    parse_generated_recap(&raw_text).ok_or_else(|| "invalid recap response".to_string())
}

#[cfg(test)]
mod tests;
