//! 浮层对话框键盘交互：输入编辑、重命名、新建工作区与关闭确认提交。

use std::time::Instant;

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, OverlayKind, OverlayTarget, RenameTarget, TextInput};
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::trust::TrustTrigger;
use crate::layout::PaneId;
use crate::ui::text;

use super::interaction::show_toast;
use super::menu::{activate_menu, close_overlay, move_menu_selection};
use super::prompt::{
    activate_page_menu_row, apply_new_node_key, apply_save_draft_key, commit_prompt_rename,
    confirm_delete_page, delete_prompt_folder, delete_prompt_item,
};
use super::trust::apply_trust_decision;
use super::ui_gallery::apply_ui_gallery_key;
use super::workspace::{
    close_pane, close_tab, close_workspace, confirm_switch_cwd, confirm_sync_workspace_cwd,
    create_workspace_with_cwd, parse_workspace_path,
};
use super::worktree::apply_worktree_open_key;

/// 浮层按键分派；输入层已按浮层种类过滤。
///
/// 确认切换工作区路径时返回写入目标窗格的 shell 命令；其余返回 None。
pub fn apply_overlay_key(state: &mut AppState, key: OverlayKey) -> Option<(PaneId, Vec<u8>)> {
    if matches!(state.overlay, Some(Overlay::WorktreeOpen(_))) {
        apply_worktree_open_key(state, key);
        return None;
    }
    // `/resume` 会话编辑模态：Enter 保存、^D 删除（二次确认）、Esc 关闭/退回。
    if matches!(state.overlay, Some(Overlay::ResumeSessionEdit(_))) {
        super::resume_edit::apply_resume_session_edit_key(state, key);
        return None;
    }
    // 引用文件夹目录弹窗：上下移动、回车打开、Ctrl+C 复制引用、Ctrl+D/Del 移除、Esc 关闭。
    if matches!(state.overlay, Some(Overlay::ReferencedFolderMenu(_))) {
        match key {
            OverlayKey::Esc => {
                close_overlay(state);
            }
            OverlayKey::Up => {
                super::move_referenced_menu_selection(state, -1);
            }
            OverlayKey::Down => {
                super::move_referenced_menu_selection(state, 1);
            }
            OverlayKey::Enter => {
                super::activate_referenced_menu_row(state);
            }
            OverlayKey::Copy => {
                super::copy_referenced_menu_selected(state);
            }
            OverlayKey::Remove => {
                super::remove_referenced_menu_selected(state);
            }
            _ => {}
        }
        return None;
    }
    // 引用文件夹路径输入：Enter 提交后台校验（校验中禁用）、Esc 取消、其余为单行编辑。
    if matches!(state.overlay, Some(Overlay::ReferencedFolderInput(_))) {
        match key {
            OverlayKey::Esc => super::close_overlay(state),
            OverlayKey::Enter => super::submit_referenced_folder_input(state),
            OverlayKey::Clear => super::clear_referenced_input(state),
            OverlayKey::Char(_)
            | OverlayKey::Backspace
            | OverlayKey::Delete
            | OverlayKey::Left
            | OverlayKey::Right
            | OverlayKey::Home
            | OverlayKey::End => super::edit_referenced_input(state, key),
            _ => {}
        }
        return None;
    }
    // 引用文件夹内容搜索：上下移动、回车复制引用、Shift+Enter 进入目录、Ctrl+Z 回退、Esc 关闭；
    // 其余可映射按键进入搜索框。
    if matches!(state.overlay, Some(Overlay::ReferencedFolderSearch(_))) {
        match key {
            OverlayKey::Esc => {
                super::referenced_search_escape(state);
            }
            OverlayKey::Up => {
                super::move_referenced_search_selection(state, -1);
            }
            OverlayKey::Down => {
                super::move_referenced_search_selection(state, 1);
            }
            OverlayKey::Enter | OverlayKey::Copy => {
                super::copy_referenced_entry_reference(state);
            }
            OverlayKey::EnterFolder => {
                super::referenced_search_enter(state);
            }
            OverlayKey::Back => {
                super::referenced_search_back(state);
            }
            OverlayKey::Char(_)
            | OverlayKey::Clear
            | OverlayKey::Backspace
            | OverlayKey::Delete
            | OverlayKey::Left
            | OverlayKey::Right
            | OverlayKey::Home
            | OverlayKey::End => {
                super::edit_referenced_search(state, key);
            }
            OverlayKey::Remove | OverlayKey::PageUp | OverlayKey::PageDown => {}
        }
        return None;
    }
    // 引用文件夹删除确认：Enter 确认、Esc 取消。
    if matches!(state.overlay, Some(Overlay::ReferencedFolderDelete(_))) {
        match key {
            OverlayKey::Enter => {
                super::confirm_referenced_delete(state);
            }
            OverlayKey::Esc => {
                close_overlay(state);
            }
            _ => {}
        }
        return None;
    }
    // UI 组件查看弹窗：上下移动、左右折叠展开、回车切换、Esc 关闭。
    if matches!(state.overlay, Some(Overlay::UiGallery(_))) {
        apply_ui_gallery_key(state, key);
        return None;
    }
    // 信任浮层：Enter 信任并续作，Esc 拒绝并回退；其余按键吞掉。
    if matches!(state.overlay, Some(Overlay::TrustWorkspace(_))) {
        match key {
            OverlayKey::Enter => apply_trust_decision(state, true),
            OverlayKey::Esc => apply_trust_decision(state, false),
            _ => {}
        }
        return None;
    }
    // 草稿保存选择器：上下移动、回车保存、Esc 关闭。
    if matches!(state.overlay, Some(Overlay::SaveDraft(_))) {
        apply_save_draft_key(state, key);
        return None;
    }
    // 新建节点命名浮层：回车确认、Esc 取消（来自选择器时恢复选择器）。
    if matches!(state.overlay, Some(Overlay::NewNode(_))) {
        apply_new_node_key(state, key);
        return None;
    }
    match key {
        OverlayKey::Esc => {
            close_overlay(state);
            None
        }
        OverlayKey::Up => {
            move_menu_selection(state, -1);
            None
        }
        OverlayKey::Down => {
            move_menu_selection(state, 1);
            None
        }
        OverlayKey::Enter => match state.overlay.as_ref().map(Overlay::kind) {
            Some(OverlayKind::Menu) => {
                activate_menu(state);
                None
            }
            Some(OverlayKind::Rename) => {
                commit_rename(state);
                None
            }
            Some(OverlayKind::NewWorkspace) => {
                commit_new_workspace(state);
                None
            }
            Some(OverlayKind::ConfirmClose) => {
                confirm_close(state);
                None
            }
            Some(OverlayKind::PageMenu) => {
                activate_page_menu_row(state);
                None
            }
            Some(OverlayKind::ConfirmSwitchCwd) => confirm_switch_cwd(state),
            Some(OverlayKind::ConfirmSyncWorkspaceCwd) => {
                confirm_sync_workspace_cwd(state);
                None
            }
            Some(
                OverlayKind::WorktreeOpen
                | OverlayKind::TrustWorkspace
                | OverlayKind::NewNode
                | OverlayKind::SaveDraft
                | OverlayKind::ReferencedFolderMenu
                | OverlayKind::ReferencedFolderInput
                | OverlayKind::ReferencedFolderSearch
                | OverlayKind::ReferencedFolderDelete
                | OverlayKind::ResumeSessionEdit
                | OverlayKind::UiGallery,
            )
            | None => None,
        },
        OverlayKey::Char(ch) => {
            edit_text_input(state, |input| input.insert_char(ch));
            None
        }
        OverlayKey::Clear => {
            edit_text_input(state, TextInput::clear);
            None
        }
        OverlayKey::Backspace => {
            edit_text_input(state, TextInput::backspace);
            None
        }
        OverlayKey::Delete => {
            edit_text_input(state, TextInput::delete);
            None
        }
        OverlayKey::Left => {
            edit_text_input(state, TextInput::move_left);
            None
        }
        OverlayKey::Right => {
            edit_text_input(state, TextInput::move_right);
            None
        }
        OverlayKey::Home => {
            edit_text_input(state, TextInput::move_home);
            None
        }
        OverlayKey::End => {
            edit_text_input(state, TextInput::move_end);
            None
        }
        // 引用文件夹浮层已在上方分支消费；此处不会到达。
        OverlayKey::EnterFolder
        | OverlayKey::Back
        | OverlayKey::Copy
        | OverlayKey::Remove
        | OverlayKey::PageUp
        | OverlayKey::PageDown => None,
    }
}

