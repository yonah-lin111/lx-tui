//! Todo 面板键盘与粘贴事件处理：列表态快捷键与行内编辑态输入处理。

use crate::app::effects::Effect;
use crate::app::state::{AppState, TodoEditTarget};
use crate::input::TodoKey;

use super::group::{
    cycle_editing_status, cycle_todo_status, group_todo_with_next, split_todo_from_group,
    ungroup_todo_all,
};
use super::{
    begin_todo_edit, blur_todo, cancel_todo_edit, commit_todo_edit, cycle_todo_priority,
    delete_todo_selected, move_todo_selection, shift_todo_week, sort_todos,
};

/// todo 按键总入口：按 `editing` 分派到单行编辑或列表动作。
pub fn apply_todo_key(state: &mut AppState, key: TodoKey) -> bool {
    state.todo_drag = None;
    if key == TodoKey::Esc && super::super::escape_clear_selection(state) {
        return true;
    }
    if state.todo.is_editing() {
        return apply_edit_key(state, key);
    }
    apply_list_key(state, key)
}

/// 编辑态按键。
fn apply_edit_key(state: &mut AppState, key: TodoKey) -> bool {
    match key {
        // 单行输入：`Enter` 与 `Shift+Enter` 都提交（Shift 只用于**列表态**的新建）。
        TodoKey::Enter | TodoKey::ShiftEnter => commit_todo_edit(state),
        TodoKey::Esc => cancel_todo_edit(state),
        TodoKey::Clear => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            if let Some(selected) = input.selected_text() {
                state.effects.push(Effect::CopyToClipboard {
                    text: selected,
                    anchor: None,
                });
                return true;
            }
            if input.text().is_empty() {
                return false;
            }
            input.clear();
            true
        }
        TodoKey::Backspace => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before_len = input.text().len();
            let before_cursor = input.cursor();
            let had_sel = input.selection().is_some();
            input.backspace();
            input.text().len() != before_len || input.cursor() != before_cursor || had_sel
        }
        TodoKey::Delete => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before_len = input.text().len();
            let had_sel = input.selection().is_some();
            input.delete();
            input.text().len() != before_len || had_sel
        }
        TodoKey::Left => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            let had_sel = input.selection().is_some();
            input.move_left();
            input.cursor() != before || had_sel
        }
        TodoKey::Right => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            let had_sel = input.selection().is_some();
            input.move_right();
            input.cursor() != before || had_sel
        }
        TodoKey::Home | TodoKey::LineStart => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            let had_sel = input.selection().is_some();
            input.move_home();
            input.cursor() != before || had_sel
        }
        TodoKey::End | TodoKey::LineEnd => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            let had_sel = input.selection().is_some();
            input.move_end();
            input.cursor() != before || had_sel
        }
        TodoKey::SelectAll => {
            if state.todo.edit_input().is_none() {
                return false;
            }
            super::super::clear_selection(state);
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            input.select_all();
            true
        }
        TodoKey::DeleteToLineStart => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before_len = input.text().len();
            let had_sel = input.selection().is_some();
            input.delete_to_line_start();
            input.text().len() != before_len || had_sel
        }
        TodoKey::DeleteToLineEnd => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before_len = input.text().len();
            let had_sel = input.selection().is_some();
            input.delete_to_line_end();
            input.text().len() != before_len || had_sel
        }
        TodoKey::Undo => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            input.undo()
        }
        TodoKey::Redo => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            input.redo()
        }
        TodoKey::Cut => {
            let text = state
                .todo
                .edit_input_mut()
                .and_then(|input| input.cut_selected());
            let Some(text) = text else {
                return false;
            };
            if !text.is_empty() {
                let anchor = Some(state.prompt.id());
                state.effects.push(Effect::CopyToClipboard { text, anchor });
            }
            true
        }
        TodoKey::WordLeft => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            let had_sel = input.selection().is_some();
            input.move_word_left();
            input.cursor() != before || had_sel
        }
        TodoKey::WordRight => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            let had_sel = input.selection().is_some();
            input.move_word_right();
            input.cursor() != before || had_sel
        }
        TodoKey::DeleteWordBackward => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before_len = input.text().len();
            let had_sel = input.selection().is_some();
            input.delete_word_backward();
            input.text().len() != before_len || had_sel
        }
        TodoKey::DeleteWordForward => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before_len = input.text().len();
            let had_sel = input.selection().is_some();
            input.delete_word_forward();
            input.text().len() != before_len || had_sel
        }
        TodoKey::Char(ch) => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            insert_bounded(input, ch)
        }
        TodoKey::Space => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            insert_bounded(input, ' ')
        }
        // 编辑态 `Tab`：循环切换该条目的状态（不退出编辑），对齐列表态 `Space`。
        TodoKey::Tab => cycle_editing_status(state),
        // 编辑态上下键：在软折行视觉行间移动光标，不退出编辑、不切行。
        TodoKey::Up | TodoKey::Down => {
            let first_width = state.todo.edit_text_width();
            let cont_width = state.todo.edit_cont_width();
            if first_width == 0 {
                return false;
            }
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            let had_sel = input.selection().is_some();
            match key {
                TodoKey::Up => input.move_up_wrapped_indented(first_width, cont_width),
                TodoKey::Down => input.move_down_wrapped_indented(first_width, cont_width),
                _ => {}
            }
            input.cursor() != before || had_sel
        }
    }
}

