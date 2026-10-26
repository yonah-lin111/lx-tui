//! 提示词分页行为：切页、页码菜单、新建/重命名/删除与当前页持久化。
//!
//! 所有离开当前正文的路径共享同一结算顺序：先提交编辑缓冲到当前页，
//! 再关闭正文相关面板/选区，最后切换当前页 id 并载入目标正文。

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::overlay::{ConfirmClose, Overlay, OverlayTarget, PageMenu, PageMenuRow};
use crate::app::prompt::{PromptEditing, PromptMode, PromptPage, PromptPageId, PromptPanel};
use crate::app::state::AppState;
use crate::ui::text;

use super::commit_editing_prompt;

/// 页码菜单行激活：新建、切页或删除当前页。
pub fn activate_page_menu_row(state: &mut AppState) -> bool {
    let Some(Overlay::PageMenu(menu)) = state.overlay.as_ref() else {
        return false;
    };
    let Some(row) = menu.rows.get(menu.selected).cloned() else {
        return false;
    };
    match row {
        PageMenuRow::NewPage => open_new_page(state),
        PageMenuRow::Page { page, .. } => {
            state.overlay = None;
            select_prompt_page(state, page)
        }
        PageMenuRow::DeletePage { enabled: true } => delete_current_page(state),
        PageMenuRow::DeletePage { enabled: false } => false,
    }
}

/// 页码菜单上下移动高亮；越界钳制。
pub fn move_page_menu_selection(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::PageMenu(menu)) = state.overlay.as_mut() else {
        return false;
    };
    let len = menu.rows.len();
    if len == 0 {
        return false;
    }
    let next = (menu.selected as isize + delta).clamp(0, len as isize - 1) as usize;
    if next == menu.selected && state.menu_hover {
        return false;
    }
    menu.selected = next;
    // 键盘移动同样显示高亮反馈。
    state.menu_hover = true;
    true
}

/// 悬停/点击选中页码菜单行；越界忽略。
pub fn set_page_menu_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::PageMenu(menu)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= menu.rows.len() {
        return false;
    }
    let changed = menu.selected != index || !state.menu_hover;
    menu.selected = index;
    state.menu_hover = true;
    changed
}

/// 打开页码菜单：先结算当前页，再按内存最新正文生成逐页统计快照。
pub fn open_prompt_page_menu(state: &mut AppState, anchor: (u16, u16)) -> bool {
    finish_page_rename(state);
    let workspace = state.prompt_workspace();
    let Some(target) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.editing)
    else {
        return false;
    };
    commit_editing_prompt(state);
    let Some(panel) = state
        .workspaces
        .get(workspace)
        .map(|workspace| &workspace.prompt_panel)
    else {
        return false;
    };
    let Some((pages, active)) = editing_pages(panel) else {
        return false;
    };
    let mut rows = vec![PageMenuRow::NewPage];
    let mut selected = 0;
    for page in pages {
        let current = page.id == active;
        if current {
            selected = rows.len();
        }
        rows.push(PageMenuRow::Page {
            page: page.id,
            current,
            empty: page.content.trim().is_empty(),
            counts: crate::app::markdown::template_counts(&page.content),
        });
    }
    rows.push(PageMenuRow::DeletePage {
        enabled: pages.len() > 1,
    });
    state.overlay = Some(Overlay::PageMenu(PageMenu {
        anchor,
        workspace,
        target,
        rows,
        selected,
    }));
    // 菜单锚定在点击处：指针位于容器内，初始显示反馈。
    state.menu_hover = true;
    true
}

/// 相对切页（标题行箭头）；越界或无法切页返回 false。
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

