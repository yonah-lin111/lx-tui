//! 引用文件夹行为：目录弹窗、路径输入、内容搜索弹窗与删除确认。
//!
//! DB 读写不在本层执行：变更登记为 `Effect` 出箱，由持久化服务执行并回报；
//! 文件系统扫描与路径校验登记为 `ReferencedRequest`，由事件层后台执行后回投。
//! 内容搜索弹窗的完整行为见子模块 `search`。

mod search;

pub use search::*;

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::markdown::ReferKind;
use crate::app::overlay::{
    Overlay, ReferencedFolderDelete, ReferencedFolderInput, ReferencedFolderMenu,
    ReferencedFolderMenuRow, TextInput,
};
use crate::app::referenced::{PathValidation, ReferencedFolder, ReferencedRequest};
use crate::app::state::AppState;
use crate::ui::text;

/// 打开引用文件夹目录弹窗：以显示中工作区的 prompt 键归属。
///
/// 无有效项目键、当前处于树视图或右栏折叠时返回 false（不打开）。
/// 打开时高亮最后一条记录（新增后滚动到新项）。
pub fn open_referenced_folder_menu(state: &mut AppState, anchor: (u16, u16)) -> bool {
    if state.prompt_collapsed || super::prompt::prompt_tree_mode(state) {
        return false;
    }
    let Some(key) = state.referenced_key() else {
        return false;
    };
    let rows = reference_menu_rows(state);
    let selected = rows.len().saturating_sub(2);
    state.overlay = Some(Overlay::ReferencedFolderMenu(ReferencedFolderMenu {
        anchor,
        key: Some(key),
        rows,
        selected,
    }));
    true
}

/// 按当前快照构建目录弹窗行：数据库不可用、空状态或全部记录，末尾恒为新建行。
fn reference_menu_rows(state: &AppState) -> Vec<ReferencedFolderMenuRow> {
    let mut rows = Vec::new();
    if state.referenced.failed {
        rows.push(ReferencedFolderMenuRow::Unavailable);
    } else if state.referenced.folders.is_empty() {
        rows.push(ReferencedFolderMenuRow::Empty);
    } else {
        for folder in &state.referenced.folders {
            rows.push(ReferencedFolderMenuRow::Folder {
                id: folder.id,
                path: folder.path.clone(),
            });
        }
    }
    rows.push(ReferencedFolderMenuRow::Add);
    rows
}

/// 打开 `[add]` 路径输入弹窗；归属键取显示中工作区。
pub fn open_referenced_folder_input(state: &mut AppState, anchor: (u16, u16)) -> bool {
    let Some(key) = state.referenced_key() else {
        return false;
    };
    state.overlay = Some(Overlay::ReferencedFolderInput(ReferencedFolderInput {
        anchor,
        input: TextInput::new(String::new()),
        key: Some(key),
        validating: false,
        error: None,
        request: None,
    }));
    true
}

/// 提交路径输入：空值本地拒绝；其余登记后台校验（携带递增请求代号）。
pub fn submit_referenced_folder_input(state: &mut AppState) {
    let Some(input) = state.overlay.as_ref().and_then(|overlay| match overlay {
        Overlay::ReferencedFolderInput(input) => Some(input),
        _ => None,
    }) else {
        return;
    };
    if input.validating {
        return;
    }
    let raw = input.input.text().trim().to_string();
    if raw.is_empty() {
        set_referenced_input_error(state, PathValidation::Empty);
        return;
    }
    let id = next_generation(state);
    if let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_mut() {
        input.validating = true;
        input.error = None;
        input.request = Some(id);
    }
    state
        .referenced_requests
        .push(ReferencedRequest::ValidateAdd { id, input: raw });
}

/// 校验失败时在输入弹窗上显示错误并解除校验中状态。
fn set_referenced_input_error(state: &mut AppState, error: PathValidation) {
    set_referenced_input_error_message(state, text::referenced_path_error(&error));
}

/// 在输入弹窗上显示指定错误并解除校验中状态。
fn set_referenced_input_error_message(state: &mut AppState, message: Option<&'static str>) {
    if let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_mut() {
        input.validating = false;
        input.error = message;
        input.request = None;
    }
}

