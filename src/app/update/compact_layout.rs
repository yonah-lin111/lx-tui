//! 紧凑布局状态机：进入/退出快照、全屏切换器与返回聊天。

use crate::app::actions::OverlayKey;
use crate::app::overlay::{CompactSwitcher, Overlay};
use crate::app::state::{AppState, CompactRestore, CompactRow, FocusedContainer, RightTab};
use crate::config::Config;

use super::editor::focus_prompt;
use super::focus::{focus_pane, focus_panes};
use super::todo::focus_todo;
use super::workspace::{create_tab, open_new_workspace_dialog, switch_tab, switch_workspace};

/// 是否进入紧凑布局：宽度非零且不超过配置阈值。
pub fn is_compact(width: u16, config: &Config) -> bool {
    width > 0 && width <= config.narrow_width
}

/// 按终端尺寸同步紧凑布局状态；每轮事件循环在布局前调用。
///
/// 进入时快照桌面折叠状态并临时展开右列；退出时恢复。全屏面板与 Chat 之间
/// 的切换由焦点推导，不在此处另存状态。
pub fn sync_compact_layout(state: &mut AppState, width: u16, height: u16, config: &Config) {
    let compact = is_compact(width, config);
    match (compact, state.compact_restore) {
        (true, None) => {
            state.compact_restore = Some(CompactRestore {
                prompt_collapsed: state.prompt_collapsed,
                panes_collapsed: state.panes_collapsed,
            });
            state.prompt_collapsed = false;
            state.panes_collapsed = false;
            if state.sidebar_focused() {
                focus_panes(state);
            }
            // 焦点与右栏 tab 对齐，避免「焦点 Todo、画面 Prompt」的分裂。
        }
        (true, Some(_)) => {}
        (false, Some(restore)) => {
            state.compact_restore = None;
            // 焦点仍在全屏面板时保留展开，避免把正在编辑的面板重新折叠。
            if !matches!(
                state.focused,
                FocusedContainer::Prompt | FocusedContainer::Todo
            ) {
                state.prompt_collapsed = restore.prompt_collapsed;
            }
            state.panes_collapsed = restore.panes_collapsed;
            if matches!(state.overlay, Some(Overlay::CompactSwitcher(_))) {
                state.overlay = None;
            }
            state.compact_switcher_scroll_drag = None;
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

    // 切换器打开期间终端可能 resize：同步可视行数，避免选中项滚出视口。
    let viewport = switcher_viewport_rows(height);
    if let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_mut() {
        switcher.set_viewport(viewport);
    }
}

/// 切换器列表可视行数：整屏减去标题行与分隔线。
pub fn switcher_viewport_rows(screen_height: u16) -> usize {
    usize::from(screen_height.saturating_sub(2)).max(1)
}

/// 返回 Chat：关闭 Prompt 瞬态面板并把焦点交回主区。
pub fn back_to_compact_chat(state: &mut AppState) {
    state.prompt.clear_panel();
    focus_panes(state);
}

/// 打开全屏切换器；行快照在打开时生成。
pub fn open_compact_switcher(state: &mut AppState, viewport_rows: usize) {
    let rows = state.compact_rows();
    state.overlay = Some(Overlay::CompactSwitcher(CompactSwitcher::new(
        rows,
        viewport_rows,
    )));
    state.compact_switcher_scroll_drag = None;
}

/// 关闭全屏切换器。
pub fn close_compact_switcher(state: &mut AppState) {
    if matches!(state.overlay, Some(Overlay::CompactSwitcher(_))) {
        state.overlay = None;
    }
    state.compact_switcher_scroll_drag = None;
}

/// 移动高亮；返回是否变化。
pub fn move_compact_switcher(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_mut() else {
        return false;
    };
    let before = switcher.selected;
    switcher.move_selection(delta);
    switcher.selected != before
}

/// 翻页；返回是否变化。
pub fn page_compact_switcher(state: &mut AppState, direction: isize) -> bool {
    let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_mut() else {
        return false;
    };
    let before = (switcher.selected, switcher.scroll);
    switcher.page(direction);
    (switcher.selected, switcher.scroll) != before
}

/// 滚轮滚动视口（不移动高亮）；返回是否变化。
pub fn scroll_compact_switcher(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_mut() else {
        return false;
    };
    let before = switcher.scroll;
    let max = switcher.max_scroll();
    switcher.scroll = if delta < 0 {
        switcher.scroll.saturating_sub(delta.unsigned_abs())
    } else {
        switcher
            .scroll
            .saturating_add(delta.unsigned_abs())
            .min(max)
    };
    switcher.scroll != before
}

/// 设置视口顶部行号（滚动条轨道点击）；返回是否变化。
pub fn set_compact_switcher_scroll(state: &mut AppState, offset: usize) -> bool {
    let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_mut() else {
        return false;
    };
    let offset = offset.min(switcher.max_scroll());
    if switcher.scroll == offset {
        return false;
    }
    switcher.scroll = offset;
    true
}

/// 设置鼠标悬停行；返回是否变化。
pub fn hover_compact_switcher(state: &mut AppState, row: Option<usize>) -> bool {
    let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_mut() else {
        return false;
    };
    if switcher.hover == row {
        return false;
    }
    switcher.hover = row;
    true
}

/// 选中并激活指定行；分区标题忽略。
pub fn select_compact_switcher_row(state: &mut AppState, index: usize) {
    let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_mut() else {
        return;
    };
    if !switcher
        .rows
        .get(index)
        .is_some_and(CompactRow::is_selectable)
    {
        return;
    }
    switcher.selected = index;
    activate_compact_switcher(state);
}

/// 激活当前高亮行并关闭切换器。
pub fn activate_compact_switcher(state: &mut AppState) {
    let row = match state.overlay.take() {
        Some(Overlay::CompactSwitcher(switcher)) => switcher.selected_row().cloned(),
        other => {
            state.overlay = other;
            return;
        }
    };
    state.compact_switcher_scroll_drag = None;
    match row {
        Some(CompactRow::Agent(agent)) => activate_agent(state, agent.workspace, agent.entry),
        Some(CompactRow::Workspace(index)) => switch_workspace(state, index),
        Some(CompactRow::Tab(index)) => switch_tab(state, index),
        Some(CompactRow::Panel(RightTab::Prompt)) => focus_prompt(state),
        Some(CompactRow::Panel(RightTab::Todo)) => {
            state.right_tab = RightTab::Todo;
            focus_todo(state);
        }
        Some(CompactRow::NewWorkspace) => open_new_workspace_dialog(state),
        Some(CompactRow::NewTab) => create_tab(state),
        Some(CompactRow::Section(_)) | None => {}
    }
}

/// 跳到指定工作区的 Agent 窗格；快照失效时只关闭切换器，不做半套切换。
fn activate_agent(
    state: &mut AppState,
    workspace: usize,
    entry: crate::app::state::SidebarAgentEntry,
) {
    let Some(target_workspace) = state.workspaces.get(workspace) else {
        return;
    };
    let Some(tab) = target_workspace.tabs.get(entry.tab_index) else {
        return;
    };
    if tab.pane(entry.pane_id).is_none() {
        return;
    }
    switch_workspace(state, workspace);
    switch_tab(state, entry.tab_index);
    focus_pane(state, entry.pane_id);
}

/// 切换器键盘：Esc/Up/Down/PageUp/PageDown/Enter，其余忽略。
pub fn apply_compact_switcher_key(state: &mut AppState, key: OverlayKey) -> bool {
    match key {
        OverlayKey::Esc => {
            close_compact_switcher(state);
            true
        }
        OverlayKey::Up => move_compact_switcher(state, -1),
        OverlayKey::Down => move_compact_switcher(state, 1),
        OverlayKey::PageUp => page_compact_switcher(state, -1),
        OverlayKey::PageDown => page_compact_switcher(state, 1),
        OverlayKey::Enter => {
            activate_compact_switcher(state);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
