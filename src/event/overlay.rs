//! 浮层对话框点击交互与全局右键上下文菜单激活。

use ratatui::layout::Rect;

use crate::app::actions::OverlayKey;
use crate::app::overlay::Overlay;
use crate::app::prompt::PromptRowKind;
use crate::app::state::AppState;
use crate::app::update;
use crate::layout::PaneId;
use crate::ui;

use super::mouse::hit::pane_at;
use super::paste;

/// 浮层左键点击：菜单项与按钮执行命令，其余位置取消。
///
/// 确认切换工作区路径时返回写入目标窗格的 shell 命令。
pub(crate) fn handle_overlay_click(
    state: &mut AppState,
    screen: Rect,
    column: u16,
    row: u16,
) -> Option<(PaneId, Vec<u8>)> {
    match state.overlay.as_ref() {
        Some(Overlay::PageMenu(menu)) => {
            let layout = ui::overlay::page_menu_layout(state, screen, menu);
            match ui::overlay::page_menu_item_at(state, screen, menu, column, row) {
                Some(index) => {
                    update::set_page_menu_selection(state, index);
                    update::activate_page_menu_row(state);
                }
                // 点击菜单内部空白保持打开；仅点击菜单之外关闭。
                None => {
                    if !layout.area.contains((column, row).into()) {
                        update::close_overlay(state);
                    }
                }
            }
            None
        }
        Some(Overlay::Menu(menu)) => {
            let layout = ui::overlay::menu_layout(state, screen, menu);
            match ui::widgets::menu::item_at(&layout, column, row) {
                Some(index) => {
                    update::set_menu_selection(state, index);
                    update::activate_menu(state);
                }
                // 点击菜单内部空白保持打开；仅点击菜单之外关闭。
                None => {
                    if !layout.area.contains((column, row).into()) {
                        update::close_overlay(state);
                    }
                }
            }
            None
        }
        Some(Overlay::Rename(_)) => {
            let Some(shell) = ui::overlay::rename_shell(screen) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            match ui::overlay::rename_button_at(&shell, column, row) {
                Some(ui::overlay::RenameButton::Save) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::RenameButton::Clear) => {
                    update::apply_overlay_key(state, OverlayKey::Clear)
                }
                Some(ui::overlay::RenameButton::Cancel) => {
                    update::close_overlay(state);
                    None
                }
                // 内部空白：保持打开。
                None => None,
            }
        }
        Some(Overlay::NewWorkspace(_)) => {
            let Some(shell) = ui::overlay::new_workspace_shell(screen) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            match ui::overlay::new_workspace_button_at(&shell, column, row) {
                Some(ui::overlay::NewWorkspaceButton::Create) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::NewWorkspaceButton::Clear) => {
                    update::apply_overlay_key(state, OverlayKey::Clear)
                }
                Some(ui::overlay::NewWorkspaceButton::Cancel) => {
                    update::close_overlay(state);
                    None
                }
                // 内部空白：保持打开。
                None => None,
            }
        }
        Some(Overlay::ConfirmClose(confirm)) => {
            let question = ui::overlay::confirm_question(state, confirm.target);
            let Some(shell) = ui::overlay::confirm_shell(screen, &question) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            match ui::overlay::confirm_button_at(&shell, column, row) {
                Some(ui::overlay::ConfirmButton::Confirm) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::ConfirmButton::Cancel) => {
                    update::close_overlay(state);
                    None
                }
                // 内部空白：保持打开。
                None => None,
            }
        }
        Some(Overlay::ConfirmSwitchCwd(confirm)) => {
            let path = confirm.path.clone();
            let Some(shell) = ui::overlay::confirm_switch_cwd_shell(screen, &path) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            match ui::overlay::confirm_button_at(&shell, column, row) {
                Some(ui::overlay::ConfirmButton::Confirm) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::ConfirmButton::Cancel) => {
                    update::close_overlay(state);
                    None
                }
                // 内部空白：保持打开。
                None => None,
            }
        }
        Some(Overlay::ConfirmSyncWorkspaceCwd(confirm)) => {
            let path = confirm.path.clone();
            let Some(shell) = ui::overlay::confirm_sync_workspace_cwd_shell(screen, &path) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            match ui::overlay::confirm_button_at(&shell, column, row) {
                Some(ui::overlay::ConfirmButton::Confirm) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::ConfirmButton::Cancel) => {
                    update::close_overlay(state);
                    None
                }
                // 内部空白：保持打开。
                None => None,
            }
        }
        Some(Overlay::WorktreeOpen(dialog)) => {
            let entry_count = dialog.entries.len();
            let Some(shell) = ui::overlay::worktree_dialog_shell(screen, entry_count) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
                return None;
            };
            if let Some(index) = ui::overlay::worktree_dialog_entry_at(&shell, dialog, column, row)
            {
                update::set_worktree_open_selection(state, index);
                return None;
            }
            match ui::overlay::worktree_dialog_button_at(&shell, column, row) {
                Some(ui::overlay::WorktreeDialogButton::Open) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::WorktreeDialogButton::Cancel) => {
                    update::close_overlay(state);
                    None
                }
                // 内部空白：保持打开。
                None => None,
            }
        }
        Some(Overlay::NewNode(_)) => {
            let Some(shell) = ui::overlay::new_node_shell(screen) else {
                update::cancel_new_node(state);
                return None;
            };
            // [✕] 或浮层之外：取消（来自选择器时恢复选择器）。
            if ui::widgets::modal::close_button_at(shell.area, column, row)
                || !shell.area.contains((column, row).into())
            {
                update::cancel_new_node(state);
                return None;
            }
            match ui::overlay::new_node_button_at(&shell, column, row) {
                Some(ui::overlay::NewNodeButton::Create) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::NewNodeButton::Clear) => {
                    update::apply_overlay_key(state, OverlayKey::Clear)
                }
                Some(ui::overlay::NewNodeButton::Cancel) => {
                    update::cancel_new_node(state);
                    None
                }
                // 内部空白：保持打开。
                None => None,
            }
        }
        Some(Overlay::SaveDraft(dialog)) => {
            let entry_count = dialog.entries.len();
            let Some(shell) = ui::overlay::save_draft_shell(screen, entry_count) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            let Some(Overlay::SaveDraft(dialog)) = state.overlay.as_ref() else {
                return None;
            };
            if let Some(index) = ui::overlay::save_draft_entry_at(&shell, dialog, column, row) {
                // 点击文件夹仅选中，不执行；动作行（新建文件夹）点击打开命名浮层。
                let is_add = matches!(
                    dialog.entries.get(index),
                    Some(crate::app::overlay::SaveDraftEntry::AddFolder)
                );
                update::set_save_draft_selection(state, index);
                if is_add {
                    update::open_new_folder_from_draft(state);
                }
                return None;
            }
            match ui::overlay::save_draft_button_at(&shell, column, row) {
                Some(ui::overlay::SaveDraftButton::Save) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::SaveDraftButton::Cancel) => {
                    update::close_overlay(state);
                    None
                }
                // 内部空白：保持打开。
                None => None,
            }
        }
        Some(Overlay::TrustWorkspace(_)) => {
            let Some(shell) = ui::overlay::trust_shell(screen) else {
                update::apply_trust_decision(state, false);
                return None;
            };
            // 点击关闭按钮或浮层之外视同拒绝信任：不因误触授予访问；内部空白保持打开。
            if ui::widgets::modal::close_button_at(shell.area, column, row)
                || !shell.area.contains((column, row).into())
            {
                update::apply_trust_decision(state, false);
                return None;
            }
            match ui::overlay::trust_button_at(&shell, column, row) {
                Some(ui::overlay::TrustButton::Trust) => update::apply_trust_decision(state, true),
                Some(ui::overlay::TrustButton::Refuse) => {
                    update::apply_trust_decision(state, false);
                }
                None => {}
            }
            None
        }
        Some(Overlay::UiGallery(_)) => {
            let Some(layout) = ui::overlay::ui_gallery_layout(screen) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &layout.shell, column, row) {
                return None;
            }
            let Some(Overlay::UiGallery(gallery)) = state.overlay.as_ref() else {
                return None;
            };
            let rows = gallery.rows().len();
            if let Some(index) = ui::overlay::ui_gallery_row_at(&layout, rows, column, row) {
                update::set_ui_gallery_selection(state, index);
                update::activate_ui_gallery_row(state);
            }
            // 分隔线、预览区或底部提示行：保持打开。
            None
        }
        None => None,
    }
}

