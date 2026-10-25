//! todo 面板的行为层：CRUD、三态循环、删除撤销、视口与编辑现场。
//!
//! 全部函数返回「是否产生变化」（供事件层置脏），且不碰 IO：落库一律登记 [`Effect`]，
//! 由持久化服务在同一帧内执行。内存列表是唯一真源，写库失败只 toast、不改内存。

use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::state::{AppState, RightTab, TodoEdit, TodoEditTarget};
use crate::app::toast::{Toast, ToastKind};
use crate::app::tooltip::TodoButton;
use crate::db::todo::{TodoRecord, TodoStatus};
use crate::input::TodoKey;

use super::show_toast;

/// 滚轮每格移动的行数（与 prompt 正文一致）。
const WHEEL_LINES: usize = 3;

/// 聚焦 todo 面板：清 prompt 焦点（互斥）、收起 prompt 瞬时指针。
///
/// 不变量：`todo_focused ⇒ right_tab == Todo`；容器折叠时拒绝聚焦。
pub fn focus_todo(state: &mut AppState) -> bool {
    if state.prompt_collapsed || state.right_tab != RightTab::Todo {
        return false;
    }
    let changed = !state.todo_focused || state.prompt_focused;
    state.todo_focused = true;
    state.prompt_focused = false;
    state.prompt.clear_panel();
    state.prompt_tree_hover = None;
    changed
}

/// 释放 todo 焦点；先提交进行中的编辑（与切 tab 同口径）。
pub fn blur_todo(state: &mut AppState) -> bool {
    let mut changed = commit_todo_edit(state);
    if state.todo_focused {
        state.todo_focused = false;
        changed = true;
    }
    if state.todo_hover.take().is_some() {
        changed = true;
    }
    changed
}

/// 编辑现场在**显示序列**里的位置，以及它是否占着一个幽灵槽位。
///
/// 编辑是单槽：编辑既有条目时不存在幽灵行，显示下标就是列表下标；
/// 新建时列表里还没有这条记录，显示下标即插入点 `at`，且它下方的行整体前移一位。
fn editing_layout(state: &AppState) -> Option<(usize, bool)> {
    let edit = state.todo.editing()?;
    match edit.target.as_deref() {
        None => Some((edit.at, true)),
        Some(id) => state
            .todo
            .items()
            .iter()
            .position(|item| item.id == id)
            .map(|index| (index, false)),
    }
}

/// 编辑现场在显示序列里的位置。
fn editing_display_index(state: &AppState) -> Option<usize> {
    editing_layout(state).map(|(index, _)| index)
}

/// 显示下标 → 列表下标；越界返回 None。
///
/// **必须在提交之前算**：幽灵行占着一个显示槽位，它下方的行在列表里都前移一位，
/// 提交后幽灵行消失，那时再换算就没有依据了。
fn resolve_display_index(state: &AppState, display: usize) -> Option<usize> {
    let shifted = match editing_layout(state) {
        Some((ghost, true)) if display > ghost => display - 1,
        _ => display,
    };
    (shifted < state.todo.len()).then_some(shifted)
}

/// 列表行左键点击：单击选中 + 聚焦，双击再进入该行的行内编辑（对齐 lx-agent 点行即改）。
///
/// 双击判定与选中在同一次点击内完成，是因为选中会推进视口而**不改行下标**，
/// 两者都必须以点击当时的显示下标为准；判定先于选中，双击序列在选中生效前复位。
/// 点击 Todo 面板的**任何**位置都让容器进入选中态（聚焦 Todo 面板）。
///
/// 用户口径：点右侧容器就该高亮，不该"只有点进输入框才算选中"。工具栏、列表行、
/// 滚动条之外的分割线/统计区/空白全部走这里；不做任何数据改动，也不打断编辑。
pub fn focus_todo_from_click(state: &mut AppState) -> bool {
    focus_todo(state)
}

