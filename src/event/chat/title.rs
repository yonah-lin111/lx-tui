//! 后台轻量流式请求：执行会话标题与模板块标题生成并回投 `AppEvent`。

use std::path::Path;
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
                    let _ = sender.send(AppEvent::SessionTitleGenerated {
                        workspace,
                        session_id,
                        initial_title,
                        title,
                    });
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
                    let _ = sender.send(AppEvent::PromptBlockTitleGenerated {
                        request_id,
                        workspace_index,
                        editing,
                        page_id,
                        block_start_line,
                        original_title,
                        title,
                    });
                });
            }
            _ => {}
        }
    }
    true
}

/// 处理新会话后台自动标题生成完成事件（双重防覆盖校验后静默落库并更新 `/resume` 列表）。
pub fn handle_session_title_generated(
    state: &mut AppState,
    persistence: &Persistence,
    workspace: &Path,
    session_id: &str,
    initial_title: &str,
    title: Option<&str>,
) -> bool {
    // 1. 内存级校验：若已不在 pending 集合中，说明用户期间手动改过名。
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
pub fn handle_prompt_block_title_generated(
    state: &mut AppState,
    workspace_index: usize,
    editing: PromptEditing,
    page_id: PromptPageId,
    block_start_line: usize,
    original_title: Option<&str>,
    title: Option<&str>,
) -> bool {
    update::on_prompt_block_title_generated(
        state,
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
    let outcome = tokio::time::timeout(Duration::from_secs(TITLE_TIMEOUT_SECS), stream_fut)
        .await
        .ok()?;
    if outcome.stop != StopReason::Complete {
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
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::chat::ModelSnapshot;

    #[test]
    fn handle_session_title_generated_updates_db_and_respects_manual_rename() {
        let db = Database::open_in_memory().expect("in memory db");
        let workspace = std::path::PathBuf::from("/tmp/lx-title-test-ws");
        let model = ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        };
        let committed = db
            .begin_turn(&workspace, None, "First long message from user", &model)
            .expect("begin turn");
        let persistence = Persistence::with_database(db);
        let mut state = AppState::demo();
        state
            .pending_session_titles
            .insert(committed.session_id.clone());

        // 自动标题正常返回：更新数据库标题
        handle_session_title_generated(
            &mut state,
            &persistence,
            &workspace,
            &committed.session_id,
            "First long message from user",
            Some("Concise Generated Title"),
        );
        let sessions = persistence
            .database()
            .expect("db")
            .list_chat_sessions(&workspace, 0, 10)
            .expect("list");
        assert_eq!(sessions[0].title, "Concise Generated Title");

        // 若已被消费或被手动重命名，再次回投不得覆盖
        persistence
            .database()
            .expect("db")
            .rename_chat_session(&workspace, &committed.session_id, "User Manual Title")
            .expect("manual rename");
        state
            .pending_session_titles
            .insert(committed.session_id.clone());
        handle_session_title_generated(
            &mut state,
            &persistence,
            &workspace,
            &committed.session_id,
            "First long message from user",
            Some("Stale Auto Title"),
        );
        let sessions = persistence
            .database()
            .expect("db")
            .list_chat_sessions(&workspace, 0, 10)
            .expect("list");
        assert_eq!(sessions[0].title, "User Manual Title");
    }
}
