//! 浮层对话框键盘交互：输入编辑、重命名、新建工作区与关闭确认提交。

use std::time::Instant;

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, OverlayKind, RenameTarget};
use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
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
    if key == OverlayKey::Esc && super::escape_clear_selection(state) {
        return None;
    }
    if key == OverlayKey::SelectAll {
        super::clear_selection(state);
    }
    // `/connect` 居中模态：导航、提交、刷新与输入编辑都在连接状态机内部处理。
    if matches!(state.overlay, Some(Overlay::Connect(_))) {
        super::chat::apply_connect_overlay_key(state, key);
        return None;
    }
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
    // 删除确认浮层：待删除工作区列表超过可视行数时，Up/Down 滚动列表。
    if matches!(state.overlay, Some(Overlay::ConfirmClose(_))) {
        match key {
            OverlayKey::Up => {
                scroll_confirm_close(state, -1);
                return None;
            }
            OverlayKey::Down => {
                scroll_confirm_close(state, 1);
                return None;
            }
            _ => {}
        }
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
                | OverlayKind::ResumeSessionEdit
                | OverlayKind::Connect,
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
        | OverlayKey::Refresh
        | OverlayKey::PageUp
        | OverlayKey::PageDown => None,
    }
}

/// 对单行输入类浮层（重命名、新建工作区路径、删除确认口令）执行一次编辑；其他浮层忽略。
fn edit_text_input(state: &mut AppState, key: OverlayKey) {
    if matches!(state.overlay, Some(Overlay::NewWorkspace(_))) {
        state.workspace_history.new_workspace_confirming = false;
    }
    if let Some(input) = state.overlay.as_mut().and_then(Overlay::input_mut) {
        input.apply_edit_key(key);
    }
}

/// 提交新建工作区：直接在首个弹窗内完成二次确认（不再弹第二个 `TrustWorkspace` 弹窗）：
/// - 显式路径第 1 次回车/点击 `[Create]`：校验路径存在且为目录后，将弹窗切换为二次确认态（主按钮变为 `[Confirm]`）；
/// - 显式路径第 2 次回车/点击 `[Confirm]`：直接记录信任并完成创建（`ws list` 下仅入库并选中，默认视图下打开激活）；
/// - 空输入继承当前路径直接创建；校验失败时弹出 Toast 错误并保持弹窗打开。
pub(crate) fn commit_new_workspace(state: &mut AppState) {
    let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_ref() else {
        return;
    };
    let text_val = dialog.input.text().to_string();
    // 手动输入与粘贴一律先做路径规范化，让解析使用去掉引号/shell 转义后的干净路径。
    // 这里不再 `.trim()`：转义尾随空格（`/tmp/foo\ `）是合法目录名的一部分。
    let normalized = crate::platform::normalize_pasted_path_input(&text_val);
    let base_cwd = state
        .active_workspace()
        .cwd
        .clone()
        .or_else(|| state.active_terminal_cwd());
    match parse_workspace_path(&normalized, base_cwd.as_deref()) {
        Ok(parsed_cwd) => match parsed_cwd {
            Some(path) => {
                if !state.workspace_history.new_workspace_confirming {
                    // 二次确认前一次性回填解析后的规范路径：单次 Enter 只产生 1 步 Undo。
                    let resolved = path.to_string_lossy().into_owned();
                    replace_new_workspace_input(state, &resolved);
                    state.workspace_history.new_workspace_confirming = true;
                    return;
                }
                state.workspace_history.new_workspace_confirming = false;
                state.overlay = None;
                let name = crate::app::state::workspace_label(
                    &path,
                    crate::app::state::home_dir().as_deref(),
                );
                state.effects.push(Effect::GrantTrust {
                    path: path.clone(),
                    name,
                });
                if super::workspace_history_mode(state) {
                    super::record_workspace_in_history_list(state, path);
                } else {
                    create_workspace_with_cwd(state, Some(path));
                }
            }
            None => {
                state.workspace_history.new_workspace_confirming = false;
                state.overlay = None;
                if super::workspace_history_mode(state) {
                    if let Some(path) =
                        base_cwd.or_else(|| crate::app::state::current_workspace_identity().0)
                    {
                        super::record_workspace_in_history_list(state, path);
                    }
                } else {
                    create_workspace_with_cwd(state, None);
                }
            }
        },
        Err(err) => {
            state.workspace_history.new_workspace_confirming = false;
            if !normalized.is_empty() {
                replace_new_workspace_input(state, &normalized);
            }
            show_toast(
                state,
                Toast::new(ToastKind::Danger, err, None, Instant::now())
                    .with_title(text::TOAST_WORKSPACE_TITLE),
            );
        }
    }
}

