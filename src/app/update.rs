//! 行为到状态的转换；几何信息由事件循环传入，保证逻辑纯且可测。

mod chat;
mod dialog;
mod editor;
mod interaction;
mod menu;
mod pane;
mod prompt;
mod referenced;
mod resume_edit;
mod selection;
mod trust;
mod workspace;
mod workspace_list;
mod worktree;

pub use chat::*;
pub use dialog::*;
pub use editor::*;
pub use interaction::*;
pub use menu::*;
pub use pane::*;
pub use prompt::*;
pub use referenced::*;
pub use resume_edit::*;
pub use selection::*;
pub use trust::*;
pub use workspace::*;
pub use workspace_list::*;
pub use worktree::{
    apply_git_refresh, apply_worktree_list, apply_worktree_open_key, commit_worktree_open,
    open_worktree_dialog, request_git_refresh, set_worktree_open_selection, take_git_requests,
    toggle_workspace_group, worktree_candidates,
};

use super::actions::Action;
use super::state::AppState;

/// 应用行为。
pub fn apply(action: Action, state: &mut AppState) {
    match action {
        Action::Quit => {
            // 退出前提交编辑中的提示词内容；效应由事件层在收尾时落库。
            commit_editing_prompt(state);
            state.should_quit = true;
        }
        Action::ToggleSidebar => state.sidebar_collapsed = !state.sidebar_collapsed,
        Action::TogglePrompt => {
            state.prompt_collapsed = !state.prompt_collapsed;
            if state.prompt_collapsed {
                state.prompt_focused = false;
                clear_selection(state);
                state.prompt.clear_panel();
                cancel_page_rename(state);
            }
        }
        Action::TogglePanes => state.panes_collapsed = !state.panes_collapsed,
        Action::ToggleAgents => state.agents_collapsed = !state.agents_collapsed,
    }
}

#[cfg(test)]
mod tests;
