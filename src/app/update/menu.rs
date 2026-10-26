//! 右键上下文菜单与窗格分割动作。

use ratatui::layout::Direction;

use crate::app::effects::Effect;
use crate::app::overlay::{
    ConfirmClose, Menu, MenuCommand, Overlay, OverlayTarget, Rename, RenameTarget, TextInput,
};
use crate::app::state::{AppState, PaneView, tab_label};
use crate::app::trust::TrustTrigger;
use crate::layout::PaneId;

use super::pane::toggle_pane_view;
use super::prompt::{
    copy_prompt_content, copy_prompt_temp_content, delete_prompt_temp, edit_prompt_item,
    edit_prompt_temp, move_page_menu_selection, open_new_folder, open_new_prompt,
    open_prompt_rename,
};
use super::selection::clear_selection;
use super::workspace::{
    create_tab, create_terminal_in_workspace, open_confirm_switch_cwd, open_new_workspace_dialog,
    open_prompt_for_pane, open_prompt_for_workspace,
};
use super::worktree::open_worktree_dialog;

/// 打开工作区右键菜单；按「新建/打开 → 切换/导航 → 重命名 → 关闭」排序，仅剩一个工作区时不提供关闭项。
pub fn open_workspace_menu(state: &mut AppState, target: usize, anchor: (u16, u16)) {
    if target >= state.workspaces.len() {
        return;
    }
    // 打开模态时结束可能残留的滚动条拖拽与排序拖拽。
    state.workspace_scroll_drag = None;
    state.workspace_drag = None;
    state.workspace_dragging = false;
    let mut commands = vec![MenuCommand::NewTerminal];
    if state
        .workspaces
        .get(target)
        .is_some_and(|workspace| workspace.git.is_some())
    {
        commands.push(MenuCommand::OpenWorktree);
    }
    commands.push(MenuCommand::OpenPrompt);
    if state
        .workspaces
        .get(target)
        .is_some_and(|workspace| workspace.cwd.is_some())
    {
        commands.push(MenuCommand::CopyPath);
    }
    commands.push(MenuCommand::RenameWorkspace);
    if state.workspaces.len() > 1 {
        commands.push(MenuCommand::CloseWorkspace);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::Workspace(target),
        commands,
        selected: 0,
    }));
}

/// 打开工作区列表空白区域右键菜单：仅新建工作区；同时结束可能残留的滚动条拖拽与排序拖拽。
pub fn open_workspace_list_menu(state: &mut AppState, anchor: (u16, u16)) {
    state.workspace_scroll_drag = None;
    state.workspace_drag = None;
    state.workspace_dragging = false;
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::WorkspaceList,
        commands: vec![MenuCommand::NewWorkspace],
        selected: 0,
    }));
}

/// 打开标签右键菜单；仅剩一个标签时不提供关闭项。
pub fn open_tab_menu(state: &mut AppState, tab: usize, anchor: (u16, u16)) {
    let workspace_index = state.active_workspace;
    let Some(workspace) = state.workspaces.get(workspace_index) else {
        return;
    };
    if tab >= workspace.tabs.len() {
        return;
    }
    state.workspace_scroll_drag = None;
    let mut commands = vec![MenuCommand::NewTab, MenuCommand::RenameTab];
    if workspace.tabs.len() > 1 {
        commands.push(MenuCommand::CloseTab);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::Tab {
            workspace: workspace_index,
            tab,
        },
        commands,
        selected: 0,
    }));
}

/// 打开窗格右键菜单；按「新建/打开 → 切换/导航 → 重命名 → 关闭」排序，仅剩一个窗格时不提供关闭项。
pub fn open_pane_menu(state: &mut AppState, pane: PaneId, anchor: (u16, u16)) {
    let workspace = state.active_workspace;
    let tab = state.active_workspace().active_tab;
    let view = state.active_tab().pane(pane).map(|target| target.view);
    let Some(view) = view else {
        return;
    };
    state.workspace_scroll_drag = None;
    state.terminal_scroll_drag = None;
    let mut commands = vec![
        MenuCommand::SplitRight,
        MenuCommand::SplitDown,
        MenuCommand::OpenPrompt,
    ];
    commands.push(match view {
        PaneView::Lx => MenuCommand::SwitchToTerminal,
        PaneView::Terminal => MenuCommand::SwitchToLx,
    });
    // 仅工作区有固定路径时提供切回工作区路径。
    if state
        .workspaces
        .get(workspace)
        .is_some_and(|workspace| workspace.cwd.is_some())
    {
        commands.push(MenuCommand::SwitchToWorkspaceCwd);
    }
    // 仅目标窗格处于终端视图、已探测到有效终端路径，且与当前工作区路径不一致时提供同步工作区路径。
    if view == PaneView::Terminal
        && let Some(pane_target) = state.active_tab().pane(pane)
        && let Some(terminal_cwd) = pane_target.cwd.as_deref()
        && state
            .workspaces
            .get(workspace)
            .is_some_and(|ws| ws.cwd.as_deref() != Some(terminal_cwd))
    {
        commands.push(MenuCommand::SyncWorkspaceToTerminalCwd);
    }
    // 目标窗格或工作区具备有效路径时提供复制路径。
    if state.workspaces.get(workspace).is_some_and(|ws| {
        ws.cwd.is_some()
            || ws
                .tabs
                .get(tab)
                .and_then(|t| t.pane(pane))
                .is_some_and(|p| p.cwd.is_some())
    }) {
        commands.push(MenuCommand::CopyPath);
    }
    if state.active_tab().layout.pane_ids().len() > 1 {
        commands.push(MenuCommand::ClosePane);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::Pane {
            workspace,
            tab,
            pane,
        },
        commands,
        selected: 0,
    }));
}

