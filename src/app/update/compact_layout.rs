//! 紧凑布局状态机：进入/退出快照、左右全屏栏切换与返回聊天。

use crate::app::state::{AppState, CompactRestore, FocusedContainer, RightTab};
use crate::config::Config;

use super::focus::{focus_panes, focus_sidebar};
use super::right_panel::select_right_tab;

/// 是否进入紧凑布局：宽度非零且不超过配置阈值。
pub fn is_compact(width: u16, config: &Config) -> bool {
    width > 0 && width <= config.narrow_width
}

/// 按终端宽度同步紧凑布局状态；每轮事件循环在布局前调用。
///
/// 进入时快照桌面折叠状态并临时展开三栏；退出时恢复。全屏左/右栏由焦点推导，
/// 不在此处另存状态。
pub fn sync_compact_layout(state: &mut AppState, width: u16, config: &Config) {
    let compact = is_compact(width, config);
    match (compact, state.compact_restore) {
        (true, None) => {
            state.compact_restore = Some(CompactRestore {
                sidebar_collapsed: state.sidebar_collapsed,
                prompt_collapsed: state.prompt_collapsed,
                panes_collapsed: state.panes_collapsed,
            });
            state.sidebar_collapsed = false;
            state.prompt_collapsed = false;
            state.panes_collapsed = false;
            clear_compact_transition_drags(state);
        }
        (false, Some(restore)) => {
            state.compact_restore = None;
            state.sidebar_collapsed = restore.sidebar_collapsed;
            // 焦点仍在全屏右栏时保留展开，避免把正在编辑的面板重新折叠。
            if !matches!(
                state.focused,
                FocusedContainer::Prompt | FocusedContainer::Todo
            ) {
                state.prompt_collapsed = restore.prompt_collapsed;
            }
            // 主区位于右列且当前停留在主区时保留展开，避免退出后焦点困在 4 列折叠窄条。
            state.panes_collapsed = if state.prompt_left && state.panes_focused() {
                false
            } else {
                restore.panes_collapsed
            };
            // 侧栏恢复为折叠窄条后不能再持有焦点：若右列主区也保持折叠，把焦点交给展开的右栏。
            if state.sidebar_collapsed && state.sidebar_focused() {
                if state.prompt_left && state.panes_collapsed {
                    if !state.prompt_collapsed {
                        select_right_tab(state, state.right_tab);
                    } else {
                        state.panes_collapsed = false;
                        focus_panes(state);
                    }
                } else {
                    focus_panes(state);
                }
            }
            clear_compact_transition_drags(state);
        }
        _ => {}
    }

    if compact {
        // 焦点是紧凑面板的唯一真源；每次同步都保证右栏 tab 与焦点一致。
        match state.focused {
            FocusedContainer::Prompt => state.right_tab = RightTab::Prompt,
            FocusedContainer::Todo => state.right_tab = RightTab::Todo,
            _ => {}
        }
    }
}

/// 跨 160/161 阈值进入或退出紧凑模式时清理进行中的拖拽/悬停，避免窄屏矩形写坏栏宽或阻塞释放。
fn clear_compact_transition_drags(state: &mut AppState) {
    super::selection::stop_active_selection_drags(state);
    state.resizing_sidebar = false;
    state.sidebar_hover = false;
    state.resizing_prompt = false;
    state.prompt_hover = false;
    state.resizing_pane = None;
    state.pane_hover = None;
    state.tooltip = None;
    state.workspace_drag = None;
    state.workspace_dragging = false;
    state.workspace_drag_moved = false;
    state.workspace_history.drag = None;
    state.workspace_history.dragging = false;
    state.workspace_history.drag_moved = false;
    state.workspace_history.scroll_drag = None;
    state.prompt_drag = None;
    state.todo_drag = None;
    state.workspace_scroll_drag = None;
    state.agent_scroll_drag = None;
    state.prompt_scroll_drag = None;
    state.prompt_recent_scroll_drag = None;
    state.todo_scroll_drag = None;
    state.terminal_scroll_drag = None;
    state.referenced_search_scroll_drag = None;
    state.referenced_menu_scroll_drag = None;
    state.connect_scroll_drag = None;
}

/// 返回 Chat：关闭 Prompt 瞬态面板、清空隐藏选区并把焦点交回主区。
pub fn back_to_compact_chat(state: &mut AppState) {
    super::selection::clear_selection(state);
    state.prompt.clear_panel();
    focus_panes(state);
}

/// 左栏按钮：未打开时聚焦侧栏，已打开时返回 Chat。
pub fn toggle_compact_left(state: &mut AppState) {
    if state.sidebar_focused() {
        back_to_compact_chat(state);
    } else {
        focus_sidebar(state);
    }
}

/// 右栏按钮：未打开时聚焦当前右栏页，已打开时返回 Chat。
pub fn toggle_compact_right(state: &mut AppState) {
    if state.prompt_focused() || state.todo_focused() {
        back_to_compact_chat(state);
    } else {
        super::selection::clear_selection(state);
        select_right_tab(state, state.right_tab);
    }
}

#[cfg(test)]
mod tests;