/// 列表态按键。
fn apply_list_key(state: &mut AppState, key: TodoKey) -> bool {
    if !matches!(key, TodoKey::Char('g')) {
        state.todo.set_pending_g(false);
    }
    match key {
        TodoKey::Up => move_todo_selection(state, -1),
        TodoKey::Down => move_todo_selection(state, 1),
        TodoKey::Space => cycle_todo_status(state),
        // `Tab` 切状态（与编辑态同义）：进入编辑走双击 / `e` / 右键菜单 `Edit`。
        TodoKey::Tab => cycle_todo_status(state),
        // `Enter` 编辑当前项（对齐 lx-agent 点行即改）；`Shift+Enter` 新建一条。
        TodoKey::Enter => match state.todo.selected() {
            Some(index) => begin_todo_edit(state, TodoEditTarget::Current(index)),
            None => begin_todo_edit(state, TodoEditTarget::New),
        },
        TodoKey::ShiftEnter => {
            let index = state.todo.selected().unwrap_or(0);
            begin_todo_edit(state, TodoEditTarget::After(index))
        }
        TodoKey::Esc => blur_todo(state),
        // 列表态完全移除/吞掉撤销与重做，不恢复已删除待办项。
        TodoKey::Undo | TodoKey::Redo => false,
        TodoKey::Clear => false,
        TodoKey::Backspace
        | TodoKey::Delete
        | TodoKey::Left
        | TodoKey::Right
        | TodoKey::WordLeft
        | TodoKey::WordRight
        | TodoKey::Home
        | TodoKey::End
        | TodoKey::LineStart
        | TodoKey::LineEnd
        | TodoKey::SelectAll
        | TodoKey::DeleteToLineStart
        | TodoKey::DeleteToLineEnd
        | TodoKey::DeleteWordBackward
        | TodoKey::DeleteWordForward
        | TodoKey::Cut => false,
        TodoKey::Char(ch) => apply_todo_command(state, ch),
    }
}

/// 列表态字符命令。
fn apply_todo_command(state: &mut AppState, ch: char) -> bool {
    match ch {
        'j' => move_todo_selection(state, 1),
        'k' => move_todo_selection(state, -1),
        'g' => {
            // 两个 `g` 跳首；中间任何非 `g` 命令都会经由 `pending_g` 重置。
            if state.todo.pending_g() {
                state.todo.set_pending_g(false);
                move_todo_selection(state, isize::MIN)
            } else {
                state.todo.set_pending_g(true);
                false
            }
        }
        'G' => {
            state.todo.set_pending_g(false);
            move_todo_selection(state, isize::MAX)
        }
        'a' => begin_todo_edit(state, TodoEditTarget::New),
        'o' => {
            let index = state.todo.selected().unwrap_or(0);
            begin_todo_edit(state, TodoEditTarget::After(index))
        }
        'e' => match state.todo.selected() {
            Some(index) => begin_todo_edit(state, TodoEditTarget::Current(index)),
            None => false,
        },
        'd' => delete_todo_selected(state),
        'm' => group_todo_with_next(state),
        'u' => split_todo_from_group(state),
        'U' => ungroup_todo_all(state),
        'p' | 'P' => {
            state.todo.set_pending_g(false);
            cycle_todo_priority(state)
        }
        's' | 'S' => {
            state.todo.set_pending_g(false);
            sort_todos(state)
        }
        'c' | 'C' => {
            state.todo.set_pending_g(false);
            state.todo.next_chart()
        }
        '[' => {
            state.todo.set_pending_g(false);
            shift_todo_week(state, -1)
        }
        ']' => {
            state.todo.set_pending_g(false);
            shift_todo_week(state, 1)
        }
        'q' => blur_todo(state),
        _ => {
            state.todo.set_pending_g(false);
            false
        }
    }
}

/// 粘贴：编辑态写入输入框，列表态吞掉。
pub fn apply_todo_paste(state: &mut AppState, text: &str) -> bool {
    let Some(input) = state.todo.edit_input_mut() else {
        return false;
    };
    let selected_len = input.selection().map_or(0, |(s, e)| e.saturating_sub(s));
    let remaining_chars = input.text().chars().count().saturating_sub(selected_len);
    let room = crate::db::todo::MAX_TODO_TEXT_CHARS.saturating_sub(remaining_chars);
    if room == 0 {
        return false;
    }
    let clipped: String = text.chars().take(room).collect();
    input.insert_str(&clipped)
}

/// 在输入框里插入一个字符；已达上限时吞掉（返回 false，不产生变化）。
fn insert_bounded(input: &mut crate::app::overlay::TextInput, ch: char) -> bool {
    let selected_len = input.selection().map_or(0, |(s, e)| e.saturating_sub(s));
    let remaining_chars = input.text().chars().count().saturating_sub(selected_len);
    if remaining_chars >= crate::db::todo::MAX_TODO_TEXT_CHARS {
        return false;
    }
    input.insert_char(ch);
    true
}