/// 新建页：正式条目走 DB 效应（成功后载入）；临时提示词直接追加并进入命名。
fn open_new_page(state: &mut AppState) -> bool {
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
                panel.start_page_rename(page, &name);
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

/// 服务回调：正式条目新页已落库并重载；立即切换并进入默认名编辑。
pub fn open_created_page(
    state: &mut AppState,
    workspace: usize,
    item_id: i64,
    page_id: i64,
    name: &str,
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
        panel.cancel_page_rename();
        panel.start_page_rename(page, name);
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

/// 服务回调：页删除已落库并重载；把编辑缓冲切换到新的当前页。
pub fn rebind_after_page_delete(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    let editing = state.workspaces.get(workspace).is_some_and(|workspace| {
        workspace.prompt_panel.editing == Some(PromptEditing::Item(item_id))
            && workspace.prompt_panel.mode == PromptMode::Editor
    });
    if !editing {
        return false;
    }
    state.prompt.clear_panel();
    state.selection = None;
    reload_editor_from_active_page(state, workspace)
}

/// 当前页名控件：进入预填页名的重命名输入态。
pub fn begin_page_rename(state: &mut AppState) -> bool {
    let workspace = state.prompt_workspace();
    let Some((name, page)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| editing_pages(&workspace.prompt_panel))
        .and_then(|(pages, active)| {
            pages
                .iter()
                .find(|page| page.id == active)
                .map(|page| (page.name.clone(), page.id))
        })
    else {
        return false;
    };
    commit_editing_prompt(state);
    with_panel(state, workspace, |panel| {
        panel.start_page_rename(page, &name);
        true
    })
}

/// 显示中面板是否存在页重命名输入态。
pub fn page_rename_active(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| workspace.prompt_panel.page_rename().is_some())
}

/// 页重命名按键：Enter 提交（空名拒绝并保持输入态）、Esc 保留旧名，其余为单行编辑。
pub fn apply_page_rename_key(state: &mut AppState, key: OverlayKey) {
    match key {
        OverlayKey::Esc => cancel_page_rename(state),
        OverlayKey::Enter => {
            let empty = state
                .workspaces
                .get(state.prompt_workspace())
                .and_then(|workspace| workspace.prompt_panel.page_rename())
                .is_some_and(|rename| rename.input.text().trim().is_empty());
            if !empty {
                finish_page_rename(state);
            }
        }
        OverlayKey::Char(_)
        | OverlayKey::Clear
        | OverlayKey::Backspace
        | OverlayKey::Delete
        | OverlayKey::Left
        | OverlayKey::Right
        | OverlayKey::Home
        | OverlayKey::End => edit_page_rename_input(state, key),
        OverlayKey::Up
        | OverlayKey::Down
        | OverlayKey::EnterFolder
        | OverlayKey::Back
        | OverlayKey::Copy
        | OverlayKey::Remove
        | OverlayKey::PageUp
        | OverlayKey::PageDown => {}
    }
}

/// 页重命名字符输入。
fn edit_page_rename_input(state: &mut AppState, key: OverlayKey) {
    let workspace = state.prompt_workspace();
    let Some(rename) = state
        .workspaces
        .get_mut(workspace)
        .and_then(|workspace| workspace.prompt_panel.page_rename_mut())
    else {
        return;
    };
    let input = &mut rename.input;
    match key {
        OverlayKey::Char(ch) => input.insert_char(ch),
        OverlayKey::Clear => input.clear(),
        OverlayKey::Backspace => input.backspace(),
        OverlayKey::Delete => input.delete(),
        OverlayKey::Left => input.move_left(),
        OverlayKey::Right => input.move_right(),
        OverlayKey::Home => input.move_home(),
        OverlayKey::End => input.move_end(),
        _ => {}
    }
}

/// 取消页重命名输入态（保留旧名）。
pub fn cancel_page_rename(state: &mut AppState) {
    let workspace = state.prompt_workspace();
    with_panel(state, workspace, |panel| {
        panel.cancel_page_rename();
        false
    });
}

/// 结束页重命名：非空输入写入（正式页登记 DB 效应），空名保留旧名。
pub fn finish_page_rename(state: &mut AppState) -> bool {
    let workspace = state.prompt_workspace();
    let Some(rename) = state
        .workspaces
        .get_mut(workspace)
        .and_then(|workspace| workspace.prompt_panel.take_page_rename())
    else {
        return false;
    };
    let page = rename.page;
    let name = rename.input.text().trim().to_string();
    if name.is_empty() {
        return false;
    }
    let renamed = with_panel(state, workspace, |panel| match panel.editing {
        Some(PromptEditing::Item(id)) => panel
            .tree
            .item_mut(id)
            .and_then(|item| item.page_mut(page))
            .map(|page| page.name.clone_from(&name))
            .is_some(),
        Some(PromptEditing::Temp(id)) => panel
            .temp_mut(id)
            .and_then(|temp| temp.page_mut(page))
            .map(|page| page.name.clone_from(&name))
            .is_some(),
        None => false,
    });
    if !renamed {
        return false;
    }
    if let (Some(page_id), Some(cwd)) = (page.saved(), workspace_cwd(state, workspace)) {
        state.effects.push(Effect::RenamePage {
            workspace: cwd,
            page_id,
            name,
        });
    }
    true
}

/// 删除当前页：含内容页先二次确认，空白页直接删除；最后一页不可删除。
pub fn delete_current_page(state: &mut AppState) -> bool {
    let workspace = state.prompt_workspace();
    let Some(panel) = state
        .workspaces
        .get(workspace)
        .map(|workspace| &workspace.prompt_panel)
    else {
        return false;
    };
    let Some(target) = panel.editing else {
        return false;
    };
    let Some((pages, active)) = editing_pages(panel) else {
        return false;
    };
    if pages.len() <= 1 {
        return false;
    }
    let Some(page) = pages.iter().find(|page| page.id == active).cloned() else {
        return false;
    };
    state.overlay = None;
    if !page.content.trim().is_empty() {
        state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
            target: OverlayTarget::PromptPage {
                workspace,
                owner: target,
                page: page.id,
            },
        }));
        return true;
    }
    perform_delete_page(state, workspace, target, page.id)
}

