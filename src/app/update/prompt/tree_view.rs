//! 提示词树视图导航与选中：移动、折叠、搜索过滤、滚动与行激活。

use std::time::{Duration, Instant};

use crate::app::actions::EditorCommand;
use crate::app::prompt::{PromptMode, PromptRow, PromptRowKind, PromptTree};
use crate::app::state::AppState;

use super::{edit_prompt_item, edit_prompt_temp_row, open_temp_prompt};

/// 提示词树内容区可见行数；取编辑器的同步尺寸（同一面板几何），扣除搜索框与分割线行。
pub fn prompt_tree_visible_rows(state: &AppState) -> usize {
    usize::from(
        state
            .prompt
            .size()
            .1
            .saturating_sub(crate::layout::PROMPT_TREE_HEADER_HEIGHT)
            .max(1),
    )
}

/// 显示中的工作区是否处于提示词树视图。
pub fn prompt_tree_mode(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| workspace.prompt_panel.mode == PromptMode::Tree)
}

/// 树上移/下移选中行：按方向逐格跳过分区标签行；`visible` 为可见行数，选中始终保持在视口内。
pub fn move_prompt_selection(state: &mut AppState, delta: isize, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let rows = workspace.prompt_panel.rows();
    if rows.is_empty() {
        return false;
    }
    let step: isize = if delta >= 0 { 1 } else { -1 };
    let mut selected = workspace.prompt_panel.selected.min(rows.len() - 1);
    let mut moved = false;
    for _ in 0..delta.unsigned_abs() {
        let mut candidate = selected as isize + step;
        let mut next = None;
        while candidate >= 0 && (candidate as usize) < rows.len() {
            if rows[candidate as usize].kind.selectable() {
                next = Some(candidate as usize);
                break;
            }
            candidate += step;
        }
        let Some(next) = next else {
            break;
        };
        selected = next;
        moved = true;
    }
    if !moved {
        return false;
    }
    workspace.prompt_panel.selected = selected;
    workspace.prompt_panel.mark_tree_selection();
    scroll_selection_into_view(&mut workspace.prompt_panel, visible);
    true
}

/// 选中指定可见行（鼠标命中）；索引越界忽略。
pub fn select_prompt_row(state: &mut AppState, row: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    if row >= workspace.prompt_panel.rows().len() || workspace.prompt_panel.selected == row {
        return false;
    }
    workspace.prompt_panel.selected = row;
    workspace.prompt_panel.mark_tree_selection();
    true
}

/// 激活选中行：文件夹切换折叠，条目/临时提示词进入编辑视图。
pub fn activate_prompt_row(state: &mut AppState) -> bool {
    let index = state.prompt_workspace();
    let Some(row) = selected_row(state, index) else {
        return false;
    };
    match row.kind {
        PromptRowKind::Folder => toggle_prompt_folder(state, index, row),
        PromptRowKind::Item => edit_prompt_row(state, index, row),
        PromptRowKind::Temp => edit_prompt_temp_row(state, index, row),
        PromptRowKind::Section(_) => false,
    }
}

/// 折叠选中文件夹（Left）；选中条目或已折叠时 no-op。
pub fn collapse_prompt_row(state: &mut AppState) -> bool {
    set_selected_folder_collapsed(state, true)
}

/// 展开选中文件夹（Right）；选中条目或已展开时 no-op。
pub fn expand_prompt_row(state: &mut AppState) -> bool {
    set_selected_folder_collapsed(state, false)
}

/// 鼠标点击行：选中；文件夹行同时切换折叠；分区标签行不可交互。
pub fn click_prompt_row(state: &mut AppState, row: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(row_data) = state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.prompt_panel.rows().get(row).copied())
    else {
        return false;
    };
    if !row_data.kind.selectable() {
        return false;
    }
    let changed = select_prompt_row(state, row);
    match row_data.kind {
        PromptRowKind::Folder => toggle_prompt_folder(state, index, row_data) || changed,
        PromptRowKind::Item | PromptRowKind::Temp => changed,
        PromptRowKind::Section(_) => false,
    }
}

/// 登记本次左键行点击并判定是否构成双击；双击后点击序列复位。
pub fn prompt_row_double_click(state: &mut AppState, row: usize, now: Instant) -> bool {
    let double = state.prompt_row_click.is_some_and(|(last_row, at)| {
        last_row == row && now.saturating_duration_since(at) <= PROMPT_DOUBLE_CLICK_WINDOW
    });
    state.prompt_row_click = (!double).then_some((row, now));
    double
}

