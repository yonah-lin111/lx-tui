//! 菜单几何、标题解析与文案映射。

use ratatui::layout::Rect;

use crate::app::overlay::{Menu, MenuCommand, OverlayTarget};
use crate::app::state::{AppState, tab_label};
use crate::ui::text;
use crate::ui::widgets;

/// 菜单几何与文案；渲染与鼠标命中共用。
pub fn menu_layout(state: &AppState, screen: Rect, menu: &Menu) -> widgets::menu::MenuLayout {
    widgets::menu::layout(
        screen,
        menu.anchor,
        &menu_title(state, menu),
        &menu_labels(menu),
    )
}

/// 菜单边框标题：目标名优先，取不到时回退种类名。
pub(crate) fn menu_title(state: &AppState, menu: &Menu) -> String {
    match menu.target {
        OverlayTarget::Workspace(index) => state
            .workspaces
            .get(index)
            .map(|workspace| workspace.name.clone())
            .unwrap_or_else(|| text::MENU_TITLE_WORKSPACE.to_string()),
        OverlayTarget::Tab { workspace, tab } => state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.tabs.get(tab))
            .map(|tab_state| tab_label(tab, tab_state.name.as_deref()))
            .unwrap_or_else(|| text::MENU_TITLE_TAB.to_string()),
        OverlayTarget::Pane {
            workspace,
            tab,
            pane,
        } => state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.tabs.get(tab))
            .and_then(|tab_state| tab_state.pane(pane))
            .map(|pane_state| {
                text::pane_title(
                    pane,
                    pane_state.terminal.title(),
                    pane_state.cwd_label.as_deref(),
                )
            })
            .unwrap_or_else(|| text::MENU_TITLE_PANE.to_string()),
        OverlayTarget::PromptFolder {
            workspace,
            folder_id,
        } => prompt_folder_name(state, workspace, folder_id)
            .unwrap_or_else(|| text::MENU_TITLE_FOLDER.to_string()),
        OverlayTarget::PromptItem { workspace, item_id } => {
            prompt_item_name(state, workspace, item_id)
                .unwrap_or_else(|| text::MENU_TITLE_PROMPT.to_string())
        }
        OverlayTarget::TempPrompt { workspace, temp_id } => {
            prompt_temp_name(state, workspace, temp_id)
                .unwrap_or_else(|| text::MENU_TITLE_PROMPT.to_string())
        }
        OverlayTarget::PromptTree { .. } => text::MENU_TITLE_PROMPT_TREE.to_string(),
        OverlayTarget::PromptPage { .. } => text::MENU_TITLE_PAGES.to_string(),
        OverlayTarget::WorkspaceList => text::MENU_TITLE_WORKSPACES.to_string(),
    }
}

/// 菜单命令到文案的映射；文案集中在 `ui/text.rs`。
pub(crate) fn menu_labels(menu: &Menu) -> Vec<&'static str> {
    menu.commands
        .iter()
        .map(|command| match command {
            MenuCommand::NewTab => text::MENU_NEW_TAB,
            MenuCommand::NewTerminal => text::MENU_NEW_TERMINAL,
            MenuCommand::NewWorkspace => text::MENU_NEW_WORKSPACE,
            MenuCommand::RenameWorkspace => text::MENU_RENAME_WORKSPACE,
            MenuCommand::OpenWorktree => text::MENU_OPEN_WORKTREE,
            MenuCommand::OpenPrompt => text::MENU_OPEN_PROMPT,
            MenuCommand::CloseWorkspace => text::MENU_CLOSE_WORKSPACE,
            MenuCommand::RenameTab => text::MENU_RENAME_TAB,
            MenuCommand::CloseTab => text::MENU_CLOSE_TAB,
            MenuCommand::SplitRight => text::MENU_SPLIT_RIGHT,
            MenuCommand::SplitDown => text::MENU_SPLIT_DOWN,
            MenuCommand::SwitchToTerminal => text::MENU_SWITCH_TO_TERMINAL,
            MenuCommand::SwitchToLx => text::MENU_SWITCH_TO_LX,
            MenuCommand::SwitchToWorkspaceCwd => text::MENU_SWITCH_TO_WORKSPACE_CWD,
            MenuCommand::SyncWorkspaceToTerminalCwd => text::MENU_SYNC_WS_TO_TERMINAL_CWD,
            MenuCommand::ClosePane => text::MENU_CLOSE_PANE,
            MenuCommand::NewPrompt => text::MENU_PROMPT_NEW_PROMPT,
            MenuCommand::NewFolder => text::MENU_PROMPT_NEW_FOLDER,
            MenuCommand::NewSubFolder => text::MENU_PROMPT_NEW_SUBFOLDER,
            MenuCommand::RenameFolder | MenuCommand::RenamePrompt => text::MENU_PROMPT_RENAME,
            MenuCommand::DeleteFolder | MenuCommand::DeletePrompt | MenuCommand::DeleteTemp => {
                text::MENU_PROMPT_DELETE
            }
            MenuCommand::EditPrompt => text::MENU_PROMPT_EDIT,
            MenuCommand::CopyPromptContent => text::MENU_PROMPT_COPY,
            MenuCommand::CopyPath => text::MENU_COPY_PATH,
        })
        .collect()
}

/// 提示词页名（删除确认用）。
pub(crate) fn prompt_page_name(
    state: &AppState,
    workspace: usize,
    owner: crate::app::prompt::PromptEditing,
    page: crate::app::prompt::PromptPageId,
) -> Option<String> {
    let panel = &state.workspaces.get(workspace)?.prompt_panel;
    let pages = match owner {
        crate::app::prompt::PromptEditing::Item(id) => &panel.tree.item(id)?.pages,
        crate::app::prompt::PromptEditing::Temp(id) => &panel.temp(id)?.pages,
    };
    pages
        .iter()
        .find(|candidate| candidate.id == page)
        .map(|page| page.name.clone())
}

/// 提示词树文件夹名。
pub(crate) fn prompt_folder_name(
    state: &AppState,
    workspace: usize,
    folder_id: i64,
) -> Option<String> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.folder(folder_id))
        .map(|folder| folder.name.clone())
}

/// 提示词条目名。
pub(crate) fn prompt_item_name(state: &AppState, workspace: usize, item_id: i64) -> Option<String> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .map(|item| item.name.clone())
}

/// 临时提示词展示名（内容首行摘要）；空白内容返回 None。
pub(crate) fn prompt_temp_name(state: &AppState, workspace: usize, temp_id: u64) -> Option<String> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.temp(temp_id))
        .map(|temp| temp.summary().to_string())
        .filter(|name| !name.is_empty())
}
