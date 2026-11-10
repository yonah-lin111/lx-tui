//! 提示词树右键菜单、重命名与删除确认。

use crate::app::effects::Effect;
use crate::app::overlay::{
    Menu, MenuCommand, Overlay, OverlayTarget, Rename, RenameTarget, TextInput,
};
use crate::app::state::AppState;

use super::displayed_workspace_cwd;

/// 打开提示词条目右键菜单：编辑、重命名、复制与删除。
///
/// 右键不改动列表选中行（对齐工作区列表：右键只挂菜单，不切换当前项）；
/// 菜单命令按 target 里的 item_id 生效。
pub fn open_prompt_item_menu(state: &mut AppState, item_id: i64, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    if state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .is_none()
    {
        return;
    }
    let commands = vec![
        MenuCommand::EditPrompt,
        MenuCommand::RenamePrompt,
        MenuCommand::CopyPromptContent,
        MenuCommand::DeletePrompt,
    ];
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptItem { workspace, item_id },
        commands,
        selected: 0,
        confirming: false,
    }));
}

/// 打开临时提示词右键菜单：编辑、复制与删除（删除仅内存、立即生效，无确认浮层）。
///
/// 同条目菜单：右键不改动列表选中行。
pub fn open_prompt_temp_menu(state: &mut AppState, temp_id: u64, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let known = state
        .workspaces
        .get(workspace)
        .is_some_and(|workspace| workspace.prompt_panel.temp(temp_id).is_some());
    if !known {
        return;
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::TempPrompt { workspace, temp_id },
        commands: vec![
            MenuCommand::EditPrompt,
            MenuCommand::CopyPromptContent,
            MenuCommand::DeleteTemp,
        ],
        selected: 0,
        confirming: false,
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
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptFolder {
            workspace,
            folder_id,
        },
        commands,
        selected: 0,
        confirming: false,
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
        confirming: false,
    }));
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