/// 双击可见行：条目与临时提示词进入编辑视图；文件夹与分区标签行不响应
/// （文件夹的单击已完成折叠/展开，双击不再叠加动作）。
pub fn open_prompt_row(state: &mut AppState, row: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(row_data) = state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.prompt_panel.rows().get(row).copied())
    else {
        return false;
    };
    match row_data.kind {
        PromptRowKind::Item => edit_prompt_row(state, index, row_data),
        PromptRowKind::Temp => edit_prompt_temp_row(state, index, row_data),
        PromptRowKind::Folder | PromptRowKind::Section(_) => false,
    }
}

/// 从指定工作区树中按 id 选中文件夹（新建后由服务调用）。
pub fn select_prompt_folder(state: &mut AppState, workspace: usize, folder_id: i64) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let rows = workspace_state.prompt_panel.rows();
    let position = rows.iter().position(|row| {
        row.kind == PromptRowKind::Folder
            && workspace_state
                .prompt_panel
                .tree
                .folder_of_row(*row)
                .is_some_and(|folder| folder.id == folder_id)
    });
    let Some(position) = position else {
        return false;
    };
    workspace_state.prompt_panel.selected = position;
    scroll_selection_into_view(&mut workspace_state.prompt_panel, visible);
    true
}

/// 打开选中条目的编辑视图（`[✎]` 按钮 / 树视图回车）；
/// 选中行非条目时新建临时提示词并进入编辑，保证 `[✎]` 永不无响应。
pub fn edit_selected_prompt(state: &mut AppState) -> bool {
    let index = state.prompt_workspace();
    let Some(item_id) = state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.prompt_panel.selected_prompt_id())
    else {
        return open_temp_prompt(state);
    };
    edit_prompt_item(state, index, item_id)
}

/// 树视图搜索框编辑：按单行输入语义执行命令（字符/移动/删除/清空）；
/// 过滤词变化后选中落到首个结果，清空后钳制；返回是否被搜索框消费。
pub fn apply_prompt_search(state: &mut AppState, command: &EditorCommand) -> bool {
    let index = state.prompt_workspace();
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(index) else {
        return false;
    };
    let panel = &mut workspace_state.prompt_panel;
    let before = panel.search.text().to_string();
    match command {
        EditorCommand::InsertChar(ch) => panel.search.insert_char(*ch),
        EditorCommand::Backspace => panel.search.backspace(),
        EditorCommand::Delete => panel.search.delete(),
        EditorCommand::Left => panel.search.move_left(),
        EditorCommand::Right => panel.search.move_right(),
        EditorCommand::Home => panel.search.move_home(),
        EditorCommand::End => panel.search.move_end(),
        EditorCommand::Escape => panel.search.clear(),
        _ => return false,
    }
    if panel.search.text() != before {
        if panel.search.text().trim().is_empty() {
            panel.clamp_selection(visible);
        } else {
            panel.selected = 0;
            panel.clamp_selection(visible);
            scroll_selection_into_view(panel, visible);
        }
    }
    true
}

/// 滚动树视口；越界钳制；返回是否变化。
pub fn scroll_prompt_tree(state: &mut AppState, direction: isize, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let max = workspace
        .prompt_panel
        .rows()
        .len()
        .saturating_sub(visible.max(1));
    let target = (workspace.prompt_panel.scroll.min(max) as isize).saturating_add(direction);
    let offset = target.clamp(0, max as isize) as usize;
    if offset == workspace.prompt_panel.scroll {
        return false;
    }
    workspace.prompt_panel.scroll = offset;
    true
}

/// 直接设置树滚动的视口偏移（滚动条点击/拖拽）；越界钳制；返回是否变化。
pub fn set_prompt_tree_scroll(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let max = workspace
        .prompt_panel
        .rows()
        .len()
        .saturating_sub(visible.max(1));
    let offset = offset.min(max);
    if offset == workspace.prompt_panel.scroll {
        return false;
    }
    workspace.prompt_panel.scroll = offset;
    true
}

