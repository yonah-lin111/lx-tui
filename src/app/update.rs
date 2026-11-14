//! 行为到状态的转换；几何信息由事件循环传入，保证逻辑纯且可测。

mod agent_title;
mod chat;
mod dialog;
mod editor;
mod focus;
mod interaction;
mod menu;
mod pane;
mod prompt;
mod quit;
mod referenced;
mod resume_edit;
mod right_panel;
mod selection;
mod todo;
mod trust;
mod workspace;
mod workspace_history;
mod workspace_list;
mod worktree;

pub use agent_title::*;
pub use chat::*;
pub use dialog::*;
pub use editor::*;
pub use focus::*;
pub use interaction::*;
pub use menu::*;
pub use pane::*;
pub use prompt::*;
pub use quit::{confirm_quit, request_quit};
pub use referenced::*;
pub use resume_edit::*;
pub use right_panel::*;
pub use selection::*;
pub use todo::*;
pub use trust::*;
pub use workspace::*;
pub use workspace_history::*;
pub use workspace_list::*;
pub use worktree::{
    GitRefresh, apply_git_refresh, apply_worktree_list, apply_worktree_open_key,
    commit_worktree_open, open_worktree_dialog, request_git_refresh, retry_git_queries,
    set_worktree_open_selection, take_git_requests, toggle_workspace_group, worktree_candidates,
};

use super::actions::Action;
use super::state::{AppState, FocusedContainer};

/// 应用行为。
pub fn apply(action: Action, state: &mut AppState) {
    match action {
        Action::RequestQuit => quit::request_quit(state),
        Action::ToggleSidebar => state.sidebar_collapsed = !state.sidebar_collapsed,
        Action::TogglePrompt => {
            state.prompt_collapsed = !state.prompt_collapsed;
            if state.prompt_collapsed {
                commit_todo_edit(state);
                state.set_focus(FocusedContainer::Panes);
                clear_selection(state);
                state.prompt.clear_panel();
                state.prompt_drag = None;
                state.todo_drag = None;
                state.todo_scroll_drag = None;
                state.todo_row_click = None;
                state.todo_input_drag = false;
                state.todo_hover = None;
            }
        }
        Action::TogglePanes => state.panes_collapsed = !state.panes_collapsed,
        Action::ToggleAgents => state.agents_collapsed = !state.agents_collapsed,
    }
}

#[cfg(test)]
mod tests;
