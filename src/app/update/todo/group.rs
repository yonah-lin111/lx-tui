//! Todo 聚合组动作与状态整组切换：向下合并、单条拆出、整组解散与点击状态框。

use crate::app::effects::Effect;
use crate::app::state::AppState;

use super::{
    commit_todo_edit, editing_layout, focus_todo, resolve_display_index, sync_current_date_todos,
};

/// 循环当前选中项（或所在聚合组全员）的状态并登记写库。
pub fn cycle_todo_status(state: &mut AppState) -> bool {
    if state.todo.is_editing() {
        return false;
    }
    let Some(index) = state.todo.selected() else {
        return false;
    };
    let records = state.todo.cycle_status(index);
    if records.is_empty() {
        return false;
    }
    for record in records {
        state.effects.push(Effect::SaveTodo { record });
    }
    true
}

/// 编辑态 `Tab`：切换被编辑条目（或所在聚合组全员）的状态并登记写库。
///
/// 独立新建时列表里还没有记录可切（提交后才出现），吞掉；组内新建幽灵行则同步切换所在聚合组全员状态。
pub(super) fn cycle_editing_status(state: &mut AppState) -> bool {
    let Some(edit) = state.todo.editing() else {
        return false;
    };
    let index = if let Some(id) = edit.target.clone() {
        state.todo.items().iter().position(|item| item.id == id)
    } else if let Some(gid) = edit.group_id.clone() {
        state
            .todo
            .items()
            .iter()
            .position(|item| item.group_id.as_deref() == Some(gid.as_str()))
    } else {
        None
    };
    let Some(index) = index else {
        return false;
    };
    let records = state.todo.cycle_status(index);
    if records.is_empty() {
        return false;
    }
    if let Some(next_status) = records.first().map(|record| record.status)
        && let Some(edit_mut) = state.todo.editing_mut()
    {
        edit_mut.initial_status = next_status;
    }
    for record in records {
        state.effects.push(Effect::SaveTodo { record });
    }
    true
}

/// 点击某行的状态框 `[ ]`：选中该行、聚焦面板并循环切换该条（或所在聚合组全员）的状态。
pub fn click_todo_status(state: &mut AppState, display: usize) -> bool {
    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_drag = None;
    if editing_layout(state) == Some((display, true)) {
        let ghost_gid = state.todo.editing().and_then(|edit| edit.group_id.clone());
        let non_empty = state
            .todo
            .editing()
            .is_some_and(|edit| !edit.input.text().trim().is_empty());
        if ghost_gid.is_none() && !non_empty {
            return false;
        }
        let mut changed = commit_todo_edit(state);
        changed |= focus_todo(state);
        let target_index = if let Some(gid) = ghost_gid {
            state
                .todo
                .items()
                .iter()
                .position(|item| item.group_id.as_deref() == Some(gid.as_str()))
        } else {
            state.todo.selected()
        };
        if let Some(index) = target_index {
            changed |= state.todo.select(index);
            let records = state.todo.cycle_status(index);
            if !records.is_empty() {
                for record in records {
                    state.effects.push(Effect::SaveTodo { record });
                }
                changed = true;
            }
        }
        return changed;
    }
    let Some(id) =
        resolve_display_index(state, display).map(|index| state.todo.items()[index].id.clone())
    else {
        return false;
    };
    let mut changed = commit_todo_edit(state);
    let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
        return changed;
    };
    changed |= state.todo.select(index);
    changed |= focus_todo(state);
    let records = state.todo.cycle_status(index);
    if !records.is_empty() {
        for record in records {
            state.effects.push(Effect::SaveTodo { record });
        }
        changed = true;
    }
    changed
}

/// 将当前选中项（或所在聚合组）与紧邻的下一项（或下一组）聚合成同一组。
pub fn group_todo_with_next(state: &mut AppState) -> bool {
    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_drag = None;
    let mut changed = commit_todo_edit(state);
    let Some(index) = state.todo.selected() else {
        return changed;
    };
    if state.todo.merge_with_next(index) {
        sync_current_date_todos(state);
        changed = true;
    }
    changed
}

/// 将当前选中的组内子项移出聚合组。
pub fn split_todo_from_group(state: &mut AppState) -> bool {
    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_drag = None;
    let mut changed = commit_todo_edit(state);
    let Some(index) = state.todo.selected() else {
        return changed;
    };
    if state.todo.split_from_group(index) {
        sync_current_date_todos(state);
        changed = true;
    }
    changed
}

/// 解散当前选中项所在的整个聚合组。
pub fn ungroup_todo_all(state: &mut AppState) -> bool {
    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_drag = None;
    let mut changed = commit_todo_edit(state);
    let Some(index) = state.todo.selected() else {
        return changed;
    };
    if state.todo.ungroup_at(index) {
        sync_current_date_todos(state);
        changed = true;
    }
    changed
}

/// 按条目 ID 将该条（或所在聚合组）与下一项（或下一组）聚合（右键菜单动作）。
pub fn group_todo_by_id_with_next(state: &mut AppState, id: &str) -> bool {
    let mut changed = commit_todo_edit(state);
    changed |= focus_todo(state);
    let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
        return changed;
    };
    changed |= state.todo.select(index);
    if state.todo.merge_with_next(index) {
        sync_current_date_todos(state);
        changed = true;
    }
    changed
}

/// 按条目 ID 将该子项从聚合组拆出（右键菜单动作）。
pub fn split_todo_by_id(state: &mut AppState, id: &str) -> bool {
    let mut changed = commit_todo_edit(state);
    changed |= focus_todo(state);
    let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
        return changed;
    };
    changed |= state.todo.select(index);
    if state.todo.split_from_group(index) {
        sync_current_date_todos(state);
        changed = true;
    }
    changed
}

/// 按条目 ID 解散其所在的整个聚合组（右键菜单动作）。
pub fn ungroup_todo_by_id(state: &mut AppState, id: &str) -> bool {
    let mut changed = commit_todo_edit(state);
    changed |= focus_todo(state);
    let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
        return changed;
    };
    changed |= state.todo.select(index);
    if state.todo.ungroup_at(index) {
        sync_current_date_todos(state);
        changed = true;
    }
    changed
}
