//! 提示词树行为：视图模式切换、选中导航、右键菜单与编辑提交。
//!
//! DB 读写不在此层执行：变更登记为 `Effect` 出箱，由持久化服务执行并回报。
//! 分页行为见 `prompt::pages`，草稿保存与新建节点浮层见 `prompt::draft`。

mod draft;
mod drag;
mod items;
mod menu;
mod pages;
mod pages_view;
mod recent;
mod save;
mod tree_view;

pub use draft::*;
pub use drag::*;
pub use items::*;
pub use menu::*;
pub use pages::*;
pub use pages_view::*;
pub use recent::*;
pub use save::*;
pub use tree_view::*;

use std::path::Path;

use crate::app::effects::Effect;
use crate::app::prompt::{
    MAX_RECENT_PAGES, PromptEditing, PromptFolder, PromptItem, PromptMode, PromptPageId, page_rows,
    recent_page_count,
};
use crate::app::state::AppState;
use crate::db::{FolderRecord, PromptRecord};

/// 用 DB 记录整体替换指定工作区的树（`key` 为该树对应的 prompt 键）；保持已有折叠状态。
///
/// 重载以数据库为正文真源，但不覆盖未确认页：内存中该页仍有待保存标记时保留内存正文，
/// 待保存回报后再确认；页已被别处删除的待保存标记一并清除（无法再重试）。
///
/// 同键另一面板改了“上次打开的页”时，本面板编辑缓冲先结算到旧页再重绑目标页，
/// 避免把旧页正文写进新页或把新页显示成旧内容。
pub fn apply_prompt_tree(
    state: &mut AppState,
    workspace: usize,
    key: &Path,
    folders: Vec<FolderRecord>,
    prompts: Vec<PromptRecord>,
) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let recorded: Vec<(i64, Option<i64>)> = prompts
        .iter()
        .map(|record| (record.id, record.active_page_id))
        .collect();
    let folders: Vec<PromptFolder> = folders.into_iter().map(PromptFolder::from).collect();
    let mut prompts: Vec<PromptItem> = prompts.into_iter().map(PromptItem::from).collect();
    let displayed = workspace == state.prompt_workspace();
    // 归属键变化（git 元数据晚到/工作区改路径）也走结算：旧树正文先落旧键。
    let key_changed = state
        .workspaces
        .get(workspace)
        .is_some_and(|workspace| workspace.prompt_panel.loaded_key.as_deref() != Some(key));
    let editing = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.editing);
    let old_active = editing.and_then(|editing| match editing {
        PromptEditing::Item(id) => state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.prompt_panel.tree.item(id))
            .map(|item| item.active_page),
        PromptEditing::Temp(_) => None,
    });
    let incoming_active = match editing {
        Some(PromptEditing::Item(id)) => prompts
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.active_page),
        _ => None,
    };
    let page_switched = old_active.is_some() && old_active != incoming_active;
    if displayed && (page_switched || key_changed) {
        commit_prompt_buffer(state, workspace);
    }
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    {
        let panel = &workspace_state.prompt_panel;
        let valid: Vec<(i64, i64)> = panel
            .pending_saves()
            .iter()
            .copied()
            .filter(|(item_id, page_id)| {
                panel
                    .tree
                    .item(*item_id)
                    .is_some_and(|item| item.page(PromptPageId::Saved(*page_id)).is_some())
            })
            .collect();
        for item in &mut prompts {
            let Some(old) = panel.tree.item(item.id) else {
                continue;
            };
            for page in &mut item.pages {
                let Some(page_id) = page.id.saved() else {
                    continue;
                };
                if !valid.contains(&(item.id, page_id)) {
                    continue;
                }
                if let Some(pending) = old.page(page.id) {
                    page.content.clone_from(&pending.content);
                }
            }
        }
    }
    workspace_state.prompt_panel.tree.replace(folders, prompts);
    let keep: Vec<(i64, i64)> = workspace_state
        .prompt_panel
        .pending_saves()
        .iter()
        .copied()
        .filter(|(item, page)| {
            workspace_state
                .prompt_panel
                .tree
                .item(*item)
                .is_some_and(|item| item.page(PromptPageId::Saved(*page)).is_some())
        })
        .collect();
    workspace_state.prompt_panel.retain_pending(&keep);
    workspace_state.prompt_panel.loaded = true;
    workspace_state.prompt_panel.loaded_key = Some(key.to_path_buf());
    // 编辑绑定的条目被同键面板删除时解除绑定，避免悬挂在已不存在的条目上。
    if let Some(PromptEditing::Item(id)) = workspace_state.prompt_panel.editing
        && workspace_state.prompt_panel.tree.item(id).is_none()
    {
        workspace_state.prompt_panel.editing = None;
        workspace_state.prompt_panel.mode = PromptMode::Tree;
    }
    workspace_state.prompt_panel.clamp_selection(visible);
    // 记录的当前页 id 失效时回退第一页并修复持久化（不依赖打开动作）。
    let mut repairs: Vec<(i64, i64)> = Vec::new();
    for (id, active) in recorded {
        let Some(item) = workspace_state.prompt_panel.tree.item(id) else {
            continue;
        };
        if active != item.active_page.saved()
            && let Some(first) = item.pages.first().and_then(|page| page.id.saved())
        {
            repairs.push((id, first));
        }
    }
    // 后台停泊工作区：同键面板切页或正文更新后刷新停泊缓冲，切回时才不会把
    // 旧页正文提交进新的当前页。页身份切换按换文档处理（光标文末、滚到光标），
    // 只有同一页的后台正文刷新才保留视口。
    if !displayed
        && let Some(PromptEditing::Item(id)) = workspace_state.prompt_panel.editing
        && let Some(item) = workspace_state.prompt_panel.tree.item(id)
    {
        let active_content = item.active_content().to_string();
        if let Some(draft) = state
            .workspaces
            .get_mut(workspace)
            .and_then(|workspace| workspace.prompt.as_mut())
        {
            if page_switched {
                draft.editor.set_text(&active_content);
                draft.editor.mark_saved();
            } else if draft.editor.is_saved() && draft.editor.text() != active_content.as_str() {
                draft.editor.replace_text_preserving_view(&active_content);
                draft.editor.mark_saved();
            }
        }
    }
    if displayed && page_switched {
        state.prompt.clear_panel();
        state.selection = None;
        reload_editor_from_active_page(state, workspace);
    }
    if let Some(cwd) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.cwd.clone())
    {
        for (id, page_id) in repairs {
            state.effects.push(Effect::SetActivePage {
                workspace: cwd.clone(),
                id,
                page_id,
            });
        }
    }
    if displayed {
        // 页数量可能变化：抽屉高度、选中下标与编辑器视口需要按新行集重算。
        let target = recent::current_recent_target(state);
        recent::sync_prompt_recent_selection(state, target);
        state.prompt_resize_pending = true;
    }
    true
}

