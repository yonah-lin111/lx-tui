//! Todo 列表行鼠标拖拽合并状态机：按下登记、跨行拖动落点判定与释放合并。

use std::time::Instant;

use crate::app::state::{AppState, TodoDrag};

use super::{
    click_todo_row, editing_display_index, resolve_display_index, sync_current_date_todos,
};

/// 列表行左键按下：执行单击选中/双击编辑判定，并在未进入编辑态时登记拖拽起点。
pub fn begin_todo_row_press(state: &mut AppState, display: usize, now: Instant) -> bool {
    let had_edit = state.todo.is_editing();
    let in_bounds = resolve_display_index(state, display).is_some()
        || editing_display_index(state) == Some(display);
    let changed = click_todo_row(state, display, now);
    if in_bounds && !had_edit && !state.todo.is_editing() {
        let from_display = state.todo.selected().unwrap_or(display);
        state.todo_drag = state.todo.selected_item().map(|item| TodoDrag {
            source_id: item.id.clone(),
            from_display,
            pressed_at: now,
            dragging: false,
            target_index: None,
        });
    } else {
        state.todo_drag = None;
    }
    changed
}

/// 列表行鼠标拖动中：一旦离开起始显示行即激活拖拽态并作废双击序列，实时计算可合并目标下标。
pub fn drag_todo_to(state: &mut AppState, display: Option<usize>) -> bool {
    let Some(drag) = state.todo_drag.as_ref() else {
        return false;
    };
    let source_id = drag.source_id.clone();
    let from_display = drag.from_display;
    let was_dragging = drag.dragging;
    let old_target = drag.target_index;

    let Some(source_idx) = state
        .todo
        .items()
        .iter()
        .position(|item| item.id == source_id)
    else {
        state.todo_drag = None;
        return was_dragging || old_target.is_some();
    };
    let source_group = state.todo.items()[source_idx].group_id.clone();

    let now_dragging = was_dragging || (display.is_some() && display != Some(from_display));
    if now_dragging && !was_dragging {
        state.todo_row_click = None;
        state.todo_hover = None;
    }

    let next_target = if now_dragging {
        display
            .and_then(|d| resolve_display_index(state, d))
            .filter(|&target_idx| {
                if target_idx == source_idx {
                    return false;
                }
                let target_group = &state.todo.items()[target_idx].group_id;
                !(source_group.is_some() && source_group == *target_group)
            })
    } else {
        None
    };

    if let Some(drag_mut) = state.todo_drag.as_mut() {
        drag_mut.dragging = now_dragging;
        drag_mut.target_index = next_target;
    }
    now_dragging != was_dragging || next_target != old_target
}

/// 列表行鼠标左键释放：若已跨行拖拽且落点有效则将源条目并入目标条目/聚合组。
pub fn finish_todo_drag(state: &mut AppState) -> bool {
    let Some(drag) = state.todo_drag.take() else {
        return false;
    };
    if !drag.dragging {
        return false;
    }
    state.todo_hover = None;
    if let Some(target_idx) = drag.target_index
        && let Some(source_idx) = state
            .todo
            .items()
            .iter()
            .position(|item| item.id == drag.source_id)
        && state.todo.merge_item_into(source_idx, target_idx)
    {
        sync_current_date_todos(state);
    }
    true
}
