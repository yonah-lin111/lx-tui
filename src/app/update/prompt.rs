//! 提示词树行为：视图模式切换、选中导航、右键菜单与编辑提交。
//!
//! DB 读写不在此层执行：变更登记为 `Effect` 出箱，由持久化服务执行并回报。

use crate::app::effects::Effect;
use crate::app::overlay::{
    Menu, MenuCommand, Overlay, OverlayTarget, Rename, RenameTarget, TextInput,
};
use crate::app::prompt::{
    PromptFolder, PromptItem, PromptItemStatus, PromptMode, PromptRow, PromptRowKind, PromptTree,
};
use crate::app::state::AppState;
use crate::db::{FolderRecord, PromptRecord};
use crate::ui::text;

/// 提示词树内容区可见行数；取编辑器的同步尺寸（同一面板几何）。
pub fn prompt_tree_visible_rows(state: &AppState) -> usize {
    usize::from(state.prompt.size().1.max(1))
}

/// 显示中的工作区是否处于提示词树视图。
pub fn prompt_tree_mode(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| workspace.prompt_panel.mode == PromptMode::Tree)
}

/// 用 DB 记录整体替换指定工作区的树；保持已有折叠状态。
pub fn apply_prompt_tree(
    state: &mut AppState,
    workspace: usize,
    folders: Vec<FolderRecord>,
    prompts: Vec<PromptRecord>,
) -> bool {
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let folders: Vec<PromptFolder> = folders.into_iter().map(PromptFolder::from).collect();
    let prompts: Vec<PromptItem> = prompts.into_iter().map(PromptItem::from).collect();
    workspace_state.prompt_panel.tree.replace(folders, prompts);
    workspace_state.prompt_panel.loaded = true;
    workspace_state.prompt_panel.clamp_selection(1);
    true
}

/// 切换树视图：提交编辑内容并清空编辑绑定。
pub fn show_prompt_tree(state: &mut AppState) {
    commit_editing_prompt(state);
    let index = state.prompt_workspace();
    if let Some(workspace) = state.workspaces.get_mut(index) {
        workspace.prompt_panel.mode = PromptMode::Tree;
        workspace.prompt_panel.editing = None;
    }
    state.prompt.clear_panel();
    state.selection = None;
}

/// 工具栏模式按钮：树视图进入选中条目编辑，编辑视图切回树。
pub fn toggle_prompt_panel_mode(state: &mut AppState) {
    if prompt_tree_mode(state) {
        edit_selected_prompt(state);
    } else {
        show_prompt_tree(state);
    }
}

/// 提交编辑视图内容到绑定条目：内存立即更新，内容变化时登记 DB 保存。
pub fn commit_editing_prompt(state: &mut AppState) {
    let index = state.prompt_workspace();
    commit_prompt_buffer(state, index);
}

/// 提交指定工作区的编辑器缓冲（其为显示态时缓冲位于 `state.prompt`）。
pub fn commit_prompt_buffer(state: &mut AppState, workspace: usize) {
    let Some(workspace_state) = state.workspaces.get(workspace) else {
        return;
    };
    let Some(id) = workspace_state.prompt_panel.editing else {
        return;
    };
    let Some(cwd) = workspace_state.cwd.clone() else {
        return;
    };
    let content = state.prompt.text().to_string();
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    let Some(item) = workspace_state.prompt_panel.tree.item_mut(id) else {
        return;
    };
    if item.content == content {
        return;
    }
    item.content.clone_from(&content);
    state.effects.push(Effect::SavePromptContent {
        workspace: cwd,
        id,
        content,
    });
}

/// 树上移/下移选中行；`visible` 为可见行数，选中始终保持在视口内。
pub fn move_prompt_selection(state: &mut AppState, delta: isize, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let len = workspace.prompt_panel.rows().len();
    if len == 0 {
        return false;
    }
    let current = workspace.prompt_panel.selected;
    let next = (current as isize + delta).clamp(0, len as isize - 1) as usize;
    if next == current {
        return false;
    }
    workspace.prompt_panel.selected = next;
    scroll_selection_into_view(&mut workspace.prompt_panel, visible);
    true
}

/// 选中指定可见行（鼠标命中）；索引越界忽略。
pub fn select_prompt_row(state: &mut AppState, row: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    if row >= workspace.prompt_panel.rows().len() || workspace.prompt_panel.selected == row {
        return false;
    }
    workspace.prompt_panel.selected = row;
    true
}

