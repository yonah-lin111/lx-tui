//! 提示词分页行为：切页、新建/删除与当前页持久化。
//!
//! 所有离开当前正文的路径共享同一结算顺序：先提交编辑缓冲到当前页，
//! 再关闭正文相关面板/选区，最后切换当前页 id 并载入目标正文。

use crate::app::effects::Effect;
use crate::app::prompt::{
    PromptEditing, PromptMode, PromptPage, PromptPageId, PromptPanel, page_rows,
};
use crate::app::state::AppState;
use crate::ui::text;

use super::pages_view::{clamp_page_view, hide_prompt_pages};
use super::{commit_editing_prompt, prompt_tree_visible_rows};

/// 相对切页（标题行箭头与 `Ctrl/Cmd+←/→`）；越界或无法切页返回 false。
pub fn switch_prompt_page(state: &mut AppState, delta: isize) -> bool {
    let Some((pages, active)) = displayed_prompt_pages(state) else {
        return false;
    };
    let index = pages.iter().position(|page| page.id == active).unwrap_or(0);
    let target = index as isize + delta;
    if target < 0 || target as usize >= pages.len() {
        return false;
    }
    let page = pages[target as usize].id;
    select_prompt_page(state, page)
}

/// 切到指定页：结算旧页 → 关闭正文面板/选区 → 改写当前页 → 载入目标正文。
pub fn select_prompt_page(state: &mut AppState, page: PromptPageId) -> bool {
    let workspace = state.prompt_workspace();
    let Some((pages, active)) = displayed_prompt_pages(state) else {
        return false;
    };
    if active == page || !pages.iter().any(|candidate| candidate.id == page) {
        return false;
    }
    commit_editing_prompt(state);
    state.prompt.clear_panel();
    state.selection = None;
    let Some(target) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.editing)
    else {
        return false;
    };
    let switched = with_panel(state, workspace, |panel| {
        set_active_page(panel, workspace, target, page)
    });
    if !switched {
        return false;
    }
    if let (Some(id), Some(page_id), Some(cwd)) = (
        item_id(target),
        page.saved(),
        workspace_cwd(state, workspace),
    ) {
        state.effects.push(Effect::SetActivePage {
            workspace: cwd,
            id,
            page_id,
        });
    }
    reload_editor_from_active_page(state, workspace);
    true
}

/// 在指定工作区面板上执行可变操作；工作区缺失返回 false。
fn with_panel(
    state: &mut AppState,
    workspace: usize,
    action: impl FnOnce(&mut PromptPanel) -> bool,
) -> bool {
    state
        .workspaces
        .get_mut(workspace)
        .is_some_and(|workspace| action(&mut workspace.prompt_panel))
}

/// 编辑目标中的正式条目 id；临时提示词为 None。
fn item_id(target: PromptEditing) -> Option<i64> {
    match target {
        PromptEditing::Item(id) => Some(id),
        PromptEditing::Temp(_) => None,
    }
}

/// 把编辑目标的当前页改写为 `page`。
fn set_active_page(
    panel: &mut PromptPanel,
    _workspace: usize,
    target: PromptEditing,
    page: PromptPageId,
) -> bool {
    match target {
        PromptEditing::Item(id) => {
            let Some(item) = panel.tree.item_mut(id) else {
                return false;
            };
            if item.page(page).is_none() {
                return false;
            }
            item.active_page = page;
            true
        }
        PromptEditing::Temp(id) => {
            let Some(temp) = panel.temp_mut(id) else {
                return false;
            };
            if temp.pages.iter().all(|candidate| candidate.id != page) {
                return false;
            }
            temp.active_page = page;
            true
        }
    }
}

/// 新建页：先回到编辑视图；正式条目走 DB 效应（成功后载入）；临时提示词直接末尾追加并切到新页。
pub fn open_new_page(state: &mut AppState) -> bool {
    hide_prompt_pages(state);
    let workspace = state.prompt_workspace();
    let Some(target) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.editing)
    else {
        return false;
    };
    commit_editing_prompt(state);
    state.prompt.clear_panel();
    state.selection = None;
    state.overlay = None;
    match target {
        PromptEditing::Item(id) => {
            let Some(name) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.prompt_panel.tree.item(id))
                .map(|item| next_default_page_name(&item.pages))
            else {
                return false;
            };
            let Some(cwd) = workspace_cwd(state, workspace) else {
                return false;
            };
            state.effects.push(Effect::CreatePage {
                workspace: cwd,
                id,
                name,
            });
            true
        }
        PromptEditing::Temp(id) => {
            let Some(name) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.prompt_panel.temp(id))
                .map(|temp| next_default_page_name(&temp.pages))
            else {
                return false;
            };
            let added = with_panel(state, workspace, |panel| {
                let page_id = panel.alloc_temp_page_id();
                let page = PromptPageId::Temp(page_id);
                let Some(temp) = panel.temp_mut(id) else {
                    return false;
                };
                temp.pages.push(PromptPage::temp(page_id, &name, ""));
                temp.active_page = page;
                panel.mode = PromptMode::Editor;
                true
            });
            if !added {
                return false;
            }
            reload_editor_from_active_page(state, workspace);
            true
        }
    }
}

/// 服务回调：正式条目新页已落库并重载；立即切换到该页并进入编辑视图。
pub fn open_created_page(
    state: &mut AppState,
    workspace: usize,
    item_id: i64,
    page_id: i64,
    _name: &str,
) -> bool {
    let page = PromptPageId::Saved(page_id);
    let opened = with_panel(state, workspace, |panel| {
        let Some(item) = panel.tree.item_mut(item_id) else {
            return false;
        };
        if item.page(page).is_none() {
            return false;
        }
        item.active_page = page;
        panel.mode = PromptMode::Editor;
        true
    });
    if !opened {
        return false;
    }
    if workspace == state.prompt_workspace() {
        reload_editor_from_active_page(state, workspace);
        state.prompt_collapsed = false;
        super::super::focus_prompt(state);
    }
    true
}

