//! 侧栏 Agents 标题编辑与 `Open prompt` 行为。
//!
//! 标题只接受 Recent 页选择或自定义输入：不再跟随 lx 会话标题或终端 OSC 标题，
//! 绑定页也只在打开时保存页标题快照，后续改名不自动跟随。

use crate::app::actions::OverlayKey;
use crate::app::overlay::{AgentTitleDialog, Overlay, TextInput};
use crate::app::state::{AgentPromptRef, AgentTitleOverride, AppState, SidebarAgentKind};
use crate::tiling::PaneId;

use super::menu::close_overlay;
use super::prompt::prompt_recent_rows_for_workspace;

/// 打开 Agents 标题编辑弹窗：预填当前标题与绑定，并快照目标工作区的 Recent 页。
pub fn open_agent_title_dialog(
    state: &mut AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
    kind: SidebarAgentKind,
) {
    let current = state
        .workspaces
        .get(workspace)
        .and_then(|workspace_state| workspace_state.tabs.get(tab))
        .and_then(|tab_state| tab_state.pane(pane))
        .and_then(|pane_state| pane_state.agent_title_override(kind))
        .cloned();
    let (input, binding) = match current {
        Some(title) => (TextInput::new(title.text), title.prompt),
        None => (TextInput::new(String::new()), None),
    };
    let rows = prompt_recent_rows_for_workspace(state, workspace);
    state.overlay = Some(Overlay::AgentTitle(AgentTitleDialog::new(
        workspace, tab, pane, kind, input, binding, rows,
    )));
}

/// Agents 标题当前绑定的提示词页；未绑定、条目或页已删除时返回 None。
pub fn agent_title_prompt(
    state: &AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
    kind: SidebarAgentKind,
) -> Option<AgentPromptRef> {
    let prompt = state
        .workspaces
        .get(workspace)?
        .tabs
        .get(tab)?
        .pane(pane)?
        .agent_title_override(kind)?
        .prompt?;
    prompt_page_exists(state, workspace, prompt).then_some(prompt)
}

/// 取消 Recent 绑定并清空标题覆盖，第二行回到 `(no title)`。
pub fn remove_agent_title(
    state: &mut AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
    kind: SidebarAgentKind,
) {
    let Some(pane_state) = state
        .workspaces
        .get_mut(workspace)
        .and_then(|workspace_state| workspace_state.tabs.get_mut(tab))
        .and_then(|tab_state| tab_state.pane_mut(pane))
    else {
        return;
    };
    pane_state.set_agent_title_override(kind, None);
}

/// 页是否仍存在于指定工作区的提示词树中。
fn prompt_page_exists(state: &AppState, workspace: usize, prompt: AgentPromptRef) -> bool {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace_state| workspace_state.prompt_panel.tree.item(prompt.item_id))
        .is_some_and(|item| item.page(prompt.page_id).is_some())
}

/// 标题弹窗按键：上下浏览 Recent 并回填，直接编辑转自定义，Enter 提交，Esc 取消。
pub fn apply_agent_title_key(state: &mut AppState, key: OverlayKey) {
    match key {
        OverlayKey::Esc => close_overlay(state),
        OverlayKey::Enter => commit_agent_title(state),
        OverlayKey::Up => {
            move_agent_title_selection(state, -1);
        }
        OverlayKey::Down => {
            move_agent_title_selection(state, 1);
        }
        _ => {
            let Some(Overlay::AgentTitle(dialog)) = state.overlay.as_mut() else {
                return;
            };
            let previous = dialog.input.text().to_string();
            if dialog.input.apply_edit_key(key) && dialog.input.text() != previous {
                dialog.mark_custom();
            }
        }
    }
}

/// 上下移动 Recent 高亮；选中行会回填标题并建立绑定。返回是否变化。
pub fn move_agent_title_selection(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::AgentTitle(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    let len = dialog.rows.len();
    if len == 0 {
        return false;
    }
    let next = match (dialog.selected, delta.is_negative()) {
        (Some(index), true) => index.saturating_sub(1),
        (Some(index), false) => index.saturating_add(1).min(len - 1),
        (None, true) => len - 1,
        (None, false) => 0,
    };
    if dialog.selected == Some(next) {
        return false;
    }
    dialog.select_row(next);
    true
}

/// 鼠标点选 Recent 行：与键盘上下选择同义（回填标题并绑定）。返回是否变化。
pub fn set_agent_title_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::AgentTitle(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    if dialog.selected == Some(index) {
        return false;
    }
    let changed = dialog.rows.get(index).is_some();
    dialog.select_row(index);
    changed
}

/// 清空标题输入与绑定（弹窗保持打开）。
pub fn clear_agent_title_input(state: &mut AppState) -> bool {
    let Some(Overlay::AgentTitle(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    dialog.clear_input();
    true
}

/// 提交标题：空输入清除覆盖；有效绑定保留 Recent 页引用，否则存为自定义标题。
pub fn commit_agent_title(state: &mut AppState) {
    let Some(Overlay::AgentTitle(dialog)) = state.overlay.take() else {
        return;
    };
    let text = dialog.input.text().trim().to_string();
    let binding = dialog
        .binding
        .filter(|prompt| prompt_page_exists(state, dialog.workspace, *prompt));
    let title = (!text.is_empty()).then_some(AgentTitleOverride {
        text,
        prompt: binding,
    });
    if let Some(pane_state) = state
        .workspaces
        .get_mut(dialog.workspace)
        .and_then(|workspace_state| workspace_state.tabs.get_mut(dialog.tab))
        .and_then(|tab_state| tab_state.pane_mut(dialog.pane))
    {
        pane_state.set_agent_title_override(dialog.kind, title);
    }
}

/// 在 Prompt 面板中打开当前 Agents 标题绑定的页并聚焦。
///
/// Prompt 面板若钉在别的工作区，先释放钉住；目标页失效时不做任何切换。
pub fn open_agent_prompt(
    state: &mut AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
    kind: SidebarAgentKind,
) -> bool {
    if state.active_workspace != workspace {
        return false;
    }
    let Some(prompt) = agent_title_prompt(state, workspace, tab, pane, kind) else {
        return false;
    };
    if state.prompt_pinned.is_some() {
        super::release_prompt_pin(state);
    }
    if state.prompt_workspace() != workspace {
        return false;
    }
    super::open_prompt_page(state, prompt.item_id, prompt.page_id)
}
