//! 最近修改页面抽屉行为：开关、键盘导航、滚动与跳转。

use crate::app::actions::EditorCommand;
use crate::app::effects::Effect;
use crate::app::prompt::{PromptEditing, PromptMode, RecentPageRow, recent_page_rows};
use crate::app::state::AppState;
use ratatui::layout::Rect;

use super::commit_editing_prompt;

/// 当前工作区的最近页面行（实时正文参与标题/状态刷新）。
pub fn prompt_recent_rows(state: &AppState) -> Vec<RecentPageRow> {
    let panel = &state.prompt_owner().prompt_panel;
    let live = if panel.mode == PromptMode::Tree {
        None
    } else {
        panel.editing.and_then(|editing| {
            let page = match editing {
                PromptEditing::Item(id) => panel.tree.item(id)?.active_page,
                PromptEditing::Temp(id) => panel.temp(id)?.active_page,
            };
            Some((editing, page, state.prompt.text()))
        })
    };
    recent_page_rows(panel, live, crate::platform::current_unix_secs())
}

/// 是否已展开 recent 抽屉（键盘将由抽屉接管）。
pub fn prompt_recent_open(state: &AppState) -> bool {
    state.prompt_owner().prompt_panel.recent_open
}

/// 默认键盘选中项：当前编辑页在最近列表中时选它，否则选第一条。
fn default_recent_index(state: &AppState) -> Option<usize> {
    let rows = prompt_recent_rows(state);
    if rows.is_empty() {
        return None;
    }
    let panel = &state.prompt_owner().prompt_panel;
    let Some(PromptEditing::Item(item_id)) = panel.editing else {
        return Some(0);
    };
    let page = panel.tree.item(item_id).map(|item| item.active_page);
    Some(
        page.and_then(|page| {
            rows.iter()
                .position(|row| row.item_id == item_id && row.page_id == page)
        })
        .unwrap_or(0),
    )
}

/// 开关底部最近修改页面抽屉；展开时默认选中当前页，收起时清空键盘选中。
/// 切换后请求按抽屉高度重算编辑器视口。
pub fn toggle_prompt_recent(state: &mut AppState) -> bool {
    if prompt_recent_open(state) {
        return close_prompt_recent(state);
    }
    let active = default_recent_index(state);
    let workspace = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut workspace_state.prompt_panel;
    panel.recent_open = true;
    panel.recent_scroll = active.map(|index| index.saturating_sub(1)).unwrap_or(0);
    panel.recent_active = active;
    state.prompt.clear_panel();
    state.prompt_recent_hover = None;
    state.prompt_resize_pending = true;
    true
}

/// 收起 recent 抽屉并把键盘交还上方视图。
pub fn close_prompt_recent(state: &mut AppState) -> bool {
    let workspace = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut workspace_state.prompt_panel;
    if !panel.recent_open {
        return false;
    }
    panel.recent_open = false;
    panel.recent_scroll = 0;
    panel.recent_active = None;
    panel.recent_visible = 0;
    state.prompt_recent_hover = None;
    state.prompt_resize_pending = true;
    true
}

