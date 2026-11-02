//! 后台轻量流式请求：执行会话标题与模板块标题生成并回投 `AppEvent`。

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::app::effects::Effect;
use crate::app::prompt::{PromptEditing, PromptPageId};
use crate::app::state::{AppState, FrozenModel};
use crate::app::update;
use crate::chat::client;
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::chat::title::{self, TITLE_MAX_OUTPUT_BYTES, TITLE_TIMEOUT_SECS};
use crate::db::chat::ModelSnapshot;
use crate::db::service::Persistence;
use crate::event::AppEvent;

/// 会话自动标题与 Prompt 模板块 `/title` 生成完成事件载荷。
#[derive(Debug, Clone)]
pub enum TitleGeneratedEvent {
    /// `lx` 新会话后台自动生成标题完成。
    Session {
        workspace: PathBuf,
        session_id: String,
        initial_title: String,
        title: Option<String>,
    },
    /// Prompt 模板块 `/title` 后台生成标题完成。
    PromptBlock {
        request_id: u64,
        workspace_index: usize,
        editing: PromptEditing,
        page_id: PromptPageId,
        block_start_line: usize,
        original_title: Option<String>,
        title: Option<String>,
    },
}

/// 抽取并后台执行标题生成效应（不阻塞主对话流或事件循环）。
pub fn apply_title_effects(state: &mut AppState, sender: &mpsc::UnboundedSender<AppEvent>) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::GenerateSessionTitle { .. } | Effect::GeneratePromptBlockTitle { .. }
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
            Effect::GenerateSessionTitle { .. } | Effect::GeneratePromptBlockTitle { .. }
        )
    });

    for effect in effects {
        match effect {
            Effect::GenerateSessionTitle {
                workspace,
                session_id,
                initial_title,
                prompt,
                model,
            } => {
                let sender = sender.clone();
                tokio::spawn(async move {
                    let title = execute_title_stream(&prompt, &model).await;
                    let _ = sender.send(AppEvent::TitleGenerated(TitleGeneratedEvent::Session {
                        workspace,
                        session_id,
                        initial_title,
                        title,
                    }));
                });
            }
            Effect::GeneratePromptBlockTitle {
                request_id,
                workspace_index,
                editing,
                page_id,
                block_start_line,
                original_title,
                prompt,
                model,
            } => {
                let sender = sender.clone();
                tokio::spawn(async move {
                    let title = execute_title_stream(&prompt, &model).await;
                    let _ =
                        sender.send(AppEvent::TitleGenerated(TitleGeneratedEvent::PromptBlock {
                            request_id,
                            workspace_index,
                            editing,
                            page_id,
                            block_start_line,
                            original_title,
                            title,
                        }));
                });
            }
            _ => {}
        }
    }
    true
}

/// 统一分发会话与模板块标题生成完成事件。
pub fn handle_title_generated(
    state: &mut AppState,
    persistence: &Persistence,
    event: TitleGeneratedEvent,
) -> bool {
    match event {
        TitleGeneratedEvent::Session {
            workspace,
            session_id,
            initial_title,
            title,
        } => handle_session_title_generated(
            state,
            persistence,
            &workspace,
            &session_id,
            &initial_title,
            title.as_deref(),
        ),
        TitleGeneratedEvent::PromptBlock {
            request_id,
            workspace_index,
            editing,
            page_id,
            block_start_line,
            original_title,
            title,
        } => handle_prompt_block_title_generated(
            state,
            request_id,
            workspace_index,
            editing,
            page_id,
            block_start_line,
            original_title.as_deref(),
            title.as_deref(),
        ),
    }
}

/// 处理新会话后台自动标题生成完成事件（双重防覆盖校验后静默落库并更新 `/resume` 列表与顶边框胶囊）。
pub fn handle_session_title_generated(
    state: &mut AppState,
    persistence: &Persistence,
    workspace: &Path,
    session_id: &str,
    initial_title: &str,
    title: Option<&str>,
) -> bool {
    // 1. 内存级校验：若已不在 pending 集合中，说明用户期间手动改过名或已删除会话。
    if !update::take_pending_session_title(state, session_id) {
        return false;
    }
    let Some(new_title) = title.filter(|value| !value.trim().is_empty()) else {
        // 模型失败或拒答被过滤时，回退使用首轮干净预览标题更新窗格顶边框胶囊。
        if let Some(fallback) = title::parse_generated_title(initial_title) {
            return update::apply_generated_session_title(state, session_id, &fallback);
        }
        return false;
    };
    // 2. 数据库级校验：仅当当前标题仍等于首轮初始兜底标题时更新。
    if !persistence.rename_chat_session_if_title_matches(
        workspace,
        session_id,
        initial_title,
        new_title,
    ) {
        return false;
    }
    update::apply_generated_session_title(state, session_id, new_title)
}

/// 处理 Prompt 模板块 `/title` 生成完成事件。
#[allow(clippy::too_many_arguments)]
pub fn handle_prompt_block_title_generated(
    state: &mut AppState,
    request_id: u64,
    workspace_index: usize,
    editing: PromptEditing,
    page_id: PromptPageId,
    block_start_line: usize,
    original_title: Option<&str>,
    title: Option<&str>,
) -> bool {
    update::on_prompt_block_title_generated(
        state,
        request_id,
        workspace_index,
        editing,
        page_id,
        block_start_line,
        original_title,
        title,
    )
}

async fn execute_title_stream(prompt: &str, model: &FrozenModel) -> Option<String> {
    let snapshot = ModelSnapshot {
        provider: model.provider.clone(),
        model: model.model.clone(),
        variant: model.variant.clone(),
    };
    let chat_request = title::build_title_request(prompt, &snapshot);
    let (_cancel_sender, cancel) = client::uncancellable();
    let stream_fut = client::run_stream(
        &model.transport,
        &model.base_url,
        Some(model.api_key.as_str()),
        Duration::from_millis(model.idle_timeout_ms),
        chat_request,
        cancel,
        |_| {},
    );
    let outcome =
        match tokio::time::timeout(Duration::from_secs(TITLE_TIMEOUT_SECS), stream_fut).await {
            Ok(outcome) => outcome,
            Err(_) => {
                tracing::debug!(model = %model.model, "title generation timed out");
                return None;
            }
        };
    if outcome.stop != StopReason::Complete {
        tracing::debug!(model = %model.model, stop = ?outcome.stop, "title generation stopped early");
        return None;
    }
    let mut raw_text = String::new();
    for delta in outcome.deltas {
        if let ChatDelta::Part(part) = delta
            && part.kind == PartKind::Text
        {
            raw_text.push_str(&part.text);
            if raw_text.len() >= TITLE_MAX_OUTPUT_BYTES {
                break;
            }
        }
    }
    title::parse_generated_title(&raw_text)
}

#[cfg(test)]
mod tests;
