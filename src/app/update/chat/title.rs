//! 会话自动标题与 Prompt 模板块标题生成的状态调度、回填、回滚与防覆盖保护。
//!
//! 保持纯状态更新（零 IO）：网络调用与 DB 落库均通过 `Effect` 与事件层完成。

use std::path::Path;
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::markdown;
use crate::app::prompt::{PromptEditing, PromptPageId, PromptTitleAction};
use crate::app::state::{AppState, PendingPromptTitle};
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
    let language = state.models.title_language();
    let Some(prompt) = crate::chat::title::session_title_prompt_for(source_text, language) else {
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

/// 用户手动重命名（`/rename` 或 `/resume` 编辑弹窗）或删除会话时作废在途自动标题生成。
pub fn cancel_session_title(state: &mut AppState, session_id: &str) {
    state.pending_session_titles.remove(session_id);
}

/// 取出并消费指定会话的在途自动标题标记；若已被用户手动改名作废则返回 false。
pub fn take_pending_session_title(state: &mut AppState, session_id: &str) -> bool {
    state.pending_session_titles.remove(session_id)
}

/// 自动标题落库成功后，静默刷新所有打开中的 `/resume` 历史面板与所属窗格胶囊。
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
///
/// 触发阶段仅在内存编辑器中展示 `{title: Generating title...}` 占位符，不立即将占位符写入 SQLite；
/// 待生成结果返回并通过 `request_id` 校验后再一次性持久化最终标题。
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
            let language = state.models.title_language();
            let Some(prompt) =
                crate::chat::title::template_block_title_prompt_for(&content, language)
            else {
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
            let workspace_cwd = state
                .workspaces
                .get(workspace_index)
                .and_then(|ws| ws.cwd.clone());
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

            // 若同一工作区同一页面同一块已有在途请求，作废旧请求并继承最初的 original_title。
            let mut preserved_original = original_title;
            let superseded_ids: Vec<u64> = state
                .pending_prompt_titles
                .iter()
                .filter(|(_, p)| {
                    p.workspace_index == workspace_index
                        && p.workspace_cwd == workspace_cwd
                        && p.editing == editing
                        && p.page_id == page_id
                        && p.block_start_line == block_start_line
                })
                .map(|(id, _)| *id)
                .collect();
            for old_id in superseded_ids {
                if let Some(old) = state.pending_prompt_titles.remove(&old_id)
                    && preserved_original.as_deref() == Some(text::PROMPT_TITLE_GENERATING)
                {
                    preserved_original = old.original_title;
                }
            }

            state.prompt_title_request_counter = state.prompt_title_request_counter.wrapping_add(1);
            let request_id = state.prompt_title_request_counter;
            state.pending_prompt_titles.insert(
                request_id,
                PendingPromptTitle {
                    request_id,
                    workspace_cwd,
                    workspace_index,
                    editing,
                    page_id,
                    block_start_line,
                    original_title: preserved_original.clone(),
                },
            );
            state.effects.push(Effect::GeneratePromptBlockTitle {
                request_id,
                workspace_index,
                editing,
                page_id,
                block_start_line,
                original_title: preserved_original,
                prompt,
                model,
            });
            true
        }
    }
}

/// Prompt 模板块 `/title` 后台生成结果回填：校验 `request_id` 有效后写回新标题或恢复原 `{title: ...}`。
#[allow(clippy::too_many_arguments)]
pub fn on_prompt_block_title_generated(
    state: &mut AppState,
    request_id: u64,
    workspace_index: usize,
    editing: PromptEditing,
    page_id: PromptPageId,
    block_start_line: usize,
    original_title: Option<&str>,
    generated_title: Option<&str>,
) -> bool {
    let pending = if request_id != 0 {
        let Some(entry) = state.pending_prompt_titles.remove(&request_id) else {
            return false;
        };
        Some(entry)
    } else {
        None
    };

    let Some(resolved_ws) = resolve_target_workspace(state, workspace_index, pending.as_ref())
    else {
        return false;
    };
    let effective_original = pending
        .as_ref()
        .and_then(|p| p.original_title.as_deref())
        .or(original_title);
    let target_title = generated_title.or(effective_original);

    // 只要当前显示工作区的 `editing` 与 `active_page` 仍指向目标页（无论处于 Editor 还是 Pages 视图），
    // `state.prompt` 就是该页的权威内存缓冲，必须同步更新。
    let is_displayed_page = state.prompt_workspace() == resolved_ws
        && workspace_targets_page(state, resolved_ws, editing, page_id);

    let mut applied = false;
    if is_displayed_page {
        if markdown::block_title_value_at_line(state.prompt.text(), block_start_line).as_deref()
            == Some(text::PROMPT_TITLE_GENERATING)
        {
            applied |= state.prompt.set_block_title(block_start_line, target_title);
            crate::app::update::commit_prompt_buffer(state, resolved_ws);
        }
    } else {
        // 若目标工作区处于后台停泊态且其停泊缓冲 `ws.prompt` 仍绑定该页，同步刷新停泊缓冲。
        if workspace_targets_page(state, resolved_ws, editing, page_id)
            && let Some(parked) = state
                .workspaces
                .get_mut(resolved_ws)
                .and_then(|ws| ws.prompt.as_mut())
            && markdown::block_title_value_at_line(parked.editor.text(), block_start_line)
                .as_deref()
                == Some(text::PROMPT_TITLE_GENERATING)
        {
            applied |= parked
                .editor
                .set_block_title(block_start_line, target_title);
        }
        applied |= update_background_page_title(
            state,
            resolved_ws,
            editing,
            page_id,
            block_start_line,
            target_title,
        );
    }

    if applied && generated_title.is_none() {
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
    }
    applied
}

fn resolve_target_workspace(
    state: &AppState,
    workspace_index: usize,
    pending: Option<&PendingPromptTitle>,
) -> Option<usize> {
    if let Some(pending) = pending
        && let Some(expected_cwd) = pending.workspace_cwd.as_ref()
    {
        if state
            .workspaces
            .get(workspace_index)
            .is_some_and(|ws| ws.cwd.as_ref() == Some(expected_cwd))
        {
            return Some(workspace_index);
        }
        return state
            .workspaces
            .iter()
            .position(|ws| ws.cwd.as_ref() == Some(expected_cwd));
    }
    (workspace_index < state.workspaces.len()).then_some(workspace_index)
}

fn workspace_targets_page(
    state: &AppState,
    workspace_index: usize,
    editing: PromptEditing,
    page_id: PromptPageId,
) -> bool {
    state.workspaces.get(workspace_index).is_some_and(|ws| {
        ws.prompt_panel.editing == Some(editing)
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
    })
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
            let Some(saved_page_id) = page_id.saved() else {
                // 未保存的新页（`PromptPageId::Temp`）仅更新内存页树，不推送 DB 保存效应。
                return true;
            };
            let Some(cwd) = state
                .workspaces
                .get(workspace_index)
                .and_then(|ws| ws.cwd.clone())
            else {
                return true;
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
mod tests;