/// 切换树视图：提交编辑内容、清除空白绑定、暂存编辑绑定并清空绑定；
/// 列表恒保留一条空白 default temp（缺失时补建），进入列表默认高亮当前编辑目标
/// （条目或临时提示词），无有效绑定时高亮 default temp。
pub fn show_prompt_tree(state: &mut AppState) {
    state.prompt_search_drag = false;
    commit_editing_prompt(state);
    let index = state.prompt_workspace();
    let visible = prompt_tree_visible_rows(state);
    // 提交后再捕获编辑状态：正文此时与页一致，返回同一目标时据此保留撤销/重做与光标。
    stash_editing_prompt(state, index);
    if let Some(workspace) = state.workspaces.get_mut(index) {
        workspace.prompt_panel.drop_blank_temp_binding();
        workspace.prompt_panel.enter_tree_view();
        workspace.prompt_panel.mode = PromptMode::Tree;
        workspace.prompt_panel.editing = None;
        workspace.prompt_panel.ensure_blank_temp();
        // 行集可能因 temp/分区标签变化：选中吸附到最近可选中行。
        workspace.prompt_panel.clamp_selection(visible);
    }
    state.prompt.clear_panel();
    state.selection = None;
    state.prompt_tree_hover = None;
    state.prompt_recent_hover = None;
    select_tree_entry_row(state, index);
}