/// 单击抽屉条目：结算当前编辑，切换对应条目 + 页，随后收起抽屉。
pub fn open_recent_page(state: &mut AppState, index: usize) -> bool {
    let workspace = state.prompt_workspace();
    let Some(target) = prompt_recent_rows(state).get(index).cloned() else {
        return false;
    };
    let Some(page_id) = target.page_id.saved() else {
        return false;
    };
    if state.workspaces.get(workspace).is_some_and(|workspace| {
        let panel = &workspace.prompt_panel;
        panel.editing == Some(PromptEditing::Item(target.item_id))
            && panel
                .tree
                .item(target.item_id)
                .is_some_and(|item| item.active_page == target.page_id)
    }) {
        // 已经是当前页：不重载正文，只收起抽屉。
        state.prompt_recent_hover = None;
        close_prompt_recent(state);
        return true;
    }
    // 先把当前编辑写入内存/登记保存，避免切换后丢字。
    commit_editing_prompt(state);
    let Some((content, page)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(target.item_id))
        .and_then(|item| {
            let page = item.page(target.page_id)?;
            Some((page.content.clone(), page.id))
        })
    else {
        return false;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let cwd = workspace_state.cwd.clone();
    {
        let panel = &mut workspace_state.prompt_panel;
        let Some(item) = panel.tree.item_mut(target.item_id) else {
            return false;
        };
        item.active_page = page;
        panel.mode = PromptMode::Editor;
        panel.editing = Some(PromptEditing::Item(target.item_id));
    }
    if let Some(cwd) = cwd {
        state.effects.push(Effect::SetActivePage {
            workspace: cwd,
            id: target.item_id,
            page_id,
        });
    }
    super::load_editor_target(
        state,
        workspace,
        PromptEditing::Item(target.item_id),
        Some(page),
        &content,
    );
    state.prompt.clear_panel();
    state.selection = None;
    state.prompt_collapsed = false;
    crate::app::update::focus_prompt(state);
    state.prompt_recent_hover = None;
    close_prompt_recent(state);
    true
}

/// recent 展开时的键盘接管：↑/↓ 选择、Enter 打开、Esc/Ctrl+R 收起，其余键吞掉。
pub fn apply_prompt_recent_key(state: &mut AppState, command: &EditorCommand, panel: Rect) -> bool {
    match command {
        EditorCommand::ToggleRecentPages | EditorCommand::Escape => close_prompt_recent(state),
        EditorCommand::Up => move_prompt_recent(state, -1, panel),
        EditorCommand::Down => move_prompt_recent(state, 1, panel),
        EditorCommand::Newline => open_active_recent_page(state),
        _ => false,
    }
}

/// 在最近列表中上下移动键盘选中项，并让选中项保持可见。
fn move_prompt_recent(state: &mut AppState, direction: isize, panel: Rect) -> bool {
    let rows = prompt_recent_rows(state);
    if rows.is_empty() {
        return false;
    }
    let visible = crate::layout::prompt_body_split(panel, true, rows.len())
        .visible_entries
        .max(1);
    let hovered = state.prompt_recent_hover;
    let workspace = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel_state = &mut workspace_state.prompt_panel;
    let current = hovered
        .or(panel_state.recent_active)
        .unwrap_or(0)
        .min(rows.len() - 1);
    let next = (current as isize + direction).clamp(0, rows.len() as isize - 1) as usize;
    let scroll = keep_visible(panel_state.recent_scroll, next, visible, rows.len());
    let changed = panel_state.recent_active != Some(next) || panel_state.recent_scroll != scroll;
    panel_state.recent_active = Some(next);
    panel_state.recent_scroll = scroll;
    state.prompt_recent_hover = None;
    changed || hovered.is_some()
}

/// 打开键盘选中的最近页面（鼠标悬停时优先打开悬停项）。
fn open_active_recent_page(state: &mut AppState) -> bool {
    let rows = prompt_recent_rows(state);
    if rows.is_empty() {
        return false;
    }
    let panel = &state.prompt_owner().prompt_panel;
    let index = state
        .prompt_recent_hover
        .or(panel.recent_active)
        .unwrap_or(0)
        .min(rows.len() - 1);
    let opened = open_recent_page(state, index);
    state.prompt_recent_hover = None;
    opened
}

/// 保持 `index` 在 `scroll..scroll + visible` 窗口内，并钳制到合法滚动范围。
fn keep_visible(scroll: usize, index: usize, visible: usize, total: usize) -> usize {
    let max = total.saturating_sub(visible);
    let scroll = if index < scroll {
        index
    } else if index >= scroll.saturating_add(visible) {
        (index + 1).saturating_sub(visible)
    } else {
        scroll
    };
    scroll.min(max)
}

/// 滚轮/键盘滚动抽屉：以条目为单位，越界钳制；返回是否移动。
pub fn scroll_prompt_recent(
    state: &mut AppState,
    direction: isize,
    visible: usize,
    total: usize,
) -> bool {
    let max = total.saturating_sub(visible.max(1));
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let current = workspace.prompt_panel.recent_scroll.min(max);
    let next = (current as isize)
        .saturating_add(direction)
        .clamp(0, max as isize) as usize;
    if next == workspace.prompt_panel.recent_scroll {
        return false;
    }
    workspace.prompt_panel.recent_scroll = next;
    clamp_recent_active(&mut workspace.prompt_panel, next, visible, total);
    state.prompt_recent_hover = None;
    true
}

/// 直接设置抽屉滚动偏移（滚动条点击/拖拽）；返回是否变化。
pub fn set_prompt_recent_scroll(
    state: &mut AppState,
    offset: usize,
    visible: usize,
    total: usize,
) -> bool {
    let max = total.saturating_sub(visible.max(1));
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let next = offset.min(max);
    let moved = next != workspace.prompt_panel.recent_scroll;
    workspace.prompt_panel.recent_scroll = next;
    clamp_recent_active(&mut workspace.prompt_panel, next, visible, total);
    state.prompt_recent_hover = None;
    moved
}

/// 滚动后把键盘选中项收进当前可见窗口与有效行范围内，避免 Enter 打开屏幕外或越界项。
fn clamp_recent_active(
    panel: &mut crate::app::prompt::PromptPanel,
    scroll: usize,
    visible: usize,
    total: usize,
) {
    if total == 0 {
        panel.recent_active = None;
        return;
    }
    if let Some(active) = panel.recent_active {
        let last = scroll
            .saturating_add(visible.max(1).saturating_sub(1))
            .min(total - 1);
        let first = scroll.min(last);
        panel.recent_active = Some(active.clamp(first, last));
    }
}

/// 当前键盘选中条目对应的 `(item_id, page_id)`（抽屉展开时用于跨重排/删除保位）。
pub(crate) fn current_recent_target(
    state: &AppState,
) -> Option<(i64, crate::app::prompt::PromptPageId)> {
    let panel = &state.prompt_owner().prompt_panel;
    if !panel.recent_open {
        return None;
    }
    let rows = prompt_recent_rows(state);
    let index = panel.recent_active?;
    rows.get(index).map(|row| (row.item_id, row.page_id))
}

/// 在自动保存重排或条目/页面增删后同步 `recent_active` 与 `recent_scroll`。
pub(crate) fn sync_prompt_recent_selection(
    state: &mut AppState,
    target: Option<(i64, crate::app::prompt::PromptPageId)>,
) {
    if !prompt_recent_open(state) {
        return;
    }
    let rows = prompt_recent_rows(state);
    let workspace = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    let panel = &mut workspace_state.prompt_panel;
    if rows.is_empty() {
        panel.recent_active = None;
        panel.recent_scroll = 0;
        state.prompt_recent_hover = None;
        return;
    }
    let matched = target.and_then(|(item_id, page_id)| {
        rows.iter()
            .position(|row| row.item_id == item_id && row.page_id == page_id)
    });
    let next = matched
        .or(panel.recent_active)
        .unwrap_or(0)
        .min(rows.len() - 1);
    let visible = panel.recent_visible.max(1);
    panel.recent_active = Some(next);
    panel.recent_scroll = keep_visible(panel.recent_scroll, next, visible, rows.len());
    if state
        .prompt_recent_hover
        .is_some_and(|hover| hover >= rows.len())
    {
        state.prompt_recent_hover = None;
    }
}

/// 鼠标悬停抽屉条目：更新高亮；返回是否变化。
pub fn set_prompt_recent_hover(state: &mut AppState, hover: Option<usize>) -> bool {
    if state.prompt_recent_hover == hover {
        return false;
    }
    state.prompt_recent_hover = hover;
    true
}

#[cfg(test)]
mod tests;