pub fn click_todo_row(state: &mut AppState, display: usize, now: Instant) -> bool {
    let double = state.todo_row_click.is_some_and(|(last, at)| {
        last == display
            && now.saturating_duration_since(at) <= crate::app::update::PROMPT_DOUBLE_CLICK_WINDOW
    });
    state.todo_row_click = (!double).then_some((display, now));
    // 点中的正是正在编辑的那一行（含幽灵行）：只聚焦，不提交、不重开输入框。
    if editing_display_index(state) == Some(display) {
        return focus_todo(state);
    }
    // 提交会改变下标（幽灵行消失、新条目插入），所以现在就把被点条目**按 id** 记下来，
    // 提交后再按 id 找回它的新位置。直接用下标会在幽灵行打开时选错行（实测点 b 选中 c）。
    let Some(id) =
        resolve_display_index(state, display).map(|index| state.todo.items()[index].id.clone())
    else {
        // 点空白：吞掉，既不提交也不聚焦（点空白不该结束正在进行的编辑）。
        return false;
    };
    // 幽灵行一旦被提交就消失，屏上行号整体上移一位：此刻屏幕已不是点击时那套坐标系，
    // 同一次「双击」的第二击会落在**另一个**条目上（实测会编辑错行）。作废点击序列。
    let ghost_open = editing_layout(state).is_some_and(|(_, ghost)| ghost);
    let mut changed = commit_todo_edit(state);
    if ghost_open {
        state.todo_row_click = None;
    }
    let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
        return changed;
    };
    changed |= state.todo.select(index);
    changed |= focus_todo(state);
    if double && !ghost_open {
        changed |= begin_todo_edit(state, TodoEditTarget::Current(index));
    }
    changed
}

/// 键盘/悬停移动选中并滚入视口。
pub fn move_todo_selection(state: &mut AppState, delta: isize) -> bool {
    if state.todo.is_editing() {
        return false;
    }
    state.todo.move_selection(delta)
}

/// 更新悬停行；返回是否变化。
pub fn set_todo_hover(state: &mut AppState, hover: Option<usize>) -> bool {
    if state.todo_hover == hover {
        return false;
    }
    state.todo_hover = hover;
    true
}

/// 循环当前选中项状态并登记写库。
pub fn cycle_todo_status(state: &mut AppState) -> bool {
    if state.todo.is_editing() {
        return false;
    }
    let Some(index) = state.todo.selected() else {
        return false;
    };
    let Some(record) = state.todo.cycle_status(index) else {
        return false;
    };
    state.effects.push(Effect::SaveTodo { record });
    true
}

/// 编辑态 `Tab`：切换被编辑条目的状态并登记写库。
///
/// 新建时列表里还没有记录可切（提交后才出现），吞掉。
fn cycle_editing_status(state: &mut AppState) -> bool {
    let Some(id) = state.todo.editing().and_then(|edit| edit.target.clone()) else {
        return false;
    };
    let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
        return false;
    };
    let Some(record) = state.todo.cycle_status(index) else {
        return false;
    };
    state.effects.push(Effect::SaveTodo { record });
    true
}

/// 进入编辑。
///
/// - `New`：末尾追加；
/// - `After(index)`：当前项之后插入（列表为空时等价 `New`）；
/// - `Current(index)`：编辑该行原文。
pub fn begin_todo_edit(state: &mut AppState, target: TodoEditTarget) -> bool {
    let changed = commit_todo_edit(state);
    // 新建不移动选中：幽灵行自己带 `❯`，选中留在原条目上（提交后落到新行）。
    let edit = match target {
        TodoEditTarget::New => TodoEdit::new(None, String::new(), state.todo.len()),
        TodoEditTarget::After(index) => {
            TodoEdit::new(None, String::new(), (index + 1).min(state.todo.len()))
        }
        TodoEditTarget::Current(index) => {
            let Some(item) = state.todo.items().get(index) else {
                return changed;
            };
            let edit = TodoEdit::new(Some(item.id.clone()), item.text.clone(), index);
            state.todo.select(index);
            edit
        }
    };
    state.todo.begin_edit(edit);
    state.todo_focused = true;
    state.prompt_focused = false;
    true
}

/// todo 按键总入口：按 `editing` 分派到单行编辑或列表动作。
pub fn apply_todo_key(state: &mut AppState, key: TodoKey) -> bool {
    if state.todo.is_editing() {
        return apply_todo_edit_key(state, key);
    }
    apply_todo_list_key(state, key)
}