/// 几何变化后按新的可见行数钳制 prompt 各列表的滚动范围。
///
/// 自动路径只钳制、不 reveal：树 / 页选中项可能留在视口外；Recent 只更新偏移上限。
/// 供 `resize_panes` 在真实尺寸变化后调用，避免偏移越界导致下一次导航跳位。
pub(crate) fn clamp_prompt_list_scroll(state: &mut AppState) {
    let index = state.prompt_workspace();
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return;
    };
    let panel = &mut workspace.prompt_panel;
    panel.clamp_selection(visible);
    // 树视图下没有编辑绑定（editing = None），页列表处于休眠态；
    // 此时不能按 0 行钳制，否则会把用户上次的页列表滚动清零。
    if let Some(page_len) = panel
        .editing_pages()
        .map(|(pages, active)| page_rows(pages, active, panel.pages_view.search.text()).len())
    {
        clamp_page_view(panel, page_len, visible);
    }
    if panel.recent_open {
        let recent_len = recent_page_count(panel).min(MAX_RECENT_PAGES);
        panel.recent_scroll = panel
            .recent_scroll
            .min(recent_len.saturating_sub(panel.recent_visible.max(1)));
    }
}

/// 离开编辑视图前暂存编辑状态：绑定目标、当前页与当前编辑器历史。
///
/// 须在提交缓冲之后调用，使暂存正文与页正文一致（返回时逐字比较成立）；
/// 无编辑绑定时不动作。
fn stash_editing_prompt(state: &mut AppState, index: usize) {
    let Some((editing, page)) = state.workspaces.get(index).and_then(|workspace| {
        let panel = &workspace.prompt_panel;
        let editing = panel.editing?;
        let page = match editing {
            PromptEditing::Item(id) => panel.tree.item(id)?.active_page,
            PromptEditing::Temp(id) => panel.temp(id)?.active_page,
        };
        Some((editing, page))
    }) else {
        return;
    };
    let history = state.prompt.history();
    if let Some(workspace) = state.workspaces.get_mut(index) {
        workspace.prompt_panel.stash_editor(editing, page, history);
    }
}

/// 进入列表后的默认高亮：优先当前编辑目标（暂存绑定仍有效时），否则 default temp。
fn select_tree_entry_row(state: &mut AppState, index: usize) {
    let target = state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.prompt_panel.tree_restore_target());
    match target {
        Some(PromptEditing::Item(id)) if select_prompt_item(state, index, id) => {}
        Some(PromptEditing::Temp(id)) if select_prompt_temp(state, index, id) => {}
        _ => {
            let blank = state
                .workspaces
                .get(index)
                .and_then(|workspace| workspace.prompt_panel.blank_temp());
            if let Some(temp_id) = blank {
                select_prompt_temp(state, index, temp_id);
            }
        }
    }
}

/// 工具栏模式按钮：树视图或页视图返回编辑，编辑视图切回树。
pub fn toggle_prompt_panel_mode(state: &mut AppState) {
    state.prompt_drag = None;
    state.prompt_search_drag = false;
    state.prompt_tree_hover = None;
    state.prompt_recent_hover = None;
    if prompt_tree_mode(state) {
        restore_or_edit_selected(state);
    } else if prompt_pages_mode(state) {
        hide_prompt_pages(state);
    } else {
        show_prompt_tree(state);
    }
}

/// 树视图返回编辑：用户未改过选中时恢复进入列表前的绑定（条目或临时提示词；
/// 绑定已失效/空白 temp 已回收时新建空临时提示词），改过选中时打开选中条目
/// （非条目回退新建临时提示词）。
fn restore_or_edit_selected(state: &mut AppState) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get(index) else {
        return false;
    };
    if workspace.prompt_panel.tree_selection_touched() {
        return edit_selected_prompt(state);
    }
    match workspace.prompt_panel.tree_restore_target() {
        Some(PromptEditing::Item(id)) => {
            // 恢复目标须在未过滤列表中可见：先清搜索，保证服务端选中成功。
            clear_search_for(state, index);
            edit_prompt_item(state, index, id)
        }
        Some(PromptEditing::Temp(id)) => edit_prompt_temp(state, index, id),
        None => open_temp_prompt(state),
    }
}

/// 清空指定工作区树视图的搜索词（不影响其余状态）。
pub(super) fn clear_search_for(state: &mut AppState, workspace: usize) {
    if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
        workspace_state.prompt_panel.search.clear();
    }
}

/// 显示中工作区的 cwd。
pub(super) fn displayed_workspace_cwd(state: &AppState) -> Option<std::path::PathBuf> {
    workspace_cwd(state, state.prompt_workspace())
}

/// 指定工作区的 cwd。
pub(super) fn workspace_cwd(state: &AppState, workspace: usize) -> Option<std::path::PathBuf> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.cwd.clone())
}

#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod tests;
