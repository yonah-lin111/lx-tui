//! todo 面板的行为层：CRUD、三态循环、聚合与拆分、优先级切换、一键排序、跨日顺延、多图表切换、删除撤销、视口与编辑现场。
//!
//! 全部函数返回「是否产生变化」（供事件层置脏），且不碰 IO：落库一律登记 [`Effect`]，
//! 由持久化服务在同一帧内执行。内存列表是唯一真源，写库失败只 toast、不改内存。

mod drag;
mod group;
mod key;

use std::time::Instant;

pub use drag::{begin_todo_row_press, drag_todo_to, finish_todo_drag};
pub use group::{
    click_todo_status, cycle_todo_status, group_todo_by_id_with_next, group_todo_with_next,
    split_todo_by_id, split_todo_from_group, ungroup_todo_all, ungroup_todo_by_id,
};
pub use key::{apply_todo_key, apply_todo_paste};

use crate::app::effects::Effect;
use crate::app::state::{AppState, FocusedContainer, RightTab, TodoEdit, TodoEditTarget};
use crate::app::toast::{Toast, ToastKind};
use crate::app::tooltip::TodoButton;
use crate::db::todo::{TodoPriority, TodoRecord, TodoStatus};

use super::selection::WHEEL_LINES;
use super::show_toast;

/// 登记当前选中日期下全部条目的整日同步效应。
pub(super) fn sync_current_date_todos(state: &mut AppState) {
    state.effects.push(Effect::SyncDateTodos {
        entry_date: state.todo.selected_date().to_string(),
        records: state.todo.items().to_vec(),
    });
}

/// 聚焦 todo 面板：收起 prompt 瞬时指针。
///
/// 不变量：焦点为 Todo ⇒ `right_tab == Todo`；容器折叠时拒绝聚焦。
/// 焦点互斥由 [`FocusedContainer`] 单一真源保证，不再逐处手工对消。
pub fn focus_todo(state: &mut AppState) -> bool {
    if state.prompt_collapsed || state.right_tab != RightTab::Todo {
        return false;
    }
    let mut changed = state.set_focus(FocusedContainer::Todo);
    // `clear_panel` 幂等且无返回值，故直接算作变化（与旧口径一致：一次聚焦就是一次重绘）。
    state.prompt.clear_panel();
    if state.prompt_tree_hover.take().is_some() {
        changed = true;
    }
    changed
}

/// 释放 todo 焦点；先提交进行中的编辑（与切 tab 同口径）。
pub fn blur_todo(state: &mut AppState) -> bool {
    let mut changed = commit_todo_edit(state);
    if state.set_focus(FocusedContainer::Panes) {
        changed = true;
    }
    if state.todo_hover.take().is_some() {
        changed = true;
    }
    if state.todo_drag.take().is_some() {
        changed = true;
    }
    changed
}