/// 几何变化后钳制树选中的可见性；返回是否变化。
pub fn clamp_prompt_tree_scroll(state: &mut AppState, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let before = (
        workspace.prompt_panel.selected,
        workspace.prompt_panel.scroll,
    );
    scroll_selection_into_view(&mut workspace.prompt_panel, visible);
    (
        workspace.prompt_panel.selected,
        workspace.prompt_panel.scroll,
    ) != before
}

/// 选中指定工作区树中的条目行（服务在新建后调用）。
pub fn select_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let rows = workspace_state.prompt_panel.rows();
    let position = rows.iter().position(|row| {
        row.kind == PromptRowKind::Item
            && workspace_state
                .prompt_panel
                .tree
                .item_of_row(*row)
                .is_some_and(|item| item.id == item_id)
    });
    let Some(position) = position else {
        return false;
    };
    workspace_state.prompt_panel.selected = position;
    scroll_selection_into_view(&mut workspace_state.prompt_panel, visible);
    true
}

/// 选中指定工作区中指定临时提示词行（服务/菜单在打开前调用）。
pub fn select_prompt_temp(state: &mut AppState, workspace: usize, temp_id: u64) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let rows = workspace_state.prompt_panel.rows();
    let position = rows.iter().position(|row| {
        workspace_state
            .prompt_panel
            .temp_of_row(*row)
            .is_some_and(|temp| temp.id == temp_id)
    });
    let Some(position) = position else {
        return false;
    };
    workspace_state.prompt_panel.selected = position;
    scroll_selection_into_view(&mut workspace_state.prompt_panel, visible);
    true
}

/// 切换指定可见行文件夹的折叠状态。
fn toggle_prompt_folder(state: &mut AppState, workspace: usize, row: PromptRow) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let Some(folder) = workspace_state.prompt_panel.tree.folder_of_row(row) else {
        return false;
    };
    let id = folder.id;
    workspace_state.prompt_panel.tree.toggle_folder(id);
    workspace_state.prompt_panel.clamp_selection(visible.max(1));
    true
}

/// 进入选中条目编辑视图。
fn edit_prompt_row(state: &mut AppState, workspace: usize, row: PromptRow) -> bool {
    let Some(item_id) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item_of_row(row))
        .map(|item| item.id)
    else {
        return false;
    };
    edit_prompt_item(state, workspace, item_id)
}

/// 设置选中文件夹行的折叠态；行非文件夹或状态未变返回 false。
fn set_selected_folder_collapsed(state: &mut AppState, collapsed: bool) -> bool {
    let index = state.prompt_workspace();
    let Some(row) = selected_row(state, index) else {
        return false;
    };
    if row.kind != PromptRowKind::Folder {
        return false;
    }
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(index) else {
        return false;
    };
    let Some(folder) = workspace_state.prompt_panel.tree.folder_of_row(row) else {
        return false;
    };
    if folder.collapsed == collapsed {
        return false;
    }
    let id = folder.id;
    workspace_state.prompt_panel.tree.toggle_folder(id);
    workspace_state.prompt_panel.clamp_selection(visible.max(1));
    true
}

/// 条目是否处于被删除文件夹的子树内。
pub(super) fn removed_with_folder(
    tree: &PromptTree,
    folder_id: i64,
    item_folder: Option<i64>,
) -> bool {
    let mut current = item_folder;
    while let Some(id) = current {
        if id == folder_id {
            return true;
        }
        current = tree.folder(id).and_then(|folder| folder.parent_id);
    }
    false
}

/// 保证选中行在可见范围内。
fn scroll_selection_into_view(panel: &mut crate::app::prompt::PromptPanel, visible: usize) {
    let visible = visible.max(1);
    let rows = panel.rows().len();
    let selected = panel.selected.min(rows.saturating_sub(1));
    panel.selected = selected;
    let max = rows.saturating_sub(visible);
    let mut scroll = panel.scroll.min(max);
    if selected < scroll {
        scroll = selected;
    } else if selected >= scroll + visible {
        scroll = selected + 1 - visible;
    }
    panel.scroll = scroll.min(max);
}

/// 指定工作区的选中行。
fn selected_row(state: &AppState, workspace: usize) -> Option<PromptRow> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.selected_row())
}

/// prompt 树行双击判定窗口：同一可见行的两次左键按下间隔不超过该时长视为双击。
pub const PROMPT_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);