/// 服务回调：临时草稿升格后，按保存时的当前页索引绑定新条目。
pub fn open_saved_draft(
    state: &mut AppState,
    workspace: usize,
    item_id: i64,
    active: usize,
) -> bool {
    let Some((page, cwd)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| {
            workspace.prompt_panel.tree.item(item_id).map(|item| {
                (
                    item.pages.get(active).map(|page| page.id),
                    workspace.cwd.clone(),
                )
            })
        })
        .and_then(|(page, cwd)| page.map(|page| (page, cwd)))
    else {
        return false;
    };
    let assigned = with_panel(state, workspace, |panel| {
        panel
            .tree
            .item_mut(item_id)
            .map(|item| item.active_page = page)
            .is_some()
    });
    if !assigned {
        return false;
    }
    if let (Some(page_id), Some(cwd)) = (page.saved(), cwd) {
        state.effects.push(Effect::SetActivePage {
            workspace: cwd,
            id: item_id,
            page_id,
        });
    }
    super::edit_prompt_item(state, workspace, item_id)
}

/// 服务回调：页删除已落库并重载；若当前编辑页正文已变化则同步编辑缓冲。
pub fn rebind_after_page_delete(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    let needs_reload = state
        .workspaces
        .get(workspace)
        .and_then(|ws| {
            let panel = &ws.prompt_panel;
            if panel.editing != Some(PromptEditing::Item(item_id)) || panel.mode == PromptMode::Tree
            {
                return None;
            }
            let (pages, active) = editing_pages(panel)?;
            let content = pages
                .iter()
                .find(|page| page.id == active)?
                .content
                .as_str();
            Some(state.prompt.text() != content)
        })
        .unwrap_or(false);
    if !needs_reload {
        return false;
    }
    state.prompt.clear_panel();
    state.selection = None;
    reload_editor_from_active_page(state, workspace)
}

/// 删除页的内存变更与持久化效应；删除当前页时按「下一页优先，末页取上一页」接续并重载编辑缓冲，
/// 删除非当前页时保持 `active_page` 与编辑缓冲（含撤销栈）不变。
pub fn perform_delete_page(
    state: &mut AppState,
    workspace: usize,
    owner: PromptEditing,
    page: PromptPageId,
) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let mut was_active = false;
    let deleted = with_panel(state, workspace, |panel| {
        let (pages, active) = match owner {
            PromptEditing::Item(id) => {
                let Some(item) = panel.tree.item_mut(id) else {
                    return false;
                };
                (&mut item.pages, &mut item.active_page)
            }
            PromptEditing::Temp(id) => {
                let Some(temp) = panel.temp_mut(id) else {
                    return false;
                };
                (&mut temp.pages, &mut temp.active_page)
            }
        };
        let Some(index) = pages.iter().position(|candidate| candidate.id == page) else {
            return false;
        };
        if pages.len() <= 1 {
            return false;
        }
        was_active = *active == page;
        pages.remove(index);
        if was_active {
            let next = pages
                .get(index)
                .or_else(|| pages.last())
                .map(|candidate| candidate.id);
            let Some(next) = next else {
                return false;
            };
            *active = next;
        }
        let rows_len = panel
            .editing_pages()
            .map(|(pages, active)| page_rows(pages, active, panel.pages_view.search.text()).len())
            .unwrap_or(0);
        clamp_page_view(panel, rows_len, visible);
        true
    });
    if !deleted {
        return false;
    }
    if let (Some(item_id), Some(page_id), Some(cwd)) = (
        item_id(owner),
        page.saved(),
        workspace_cwd(state, workspace),
    ) {
        state.effects.push(Effect::DeletePage {
            workspace: cwd,
            id: item_id,
            page_id,
        });
    }
    if was_active {
        state.prompt.clear_panel();
        state.selection = None;
        reload_editor_from_active_page(state, workspace);
    }
    true
}

/// 用编辑目标的当前页内容重装编辑缓冲（切换/删除当前页/新建后；Pages 与 Editor 视图下均生效）。
pub fn reload_editor_from_active_page(state: &mut AppState, workspace: usize) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    let content = {
        let Some(panel) = state
            .workspaces
            .get(workspace)
            .map(|workspace| &workspace.prompt_panel)
        else {
            return false;
        };
        if panel.mode == PromptMode::Tree {
            return false;
        }
        editing_pages(panel).and_then(|(pages, active)| {
            pages
                .iter()
                .find(|page| page.id == active)
                .map(|page| page.content.clone())
        })
    };
    let Some(content) = content else {
        return false;
    };
    state.prompt.set_text(&content);
    true
}

/// 编辑目标的页集合与当前页 id。
pub(super) fn editing_pages(panel: &PromptPanel) -> Option<(&[PromptPage], PromptPageId)> {
    panel.editing_pages()
}

/// 显示中面板的编辑目标页集合与当前页 id。
fn displayed_prompt_pages(state: &AppState) -> Option<(&[PromptPage], PromptPageId)> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    panel.editing_pages()
}

/// 下一个未使用的默认页名 `Page N`。
pub(super) fn next_default_page_name(pages: &[PromptPage]) -> String {
    let mut index = 1;
    loop {
        let candidate = text::default_page_name(index);
        if pages.iter().all(|page| page.name != candidate) {
            return candidate;
        }
        index += 1;
    }
}

/// 工作区 cwd；工作区已关闭时为 None。
pub(super) fn workspace_cwd(state: &AppState, workspace: usize) -> Option<std::path::PathBuf> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.cwd.clone())
}