/// 编辑态按键。
fn apply_todo_edit_key(state: &mut AppState, key: TodoKey) -> bool {
    match key {
        TodoKey::Enter => commit_todo_edit(state),
        TodoKey::Esc => cancel_todo_edit(state),
        TodoKey::Clear => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
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
            let before = input.cursor();
            input.backspace();
            input.cursor() != before
        }
        TodoKey::Delete => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.text().chars().count();
            input.delete();
            input.text().chars().count() != before
        }
        TodoKey::Left => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            input.move_left();
            input.cursor() != before
        }
        TodoKey::Right => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            input.move_right();
            input.cursor() != before
        }
        TodoKey::Home => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            input.move_home();
            input.cursor() != before
        }
        TodoKey::End => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            let before = input.cursor();
            input.move_end();
            input.cursor() != before
        }
        TodoKey::Char(ch) => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            input.insert_char(ch);
            true
        }
        TodoKey::Space => {
            let Some(input) = state.todo.edit_input_mut() else {
                return false;
            };
            input.insert_char(' ');
            true
        }
        // `Tab` 切状态（对齐 lx-agent 行内编辑：Tab 循环状态、Enter 提交、Esc 取消）。
        TodoKey::Tab => cycle_editing_status(state),
        // 编辑中不允许上下移动选中、不撤销删除：一律吞掉。
        TodoKey::Up | TodoKey::Down | TodoKey::Undo => false,
    }
}

/// 列表态按键。
fn apply_todo_list_key(state: &mut AppState, key: TodoKey) -> bool {
    match key {
        TodoKey::Up => move_todo_selection(state, -1),
        TodoKey::Down => move_todo_selection(state, 1),
        TodoKey::Space => cycle_todo_status(state),
        // `Tab` 切状态（与编辑态同义）：进入编辑只走双击 / `e` / `[edit]`。
        TodoKey::Tab => cycle_todo_status(state),
        TodoKey::Enter => {
            let index = state.todo.selected().unwrap_or(0);
            begin_todo_edit(state, TodoEditTarget::After(index))
        }
        TodoKey::Esc => blur_todo(state),
        TodoKey::Undo => undo_todo_delete(state),
        TodoKey::Clear => false,
        TodoKey::Backspace
        | TodoKey::Delete
        | TodoKey::Left
        | TodoKey::Right
        | TodoKey::Home
        | TodoKey::End => false,
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
    input.insert_str(text)
}

/// 启动加载：整体替换列表并重置选中/视口/编辑/撤销槽。
pub fn apply_todo_list(state: &mut AppState, items: Vec<TodoRecord>) {
    state.todo.replace(items);
    state.todo_scroll_drag = None;
    state.todo_hover = None;
}

/// 提交编辑：`Esc` 之外的打断源统一调用。
///
/// 分支（唯一汇合点）：
/// 新建 + 空 → 不创建；新建 + 非空 → 在 `at` 插入；
/// 已有 + 空 → 删除该条（进撤销槽）；已有 + 相同 → 不写库；已有 + 不同 → 改文本。
pub fn commit_todo_edit(state: &mut AppState) -> bool {
    let Some(edit) = state.todo.take_edit() else {
        return false;
    };
    let text = edit.input.text().trim().to_string();
    let target = edit.target;
    let at = edit.at;
    if text.is_empty() {
        // 新建不创建；已有则删除该条。
        let Some(id) = target else {
            return true;
        };
        let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
            return true;
        };
        return remove_todo_at(state, index);
    }
    match target {
        None => {
            let record = TodoRecord::new(text, TodoStatus::Todo, at as i64);
            let index = state.todo.insert(at, record.clone());
            state.todo.select(index);
            state.effects.push(Effect::SaveTodo {
                record: TodoRecord {
                    position: index as i64,
                    ..record
                },
            });
            true
        }
        Some(id) => {
            let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
                return true;
            };
            let existing = &state.todo.items()[index];
            if existing.text == text {
                return true;
            }
            let status = existing.status;
            let Some(record) = state.todo.update(index, text, status) else {
                return true;
            };
            state.effects.push(Effect::SaveTodo { record });
            true
        }
    }
}

