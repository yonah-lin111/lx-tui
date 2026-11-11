//! 提示词条目与临时提示词生命周期：编辑、保存、复制与删除。

use crate::app::effects::Effect;
use crate::app::prompt::{PromptEditing, PromptMode, PromptPageId, PromptRow};
use crate::app::state::AppState;

use super::save::commit_editing_prompt;
use super::tree_view::{prompt_tree_visible_rows, select_prompt_item};
use super::{recent, tree_view, workspace_cwd};

/// 进入指定条目的编辑视图并聚焦面板（服务在新建后调用；工作区须为显示中）。
///
/// 打开上次所在的页；记录页 id 失效时回退并修复为第一页（登记持久化效应）。
pub fn edit_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    if workspace != state.prompt_workspace() || !select_prompt_item(state, workspace, item_id) {
        return false;
    }
    let Some((content, repaired)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .map(|item| {
            let index = item.active_index();
            let repaired = item.pages.get(index).map(|page| page.id) != Some(item.active_page);
            (
                item.pages.get(index).map(|page| page.content.clone()),
                repaired,
            )
        })
    else {
        return false;
    };
    let Some(content) = content else {
        return false;
    };
    if repaired {
        let Some(page_id) = state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
            .and_then(|item| item.pages.first())
            .and_then(|page| page.id.saved())
        else {
            return false;
        };
        if let Some(workspace_state) = state.workspaces.get_mut(workspace)
            && let Some(item) = workspace_state.prompt_panel.tree.item_mut(item_id)
            && let Some(page) = item.pages.first()
        {
            item.active_page = page.id;
        }
        if let Some(cwd) = state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.cwd.clone())
        {
            state.effects.push(Effect::SetActivePage {
                workspace: cwd,
                id: item_id,
                page_id,
            });
        }
    }
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(PromptEditing::Item(item_id));
    let page = workspace_state
        .prompt_panel
        .tree
        .item(item_id)
        .map(|item| item.active_page);
    load_editor_target(
        state,
        workspace,
        PromptEditing::Item(item_id),
        page,
        &content,
    );
    true
}

/// 打开指定临时提示词的编辑视图并聚焦面板；内容只存内存，恢复该临时提示词的当前页。
pub fn edit_prompt_temp(state: &mut AppState, workspace: usize, temp_id: u64) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    let Some((content, page)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.temp(temp_id))
        .and_then(|temp| {
            temp.active_page()
                .map(|page| (page.content.clone(), page.id))
        })
    else {
        return false;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(PromptEditing::Temp(temp_id));
    load_editor_target(
        state,
        workspace,
        PromptEditing::Temp(temp_id),
        Some(page),
        &content,
    );
    true
}

/// 载入编辑目标正文到编辑器缓冲：暂存的目标、页与正文均一致时恢复完整历史与光标，
/// 否则按正文重建（历史清空）。无论是否恢复都消费暂存，避免陈旧历史误用于新目标。
pub(super) fn load_editor_target(
    state: &mut AppState,
    workspace: usize,
    editing: PromptEditing,
    page: Option<PromptPageId>,
    content: &str,
) {
    let restored = state
        .workspaces
        .get_mut(workspace)
        .and_then(|workspace| workspace.prompt_panel.take_editor_stash())
        .filter(|stash| {
            Some(stash.page) == page && stash.editing == editing && stash.history.text() == content
        })
        .map(|stash| stash.history);
    match restored {
        Some(history) => state.prompt.restore_history(history),
        None => state.prompt.set_text(content),
    }
    state.prompt_collapsed = false;
    super::super::focus_prompt(state);
}

/// 打开选中临时提示词行的编辑视图。
pub(super) fn edit_prompt_temp_row(state: &mut AppState, workspace: usize, row: PromptRow) -> bool {
    let Some(temp_id) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.temp_of_row(row))
        .map(|temp| temp.id)
    else {
        return false;
    };
    edit_prompt_temp(state, workspace, temp_id)
}

/// 工具栏 `[tmp]`：提交当前编辑内容并新建临时提示词编辑视图（不入库）。
pub fn open_temp_prompt(state: &mut AppState) -> bool {
    commit_editing_prompt(state);
    let index = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get_mut(index) else {
        return false;
    };
    // 不变量兜底：空白项至多一条，残留空白项先回收（default temp 由新建项接替）。
    workspace_state.prompt_panel.remove_blank_temps();
    // 新建目标与旧暂存无关：丢弃，避免陈旧历史被同名目标误恢复。
    workspace_state.prompt_panel.clear_editor_stash();
    let id = workspace_state.prompt_panel.new_temp();
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(PromptEditing::Temp(id));
    state.prompt.set_text("");
    state.prompt_collapsed = false;
    super::super::focus_prompt(state);
    true
}