/// 激活选中行：文件夹切换折叠，条目进入编辑视图。
pub fn activate_prompt_row(state: &mut AppState) -> bool {
    let index = state.prompt_workspace();
    let Some(row) = selected_row(state, index) else {
        return false;
    };
    match row.kind {
        PromptRowKind::Folder => toggle_prompt_folder(state, index, row),
        PromptRowKind::Item => edit_prompt_row(state, index, row),
    }
}

/// 折叠选中文件夹（Left）；选中条目或已折叠时 no-op。
pub fn collapse_prompt_row(state: &mut AppState) -> bool {
    set_selected_folder_collapsed(state, true)
}

/// 展开选中文件夹（Right）；选中条目或已展开时 no-op。
pub fn expand_prompt_row(state: &mut AppState) -> bool {
    set_selected_folder_collapsed(state, false)
}

/// 鼠标点击行：选中；文件夹行同时切换折叠。
pub fn click_prompt_row(state: &mut AppState, row: usize) -> bool {
    let changed = select_prompt_row(state, row);
    let index = state.prompt_workspace();
    let Some(row_data) = state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.prompt_panel.rows().get(row).copied())
    else {
        return changed;
    };
    if row_data.kind == PromptRowKind::Folder {
        return toggle_prompt_folder(state, index, row_data) || changed;
    }
    changed
}

/// 从指定工作区树中按 id 选中文件夹（新建后由服务调用）。
pub fn select_prompt_folder(state: &mut AppState, workspace: usize, folder_id: i64) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let rows = workspace_state.prompt_panel.rows();
    let position = rows.iter().position(|row| {
        row.kind == PromptRowKind::Folder
            && workspace_state
                .prompt_panel
                .tree
                .folder_of_row(*row)
                .is_some_and(|folder| folder.id == folder_id)
    });
    let Some(position) = position else {
        return false;
    };
    workspace_state.prompt_panel.selected = position;
    scroll_selection_into_view(&mut workspace_state.prompt_panel, visible);
    true
}

/// 进入指定条目的编辑视图并聚焦面板（服务在新建后调用；工作区须为显示中）。
pub fn edit_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    if workspace != state.prompt_workspace() || !select_prompt_item(state, workspace, item_id) {
        return false;
    }
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .map(|item| item.content.clone())
    else {
        return false;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(item_id);
    state.prompt.set_text(&content);
    state.prompt_collapsed = false;
    super::focus_prompt(state);
    true
}

/// 打开选中条目的编辑视图（`[edit]` 按钮 / 树视图回车）。
pub fn edit_selected_prompt(state: &mut AppState) -> bool {
    let index = state.prompt_workspace();
    let Some(item_id) = state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.prompt_panel.selected_prompt_id())
    else {
        return false;
    };
    edit_prompt_item(state, index, item_id)
}

/// 滚动树视口；越界钳制；返回是否变化。
pub fn scroll_prompt_tree(state: &mut AppState, direction: isize, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let max = workspace
        .prompt_panel
        .rows()
        .len()
        .saturating_sub(visible.max(1));
    let target = (workspace.prompt_panel.scroll.min(max) as isize).saturating_add(direction);
    let offset = target.clamp(0, max as isize) as usize;
    if offset == workspace.prompt_panel.scroll {
        return false;
    }
    workspace.prompt_panel.scroll = offset;
    true
}

/// 直接设置树滚动的视口偏移（滚动条点击/拖拽）；越界钳制；返回是否变化。
pub fn set_prompt_tree_scroll(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let max = workspace
        .prompt_panel
        .rows()
        .len()
        .saturating_sub(visible.max(1));
    let offset = offset.min(max);
    if offset == workspace.prompt_panel.scroll {
        return false;
    }
    workspace.prompt_panel.scroll = offset;
    true
}

/// 几何变化后钳制树选中的可见性；返回是否变化。
pub fn clamp_prompt_tree_scroll(state: &mut AppState, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let before = (
        workspace.prompt_panel.selected,
        workspace.prompt_panel.scroll,
    );
    scroll_selection_into_view(&mut workspace.prompt_panel, visible);
    (
        workspace.prompt_panel.selected,
        workspace.prompt_panel.scroll,
    ) != before
}

/// 打开提示词条目右键菜单：编辑、状态切换、重命名、复制与删除。
pub fn open_prompt_item_menu(state: &mut AppState, item_id: i64, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let Some(item) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
    else {
        return;
    };
    let commands = vec![
        MenuCommand::EditPrompt,
        MenuCommand::ChangePromptStatus(item.status.next()),
        MenuCommand::RenamePrompt,
        MenuCommand::CopyPromptContent,
        MenuCommand::DeletePrompt,
    ];
    select_prompt_item(state, workspace, item_id);
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptItem { workspace, item_id },
        commands,
        selected: 0,
    }));
}

