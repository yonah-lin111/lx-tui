//! 会话自动标题与 Prompt 模板块标题生成的状态调度、回填、回滚与防覆盖保护。
//!
//! 保持纯状态更新（零 IO）：网络调用与 DB 落库均通过 `Effect` 与事件层完成。

use std::path::Path;
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::markdown;
use crate::app::prompt::{PromptEditing, PromptMode, PromptPageId, PromptTitleAction};
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::db::chat::TurnCommitted;
use crate::layout::PaneId;
use crate::ui::text;

/// 在新会话首轮提交成功后调度异步标题生成（对齐 codex `maybe_start_thread_title_update`）。
pub fn maybe_schedule_session_title(
    state: &mut AppState,
    pane: PaneId,
    committed: &TurnCommitted,
    frozen_draft: &str,
) {
    if committed.turn_index != 0 {
        return;
    }
    let workspace = state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.active_workspace().repo_key().map(Path::to_path_buf));
    let Some(workspace) = workspace else {
        return;
    };
    let fallback_selection = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.effective_selection(&state.models));
    let Some(model) = state
        .models
        .resolve_session_title_model(fallback_selection.as_ref())
    else {
        return;
    };
    let display_text =
        crate::chat::strip_references_for_display(frozen_draft, &committed.attachments);
    let source_text = if display_text.trim().is_empty() {
        frozen_draft
    } else {
        display_text.as_str()
    };
    let Some(prompt) = crate::chat::title::session_title_prompt(source_text) else {
        return;
    };
    let initial_title =
        crate::db::chat::initial_session_title_for_meta(frozen_draft, &committed.attachments);
    state
        .pending_session_titles
        .insert(committed.session_id.clone());
    state.effects.push(Effect::GenerateSessionTitle {
        workspace,
        session_id: committed.session_id.clone(),
        initial_title,
        prompt,
        model,
    });
}

/// 用户手动重命名（`/rename` 或 `/resume` 编辑弹窗）时作废在途自动标题生成。
pub fn cancel_session_title(state: &mut AppState, session_id: &str) {
    state.pending_session_titles.remove(session_id);
}

/// 取出并消费指定会话的在途自动标题标记；若已被用户手动改名作废则返回 false。
pub fn take_pending_session_title(state: &mut AppState, session_id: &str) -> bool {
    state.pending_session_titles.remove(session_id)
}

/// 自动标题落库成功后，静默刷新所有打开中的 `/resume` 历史面板。
pub fn apply_generated_session_title(
    state: &mut AppState,
    session_id: &str,
    new_title: &str,
) -> bool {
    let panes = state.all_pane_ids();
    let mut changed = false;
    for pane in panes {
        changed |= crate::app::update::on_session_renamed(state, pane, session_id, new_title);
    }
    changed
}

/// 消费 `Prompt` 在确认 `/title` 时登记的标题动作。
pub fn flush_prompt_title_action(state: &mut AppState) -> bool {
    let Some(action) = state.prompt.take_title_action() else {
        return false;
    };
    let anchor = Some(state.prompt.id());
    match action {
        PromptTitleAction::EmptyBlock => {
            crate::app::update::show_toast_at(
                state,
                anchor,
                Toast::new(
                    ToastKind::Warning,
                    text::PROMPT_TITLE_EMPTY_BLOCK,
                    anchor,
                    Instant::now(),
                ),
            );
            true
        }
        PromptTitleAction::Generate {
            block_start_line,
            original_title,
            content,
        } => {
            let Some(model) = state.models.resolve_prompt_title_model() else {
                // 未配置标题模型且无默认模型：立即回滚标题行并弹出英文警告。
                state
                    .prompt
                    .set_block_title(block_start_line, original_title.as_deref());
                crate::app::update::show_toast_at(
                    state,
                    anchor,
                    Toast::new(
                        ToastKind::Warning,
                        text::PROMPT_TITLE_NO_MODEL,
                        anchor,
                        Instant::now(),
                    ),
                );
                return true;
            };
            let Some(prompt) = crate::chat::title::template_block_title_prompt(&content) else {
                state
                    .prompt
                    .set_block_title(block_start_line, original_title.as_deref());
                crate::app::update::show_toast_at(
                    state,
                    anchor,
                    Toast::new(
                        ToastKind::Warning,
                        text::PROMPT_TITLE_EMPTY_BLOCK,
                        anchor,
                        Instant::now(),
                    ),
                );
                return true;
            };
            let workspace_index = state.prompt_workspace();
            let editing = state
                .workspaces
                .get(workspace_index)
                .and_then(|ws| ws.prompt_panel.editing)
                .unwrap_or(PromptEditing::Temp(0));
            let page_id = state
                .workspaces
                .get(workspace_index)
                .and_then(|ws| match editing {
                    PromptEditing::Item(id) => {
                        ws.prompt_panel.tree.item(id).map(|item| item.active_page)
                    }
                    PromptEditing::Temp(id) => {
                        ws.prompt_panel.temp(id).map(|temp| temp.active_page)
                    }
                })
                .unwrap_or(PromptPageId::Temp(0));
            crate::app::update::commit_editing_prompt(state);
            state.prompt_title_request_counter = state.prompt_title_request_counter.wrapping_add(1);
            let request_id = state.prompt_title_request_counter;
            state.effects.push(Effect::GeneratePromptBlockTitle {
                request_id,
                workspace_index,
                editing,
                page_id,
                block_start_line,
                original_title,
                prompt,
                model,
            });
            true
        }
    }
}