/// 对单行输入类浮层（重命名、新建工作区路径）执行一次编辑；其他浮层忽略。
fn edit_text_input(state: &mut AppState, edit: impl FnOnce(&mut TextInput)) {
    match state.overlay.as_mut() {
        Some(Overlay::Rename(rename)) => edit(&mut rename.input),
        Some(Overlay::NewWorkspace(new_ws)) => edit(&mut new_ws.input),
        _ => {}
    }
}

/// 提交新建工作区：校验路径，成功时关闭弹窗；
/// 显式路径先登记信任检查（效应由事件层执行；仓库归属未知时后台解析仓库根），
/// 空输入继承当前路径直接创建；失败时弹出 Toast 错误并保持弹窗打开以便用户修改。
pub(crate) fn commit_new_workspace(state: &mut AppState) {
    let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_ref() else {
        return;
    };
    let text_val = dialog.input.text().to_string();
    let base_cwd = state
        .active_workspace()
        .cwd
        .clone()
        .or_else(|| state.active_terminal_cwd());
    match parse_workspace_path(&text_val, base_cwd.as_deref()) {
        Ok(parsed_cwd) => {
            state.overlay = None;
            match parsed_cwd {
                Some(path) => state.effects.push(Effect::CheckTrust {
                    path: path.clone(),
                    root: None,
                    trigger: TrustTrigger::CreateWorkspace { cwd: path },
                }),
                None => create_workspace_with_cwd(state, None),
            }
        }
        Err(err) => {
            show_toast(
                state,
                Toast::new(ToastKind::Error, err, None, Instant::now())
                    .with_title(text::TOAST_WORKSPACE_TITLE),
            );
        }
    }
}