/// 关闭当前浮层。
pub fn close_overlay(state: &mut AppState) {
    state.overlay = None;
}

/// 菜单高亮按步长循环移动；页码菜单按列表语义钳制移动。
pub fn move_menu_selection(state: &mut AppState, step: isize) {
    if matches!(state.overlay, Some(Overlay::PageMenu(_))) {
        move_page_menu_selection(state, step);
        return;
    }
    let Some(Overlay::Menu(menu)) = state.overlay.as_mut() else {
        return;
    };
    if menu.commands.is_empty() {
        return;
    }
    let len = menu.commands.len() as isize;
    menu.selected = (menu.selected as isize + step).rem_euclid(len) as usize;
    // 键盘移动同样显示高亮反馈。
    state.menu_hover = true;
}

/// 菜单悬停高亮；索引越界忽略；返回是否变化。
pub fn set_menu_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::Menu(menu)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= menu.commands.len() {
        return false;
    }
    let changed = menu.selected != index || !state.menu_hover;
    menu.selected = index;
    state.menu_hover = true;
    changed
}

/// 指针离开菜单容器：移除悬停高亮反馈；返回是否变化。
pub fn clear_menu_hover(state: &mut AppState) -> bool {
    if !state.menu_hover {
        return false;
    }
    state.menu_hover = false;
    true
}