/// 模态浮层通用点击前置：命中右上角关闭按钮或点击浮层之外时关闭；返回是否已处理。
fn modal_dismissed(
    state: &mut AppState,
    shell: &ui::widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> bool {
    if ui::widgets::modal::close_button_at(shell.area, column, row)
        || !shell.area.contains((column, row).into())
    {
        update::close_overlay(state);
        return true;
    }
    false
}

/// 提示词树右键命中目标；分区标签行不打开菜单。
enum PromptHit {
    Folder(i64),
    Item(i64),
    Temp(u64),
    Section,
}

/// 命中标签、工作区项、工作区列表空白区域或主区窗格时打开对应右键菜单；返回是否打开。
///
/// prompt 右栏、边框与切换按钮不参与：`pane_at` 只用窗格内容区，
/// 且目标必须在当前标签的窗格载荷中。
pub(crate) fn open_right_click_menu(
    state: &mut AppState,
    rects: &[(PaneId, Rect)],
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> bool {
    let tab_bar = ui::tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
    if let Some(index) = ui::tab_bar::tab_at(&tab_bar, column, row) {
        update::open_tab_menu(state, index, (column, row));
        return true;
    }
    if let Some(index) = ui::workspace_item_at(view, state, column, row) {
        update::open_workspace_menu(state, index, (column, row));
        return true;
    }
    // 工作区列表空白区域：打开公共新建菜单。
    if ui::workspace_list_blank_at(view, state, column, row) {
        update::open_workspace_list_menu(state, (column, row));
        return true;
    }
    // 提示词树：行上打开节点菜单，空白区域打开新建菜单；分区标签行不响应。
    if !state.prompt_collapsed && update::prompt_tree_mode(state) {
        let area = paste::prompt_text_area(view);
        if area.contains((column, row).into()) {
            let hit = {
                let panel = &state.prompt_owner().prompt_panel;
                ui::prompt::tree::row_at(area, panel, column, row).and_then(|index| {
                    let row_data = panel.rows().get(index).copied()?;
                    match row_data.kind {
                        PromptRowKind::Folder => panel
                            .tree
                            .folder_of_row(row_data)
                            .map(|folder| PromptHit::Folder(folder.id)),
                        PromptRowKind::Item => panel
                            .tree
                            .item_of_row(row_data)
                            .map(|item| PromptHit::Item(item.id)),
                        PromptRowKind::Temp => panel
                            .temp_of_row(row_data)
                            .map(|temp| PromptHit::Temp(temp.id)),
                        PromptRowKind::Section(_) => Some(PromptHit::Section),
                    }
                })
            };
            match hit {
                Some(PromptHit::Folder(folder_id)) => {
                    update::open_prompt_folder_menu(state, folder_id, (column, row));
                }
                Some(PromptHit::Item(item_id)) => {
                    update::open_prompt_item_menu(state, item_id, (column, row));
                }
                Some(PromptHit::Temp(temp_id)) => {
                    update::open_prompt_temp_menu(state, temp_id, (column, row));
                }
                Some(PromptHit::Section) => {}
                None => update::open_prompt_tree_menu(state, (column, row)),
            }
            return true;
        }
    }
    if let Some((pane, _)) = pane_at(rects, column, row)
        && state.active_tab().pane(pane).is_some()
    {
        update::open_pane_menu(state, pane, (column, row));
        return true;
    }
    false
}