/// 提交重命名；空名不保存且浮层保持打开。
fn commit_rename(state: &mut AppState) {
    let Some(Overlay::Rename(rename)) = state.overlay.as_ref() else {
        return;
    };
    let name = rename.input.text().trim().to_string();
    if name.is_empty() {
        return;
    }
    let target = rename.target;
    match target {
        RenameTarget::Workspace(index) => {
            let Some(workspace) = state.workspaces.get_mut(index) else {
                state.overlay = None;
                return;
            };
            workspace.name = name;
            workspace.name_is_manual = true;
        }
        RenameTarget::Tab { workspace, tab } => {
            let Some(tab_state) = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.tabs.get_mut(tab))
            else {
                state.overlay = None;
                return;
            };
            tab_state.name = Some(name);
        }
        RenameTarget::PromptFolder { .. } | RenameTarget::PromptItem { .. } => {
            commit_prompt_rename(state, target, name);
        }
    }
    state.overlay = None;
}

/// 确认关闭目标：工作区、标签或窗格；工作区与标签都至少保留一个。
fn confirm_close(state: &mut AppState) {
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.take() else {
        return;
    };
    match confirm.target {
        OverlayTarget::Workspace(target) => close_workspace(state, target),
        OverlayTarget::Tab { workspace, tab } => close_tab(state, workspace, tab),
        OverlayTarget::Pane {
            workspace,
            tab,
            pane,
        } => close_pane(state, workspace, tab, pane),
        OverlayTarget::PromptFolder {
            workspace,
            folder_id,
        } => delete_prompt_folder(state, workspace, folder_id),
        OverlayTarget::PromptItem { workspace, item_id } => {
            delete_prompt_item(state, workspace, item_id);
        }
        OverlayTarget::PromptPage {
            workspace,
            owner,
            page,
        } => {
            confirm_delete_page(state, workspace, owner, page);
        }
        OverlayTarget::TempPrompt { .. } => {}
        OverlayTarget::PromptTree { .. } => {}
        OverlayTarget::WorkspaceList => {}
    }
}