/// 编辑现场在**显示序列**里的位置，以及它是否占着一个幽灵槽位。
///
/// 编辑是单槽：编辑既有条目时不存在幽灵行，显示下标就是列表下标；
/// 新建时列表里还没有这条记录，显示下标即插入点 `at`，且它下方的行整体前移一位。
pub(super) fn editing_layout(state: &AppState) -> Option<(usize, bool)> {
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
pub(super) fn editing_display_index(state: &AppState) -> Option<usize> {
    editing_layout(state).map(|(index, _)| index)
}

/// 显示下标 → 列表下标；越界返回 None。
///
/// **必须在提交之前算**：幽灵行占着一个显示槽位，它下方的行在列表里都前移一位，
/// 提交后幽灵行消失，那时再换算就没有依据了。
pub(super) fn resolve_display_index(state: &AppState, display: usize) -> Option<usize> {
    let shifted = match editing_layout(state) {
        Some((ghost, true)) if display == ghost => return None,
        Some((ghost, true)) if display > ghost => display - 1,
        _ => display,
    };
    (shifted < state.todo.len()).then_some(shifted)
}

/// 点击 Todo 面板的**任何**位置都让容器进入选中态（聚焦 Todo 面板）。
///
/// 用户口径：点右侧容器就该高亮，不该"只有点进输入框才算选中"。工具栏、列表行、
/// 滚动条之外的分割线/统计区/空白全部走这里；不做任何数据改动，也不打断编辑。
pub fn focus_todo_from_click(state: &mut AppState) -> bool {
    focus_todo(state)
}

/// 列表行左键点击：单击选中 + 聚焦，双击再进入该行的行内编辑（对齐 lx-agent 点行即改）。
pub fn click_todo_row(state: &mut AppState, display: usize, now: Instant) -> bool {
    state.todo.set_pending_g(false);
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
    // 提交后再按 id 找回它的新位置。
    let Some(id) =
        resolve_display_index(state, display).map(|index| state.todo.items()[index].id.clone())
    else {
        return false;
    };
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

/// 点击某行的优先级徽标：选中该行、聚焦面板并循环切换该条目的优先级（`P0 → P1 → P2 → P3 → P0`）。
pub fn click_todo_priority(state: &mut AppState, display: usize) -> bool {
    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_drag = None;
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
    if let Some(record) = state.todo.cycle_priority(index) {
        state.effects.push(Effect::SaveTodo { record });
        changed = true;
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

/// 循环当前选中项的优先级并登记写库。
pub fn cycle_todo_priority(state: &mut AppState) -> bool {
    if state.todo.is_editing() {
        return false;
    }
    let Some(index) = state.todo.selected() else {
        return false;
    };
    let Some(record) = state.todo.cycle_priority(index) else {
        return false;
    };
    state.effects.push(Effect::SaveTodo { record });
    true
}

/// 一键排序当前选中日期下的条目（未完成优先 → 组内最高优先级 → 原顺序 → id）。
pub fn sort_todos(state: &mut AppState) -> bool {
    let mut changed = commit_todo_edit(state);
    state.todo_row_click = None;
    state.todo_drag = None;
    if let Some(ordered_ids) = state.todo.sort_by_priority() {
        state.effects.push(Effect::ReorderTodos {
            entry_date: state.todo.selected_date().to_string(),
            ordered_ids,
        });
        sync_current_date_todos(state);
        changed = true;
    }
    changed
}

/// 切换到指定日期（一周快速点击）。
pub fn select_todo_date(state: &mut AppState, date: impl Into<String>) -> bool {
    let mut changed = commit_todo_edit(state);
    if state.todo.select_date(date) {
        state.todo_row_click = None;
        state.todo_hover = None;
        state.todo_scroll_drag = None;
        state.todo_drag = None;
        changed = true;
    }
    changed |= focus_todo(state);
    changed
}

/// 按周快速切换选中日期（`‹` / `›` 翻周）。
pub fn shift_todo_week(state: &mut AppState, delta_weeks: i32) -> bool {
    let mut changed = commit_todo_edit(state);
    if state.todo.shift_week(delta_weeks) {
        state.todo_row_click = None;
        state.todo_hover = None;
        state.todo_scroll_drag = None;
        state.todo_drag = None;
        changed = true;
    }
    changed |= focus_todo(state);
    changed
}

/// 执行「将历史未完成待办顺延移动到今天」。
pub fn execute_todo_rollover(state: &mut AppState) -> bool {
    state.todo.set_pending_g(false);
    let mut changed = commit_todo_edit(state);
    if !state.todo.has_rollover_banner() {
        return changed;
    }
    let moved_ids = state.todo.rollover_past_incomplete_to_today();
    if !moved_ids.is_empty() {
        let count = moved_ids.len();
        let target_date = state.todo.today_date().to_string();
        state.effects.push(Effect::RolloverTodos {
            ids: moved_ids,
            target_date,
        });
        sync_current_date_todos(state);
        let message = crate::ui::text::toast_todo_rollover(count);
        let anchor = Some(state.prompt.id());
        show_toast(
            state,
            Toast::new(ToastKind::Default, message, anchor, Instant::now()),
        );
        state.todo_row_click = None;
        state.todo_drag = None;
        changed = true;
    }
    changed |= focus_todo(state);
    changed
}

/// 关闭「历史未完成顺延」提醒横幅。
pub fn dismiss_todo_rollover(state: &mut AppState) -> bool {
    state.todo.set_pending_g(false);
    let mut changed = state.todo.dismiss_rollover();
    changed |= focus_todo(state);
    changed
}

/// 进入编辑。
///
/// - `New`：末尾追加（独立条目，`group_id == None`）；
/// - `After(index)`：当前项之后插入；若当前项在聚合组内，则幽灵行继承该组的 `group_id` 与 `status`；
/// - `Current(index)`：编辑该行原文。
pub fn begin_todo_edit(state: &mut AppState, target: TodoEditTarget) -> bool {
    let changed = commit_todo_edit(state);
    state.todo_drag = None;
    // 新建不移动选中：幽灵行自己带 `❯`，选中留在原条目上（提交后落到新行）。
    let edit = match target {
        TodoEditTarget::New => TodoEdit::new(None, String::new(), state.todo.len()),
        TodoEditTarget::After(index) => {
            let at = (index + 1).min(state.todo.len());
            let edit = TodoEdit::new(None, String::new(), at);
            if let Some(item) = state.todo.items().get(index)
                && let Some(ref gid) = item.group_id
            {
                edit.with_group(Some(gid.clone()), item.status)
            } else {
                edit
            }
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
    // 编辑行可能落在视口外（列表溢出时在末尾新建）：先滚过去，否则用户看不到输入位置。
    state.todo.scroll_to_edit();
    state.set_focus(FocusedContainer::Todo);
    true
}

/// 启动加载：整体替换列表并重置选中/视口/编辑/撤销槽。
pub fn apply_todo_list(state: &mut AppState, items: Vec<TodoRecord>) {
    state.todo.replace(items);
    state.todo_scroll_drag = None;
    state.todo_hover = None;
    state.todo_drag = None;
}

/// 提交编辑：`Esc` 之外的打断源统一调用。
pub fn commit_todo_edit(state: &mut AppState) -> bool {
    let Some(edit) = state.todo.take_edit() else {
        return false;
    };
    let text = edit.input.text().trim().to_string();
    let target = edit.target;
    let at = edit.at;
    let group_id = edit.group_id;
    let initial_status = if group_id.is_some() {
        edit.initial_status
    } else {
        TodoStatus::Todo
    };
    if text.is_empty() {
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
            let has_group = group_id.is_some();
            let record = TodoRecord::new_on(
                state.todo.selected_date(),
                text,
                initial_status,
                TodoPriority::P1,
                at as i64,
            )
            .with_group_id(group_id);
            let index = state.todo.insert(at, record.clone());
            state.todo.select(index);
            let saved = state
                .todo
                .items()
                .get(index)
                .cloned()
                .unwrap_or(TodoRecord {
                    position: index as i64,
                    ..record
                });
            state.effects.push(Effect::SaveTodo { record: saved });
            if has_group {
                sync_current_date_todos(state);
            }
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

/// 按条目 ID 删除指定待办。
pub fn delete_todo_by_id(state: &mut AppState, id: &str) -> bool {
    let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
        return false;
    };
    remove_todo_at(state, index)
}

/// 右键点击 Todo 列表区（`effective_list_rect`）：
///
/// 1. **在提交编辑前**解析 `display`（幽灵行占用显示槽位、空编辑提交会删除条目并重排下标，提交后再按屏幕行号换算会错位到相邻条目）；
/// 2. 复位 `pending_g`、`todo_row_click` 与 `todo_hover`，提交在途编辑并聚焦 Todo 面板；
/// 3. 若命中有效条目（含刚由非空幽灵行提交生成的新条目）则选中该行并打开条目菜单，否则打开列表空白菜单。
pub fn right_click_todo_list(state: &mut AppState, display: Option<usize>, anchor: (u16, u16)) {
    enum HitBeforeCommit {
        Ghost { non_empty: bool },
        Item(String),
        Blank,
    }

    let hit = match display {
        Some(d) => match editing_layout(state) {
            Some((ghost_at, true)) if d == ghost_at => {
                let non_empty = state
                    .todo
                    .editing()
                    .is_some_and(|edit| !edit.input.text().trim().is_empty());
                HitBeforeCommit::Ghost { non_empty }
            }
            _ => match resolve_display_index(state, d)
                .and_then(|idx| state.todo.items().get(idx))
                .map(|item| item.id.clone())
            {
                Some(id) => HitBeforeCommit::Item(id),
                None => HitBeforeCommit::Blank,
            },
        },
        None => HitBeforeCommit::Blank,
    };

    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_hover = None;
    state.todo_drag = None;
    commit_todo_edit(state);
    focus_todo(state);

    let target_id = match hit {
        HitBeforeCommit::Ghost { non_empty: true } => {
            state.todo.selected_item().map(|item| item.id.clone())
        }
        HitBeforeCommit::Ghost { non_empty: false } | HitBeforeCommit::Blank => None,
        HitBeforeCommit::Item(id) => state
            .todo
            .items()
            .iter()
            .position(|item| item.id == id)
            .map(|index| {
                state.todo.select(index);
                id
            }),
    };

    if let Some(id) = target_id {
        open_todo_item_menu(state, id, anchor);
    } else {
        open_todo_blank_menu(state, anchor);
    }
}

/// 打开 Todo 条目右键菜单：按所处位置与聚合状态动态暴露聚合/拆分命令。
pub fn open_todo_item_menu(state: &mut AppState, id: impl Into<String>, anchor: (u16, u16)) {
    state.todo_drag = None;
    let id = id.into();
    let Some(index) = state.todo.items().iter().position(|item| item.id == id) else {
        return;
    };
    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_hover = None;
    let mut commands = vec![crate::app::overlay::MenuCommand::EditTodo];
    if state.todo.has_next_to_group(index) {
        commands.push(crate::app::overlay::MenuCommand::GroupTodoWithNext);
    }
    if state.todo.is_grouped(index) {
        commands.push(crate::app::overlay::MenuCommand::SplitTodoFromGroup);
        commands.push(crate::app::overlay::MenuCommand::UngroupTodoAll);
    }
    commands.push(crate::app::overlay::MenuCommand::CycleTodoPriority);
    commands.push(crate::app::overlay::MenuCommand::DeleteTodo);
    state.overlay = Some(crate::app::overlay::Overlay::Menu(
        crate::app::overlay::Menu {
            anchor,
            target: crate::app::overlay::OverlayTarget::TodoItem { id },
            commands,
            selected: 0,
            confirming: false,
        },
    ));
}

/// 打开 Todo 列表空白区域右键菜单：新建待办与智能排序。
pub fn open_todo_blank_menu(state: &mut AppState, anchor: (u16, u16)) {
    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_hover = None;
    state.todo_drag = None;
    state.overlay = Some(crate::app::overlay::Overlay::Menu(
        crate::app::overlay::Menu {
            anchor,
            target: crate::app::overlay::OverlayTarget::TodoBlank,
            commands: vec![
                crate::app::overlay::MenuCommand::NewTodo,
                crate::app::overlay::MenuCommand::SortTodos,
            ],
            selected: 0,
            confirming: false,
        },
    ));
}

/// 撤销最近一次删除。
pub fn undo_todo_delete(state: &mut AppState) -> bool {
    let Some(records) = state.todo.take_undo() else {
        return false;
    };
    state.todo.restore(records.clone());
    state.todo.clamp_selection();
    state.effects.push(Effect::RestoreTodos { records });
    sync_current_date_todos(state);
    true
}

/// 滚轮：只动视口，不动高亮。
pub fn scroll_todo(state: &mut AppState, direction: isize) -> bool {
    state.todo.scroll_by(direction.saturating_mul(WHEEL_LINES))
}

/// 滚动条拖拽/轨道点击。
pub fn set_todo_scroll(state: &mut AppState, offset: usize) -> bool {
    state.todo.set_scroll(offset)
}

/// 服务回执：写入失败；在右侧面板展示错误 toast（内存列表保持不变）。
pub fn fail_todo_write(state: &mut AppState) {
    let anchor = Some(state.prompt.id());
    show_toast(
        state,
        Toast::new(
            ToastKind::Danger,
            crate::ui::text::TOAST_TODO_SAVE_FAILED,
            anchor,
            Instant::now(),
        ),
    );
}

/// 删除指定下标的条目并登记效应（含孤儿聚合组自动退化的整日同步）。
pub fn remove_todo_at(state: &mut AppState, index: usize) -> bool {
    let partner_snapshot = if state.todo.is_grouped(index) {
        let range = state.todo.group_range(index);
        if range.len() == 2 {
            let partner_idx = if range.start == index {
                range.start + 1
            } else {
                range.start
            };
            state.todo.items().get(partner_idx).cloned()
        } else {
            None
        }
    } else {
        None
    };
    let Some(removed) = state.todo.remove(index) else {
        return false;
    };
    state.todo.set_pending_g(false);
    state.todo_row_click = None;
    state.todo_drag = None;
    if state
        .todo_hover
        .is_some_and(|hover| hover >= state.todo.len())
    {
        state.todo_hover = None;
    }
    let mut undo_records = vec![removed.clone()];
    if let Some(partner) = partner_snapshot {
        undo_records.push(partner);
    }
    state.todo.set_undo(undo_records);
    state.todo.clamp_selection();
    state.effects.push(Effect::DeleteTodo { id: removed.id });
    if !state.todo.is_empty() {
        sync_current_date_todos(state);
    }
    true
}

/// 工具栏按钮是否可用（与渲染层的置灰口径必须一致）。
pub fn todo_button_enabled(state: &AppState, button: TodoButton) -> bool {
    match button {
        TodoButton::New => true,
        TodoButton::Sort => state.todo.len() > 1 && !state.todo.is_editing(),
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
        TodoButton::Sort => sort_todos(state),
    };
    changed
}

/// 每轮绘制前把 Todo 列表的可见行数、各条目视觉行高与多行编辑视口起点同步落状态。
///
/// `app/` 拿不到布局，几何只有渲染层一个来源；这里只做一次纯算术。
pub fn resolve_todo_viewport(state: &mut AppState, panel: ratatui::layout::Rect) -> bool {
    let (rows, item_rows, edit_scroll_row, edit_text_width, edit_cont_width) =
        crate::ui::right_panel::todo::viewport_metrics(panel, state);
    state.todo.sync_viewport_indented(
        rows,
        item_rows,
        edit_scroll_row,
        edit_text_width,
        edit_cont_width,
    )
}

#[cfg(test)]
mod tests;