/// 执行菜单当前项：新建立即生效；重命名打开输入浮层，关闭打开确认浮层。
pub fn activate_menu(state: &mut AppState) {
    let Some(Overlay::Menu(menu)) = state.overlay.take() else {
        return;
    };
    match (menu.commands.get(menu.selected), menu.target) {
        (Some(MenuCommand::NewTab), OverlayTarget::Tab { .. }) => create_tab(state),
        (Some(MenuCommand::NewWorkspace), OverlayTarget::WorkspaceList) => {
            open_new_workspace_dialog(state);
        }
        (Some(MenuCommand::NewTerminal), OverlayTarget::Workspace(target)) => {
            create_terminal_in_workspace(state, target);
        }
        (Some(MenuCommand::OpenPrompt), OverlayTarget::Workspace(target)) => {
            open_prompt_for_workspace(state, target);
        }
        (Some(MenuCommand::OpenPrompt), OverlayTarget::Pane { pane, .. }) => {
            open_prompt_for_pane(state, pane);
        }
        (
            Some(MenuCommand::SwitchToWorkspaceCwd),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            open_confirm_switch_cwd(state, workspace, tab, pane);
        }
        (
            Some(MenuCommand::SyncWorkspaceToTerminalCwd),
            OverlayTarget::Pane {
                workspace, pane, ..
            },
        ) => {
            // 同步触发前做信任检查：未信任弹信任浮层，信任后进入二次确认。
            // 仓库归属未知（root: None），未命中快速判定时由事件层后台解析。
            if let Some(path) = state
                .workspaces
                .get(workspace)
                .and_then(|ws| ws.tabs.iter().find_map(|t| t.pane(pane)))
                .and_then(|target| target.cwd.clone())
            {
                state.effects.push(Effect::CheckTrust {
                    path,
                    root: None,
                    trigger: TrustTrigger::SyncWorkspaceCwd { workspace, pane },
                });
            }
        }
        (Some(MenuCommand::RenameWorkspace), OverlayTarget::Workspace(target)) => {
            let Some(workspace) = state.workspaces.get(target) else {
                return;
            };
            state.overlay = Some(Overlay::Rename(Rename {
                target: RenameTarget::Workspace(target),
                input: TextInput::new(workspace.name.clone()),
            }));
        }
        (Some(MenuCommand::OpenWorktree), OverlayTarget::Workspace(target)) => {
            open_worktree_dialog(state, target);
        }
        (Some(MenuCommand::CloseWorkspace), OverlayTarget::Workspace(_)) => {
            state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
                target: menu.target,
            }));
        }
        (Some(MenuCommand::RenameTab), OverlayTarget::Tab { workspace, tab }) => {
            let Some(label) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.tabs.get(tab))
                .map(|tab_state| tab_label(tab, tab_state.name.as_deref()))
            else {
                return;
            };
            state.overlay = Some(Overlay::Rename(Rename {
                target: RenameTarget::Tab { workspace, tab },
                input: TextInput::new(label),
            }));
        }
        (Some(MenuCommand::CloseTab), OverlayTarget::Tab { .. }) => {
            state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
                target: menu.target,
            }));
        }
        (
            Some(MenuCommand::SplitRight),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            split_pane(state, workspace, tab, pane, Direction::Horizontal);
        }
        (
            Some(MenuCommand::SplitDown),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            split_pane(state, workspace, tab, pane, Direction::Vertical);
        }
        (
            Some(MenuCommand::SwitchToTerminal | MenuCommand::SwitchToLx),
            OverlayTarget::Pane { pane, .. },
        ) => {
            toggle_pane_view(state, pane);
        }
        (Some(MenuCommand::ClosePane), OverlayTarget::Pane { .. }) => {
            state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
                target: menu.target,
            }));
        }
        (Some(MenuCommand::NewPrompt), OverlayTarget::PromptFolder { folder_id, .. }) => {
            open_new_prompt(state, Some(folder_id));
        }
        (Some(MenuCommand::NewPrompt), OverlayTarget::PromptTree { .. }) => {
            open_new_prompt(state, None);
        }
        (
            Some(MenuCommand::NewFolder | MenuCommand::NewSubFolder),
            OverlayTarget::PromptFolder { folder_id, .. },
        ) => {
            open_new_folder(state, Some(folder_id));
        }
        (Some(MenuCommand::NewFolder), OverlayTarget::PromptTree { .. }) => {
            open_new_folder(state, None);
        }
        (
            Some(MenuCommand::RenameFolder),
            OverlayTarget::PromptFolder {
                workspace,
                folder_id,
            },
        ) => {
            open_prompt_rename(
                state,
                RenameTarget::PromptFolder {
                    workspace,
                    folder_id,
                },
            );
        }
        (Some(MenuCommand::DeleteFolder), OverlayTarget::PromptFolder { .. })
        | (Some(MenuCommand::DeletePrompt), OverlayTarget::PromptItem { .. }) => {
            state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
                target: menu.target,
            }));
        }
        (Some(MenuCommand::EditPrompt), OverlayTarget::PromptItem { workspace, item_id }) => {
            edit_prompt_item(state, workspace, item_id);
        }
        (Some(MenuCommand::RenamePrompt), OverlayTarget::PromptItem { workspace, item_id }) => {
            open_prompt_rename(state, RenameTarget::PromptItem { workspace, item_id });
        }
        (
            Some(MenuCommand::CopyPromptContent),
            OverlayTarget::PromptItem { workspace, item_id },
        ) => {
            copy_prompt_content(state, workspace, item_id);
        }
        (Some(MenuCommand::EditPrompt), OverlayTarget::TempPrompt { workspace, temp_id }) => {
            edit_prompt_temp(state, workspace, temp_id);
        }
        (
            Some(MenuCommand::CopyPromptContent),
            OverlayTarget::TempPrompt { workspace, temp_id },
        ) => {
            copy_prompt_temp_content(state, workspace, temp_id);
        }
        (Some(MenuCommand::CopyPath), OverlayTarget::Workspace(target)) => {
            if let Some(path) = state
                .workspaces
                .get(target)
                .and_then(|workspace| workspace.cwd.as_ref())
            {
                state.effects.push(Effect::CopyToClipboard {
                    text: path.display().to_string(),
                    anchor: None,
                });
            }
        }
        (
            Some(MenuCommand::CopyPath),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            let path = state.workspaces.get(workspace).and_then(|ws| {
                ws.tabs
                    .get(tab)
                    .and_then(|t| t.pane(pane))
                    .and_then(|p| p.cwd.clone())
                    .or_else(|| ws.cwd.clone())
            });
            if let Some(path) = path {
                state.effects.push(Effect::CopyToClipboard {
                    text: path.display().to_string(),
                    anchor: None,
                });
            }
        }
        (Some(MenuCommand::DeleteTemp), OverlayTarget::TempPrompt { workspace, temp_id }) => {
            delete_prompt_temp(state, workspace, temp_id);
        }
        _ => {}
    }
}

/// 在目标窗格处按方向分割并聚焦新窗格；新窗格继承源窗格视图。
pub(crate) fn split_pane(
    state: &mut AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
    direction: Direction,
) -> bool {
    let Some(tab_state) = state
        .workspaces
        .get_mut(workspace)
        .and_then(|workspace| workspace.tabs.get_mut(tab))
    else {
        return false;
    };
    if tab_state.split_pane(pane, direction).is_none() {
        return false;
    }
    super::focus::focus_panes(state);
    state.prompt.clear_panel();
    clear_selection(state);
    true
}
