//! 浮层渲染：右键菜单、重命名与关闭确认；几何与命中供事件循环复用。

pub mod dialog;
pub mod menu;
mod page_menu;
pub mod save_draft;
mod ui_gallery;
mod worktree;

pub use dialog::*;
pub use menu::*;
pub use page_menu::{PageMenuLayout, page_menu_item_at, page_menu_layout, page_menu_visible_pages};
pub use save_draft::*;
pub use ui_gallery::{UiGalleryLayout, ui_gallery_layout, ui_gallery_row_at, ui_gallery_shell};
pub use worktree::{
    WorktreeDialogButton, worktree_dialog_button_at, worktree_dialog_entry_at,
    worktree_dialog_max_rows, worktree_dialog_shell, worktree_dialog_visible_start,
};

#[cfg(test)]
pub(crate) use dialog::{
    BUTTON_GAP, CONFIRM_WIDTH, NEW_NODE_BUTTONS, NEW_WORKSPACE_BUTTON_ROW, NEW_WORKSPACE_BUTTONS,
    RENAME_BUTTON_ROW, TRUST_BUTTON_ROW, TRUST_BUTTONS,
};
#[cfg(test)]
pub(crate) use save_draft::SAVE_DRAFT_BUTTONS;

use page_menu::render_page_menu;
use ui_gallery::render_ui_gallery;
use worktree::render_worktree_open;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::overlay::Overlay;
use crate::app::state::AppState;
use crate::ui::widgets;

/// 渲染当前浮层；返回需要同步的硬件光标位置（重命名输入或新建工作区路径输入）。
pub fn render(frame: &mut Frame<'_>, screen: Rect, state: &AppState) -> Option<(u16, u16)> {
    match state.overlay.as_ref()? {
        Overlay::Menu(menu_state) => {
            let labels = menu::menu_labels(menu_state);
            let title = menu::menu_title(state, menu_state);
            let layout = widgets::menu::layout(screen, menu_state.anchor, &title, &labels);
            let selected = state.menu_hover.then_some(menu_state.selected);
            widgets::menu::render(frame, &layout, &title, &labels, selected);
            None
        }
        Overlay::Rename(rename) => dialog::render_rename(frame, screen, rename),
        Overlay::NewWorkspace(new_ws) => dialog::render_new_workspace(frame, screen, new_ws),
        Overlay::ConfirmClose(confirm) => {
            dialog::render_confirm(frame, screen, state, confirm);
            None
        }
        Overlay::ConfirmSwitchCwd(confirm) => {
            dialog::render_confirm_switch_cwd(frame, screen, confirm);
            None
        }
        Overlay::ConfirmSyncWorkspaceCwd(confirm) => {
            dialog::render_confirm_sync_workspace_cwd(frame, screen, confirm);
            None
        }
        Overlay::WorktreeOpen(dialog) => render_worktree_open(frame, screen, dialog),
        Overlay::TrustWorkspace(trust) => {
            dialog::render_trust(frame, screen, trust);
            None
        }
        Overlay::NewNode(node) => dialog::render_new_node(frame, screen, node),
        Overlay::SaveDraft(dialog) => save_draft::render_save_draft(frame, screen, dialog),
        Overlay::PageMenu(menu) => {
            render_page_menu(frame, screen, state, menu);
            None
        }
        Overlay::UiGallery(gallery) => {
            render_ui_gallery(frame, screen, gallery);
            None
        }
    }
}

#[cfg(test)]
mod tests;