/// 取消编辑：新建丢弃、编辑恢复原文（原文本来就没被改过，直接丢弃现场即可）。
pub fn cancel_todo_edit(state: &mut AppState) -> bool {
    state.todo.take_edit().is_some()
}

/// 删除当前项（写入撤销槽）。
pub fn delete_todo_selected(state: &mut AppState) -> bool {
    if state.todo.is_editing() {
        return false;
    }
    let Some(index) = state.todo.selected() else {
        return false;
    };
    remove_todo_at(state, index)
}

/// 删除全部完成项（写入撤销槽）。
pub fn clear_done_todos(state: &mut AppState) -> bool {
    let _ = commit_todo_edit(state);
    if state.todo.done_count() == 0 {
        return false;
    }
    let removed = state.todo.take_with_status(TodoStatus::Done);
    if removed.is_empty() {
        return false;
    }
    state.todo.set_undo(removed);
    state.todo.clamp_selection();
    state.effects.push(Effect::DeleteTodosWithStatus {
        status: TodoStatus::Done,
    });
    true
}

/// 撤销最近一次删除。
pub fn undo_todo_delete(state: &mut AppState) -> bool {
    let Some(records) = state.todo.take_undo() else {
        return false;
    };
    state.todo.restore(records.clone());
    state.todo.clamp_selection();
    state.effects.push(Effect::RestoreTodos { records });
    true
}

/// 滚轮：只动视口，不动高亮。
pub fn scroll_todo(state: &mut AppState, direction: isize) -> bool {
    let step = isize::try_from(WHEEL_LINES).unwrap_or(3);
    state.todo.scroll_by(direction.saturating_mul(step))
}

/// 滚动条拖拽/轨道点击。
pub fn set_todo_scroll(state: &mut AppState, offset: usize) -> bool {
    state.todo.set_scroll(offset)
}

/// 服务回执：写入失败；全局 toast（内存列表保持不变）。
pub fn fail_todo_write(state: &mut AppState) {
    show_toast(
        state,
        Toast::new(
            ToastKind::Danger,
            crate::ui::text::TOAST_TODO_SAVE_FAILED,
            None,
            Instant::now(),
        ),
    );
}

/// 删除指定下标的条目并登记效应。
fn remove_todo_at(state: &mut AppState, index: usize) -> bool {
    let Some(removed) = state.todo.remove(index) else {
        return false;
    };
    state.todo.set_undo(vec![removed.clone()]);
    state.todo.clamp_selection();
    state.effects.push(Effect::DeleteTodo { id: removed.id });
    true
}

/// 工具栏按钮是否可用（与渲染层的置灰口径必须一致）。
pub fn todo_button_enabled(state: &AppState, button: TodoButton) -> bool {
    match button {
        TodoButton::New => true,
        TodoButton::Edit | TodoButton::Delete => !state.todo.is_empty() && !state.todo.is_editing(),
        TodoButton::ClearDone => state.todo.done_count() > 0,
    }
}

/// 工具栏按钮动作。
///
/// 无效按钮**什么都不做**：不聚焦、不置脏、不产生任何状态变化。
/// 有效按钮先隐式聚焦（与点击行同口径），再执行动作。
pub fn apply_todo_button(state: &mut AppState, button: TodoButton) -> bool {
    if !todo_button_enabled(state, button) {
        return false;
    }
    let mut changed = focus_todo(state);
    changed |= match button {
        TodoButton::New => begin_todo_edit(state, TodoEditTarget::New),
        TodoButton::Edit => match state.todo.selected() {
            Some(index) => begin_todo_edit(state, TodoEditTarget::Current(index)),
            None => false,
        },
        TodoButton::Delete => delete_todo_selected(state),
        TodoButton::ClearDone => clear_done_todos(state),
    };
    changed
}

/// 每轮绘制前把 Todo 列表的可见行数落状态（键盘移动/删除后的视口钳制与滚入）。
///
/// `app/` 拿不到布局，几何只有渲染层一个来源；这里只做一次纯算术。
pub fn resolve_todo_viewport(state: &mut AppState, panel: ratatui::layout::Rect) -> bool {
    let rows = crate::ui::right_panel::todo::visible_rows(panel);
    state.todo.set_viewport_rows(rows)
}

#[cfg(test)]
mod tests;