/// Prompt 模板块 `/title` 后台生成结果回填：成功写回新标题，失败恢复原 `{title: ...}` 并弹 Toast。
pub fn on_prompt_block_title_generated(
    state: &mut AppState,
    workspace_index: usize,
    editing: PromptEditing,
    page_id: PromptPageId,
    block_start_line: usize,
    original_title: Option<&str>,
    generated_title: Option<&str>,
) -> bool {
    let target_title = generated_title.or(original_title);
    let is_active_editor = state.prompt_workspace() == workspace_index
        && state.workspaces.get(workspace_index).is_some_and(|ws| {
            ws.prompt_panel.mode == PromptMode::Editor
                && ws.prompt_panel.editing == Some(editing)
                && match editing {
                    PromptEditing::Item(id) => ws
                        .prompt_panel
                        .tree
                        .item(id)
                        .is_some_and(|item| item.active_page == page_id),
                    PromptEditing::Temp(id) => ws
                        .prompt_panel
                        .temp(id)
                        .is_some_and(|temp| temp.active_page == page_id),
                }
        });

    let mut changed = false;
    if is_active_editor {
        if markdown::block_title_value_at_line(state.prompt.text(), block_start_line).as_deref()
            == Some(text::PROMPT_TITLE_GENERATING)
        {
            changed |= state.prompt.set_block_title(block_start_line, target_title);
            crate::app::update::commit_prompt_buffer(state, workspace_index);
        }
    } else {
        changed |= update_background_page_title(
            state,
            workspace_index,
            editing,
            page_id,
            block_start_line,
            target_title,
        );
    }

    if generated_title.is_none() {
        let anchor = Some(state.prompt.id());
        crate::app::update::show_toast_at(
            state,
            anchor,
            Toast::new(
                ToastKind::Warning,
                text::PROMPT_TITLE_FAILED,
                anchor,
                Instant::now(),
            ),
        );
        changed = true;
    }
    changed
}