/// 复制提示词当前页正文到剪贴板（经效应出箱由事件层写入系统剪贴板，Toast 锚定右侧面板）。
pub fn copy_prompt_content(state: &mut AppState, workspace: usize, item_id: i64) {
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .and_then(|item| item.active_page())
        .map(|page| page.content.clone())
    else {
        return;
    };
    let anchor = Some(state.prompt.id());
    state.effects.push(Effect::CopyToClipboard {
        text: content,
        anchor,
    });
}

/// 复制临时提示词当前页正文到剪贴板；不入库、无确认，Toast 锚定右侧面板。
pub fn copy_prompt_temp_content(state: &mut AppState, workspace: usize, temp_id: u64) {
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.temp(temp_id))
        .and_then(|temp| temp.active_page())
        .map(|page| page.content.clone())
    else {
        return;
    };
    let anchor = Some(state.prompt.id());
    state.effects.push(Effect::CopyToClipboard {
        text: content,
        anchor,
    });
}

/// 删除提示词条目：内存移除、必要时退出编辑视图，并登记删除。
pub fn delete_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let visible = prompt_tree_visible_rows(state);
    let recent_target = (workspace == state.prompt_workspace())
        .then(|| recent::current_recent_target(state))
        .flatten();
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    if workspace_state.prompt_panel.tree.item(item_id).is_none() {
        return;
    }
    workspace_state.prompt_panel.tree.delete_item(item_id);
    if workspace_state.prompt_panel.editing == Some(PromptEditing::Item(item_id)) {
        workspace_state.prompt_panel.editing = None;
        workspace_state.prompt_panel.mode = PromptMode::Tree;
    }
    workspace_state.prompt_panel.clamp_selection(visible);
    if workspace == state.prompt_workspace() {
        state.prompt.clear_panel();
        recent::sync_prompt_recent_selection(state, recent_target);
        state.prompt_resize_pending = true;
    }
    state.effects.push(Effect::DeletePrompt {
        workspace: cwd,
        id: item_id,
    });
}

/// 删除文件夹（级联）：内存移除、必要时退出编辑视图，并登记删除。
pub fn delete_prompt_folder(state: &mut AppState, workspace: usize, folder_id: i64) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let visible = prompt_tree_visible_rows(state);
    let recent_target = (workspace == state.prompt_workspace())
        .then(|| recent::current_recent_target(state))
        .flatten();
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    if workspace_state
        .prompt_panel
        .tree
        .folder(folder_id)
        .is_none()
    {
        return;
    }
    let editing_removed = workspace_state
        .prompt_panel
        .editing
        .and_then(|editing| match editing {
            PromptEditing::Item(id) => Some(id),
            PromptEditing::Temp(_) => None,
        })
        .is_some_and(|id| {
            workspace_state
                .prompt_panel
                .tree
                .item(id)
                .is_none_or(|item| {
                    tree_view::removed_with_folder(
                        &workspace_state.prompt_panel.tree,
                        folder_id,
                        item.folder_id,
                    )
                })
        });
    workspace_state.prompt_panel.tree.delete_folder(folder_id);
    if editing_removed {
        workspace_state.prompt_panel.editing = None;
        workspace_state.prompt_panel.mode = PromptMode::Tree;
    }
    workspace_state.prompt_panel.clamp_selection(visible);
    if workspace == state.prompt_workspace() {
        state.prompt.clear_panel();
        recent::sync_prompt_recent_selection(state, recent_target);
        state.prompt_resize_pending = true;
    }
    state.effects.push(Effect::DeleteFolder {
        workspace: cwd,
        id: folder_id,
    });
}

/// 删除临时提示词：仅内存移除并钳制选中；不入库、无确认浮层。
pub fn delete_prompt_temp(state: &mut AppState, workspace: usize, temp_id: u64) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    if !workspace_state.prompt_panel.remove_temp(temp_id) {
        return false;
    }
    workspace_state.prompt_panel.clamp_selection(visible);
    true
}

/// 显示中编辑视图是否绑定临时提示词。
pub fn prompt_temp_editing(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| {
            matches!(workspace.prompt_panel.editing, Some(PromptEditing::Temp(_)))
        })
}
