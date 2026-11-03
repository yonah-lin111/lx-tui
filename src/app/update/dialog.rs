//! 浮层对话框键盘交互：输入编辑、重命名、新建工作区与关闭确认提交。

use std::time::Instant;

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, OverlayKind, RenameTarget};
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::trust::TrustTrigger;
use crate::layout::PaneId;
use crate::ui::text;

use super::interaction::show_toast;
use super::menu::{activate_menu, close_overlay, move_menu_selection};
use super::prompt::{apply_new_node_key, apply_save_draft_key, commit_prompt_rename};
use super::trust::apply_trust_decision;
use super::workspace::{create_workspace_with_cwd, parse_workspace_path};
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
            _ => super::edit_referenced_input(state, key),
        }
        return None;
    }
    // 引用文件夹内容搜索：上下移动、回车复制引用、Shift+Enter 进入目录、Ctrl+Z 回退、Esc 关闭；
    // 其余可映射按键进入搜索框。Ctrl+Z 优先撤销搜索词，撤销栈为空时才回退目录。
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
            OverlayKey::Undo => {
                let can_undo = matches!(
                    state.overlay.as_ref(),
                    Some(Overlay::ReferencedFolderSearch(search)) if search.query.can_undo()
                );
                if can_undo {
                    super::edit_referenced_search(state, key);
                } else {
                    super::referenced_search_back(state);
                }
            }
            _ => {
                super::edit_referenced_search(state, key);
            }
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
            Some(OverlayKind::Menu) => activate_menu(state),
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
            Some(
                OverlayKind::WorktreeOpen
                | OverlayKind::TrustWorkspace
                | OverlayKind::NewNode
                | OverlayKind::SaveDraft
                | OverlayKind::ReferencedFolderMenu
                | OverlayKind::ReferencedFolderInput
                | OverlayKind::ReferencedFolderSearch
                | OverlayKind::ReferencedFolderDelete
                | OverlayKind::ResumeSessionEdit,
            )
            | None => None,
        },
        OverlayKey::Char(_)
        | OverlayKey::Clear
        | OverlayKey::Cut
        | OverlayKey::Backspace
        | OverlayKey::Delete
        | OverlayKey::Left
        | OverlayKey::Right
        | OverlayKey::WordLeft
        | OverlayKey::WordRight
        | OverlayKey::Home
        | OverlayKey::End
        | OverlayKey::LineStart
        | OverlayKey::SelectAll
        | OverlayKey::LineEnd
        | OverlayKey::DeleteToLineStart
        | OverlayKey::DeleteToLineEnd
        | OverlayKey::DeleteWordBackward
        | OverlayKey::DeleteWordForward
        | OverlayKey::Undo
        | OverlayKey::Redo => {
            edit_text_input(state, key);
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
fn edit_text_input(state: &mut AppState, key: OverlayKey) {
    if let Some(input) = state.overlay.as_mut().and_then(Overlay::input_mut) {
        input.apply_edit_key(key);
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
                Toast::new(ToastKind::Danger, err, None, Instant::now())
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

/// 确认关闭浮层回调。
fn confirm_close(state: &mut AppState) {
    let _ = state.overlay.take();
}

/// 终端粘贴进入当前浮层的单行输入框；返回是否实际写入。
///
/// 无输入框的浮层与受保护状态（校验中、删除确认中）只吞掉粘贴，绝不穿透到底层终端。
pub fn apply_overlay_paste(state: &mut AppState, text: &str) -> bool {
    let refresh_search = {
        let Some(overlay) = state.overlay.as_mut() else {
            return false;
        };
        match overlay {
            Overlay::ReferencedFolderInput(input) if input.validating => return false,
            Overlay::ResumeSessionEdit(edit) if edit.confirming_delete => return false,
            _ => {}
        }
        let Some(input) = overlay.input_mut() else {
            return false;
        };
        if !input.insert_str(text) {
            return false;
        }
        match overlay {
            Overlay::ReferencedFolderInput(input) => input.error = None,
            Overlay::ResumeSessionEdit(edit) => edit.hint = None,
            Overlay::WorktreeOpen(dialog) => dialog.normalize_selection(),
            _ => {}
        }
        matches!(overlay, Overlay::ReferencedFolderSearch(_))
    };
    if refresh_search {
        super::refresh_referenced_search(state);
    }
    true
}

/// 剪切当前浮层输入框的选区；返回被剪切文本（无选区返回 None）。
///
/// 删除记入输入框自身的一步撤销；列表类浮层同步重算高亮/过滤。
pub fn cut_overlay_selected_text(state: &mut AppState) -> Option<String> {
    let (text, refresh_search) = {
        let overlay = state.overlay.as_mut()?;
        match overlay {
            Overlay::ReferencedFolderInput(input) if input.validating => return None,
            Overlay::ResumeSessionEdit(edit) if edit.confirming_delete => return None,
            _ => {}
        }
        let text = overlay.input_mut()?.cut_selected()?;
        match overlay {
            Overlay::ReferencedFolderInput(input) => input.error = None,
            Overlay::ResumeSessionEdit(edit) => {
                edit.hint = None;
                edit.confirming_delete = false;
            }
            Overlay::WorktreeOpen(dialog) => dialog.normalize_selection(),
            _ => {}
        }
        (text, matches!(overlay, Overlay::ReferencedFolderSearch(_)))
    };
    if refresh_search {
        super::refresh_referenced_search(state);
    }
    Some(text)
}

#[cfg(test)]
mod tests;
