//! 浮层对话框点击交互与全局右键上下文菜单激活。

use std::time::Instant;

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
        Some(Overlay::ReferencedFolderMenu(menu)) => {
            let layout = ui::overlay::referenced_menu_layout(screen, menu);
            // 与 prompt 列表一致：记录行单击只选中、双击打开；新建行动作项单击即打开。
            let hit = ui::overlay::referenced_menu_row_at(screen, menu, column, row).map(|index| {
                (
                    index,
                    matches!(
                        menu.rows.get(index),
                        Some(crate::app::overlay::ReferencedFolderMenuRow::Add)
                    ),
                )
            });
            if let Some((index, is_add)) = hit {
                let double = update::referenced_menu_double_click(state, index, Instant::now());
                update::set_referenced_menu_selection(state, index);
                if is_add || double {
                    update::activate_referenced_menu_row(state);
                }
                return None;
            }
            // 点击弹窗内部空白保持打开；仅点击弹窗之外关闭。
            if !layout.area.contains((column, row).into()) {
                update::close_overlay(state);
            }
            None
        }
        Some(Overlay::ReferencedFolderInput(_)) => {
            let Some(shell) = ui::overlay::referenced_input_shell(screen) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            match ui::overlay::referenced_input_button_at(&shell, column, row) {
                Some(ui::overlay::ReferencedInputButton::Create) => {
                    update::submit_referenced_folder_input(state);
                }
                Some(ui::overlay::ReferencedInputButton::Clear) => {
                    update::clear_referenced_input(state);
                }
                Some(ui::overlay::ReferencedInputButton::Cancel) => {
                    update::close_overlay(state);
                }
                // 内部空白：保持打开。
                None => {}
            }
            None
        }
        Some(Overlay::ReferencedFolderSearch(search)) => {
            if ui::overlay::referenced_search_copy_at(screen, search, column, row) {
                update::copy_referenced_root_path(state);
                return None;
            }
            if let Some(index) = ui::overlay::referenced_search_item_at(screen, search, column, row)
            {
                update::set_referenced_search_selection(state, index);
                update::copy_referenced_entry_reference(state);
                return None;
            }
            // 点击弹窗内部空白保持打开；仅点击弹窗之外关闭并回目录弹窗。
            if ui::overlay::referenced_search_layout(screen, search)
                .is_some_and(|layout| !layout.area.contains((column, row).into()))
            {
                update::referenced_search_escape(state);
            }
            None
        }
        Some(Overlay::ReferencedFolderDelete(delete)) => {
            let path = delete.path.display().to_string();
            let name = delete
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or(path);
            let question = ui::text::referenced_delete_question(&name);
            let Some(shell) = ui::overlay::confirm_shell(screen, &question) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            match ui::overlay::confirm_button_at(&shell, column, row) {
                Some(ui::overlay::ConfirmButton::Confirm) => {
                    update::apply_overlay_key(state, crate::app::actions::OverlayKey::Enter)
                }
                Some(ui::overlay::ConfirmButton::Cancel) => {
                    update::close_overlay(state);
                    None
                }
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
        Some(Overlay::ResumeSessionEdit(edit)) => {
            let confirming = edit.confirming_delete;
            let Some(shell) = ui::overlay::resume_edit_shell(screen) else {
                update::close_overlay(state);
                return None;
            };
            if modal_dismissed(state, &shell, column, row) {
                return None;
            }
            match ui::overlay::resume_edit_button_at(&shell, confirming, column, row) {
                Some(ui::overlay::ResumeEditButton::Primary) => {
                    update::apply_overlay_key(state, OverlayKey::Enter)
                }
                Some(ui::overlay::ResumeEditButton::Delete) => {
                    update::apply_overlay_key(state, OverlayKey::Remove)
                }
                // 取消：确认态退回编辑态（不关窗），编辑态关闭模态。
                Some(ui::overlay::ResumeEditButton::Cancel) => {
                    update::apply_overlay_key(state, OverlayKey::Esc)
                }
                // 内部空白：保持打开。
                None => None,
            }
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
    // 右键菜单是新的模态动作：旧屏幕选区不能留在菜单下面继续被复制/反显。
    update::clear_selection(state);
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
    if update::prompt_panel_active(state) && update::prompt_tree_mode(state) {
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

/// 浮层内一次左键按下的分类：决定立刻执行、延迟执行还是开始屏幕选字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayHit {
    /// 按钮、关闭按钮、浮层之外等原有控件路径。
    Control,
    /// 可点击文字行：拖动选字，未拖动时松开再执行动作。
    Row,
    /// 只读文字或输入框：普通拖动开始屏幕选字。
    Text,
}

/// 判定浮层内点击是控件、可点击文字行还是只读文字。
///
/// 命中函数与渲染共用同一套几何；非按钮、非行文字都算可拖选文本。
pub(crate) fn overlay_hit(state: &AppState, screen: Rect, column: u16, row: u16) -> OverlayHit {
    use ui::overlay as o;

    let Some(overlay) = state.overlay.as_ref() else {
        return OverlayHit::Control;
    };
    let inside = |area: Rect| area.contains((column, row).into());
    match overlay {
        Overlay::Menu(menu) => {
            let layout = o::menu_layout(state, screen, menu);
            if !inside(layout.area) {
                return OverlayHit::Control;
            }
            if ui::widgets::menu::item_at(&layout, column, row).is_some() {
                OverlayHit::Row
            } else {
                OverlayHit::Text
            }
        }
        Overlay::PageMenu(menu) => {
            let layout = o::page_menu_layout(state, screen, menu);
            if !inside(layout.area) {
                return OverlayHit::Control;
            }
            if o::page_menu_item_at(state, screen, menu, column, row).is_some() {
                OverlayHit::Row
            } else {
                OverlayHit::Text
            }
        }
        Overlay::Rename(_) => {
            let Some(shell) = o::rename_shell(screen) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::rename_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::NewWorkspace(_) => {
            let Some(shell) = o::new_workspace_shell(screen) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::new_workspace_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::ConfirmClose(confirm) => {
            let question = o::confirm_question(state, confirm.target);
            let Some(shell) = o::confirm_shell(screen, &question) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::confirm_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::ConfirmSwitchCwd(confirm) => {
            let Some(shell) = o::confirm_switch_cwd_shell(screen, &confirm.path) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::confirm_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::ConfirmSyncWorkspaceCwd(confirm) => {
            let Some(shell) = o::confirm_sync_workspace_cwd_shell(screen, &confirm.path) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::confirm_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::WorktreeOpen(dialog) => {
            let Some(shell) = o::worktree_dialog_shell(screen, dialog.entries.len()) else {
                return OverlayHit::Control;
            };
            if dialog_shell_outside_or_close(&shell, column, row) {
                return OverlayHit::Control;
            }
            if o::worktree_dialog_entry_at(&shell, dialog, column, row).is_some() {
                return OverlayHit::Row;
            }
            if o::worktree_dialog_button_at(&shell, column, row).is_some() {
                return OverlayHit::Control;
            }
            OverlayHit::Text
        }
        Overlay::TrustWorkspace(_) => {
            let Some(shell) = o::trust_shell(screen) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::trust_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::NewNode(_) => {
            let Some(shell) = o::new_node_shell(screen) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::new_node_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::SaveDraft(dialog) => {
            let Some(shell) = o::save_draft_shell(screen, dialog.entries.len()) else {
                return OverlayHit::Control;
            };
            if dialog_shell_outside_or_close(&shell, column, row) {
                return OverlayHit::Control;
            }
            if o::save_draft_entry_at(&shell, dialog, column, row).is_some() {
                return OverlayHit::Row;
            }
            if o::save_draft_button_at(&shell, column, row).is_some() {
                return OverlayHit::Control;
            }
            OverlayHit::Text
        }
        Overlay::ReferencedFolderMenu(menu) => {
            let layout = o::referenced_menu_layout(screen, menu);
            if !inside(layout.area) {
                return OverlayHit::Control;
            }
            if o::referenced_menu_row_at(screen, menu, column, row).is_some() {
                OverlayHit::Row
            } else {
                OverlayHit::Text
            }
        }
        Overlay::ReferencedFolderInput(_) => {
            let Some(shell) = o::referenced_input_shell(screen) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::referenced_input_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::ReferencedFolderSearch(search) => {
            let Some(layout) = o::referenced_search_layout(screen, search) else {
                return OverlayHit::Control;
            };
            if !inside(layout.area) {
                return OverlayHit::Control;
            }
            if o::referenced_search_copy_at(screen, search, column, row) {
                return OverlayHit::Control;
            }
            if o::referenced_search_item_at(screen, search, column, row).is_some() {
                return OverlayHit::Row;
            }
            OverlayHit::Text
        }
        Overlay::ReferencedFolderDelete(delete) => {
            let path = delete.path.display().to_string();
            let name = delete
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or(path);
            let question = ui::text::referenced_delete_question(&name);
            let Some(shell) = o::confirm_shell(screen, &question) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::confirm_button_at(&shell, column, row).is_some()
            })
        }
        Overlay::ResumeSessionEdit(edit) => {
            let Some(shell) = o::resume_edit_shell(screen) else {
                return OverlayHit::Control;
            };
            dialog_hit(&shell, column, row, || {
                o::resume_edit_button_at(&shell, edit.confirming_delete, column, row).is_some()
            })
        }
    }
}

/// 对话框内：关闭按钮/外部/按钮走控件，其余文字走可选文本。
fn dialog_hit(
    shell: &ui::widgets::modal::ModalShell,
    column: u16,
    row: u16,
    button: impl FnOnce() -> bool,
) -> OverlayHit {
    if dialog_shell_outside_or_close(shell, column, row) || button() {
        OverlayHit::Control
    } else {
        OverlayHit::Text
    }
}

fn dialog_shell_outside_or_close(
    shell: &ui::widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> bool {
    !shell.area.contains((column, row).into())
        || ui::widgets::modal::close_button_at(shell.area, column, row)
}