/// 后台校验成功回投：迟到（代号不匹配/弹窗已关/键变化）时丢弃。
///
/// 命中重复项时显示重复提示且不入库；否则保持输入弹窗的“校验中”状态并登记添加效应，
/// 由服务写成功后关闭弹窗（失败时在弹窗上显示错误，输入保留可重试）。
pub fn apply_referenced_validation(state: &mut AppState, id: u64, canonical: PathBuf) -> bool {
    let (key, current, anchor) = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderInput(input)) => {
            (input.key.clone(), input.request, input.anchor)
        }
        _ => return false,
    };
    if current != Some(id) || key.as_deref() != state.referenced_key().as_deref() {
        return false;
    }
    let Some(key) = key else {
        return false;
    };
    state.effects.push(Effect::AddReferencedFolder {
        workspace: key,
        path: canonical.to_string_lossy().into_owned(),
        anchor,
    });
    true
}

/// 添加写失败：在输入弹窗上显示数据库错误并解除校验中状态（保留输入可重试）。
pub fn apply_referenced_add_failure(state: &mut AppState) {
    set_referenced_input_error_message(state, Some(text::TOAST_REFERENCED_SAVE_FAILED));
}

/// 添加写成功：关闭输入弹窗并重开目录弹窗（新增项已滚入可见范围）。
pub fn finish_referenced_add(state: &mut AppState, anchor: (u16, u16)) -> bool {
    if let Some(Overlay::ReferencedFolderInput(_)) = state.overlay.as_ref() {
        state.overlay = None;
    }
    open_referenced_folder_menu(state, anchor)
}

/// 后台校验失败回投：迟到时丢弃；否则在输入弹窗显示对应原因。
pub fn apply_referenced_validation_error(
    state: &mut AppState,
    id: u64,
    error: PathValidation,
) -> bool {
    let (key, current) = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderInput(input)) => (input.key.clone(), input.request),
        _ => return false,
    };
    if current != Some(id) || key.as_deref() != state.referenced_key().as_deref() {
        return false;
    }
    set_referenced_input_error(state, error);
    true
}

/// 清空路径输入弹窗（Ctrl+C / `[clear]`），并清除既有错误提示。
pub fn clear_referenced_input(state: &mut AppState) {
    if let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_mut() {
        input.input.clear();
        input.error = None;
    }
}

/// 路径输入弹窗的单行编辑键。
pub fn edit_referenced_input(state: &mut AppState, key: crate::app::actions::OverlayKey) {
    let Some(Overlay::ReferencedFolderInput(input)) = state.overlay.as_mut() else {
        return;
    };
    edit_text_input(&mut input.input, key);
}

/// 单行输入的通用编辑键分发。
fn edit_text_input(input: &mut TextInput, key: crate::app::actions::OverlayKey) {
    use crate::app::actions::OverlayKey;
    match key {
        OverlayKey::Char(ch) => input.insert_char(ch),
        OverlayKey::Clear => input.clear(),
        OverlayKey::Backspace => input.backspace(),
        OverlayKey::Delete => input.delete(),
        OverlayKey::Left => input.move_left(),
        OverlayKey::Right => input.move_right(),
        OverlayKey::Home => input.move_home(),
        OverlayKey::End => input.move_end(),
        _ => {}
    }
}

/// 引用文件夹菜单键盘/鼠标选中项。
pub fn set_referenced_menu_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= menu.rows.len() {
        return false;
    }
    let changed = menu.selected != index || !state.menu_hover;
    menu.selected = index;
    state.menu_hover = true;
    changed
}

/// 引用文件夹菜单上下移动高亮；越界钳制。
pub fn move_referenced_menu_selection(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_mut() else {
        return false;
    };
    let len = menu.rows.len();
    if len == 0 {
        return false;
    }
    let next = (menu.selected as isize + delta).clamp(0, len as isize - 1) as usize;
    if next == menu.selected && state.menu_hover {
        return false;
    }
    menu.selected = next;
    state.menu_hover = true;
    true
}

/// 登记本次目录弹窗左键行点击并判定是否构成双击；双击后点击序列复位。
///
/// 与 prompt 列表一致：单击只选中，双击才打开（行内不再放动作按钮）。
pub fn referenced_menu_double_click(state: &mut AppState, index: usize, now: Instant) -> bool {
    let double = state.referenced_menu_click.is_some_and(|(last, at)| {
        last == index
            && now.saturating_duration_since(at) <= crate::app::update::PROMPT_DOUBLE_CLICK_WINDOW
    });
    state.referenced_menu_click = (!double).then_some((index, now));
    double
}

