//! 最近修改页面抽屉行为：开关、滚动与单击跳转。

use crate::app::effects::Effect;
use crate::app::prompt::{PromptEditing, PromptMode, RecentPageRow, recent_page_rows};
use crate::app::state::AppState;

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

/// 开关底部最近修改页面抽屉；切换后请求按抽屉高度重算编辑器视口。
pub fn toggle_prompt_recent(state: &mut AppState) -> bool {
    let workspace = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let panel = &mut workspace_state.prompt_panel;
    panel.recent_open = !panel.recent_open;
    panel.recent_scroll = 0;
    state.prompt_recent_hover = None;
    state.prompt_resize_pending = true;
    true
}

/// 单击抽屉条目：结算当前编辑，切换对应条目 + 页并保持抽屉展开。
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
        // 已经是当前页：不动历史与光标。
        return false;
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
    true
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
    moved
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
