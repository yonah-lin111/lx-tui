//! 编辑缓冲提交、页持久化确认与自动保存复位。

use std::path::Path;

use crate::app::effects::Effect;
use crate::app::prompt::{PromptEditing, PromptPageId};
use crate::app::state::AppState;

use super::draft::open_save_draft;
use super::items::prompt_temp_editing;
use super::recent;

/// 提交编辑视图内容到绑定目标：条目立即更新并登记 DB 保存；草稿更新内存槽位。
pub fn commit_editing_prompt(state: &mut AppState) {
    let index = state.prompt_workspace();
    commit_prompt_buffer(state, index);
}

/// 提交指定工作区的编辑器缓冲（其为显示态时缓冲位于 `state.prompt`）。
///
/// 提交写入编辑目标的当前页；正式条目按（条目 id、页 id）粒度标记待保存并登记
/// 携带页 id 与修订号的保存效应。空白页正文同样落库（区分空白页与未编辑页）。
///
/// 粘贴提案未收束时先取消提案回滚占位：占位只是待改写的中转态，取消无法逆向写库，
/// 因此占位文本绝不落库；随后的提交只落粘贴前的正文，切换编辑目标/退出不丢编辑。
pub fn commit_prompt_buffer(state: &mut AppState, workspace: usize) {
    state.prompt.paste_cancel();
    state.prompt.text_paste_cancel();
    let Some(target) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.editing)
    else {
        return;
    };
    let content = state.prompt.text().to_string();
    let prompt = state.prompt.id();
    let revision = state.prompt.revision();
    let now = crate::platform::current_unix_secs();
    match target {
        PromptEditing::Temp(id) => {
            let Some(temp) = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.prompt_panel.temp_mut(id))
            else {
                return;
            };
            if let Some(page) = temp.active_page_mut() {
                page.content.clone_from(&content);
            }
        }
        PromptEditing::Item(id) => {
            let Some(cwd) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.cwd.clone())
            else {
                return;
            };
            let Some(page_id) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.prompt_panel.tree.item(id))
                .map(|item| item.active_page)
                .and_then(PromptPageId::saved)
            else {
                return;
            };
            let pending = state
                .workspaces
                .get(workspace)
                .is_some_and(|workspace| workspace.prompt_panel.page_pending(id, page_id));
            let recent_target = (workspace == state.prompt_workspace())
                .then(|| recent::current_recent_target(state))
                .flatten();
            {
                let Some(page) = state
                    .workspaces
                    .get_mut(workspace)
                    .and_then(|workspace| workspace.prompt_panel.tree.item_mut(id))
                    .and_then(|item| item.page_mut(PromptPageId::Saved(page_id)))
                else {
                    return;
                };
                if page.content == content && !pending {
                    // 内容与库中一致且无未确认保存：直接确认对应修订，避免空转自动保存。
                    state.prompt.confirm_saved(revision);
                    return;
                }
                page.content.clone_from(&content);
                page.touch(now);
            }
            if workspace == state.prompt_workspace() {
                recent::sync_prompt_recent_selection(state, recent_target);
            }
            if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
                workspace_state.prompt_panel.mark_page_pending(id, page_id);
            }
            state.effects.push(Effect::SavePromptContent {
                workspace: cwd,
                id,
                page_id,
                content,
                prompt,
                revision,
            });
        }
    }
}

/// 保存成功回执：仅当编辑器仍显示对应（条目、页）时按修订确认保存点。
///
/// 旧页保存完成不得把新页的保存点误标为已保存；页内容与待确认标记照常清除。
/// 同键停泊工作区绑定该页时，其缓冲随保存正文刷新（回报到达前已切走的工作区）。
pub fn mark_prompt_page_persisted(
    state: &mut AppState,
    id: i64,
    page_id: i64,
    content: &str,
    prompt: crate::tiling::PaneId,
    revision: u64,
) -> bool {
    // 本次保存的归属键：由持有该编辑器的显示态或停泊工作区决定；同键工作区共享正文，
    // 其停泊缓冲也须跟随刷新，否则切回后会拿旧正文覆盖已保存内容。
    let source_key = state
        .workspaces
        .iter()
        .find(|workspace| {
            workspace
                .prompt
                .as_ref()
                .is_some_and(|draft| draft.editor.id() == prompt)
        })
        .or_else(|| state.workspaces.get(state.prompt_workspace()))
        .and_then(|workspace| workspace.repo_key().map(Path::to_path_buf));
    let mut found = false;
    for workspace in &mut state.workspaces {
        if let Some(page) = workspace
            .prompt_panel
            .tree
            .item_mut(id)
            .and_then(|item| item.page_mut(PromptPageId::Saved(page_id)))
        {
            page.content = content.to_string();
            found = true;
        }
        workspace.prompt_panel.clear_page_pending(id, page_id);
        let sharing = workspace.repo_key() == source_key.as_deref()
            && workspace.prompt_panel.editing == Some(PromptEditing::Item(id))
            && workspace
                .prompt_panel
                .tree
                .item(id)
                .is_some_and(|item| item.active_page == PromptPageId::Saved(page_id));
        if sharing
            && let Some(draft) = workspace.prompt.as_mut()
            && draft.editor.is_saved()
            && draft.editor.text() != content
        {
            draft.editor.set_text(content);
            draft.editor.mark_saved();
        }
    }
    let editing_page = |workspace: &crate::app::state::Workspace| {
        workspace.prompt_panel.editing == Some(PromptEditing::Item(id))
            && workspace
                .prompt_panel
                .tree
                .item(id)
                .is_some_and(|item| item.active_page == PromptPageId::Saved(page_id))
    };
    if state.prompt.id() == prompt {
        let index = state.prompt_workspace();
        if state.workspaces.get(index).is_some_and(editing_page) {
            state.prompt.confirm_saved(revision);
        }
    } else {
        for workspace in &mut state.workspaces {
            let editor_matches = workspace
                .prompt
                .as_ref()
                .is_some_and(|draft| draft.editor.id() == prompt);
            if editor_matches
                && editing_page(workspace)
                && let Some(draft) = workspace.prompt.as_mut()
            {
                draft.editor.confirm_saved(revision);
            }
        }
    }
    found
}

/// 保存失败：重置对应编辑器的自动保存观察，等待下一轮静置重试。
pub fn reset_prompt_autosave(state: &mut AppState, prompt: crate::tiling::PaneId) -> bool {
    if state.prompt.id() == prompt {
        state.prompt.reset_autosave_watch();
        return true;
    }
    for workspace in &mut state.workspaces {
        if let Some(draft) = workspace.prompt.as_mut()
            && draft.editor.id() == prompt
        {
            draft.editor.reset_autosave_watch();
            return true;
        }
    }
    false
}

/// 立即保存（工具栏圆点）：条目提交落库；临时提示词提交缓冲后打开保存到文件夹选择器。
pub fn save_prompt_now(state: &mut AppState) {
    // 粘贴提案未收束：占位文本不得落库，也不改已保存态（面板收束后再保存）。
    if state.prompt.paste_proposal_open() {
        return;
    }
    if prompt_temp_editing(state) {
        commit_editing_prompt(state);
        open_save_draft(state);
    } else {
        state.prompt.mark_saved();
        commit_editing_prompt(state);
    }
}

/// Ctrl/Cmd+S 语义：内容为空（去空白）时不响应——空草稿不打开选择器、空条目不落库；
/// 非空时与工具栏圆点一致。
pub fn save_prompt_shortcut(state: &mut AppState) {
    if state.prompt.text().trim().is_empty() {
        return;
    }
    save_prompt_now(state);
}