/// 激活引用文件夹菜单当前行：空状态不动作；`[add]` 打开输入弹窗；记录行打开搜索弹窗。
pub fn activate_referenced_menu_row(state: &mut AppState) -> bool {
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        return false;
    };
    let Some(row) = menu.rows.get(menu.selected).cloned() else {
        return false;
    };
    match row {
        ReferencedFolderMenuRow::Empty | ReferencedFolderMenuRow::Unavailable => false,
        ReferencedFolderMenuRow::Add => {
            let anchor = menu.anchor;
            state.overlay = None;
            open_referenced_folder_input(state, anchor)
        }
        ReferencedFolderMenuRow::Folder { id, path } => {
            let anchor = menu.anchor;
            open_referenced_folder_search(state, id, path, anchor)
        }
    }
}

/// 复制目录弹窗高亮记录的 Markdown 目录引用。
pub fn copy_referenced_menu_selected(state: &mut AppState) -> bool {
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        return false;
    };
    let Some(ReferencedFolderMenuRow::Folder { path, .. }) = menu.rows.get(menu.selected) else {
        return false;
    };
    let Some(text_value) = crate::app::markdown::format_reference(ReferKind::Folder, path) else {
        return false;
    };
    state.effects.push(Effect::CopyToClipboard(text_value));
    true
}

/// 打开目录弹窗高亮记录的删除确认浮层（仅移除记录，不删磁盘内容）。
pub fn remove_referenced_menu_selected(state: &mut AppState) -> bool {
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        return false;
    };
    let Some(ReferencedFolderMenuRow::Folder { id, path }) = menu.rows.get(menu.selected) else {
        return false;
    };
    open_referenced_delete(state, *id, path.clone())
}

/// 打开删除确认浮层（仅移除记录，不删磁盘内容）。
pub fn open_referenced_delete(state: &mut AppState, id: i64, path: PathBuf) -> bool {
    let Some(key) = state.referenced_key() else {
        return false;
    };
    state.overlay = Some(Overlay::ReferencedFolderDelete(ReferencedFolderDelete {
        key: Some(key),
        id,
        path,
    }));
    true
}

/// 确认删除：仅当记录仍归属当前键时登记删除效应；屏幕键盘路由已确保模态。
pub fn confirm_referenced_delete(state: &mut AppState) -> bool {
    let Some(Overlay::ReferencedFolderDelete(delete)) = state.overlay.take() else {
        return false;
    };
    let Some(key) = delete.key else {
        return false;
    };
    state.effects.push(Effect::DeleteReferencedFolder {
        workspace: key,
        id: delete.id,
    });
    true
}

/// 删除记录后作废仍在显示的搜索弹窗：关闭并丢弃在途扫描结果。
pub fn invalidate_referenced_search(state: &mut AppState, id: i64) -> bool {
    let stale = matches!(
        state.overlay.as_ref(),
        Some(Overlay::ReferencedFolderSearch(search)) if search.folder == id
    );
    if stale {
        state.overlay = None;
    }
    stale
}

/// 项目键变化或工作区切换时作废引用文件夹浮层，避免把旧结果归给新项目。
pub fn invalidate_referenced_overlays(state: &mut AppState) -> bool {
    let stale = matches!(
        state.overlay.as_ref(),
        Some(
            Overlay::ReferencedFolderMenu(_)
                | Overlay::ReferencedFolderInput(_)
                | Overlay::ReferencedFolderSearch(_)
                | Overlay::ReferencedFolderDelete(_)
        )
    );
    if stale {
        state.overlay = None;
    }
    stale
}

/// 生成递增的引用文件夹请求代号。
fn next_generation(state: &mut AppState) -> u64 {
    state.referenced_generation = state.referenced_generation.wrapping_add(1);
    state.referenced_generation
}

/// 引用文件夹快照（数据库记录 → 领域模型）。
pub fn referenced_folder_from_record(
    record: crate::db::ReferencedFolderRecord,
) -> ReferencedFolder {
    ReferencedFolder {
        id: record.id,
        path: PathBuf::from(record.path),
    }
}

/// 路径是否落在项目已存在的记录中（规范化路径精确比较）。
pub fn referenced_duplicate(state: &AppState, canonical: &Path) -> bool {
    state
        .referenced
        .folders
        .iter()
        .any(|folder| folder.path == canonical)
}

#[cfg(test)]
mod tests;