/// 打开文件夹右键菜单：新建提示词、新建子文件夹（仅根层）、重命名与删除。
pub fn open_prompt_folder_menu(state: &mut AppState, folder_id: i64, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let Some(folder) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.folder(folder_id))
    else {
        return;
    };
    let mut commands = vec![MenuCommand::NewPrompt];
    if folder.parent_id.is_none() {
        commands.push(MenuCommand::NewSubFolder);
    }
    commands.push(MenuCommand::RenameFolder);
    commands.push(MenuCommand::DeleteFolder);
    select_prompt_folder(state, workspace, folder_id);
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptFolder {
            workspace,
            folder_id,
        },
        commands,
        selected: 0,
    }));
}

/// 打开树空白区域菜单：新建提示词与新建文件夹。
pub fn open_prompt_tree_menu(state: &mut AppState, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptTree { workspace },
        commands: vec![MenuCommand::NewPrompt, MenuCommand::NewFolder],
        selected: 0,
    }));
}

/// 新建提示词：登记创建效应（默认名由文案层提供）；服务创建后进入编辑视图。
pub fn create_prompt(state: &mut AppState, folder_id: Option<i64>) {
    let Some(cwd) = displayed_workspace_cwd(state) else {
        return;
    };
    state.effects.push(Effect::CreatePrompt {
        workspace: cwd,
        folder_id,
        name: text::PROMPT_DEFAULT_NAME.to_string(),
    });
}

/// 新建文件夹：登记创建效应；服务创建后选中新行。
pub fn create_folder(state: &mut AppState, parent_id: Option<i64>) {
    let Some(cwd) = displayed_workspace_cwd(state) else {
        return;
    };
    state.effects.push(Effect::CreateFolder {
        workspace: cwd,
        parent_id,
        name: text::PROMPT_FOLDER_DEFAULT_NAME.to_string(),
    });
}

/// 打开文件夹/条目的重命名浮层（预填当前名）。
pub fn open_prompt_rename(state: &mut AppState, target: RenameTarget) {
    let workspace = state.prompt_workspace();
    let name = state.workspaces.get(workspace).and_then(|workspace| {
        let panel = &workspace.prompt_panel;
        match target {
            RenameTarget::PromptFolder { folder_id, .. } => panel
                .tree
                .folder(folder_id)
                .map(|folder| folder.name.clone()),
            RenameTarget::PromptItem { item_id, .. } => {
                panel.tree.item(item_id).map(|item| item.name.clone())
            }
            _ => None,
        }
    });
    let Some(name) = name else {
        return;
    };
    state.overlay = Some(Overlay::Rename(Rename {
        target,
        input: TextInput::new(name),
    }));
}

/// 提交提示词树重命名；空名拒绝（浮层保持打开）。
pub fn commit_prompt_rename(state: &mut AppState, target: RenameTarget, name: String) {
    let workspace = state.prompt_workspace();
    let Some(cwd) = displayed_workspace_cwd(state) else {
        return;
    };
    match target {
        RenameTarget::PromptFolder { folder_id, .. } => {
            let renamed = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.prompt_panel.tree.folder_mut(folder_id))
                .map(|folder| folder.name.clone_from(&name))
                .is_some();
            if renamed {
                state.effects.push(Effect::RenameFolder {
                    workspace: cwd,
                    id: folder_id,
                    name,
                });
            }
        }
        RenameTarget::PromptItem { item_id, .. } => {
            let renamed = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.prompt_panel.tree.item_mut(item_id))
                .map(|item| item.name.clone_from(&name))
                .is_some();
            if renamed {
                state.effects.push(Effect::RenamePrompt {
                    workspace: cwd,
                    id: item_id,
                    name,
                });
            }
        }
        _ => {}
    }
}

