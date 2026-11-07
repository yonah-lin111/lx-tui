//! 浮层渲染：右键菜单、重命名与关闭确认；几何与命中供事件循环复用。

mod connect;
pub mod dialog;
pub mod menu;
mod referenced;
pub mod save_draft;
mod worktree;

pub use connect::{
    CONNECT_HEIGHT, CONNECT_WIDTH, ConnectLayout, connect_layout, connect_row_at, render_connect,
};
pub use dialog::*;
pub use menu::*;
pub use referenced::{
    ReferencedInputButton, ReferencedMenuLayout, ReferencedSearchLayout,
    referenced_input_button_at, referenced_input_shell, referenced_menu_layout,
    referenced_menu_row_at, referenced_search_copy_at, referenced_search_item_at,
    referenced_search_layout,
};
pub use save_draft::*;
pub use worktree::{
    WorktreeDialogButton, worktree_dialog_button_at, worktree_dialog_entry_at,
    worktree_dialog_max_rows, worktree_dialog_shell, worktree_dialog_visible_start,
};

#[cfg(test)]
pub(crate) use dialog::{
    BUTTON_GAP, NEW_NODE_BUTTONS, NEW_WORKSPACE_BUTTON_ROW, NEW_WORKSPACE_BUTTONS,
    RENAME_BUTTON_ROW, TRUST_BUTTON_ROW, TRUST_BUTTONS,
};
#[cfg(test)]
pub(crate) use save_draft::SAVE_DRAFT_BUTTONS;

use referenced::{
    render_referenced_input, render_referenced_menu, render_referenced_menu_footer,
    render_referenced_search, render_referenced_search_footer,
};
use worktree::render_worktree_open;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::overlay::Overlay;
use crate::app::state::AppState;
use crate::ui::widgets;

/// 渲染当前浮层；返回需要同步的硬件光标位置（重命名、新建工作区或删除确认口令输入）。
pub fn render(frame: &mut Frame<'_>, screen: Rect, state: &AppState) -> Option<(u16, u16)> {
    match state.overlay.as_ref()? {
        Overlay::Menu(menu_state) => {
            let labels = menu::menu_labels(menu_state);
            let title = menu::menu_title(state, menu_state);
            let layout = widgets::menu::layout(screen, menu_state.anchor, &title, &labels);
            let selected = state.menu_hover.then_some(menu_state.selected);
            let confirming = menu_state.confirming.then_some(menu_state.selected);
            widgets::menu::render_with_confirm(
                frame, &layout, &title, &labels, selected, confirming,
            );
            None
        }
        Overlay::Rename(rename) => dialog::render_rename(frame, screen, rename),
        Overlay::ResumeSessionEdit(edit) => dialog::render_resume_session_edit(frame, screen, edit),
        Overlay::NewWorkspace(new_ws) => dialog::render_new_workspace(
            frame,
            screen,
            new_ws,
            state.workspace_history.new_workspace_confirming,
        ),
        Overlay::ConfirmClose(confirm) => dialog::render_confirm(frame, screen, state, confirm),
        Overlay::WorktreeOpen(dialog) => render_worktree_open(frame, screen, dialog),
        Overlay::TrustWorkspace(trust) => {
            dialog::render_trust(frame, screen, trust);
            None
        }
        Overlay::NewNode(node) => dialog::render_new_node(frame, screen, node),
        Overlay::SaveDraft(dialog) => save_draft::render_save_draft(frame, screen, dialog),
        Overlay::ReferencedFolderMenu(menu) => {
            render_referenced_menu(frame, screen, menu);
            render_referenced_menu_footer(frame, screen, menu);
            None
        }
        Overlay::ReferencedFolderInput(input) => render_referenced_input(frame, screen, input),
        Overlay::ReferencedFolderSearch(search) => {
            let cursor = render_referenced_search(frame, screen, search);
            render_referenced_search_footer(frame, screen, search);
            cursor
        }
        Overlay::ReferencedFolderDelete(delete) => {
            dialog::render_referenced_delete(frame, screen, delete);
            None
        }
        Overlay::Connect(panel) => render_connect(frame, screen, panel),
    }
}

#[cfg(test)]
#[path = "overlay/connect_tests.rs"]
mod connect_tests;
#[cfg(test)]
mod tests;