fn update_background_page_title(
    state: &mut AppState,
    workspace_index: usize,
    editing: PromptEditing,
    page_id: PromptPageId,
    block_start_line: usize,
    target_title: Option<&str>,
) -> bool {
    let prompt_pane = state.prompt.id();
    let revision = state.prompt.revision();
    match editing {
        PromptEditing::Temp(id) => {
            let Some(page) = state
                .workspaces
                .get_mut(workspace_index)
                .and_then(|ws| ws.prompt_panel.temp_mut(id))
                .and_then(|temp| temp.page_mut(page_id))
            else {
                return false;
            };
            if markdown::block_title_value_at_line(&page.content, block_start_line).as_deref()
                != Some(text::PROMPT_TITLE_GENERATING)
            {
                return false;
            }
            let Some((range, replacement)) =
                markdown::set_block_title_edit(&page.content, block_start_line, target_title)
            else {
                return false;
            };
            page.content.replace_range(range, &replacement);
            true
        }
        PromptEditing::Item(id) => {
            let Some(cwd) = state
                .workspaces
                .get(workspace_index)
                .and_then(|ws| ws.cwd.clone())
            else {
                return false;
            };
            let Some(saved_page_id) = page_id.saved() else {
                return false;
            };
            let updated_content = {
                let Some(page) = state
                    .workspaces
                    .get_mut(workspace_index)
                    .and_then(|ws| ws.prompt_panel.tree.item_mut(id))
                    .and_then(|item| item.page_mut(page_id))
                else {
                    return false;
                };
                if markdown::block_title_value_at_line(&page.content, block_start_line).as_deref()
                    != Some(text::PROMPT_TITLE_GENERATING)
                {
                    return false;
                }
                let Some((range, replacement)) =
                    markdown::set_block_title_edit(&page.content, block_start_line, target_title)
                else {
                    return false;
                };
                page.content.replace_range(range, &replacement);
                page.content.clone()
            };
            if let Some(ws) = state.workspaces.get_mut(workspace_index) {
                ws.prompt_panel.mark_page_pending(id, saved_page_id);
            }
            state.effects.push(Effect::SavePromptContent {
                workspace: cwd,
                id,
                page_id: saved_page_id,
                content: updated_content,
                prompt: prompt_pane,
                revision,
            });
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::ModelsSnapshot;
    use crate::config::ai::{
        AiSettings, ModelConfig, ModelSelection, ProviderConfig, ProviderOptions, ProviderTransport,
    };
    use std::collections::BTreeMap;

    fn sample_models(with_title_model: bool) -> ModelsSnapshot {
        let mut models = BTreeMap::new();
        models.insert(
            "chat-main".to_string(),
            ModelConfig {
                id: "chat-main".to_string(),
                name: "Chat Main".to_string(),
                transport: None,
                limit: None,
                modalities: None,
                variants: BTreeMap::new(),
                variant: None,
                pricing: None,
                extra: serde_json::Map::new(),
            },
        );
        models.insert(
            "title-fast".to_string(),
            ModelConfig {
                id: "title-fast".to_string(),
                name: "Title Fast".to_string(),
                transport: None,
                limit: None,
                modalities: None,
                variants: BTreeMap::new(),
                variant: None,
                pricing: None,
                extra: serde_json::Map::new(),
            },
        );
        let mut providers = BTreeMap::new();
        providers.insert(
            "p".to_string(),
            ProviderConfig {
                id: "p".to_string(),
                kind: ProviderTransport::OpenaiCompatible,
                name: "Provider".to_string(),
                options: ProviderOptions {
                    api_key: Some("test-key".to_string()),
                    base_url: "https://api.example.invalid/v1".to_string(),
                },
                models,
                extra: serde_json::Map::new(),
            },
        );
        ModelsSnapshot {
            settings: AiSettings {
                enabled_providers: vec!["p".to_string()],
                providers,
                default_model: Some(ModelSelection {
                    provider: "p".to_string(),
                    model: "chat-main".to_string(),
                    variant: None,
                }),
                title_summary: with_title_model.then(|| ModelSelection {
                    provider: "p".to_string(),
                    model: "title-fast".to_string(),
                    variant: None,
                }),
                stream_idle_timeout_ms: None,
                web_search: Default::default(),
                extra: serde_json::Map::new(),
            },
        }
    }

    #[test]
    fn first_turn_schedules_session_title_and_manual_rename_cancels_it() {
        let mut state = AppState::demo();
        state.models = sample_models(true);
        let pane = state.active_tab().layout.focus();
        state.pane_mut_anywhere(pane).expect("pane").lx.input = "Implement session title".into();
        crate::app::update::try_send(&mut state, pane);
        let generation = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
            crate::app::state::RequestState::Committing { generation, .. } => *generation,
            _ => panic!("expected committing"),
        };
        let committed = TurnCommitted {
            session_id: "sess-title-1".into(),
            user_message_id: "u1".into(),
            assistant_message_id: "a1".into(),
            turn_index: 0,
            user_sequence: 0,
            assistant_sequence: 1,
            revision: 1,
            created_at_ms: 1000,
            attachments: Vec::new(),
        };
        assert!(crate::app::update::on_turn_committed(
            &mut state, pane, generation, &committed
        ));
        let title_effect = state.effects.iter().find_map(|effect| match effect {
            Effect::GenerateSessionTitle {
                session_id,
                initial_title,
                model,
                ..
            } => Some((
                session_id.clone(),
                initial_title.clone(),
                model.model.clone(),
            )),
            _ => None,
        });
        assert_eq!(
            title_effect,
            Some((
                "sess-title-1".to_string(),
                "Implement session title".to_string(),
                "title-fast".to_string()
            )),
            "首轮提交应使用配置的 titleSummary 模型入队 GenerateSessionTitle"
        );
        assert!(state.pending_session_titles.contains("sess-title-1"));

        // 手动 /rename 立即作废在途自动标题生成
        assert!(crate::app::update::rename_session(
            &mut state,
            pane,
            "sess-title-1",
            "Manual Name"
        ));
        assert!(
            !take_pending_session_title(&mut state, "sess-title-1"),
            "手动重命名后自动标题已被作废"
        );
    }

    #[test]
    fn prompt_block_title_rolls_back_when_no_model_or_when_generation_fails() {
        let mut state = AppState::demo();
        let initial = "&&& addTemplate --start\n{title: Old}\n\n- Description: Build feature\n/title\n&&& addTemplate --end";
        state.prompt.set_text(initial);
        let cursor = initial.find("/title").expect("slash") + "/title".len();
        state.prompt.set_cursor_offset(cursor);

        // 1. 未配置任何模型：立即回滚为 Old 并弹 Toast
        assert!(state.prompt.slash_confirm());
        assert!(flush_prompt_title_action(&mut state));
        assert!(
            state.prompt.text().contains("{title: Old}"),
            "无模型时应回滚标题行"
        );
        assert_eq!(
            state.toast.as_ref().map(|t| t.message.as_str()),
            Some(text::PROMPT_TITLE_NO_MODEL)
        );

        // 2. 配置默认模型后（无 titleSummary 回退 defaultModel）：生成成功与失败
        state.models = sample_models(false);
        state.prompt.set_text(initial);
        state.prompt.set_cursor_offset(cursor);
        assert!(state.prompt.slash_confirm());
        assert!(flush_prompt_title_action(&mut state));
        assert!(state.prompt.text().contains("{title: Generating title...}"));

        let ws = state.prompt_workspace();
        assert!(on_prompt_block_title_generated(
            &mut state,
            ws,
            PromptEditing::Temp(0),
            PromptPageId::Temp(0),
            0,
            Some("Old"),
            Some("Build feature title"),
        ));
        assert!(
            state.prompt.text().contains("{title: Build feature title}"),
            "生成成功应写入新标题"
        );
    }
}