/// 用规范文本替换新建工作区输入框内容：保持单步撤销，文本相同时不动光标。
fn replace_new_workspace_input(state: &mut AppState, text: &str) {
    let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_mut() else {
        return;
    };
    if dialog.input.text() == text {
        return;
    }
    dialog.input.select_all();
    dialog.input.insert_str(text);
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
            let cwd = workspace.cwd.clone();
            workspace.name = name.clone();
            workspace.name_is_manual = true;
            if let Some(path) = cwd {
                for entry in &mut state.workspace_history.entries {
                    if entry.path == path {
                        entry.name = name.clone();
                        entry.name_is_manual = true;
                    }
                }
                state.effects.push(Effect::RenameWorkspace { path, name });
            }
        }
        RenameTarget::WorkspaceHistory(entry_index) => {
            let Some(entry) = state.workspace_history.entries.get_mut(entry_index) else {
                state.overlay = None;
                return;
            };
            let path = entry.path.clone();
            entry.name = name.clone();
            entry.name_is_manual = true;
            for workspace in &mut state.workspaces {
                if workspace.cwd.as_deref() == Some(path.as_path()) {
                    workspace.name = name.clone();
                    workspace.name_is_manual = true;
                }
            }
            state.effects.push(Effect::RenameWorkspace { path, name });
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

/// 确认关闭浮层回调（要求确认短语匹配 `confirm delete` 后才执行工作区级联删除）。
fn confirm_close(state: &mut AppState) {
    let confirmed = matches!(
        state.overlay.as_ref(),
        Some(Overlay::ConfirmClose(confirm)) if confirm.is_confirmed()
    );
    if !confirmed {
        return;
    }
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.take() else {
        return;
    };
    match confirm.target {
        crate::app::overlay::OverlayTarget::WorkspaceHistory(entry_index) => {
            super::workspace_history::delete_workspace_history_entry(state, entry_index);
        }
        crate::app::overlay::OverlayTarget::MissingWorkspaces => {
            super::workspace_history::delete_missing_workspace_history_entries(state);
        }
        _ => {}
    }
}

/// 按行偏移滚动 `ConfirmClose` 浮层中的待删除工作区列表（默认按最大可视行数夹取，
/// 供无屏幕几何的纯状态调用方使用）；返回滚动位置是否变化。
pub fn scroll_confirm_close(state: &mut AppState, delta: isize) -> bool {
    scroll_confirm_close_with_rows(
        state,
        delta,
        crate::app::overlay::CONFIRM_DELETE_LIST_MAX_ROWS,
    )
}

/// 按真实视口可见行数 `visible_rows` 与增量滚动确认删除浮层中的待删除工作区列表；
/// 返回滚动位置是否变化。
pub fn scroll_confirm_close_with_rows(
    state: &mut AppState,
    delta: isize,
    visible_rows: usize,
) -> bool {
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        return false;
    };
    let total = state
        .workspace_history
        .target_delete_entries(&confirm.target)
        .len();
    if visible_rows == 0 {
        return false;
    }
    let visible = total.min(visible_rows);
    let max_scroll = total.saturating_sub(visible);
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_mut() else {
        return false;
    };
    let current = confirm.scroll.min(max_scroll);
    let next = if delta < 0 {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as usize).min(max_scroll)
    };
    if confirm.scroll == next {
        return false;
    }
    confirm.scroll = next;
    true
}

/// 直接设置 `ConfirmClose` 浮层中待删除工作区列表的顶部滚动偏移
/// （默认按最大可视行数夹取）；返回滚动位置是否变化。
pub fn set_confirm_close_scroll(state: &mut AppState, offset: usize) -> bool {
    set_confirm_close_scroll_with_rows(
        state,
        offset,
        crate::app::overlay::CONFIRM_DELETE_LIST_MAX_ROWS,
    )
}

/// 按真实视口可见行数 `visible_rows` 直接设置确认删除浮层列表的顶部滚动偏移
/// （夹取到合法区间）；返回滚动位置是否变化。
pub fn set_confirm_close_scroll_with_rows(
    state: &mut AppState,
    offset: usize,
    visible_rows: usize,
) -> bool {
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        return false;
    };
    let total = state
        .workspace_history
        .target_delete_entries(&confirm.target)
        .len();
    if visible_rows == 0 {
        return false;
    }
    let visible = total.min(visible_rows);
    let max_scroll = total.saturating_sub(visible);
    let next = offset.min(max_scroll);
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_mut() else {
        return false;
    };
    if confirm.scroll == next {
        return false;
    }
    confirm.scroll = next;
    true
}