/// 确认浮层回调：执行页删除。
pub fn confirm_delete_page(
    state: &mut AppState,
    workspace: usize,
    owner: PromptEditing,
    page: PromptPageId,
) -> bool {
    state.overlay = None;
    perform_delete_page(state, workspace, owner, page)
}

/// 删除页的内存变更与持久化效应；删除后优先下一页，末页删除后前一页。
fn perform_delete_page(
    state: &mut AppState,
    workspace: usize,
    owner: PromptEditing,
    page: PromptPageId,
) -> bool {
    let deleted = with_panel(state, workspace, |panel| {
        let pages = match owner {
            PromptEditing::Item(id) => panel.tree.item_mut(id).map(|item| &mut item.pages),
            PromptEditing::Temp(id) => panel.temp_mut(id).map(|temp| &mut temp.pages),
        };
        let Some(pages) = pages else {
            return false;
        };
        let Some(index) = pages.iter().position(|candidate| candidate.id == page) else {
            return false;
        };
        if pages.len() <= 1 {
            return false;
        }
        pages.remove(index);
        let next = pages
            .get(index)
            .or_else(|| pages.last())
            .map(|page| page.id);
        let Some(next) = next else {
            return false;
        };
        match owner {
            PromptEditing::Item(id) => {
                if let Some(item) = panel.tree.item_mut(id) {
                    item.active_page = next;
                }
            }
            PromptEditing::Temp(id) => {
                if let Some(temp) = panel.temp_mut(id) {
                    temp.active_page = next;
                }
            }
        }
        if panel
            .page_rename()
            .is_some_and(|rename| rename.page == page)
        {
            panel.cancel_page_rename();
        }
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
    state.prompt.clear_panel();
    state.selection = None;
    reload_editor_from_active_page(state, workspace);
    true
}

/// 用编辑目标的当前页内容重装编辑缓冲（切换/删除/新建后）。
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
        if panel.mode != PromptMode::Editor {
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
    state.prompt.exit_block_focus();
    state.prompt.set_text(&content);
    true
}

/// 编辑目标的页集合与当前页 id。
pub(super) fn editing_pages(panel: &PromptPanel) -> Option<(&[PromptPage], PromptPageId)> {
    match panel.editing? {
        PromptEditing::Item(id) => panel
            .tree
            .item(id)
            .map(|item| (item.pages.as_slice(), item.active_page)),
        PromptEditing::Temp(id) => panel
            .temp(id)
            .map(|temp| (temp.pages.as_slice(), temp.active_page)),
    }
}

/// 显示中面板的编辑目标页集合与当前页 id。
fn displayed_prompt_pages(state: &AppState) -> Option<(&[PromptPage], PromptPageId)> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    editing_pages(panel)
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
fn workspace_cwd(state: &AppState, workspace: usize) -> Option<std::path::PathBuf> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.cwd.clone())
}