/// 删除提示词条目：内存移除、必要时退出编辑视图，并登记删除。
pub fn delete_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    if workspace_state.prompt_panel.tree.item(item_id).is_none() {
        return;
    }
    workspace_state.prompt_panel.tree.delete_item(item_id);
    if workspace_state.prompt_panel.editing == Some(item_id) {
        workspace_state.prompt_panel.editing = None;
        workspace_state.prompt_panel.mode = PromptMode::Tree;
    }
    workspace_state.prompt_panel.clamp_selection(1);
    if workspace == state.prompt_workspace() {
        state.prompt.clear_panel();
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
    let editing_removed = workspace_state.prompt_panel.editing.is_some_and(|id| {
        workspace_state
            .prompt_panel
            .tree
            .item(id)
            .is_none_or(|item| {
                removed_with_folder(
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
    workspace_state.prompt_panel.clamp_selection(1);
    if workspace == state.prompt_workspace() {
        state.prompt.clear_panel();
    }
    state.effects.push(Effect::DeleteFolder {
        workspace: cwd,
        id: folder_id,
    });
}

/// 切换提示词状态，登记状态更新。
pub fn change_prompt_status(
    state: &mut AppState,
    workspace: usize,
    item_id: i64,
    status: PromptItemStatus,
) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    let Some(item) = workspace_state.prompt_panel.tree.item_mut(item_id) else {
        return;
    };
    if item.status == status {
        return;
    }
    item.status = status;
    state.effects.push(Effect::SetPromptStatus {
        workspace: cwd,
        id: item_id,
        status,
    });
}

/// 复制提示词正文到剪贴板（经效应出箱由事件层写入系统剪贴板）。
pub fn copy_prompt_content(state: &mut AppState, workspace: usize, item_id: i64) {
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .map(|item| item.content.clone())
    else {
        return;
    };
    state.effects.push(Effect::CopyToClipboard(content));
}

/// 显示中工作区的 cwd。
fn displayed_workspace_cwd(state: &AppState) -> Option<std::path::PathBuf> {
    workspace_cwd(state, state.prompt_workspace())
}

fn workspace_cwd(state: &AppState, workspace: usize) -> Option<std::path::PathBuf> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.cwd.clone())
}

/// 指定工作区的选中行。
fn selected_row(state: &AppState, workspace: usize) -> Option<PromptRow> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.selected_row())
}

/// 选中指定工作区树中的条目行。
fn select_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let rows = workspace_state.prompt_panel.rows();
    let position = rows.iter().position(|row| {
        row.kind == PromptRowKind::Item
            && workspace_state
                .prompt_panel
                .tree
                .item_of_row(*row)
                .is_some_and(|item| item.id == item_id)
    });
    let Some(position) = position else {
        return false;
    };
    workspace_state.prompt_panel.selected = position;
    scroll_selection_into_view(&mut workspace_state.prompt_panel, visible);
    true
}

/// 切换指定可见行文件夹的折叠状态。
fn toggle_prompt_folder(state: &mut AppState, workspace: usize, row: PromptRow) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let Some(folder) = workspace_state.prompt_panel.tree.folder_of_row(row) else {
        return false;
    };
    let id = folder.id;
    workspace_state.prompt_panel.tree.toggle_folder(id);
    workspace_state.prompt_panel.clamp_selection(visible.max(1));
    true
}

/// 进入选中条目编辑视图。
fn edit_prompt_row(state: &mut AppState, workspace: usize, row: PromptRow) -> bool {
    let Some(item_id) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item_of_row(row))
        .map(|item| item.id)
    else {
        return false;
    };
    edit_prompt_item(state, workspace, item_id)
}

/// 设置选中文件夹行的折叠态；行非文件夹或状态未变返回 false。
fn set_selected_folder_collapsed(state: &mut AppState, collapsed: bool) -> bool {
    let index = state.prompt_workspace();
    let Some(row) = selected_row(state, index) else {
        return false;
    };
    if row.kind != PromptRowKind::Folder {
        return false;
    }
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(index) else {
        return false;
    };
    let Some(folder) = workspace_state.prompt_panel.tree.folder_of_row(row) else {
        return false;
    };
    if folder.collapsed == collapsed {
        return false;
    }
    let id = folder.id;
    workspace_state.prompt_panel.tree.toggle_folder(id);
    workspace_state.prompt_panel.clamp_selection(visible.max(1));
    true
}

/// 条目是否处于被删除文件夹的子树内。
fn removed_with_folder(tree: &PromptTree, folder_id: i64, item_folder: Option<i64>) -> bool {
    let mut current = item_folder;
    while let Some(id) = current {
        if id == folder_id {
            return true;
        }
        current = tree.folder(id).and_then(|folder| folder.parent_id);
    }
    false
}

/// 保证选中行在可见范围内。
fn scroll_selection_into_view(panel: &mut crate::app::prompt::PromptPanel, visible: usize) {
    let visible = visible.max(1);
    let rows = panel.rows().len();
    let selected = panel.selected.min(rows.saturating_sub(1));
    panel.selected = selected;
    let max = rows.saturating_sub(visible);
    let mut scroll = panel.scroll.min(max);
    if selected < scroll {
        scroll = selected;
    } else if selected >= scroll + visible {
        scroll = selected + 1 - visible;
    }
    panel.scroll = scroll.min(max);
}

#[cfg(test)]
mod tests;