/// 终端粘贴进入当前浮层的单行输入框；返回是否实际写入。
///
/// 无输入框的浮层与受保护状态（校验中、删除确认中）只吞掉粘贴，绝不穿透到底层终端。
pub fn apply_overlay_paste(state: &mut AppState, text: &str) -> bool {
    let (refresh_search, refresh_connect) = {
        let Some(overlay) = state.overlay.as_mut() else {
            return false;
        };
        match overlay {
            Overlay::ReferencedFolderInput(input) if input.validating => return false,
            Overlay::ResumeSessionEdit(edit) if edit.confirming_delete => return false,
            Overlay::Connect(panel) if panel.validating => return false,
            _ => {}
        }
        let path_dialog = matches!(
            overlay,
            Overlay::NewWorkspace(_) | Overlay::ReferencedFolderInput(_)
        );
        let Some(input) = overlay.input_mut() else {
            return false;
        };
        // 路径弹窗整框替换（空输入或全选）时先做粘贴规范化，避免把 shell 转义、
        // 外层引号或 `file://` URI 原样塞进输入框。
        let whole_replace = path_dialog && {
            let current = input.text();
            current.trim().is_empty()
                || input
                    .selection()
                    .is_some_and(|(start, end)| start == 0 && end == current.chars().count())
        };
        if whole_replace {
            let normalized = crate::platform::normalize_pasted_path_input(text);
            // 纯引号/空 URI 归一化后没有有效路径：吞掉粘贴，不插入字面量 `''`。
            if normalized.is_empty() {
                return false;
            }
            // 纯空白内容同样视为整框替换：先全选再插入，避免保留前置空格。
            if !input.text().is_empty() {
                input.select_all();
            }
            input.insert_str(&normalized)
        } else {
            input.insert_str(text)
        };
        match overlay {
            Overlay::ReferencedFolderInput(input) => input.error = None,
            Overlay::ResumeSessionEdit(edit) => edit.hint = None,
            Overlay::WorktreeOpen(dialog) => dialog.normalize_selection(),
            Overlay::Connect(panel) => panel.clear_error(),
            _ => {}
        }
        (
            matches!(overlay, Overlay::ReferencedFolderSearch(_)),
            matches!(
                overlay,
                Overlay::Connect(panel) if matches!(panel.stage, crate::app::state::ConnectStage::Providers)
            ),
        )
    };
    if refresh_search {
        super::refresh_referenced_search(state);
    }
    if refresh_connect && let Some(Overlay::Connect(panel)) = state.overlay.as_mut() {
        panel.refilter_from_input();
    }
    if matches!(state.overlay, Some(Overlay::NewWorkspace(_))) {
        state.workspace_history.new_workspace_confirming = false;
    }
    true
}

/// 剪切当前浮层输入框的选区；返回被剪切文本（无选区返回 None）。
///
/// 删除记入输入框自身的一步撤销；列表类浮层同步重算高亮/过滤。
pub fn cut_overlay_selected_text(state: &mut AppState) -> Option<String> {
    let (text, refresh_search, refresh_connect) = {
        let overlay = state.overlay.as_mut()?;
        match overlay {
            Overlay::ReferencedFolderInput(input) if input.validating => return None,
            Overlay::ResumeSessionEdit(edit) if edit.confirming_delete => return None,
            Overlay::Connect(panel) if panel.validating => return None,
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
            Overlay::Connect(panel) => panel.clear_error(),
            _ => {}
        }
        (
            text,
            matches!(overlay, Overlay::ReferencedFolderSearch(_)),
            matches!(
                overlay,
                Overlay::Connect(panel) if matches!(panel.stage, crate::app::state::ConnectStage::Providers)
            ),
        )
    };
    if refresh_search {
        super::refresh_referenced_search(state);
    }
    if refresh_connect && let Some(Overlay::Connect(panel)) = state.overlay.as_mut() {
        panel.refilter_from_input();
    }
    if matches!(state.overlay, Some(Overlay::NewWorkspace(_))) {
        state.workspace_history.new_workspace_confirming = false;
    }
    Some(text)
}

#[cfg(test)]
mod tests;
