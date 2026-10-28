//! 提示词树行为：视图模式切换、选中导航、右键菜单与编辑提交。
//!
//! DB 读写不在此层执行：变更登记为 `Effect` 出箱，由持久化服务执行并回报。
//! 分页行为见 `prompt::pages`，草稿保存与新建节点浮层见 `prompt::draft`。

mod draft;
mod drag;
mod pages;
mod tree_view;

pub use draft::*;
pub use drag::*;
pub use pages::*;
pub use tree_view::*;

use std::path::Path;

use crate::app::effects::Effect;
use crate::app::overlay::{
    Menu, MenuCommand, Overlay, OverlayTarget, Rename, RenameTarget, TextInput,
};
use crate::app::prompt::{
    PromptEditing, PromptFolder, PromptItem, PromptMode, PromptPageId, PromptRow,
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
        workspace_state.prompt_panel.cancel_page_rename();
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
    true
}

/// 切换树视图：提交编辑内容、清除空白绑定、暂存编辑绑定并清空绑定；
/// 列表恒保留一条空白 default temp（缺失时补建），进入列表默认高亮当前编辑目标
/// （条目或临时提示词），无有效绑定时高亮 default temp。
pub fn show_prompt_tree(state: &mut AppState) {
    commit_editing_prompt(state);
    cancel_page_rename(state);
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
    select_tree_entry_row(state, index);
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

/// 工具栏块聚焦按钮：未进入则尝试进入（无模板块时 no-op），已进入则退出。
pub fn toggle_prompt_block_focus(state: &mut AppState) -> bool {
    if state.prompt.block_focus_active() {
        state.prompt.exit_block_focus();
        // 视口行集合切换后旧选区坐标失效，退出时一并清除。
        super::clear_selection(state);
        true
    } else if state.prompt.enter_block_focus() {
        super::clear_selection(state);
        true
    } else {
        false
    }
}

/// 工具栏模式按钮：树视图返回编辑（用户未显式改过选中时恢复进入列表前的编辑绑定），
/// 编辑视图切回树。
pub fn toggle_prompt_panel_mode(state: &mut AppState) {
    state.prompt_drag = None;
    if prompt_tree_mode(state) {
        restore_or_edit_selected(state);
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

/// 提交编辑视图内容到绑定目标：条目立即更新并登记 DB 保存；草稿更新内存槽位。
pub fn commit_editing_prompt(state: &mut AppState) {
    let index = state.prompt_workspace();
    commit_prompt_buffer(state, index);
}

/// 提交指定工作区的编辑器缓冲（其为显示态时缓冲位于 `state.prompt`）。
///
/// 提交写入编辑目标的当前页；正式条目按（条目 id、页 id）粒度标记待保存并登记
/// 携带页 id 与修订号的保存效应。空白页正文同样落库（区分空白页与未编辑页）。
///
/// 粘贴提案未收束时先取消提案回滚占位：占位只是待改写的中转态，取消无法逆向写库，
/// 因此占位文本绝不落库；随后的提交只落粘贴前的正文，切换编辑目标/退出不丢编辑。
pub fn commit_prompt_buffer(state: &mut AppState, workspace: usize) {
    state.prompt.paste_cancel();
    let Some(target) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.editing)
    else {
        return;
    };
    let content = state.prompt.text().to_string();
    let prompt = state.prompt.id();
    let revision = state.prompt.revision();
    match target {
        PromptEditing::Temp(id) => {
            let Some(temp) = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.prompt_panel.temp_mut(id))
            else {
                return;
            };
            if let Some(page) = temp.active_page_mut() {
                page.content.clone_from(&content);
            }
        }
        PromptEditing::Item(id) => {
            let Some(cwd) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.cwd.clone())
            else {
                return;
            };
            let Some(page_id) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.prompt_panel.tree.item(id))
                .map(|item| item.active_page)
                .and_then(PromptPageId::saved)
            else {
                return;
            };
            let pending = state
                .workspaces
                .get(workspace)
                .is_some_and(|workspace| workspace.prompt_panel.page_pending(id, page_id));
            {
                let Some(page) = state
                    .workspaces
                    .get_mut(workspace)
                    .and_then(|workspace| workspace.prompt_panel.tree.item_mut(id))
                    .and_then(|item| item.page_mut(PromptPageId::Saved(page_id)))
                else {
                    return;
                };
                if page.content == content && !pending {
                    // 内容与库中一致且无未确认保存：直接确认对应修订，避免空转自动保存。
                    state.prompt.confirm_saved(revision);
                    return;
                }
                page.content.clone_from(&content);
            }
            if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
                workspace_state.prompt_panel.mark_page_pending(id, page_id);
            }
            state.effects.push(Effect::SavePromptContent {
                workspace: cwd,
                id,
                page_id,
                content,
                prompt,
                revision,
            });
        }
    }
}

/// 进入指定条目的编辑视图并聚焦面板（服务在新建后调用；工作区须为显示中）。
///
/// 打开上次所在的页；记录页 id 失效时回退并修复为第一页（登记持久化效应）。
pub fn edit_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    if workspace != state.prompt_workspace() || !select_prompt_item(state, workspace, item_id) {
        return false;
    }
    let Some((content, repaired)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .map(|item| {
            let index = item.active_index();
            let repaired = item.pages.get(index).map(|page| page.id) != Some(item.active_page);
            (
                item.pages.get(index).map(|page| page.content.clone()),
                repaired,
            )
        })
    else {
        return false;
    };
    let Some(content) = content else {
        return false;
    };
    if repaired {
        let Some(page_id) = state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
            .and_then(|item| item.pages.first())
            .and_then(|page| page.id.saved())
        else {
            return false;
        };
        if let Some(workspace_state) = state.workspaces.get_mut(workspace)
            && let Some(item) = workspace_state.prompt_panel.tree.item_mut(item_id)
            && let Some(page) = item.pages.first()
        {
            item.active_page = page.id;
        }
        if let Some(cwd) = state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.cwd.clone())
        {
            state.effects.push(Effect::SetActivePage {
                workspace: cwd,
                id: item_id,
                page_id,
            });
        }
    }
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(PromptEditing::Item(item_id));
    workspace_state.prompt_panel.cancel_page_rename();
    let page = workspace_state
        .prompt_panel
        .tree
        .item(item_id)
        .map(|item| item.active_page);
    load_editor_target(
        state,
        workspace,
        PromptEditing::Item(item_id),
        page,
        &content,
    );
    true
}

/// 打开指定临时提示词的编辑视图并聚焦面板；内容只存内存，恢复该临时提示词的当前页。
pub fn edit_prompt_temp(state: &mut AppState, workspace: usize, temp_id: u64) -> bool {
    if workspace != state.prompt_workspace() {
        return false;
    }
    let Some((content, page)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.temp(temp_id))
        .and_then(|temp| {
            temp.active_page()
                .map(|page| (page.content.clone(), page.id))
        })
    else {
        return false;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(PromptEditing::Temp(temp_id));
    workspace_state.prompt_panel.cancel_page_rename();
    load_editor_target(
        state,
        workspace,
        PromptEditing::Temp(temp_id),
        Some(page),
        &content,
    );
    true
}

/// 载入编辑目标正文到编辑器缓冲：暂存的目标、页与正文均一致时恢复完整历史与光标，
/// 否则按正文重建（历史清空）。无论是否恢复都消费暂存，避免陈旧历史误用于新目标。
fn load_editor_target(
    state: &mut AppState,
    workspace: usize,
    editing: PromptEditing,
    page: Option<PromptPageId>,
    content: &str,
) {
    let restored = state
        .workspaces
        .get_mut(workspace)
        .and_then(|workspace| workspace.prompt_panel.take_editor_stash())
        .filter(|stash| {
            Some(stash.page) == page && stash.editing == editing && stash.history.text() == content
        })
        .map(|stash| stash.history);
    match restored {
        Some(history) => state.prompt.restore_history(history),
        None => {
            state.prompt.exit_block_focus();
            state.prompt.set_text(content);
        }
    }
    state.prompt_collapsed = false;
    super::focus_prompt(state);
}

/// 打开选中临时提示词行的编辑视图。
fn edit_prompt_temp_row(state: &mut AppState, workspace: usize, row: PromptRow) -> bool {
    let Some(temp_id) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.temp_of_row(row))
        .map(|temp| temp.id)
    else {
        return false;
    };
    edit_prompt_temp(state, workspace, temp_id)
}

/// 立即保存（工具栏圆点）：条目提交落库；临时提示词提交缓冲后打开保存到文件夹选择器。
pub fn save_prompt_now(state: &mut AppState) {
    // 粘贴提案未收束：占位文本不得落库，也不改已保存态（面板收束后再保存）。
    if state.prompt.paste_panel().is_some() {
        return;
    }
    if prompt_temp_editing(state) {
        commit_editing_prompt(state);
        open_save_draft(state);
    } else {
        state.prompt.mark_saved();
        commit_editing_prompt(state);
    }
}

/// Ctrl/Cmd+S 语义：内容为空（去空白）时不响应——空草稿不打开选择器、空条目不落库；
/// 非空时与工具栏圆点一致。
pub fn save_prompt_shortcut(state: &mut AppState) {
    if state.prompt.text().trim().is_empty() {
        return;
    }
    save_prompt_now(state);
}

/// 工具栏 `[tmp]`：提交当前编辑内容并新建临时提示词编辑视图（不入库）。
pub fn open_temp_prompt(state: &mut AppState) -> bool {
    commit_editing_prompt(state);
    let index = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get_mut(index) else {
        return false;
    };
    // 不变量兜底：空白项至多一条，残留空白项先回收（default temp 由新建项接替）。
    workspace_state.prompt_panel.remove_blank_temps();
    workspace_state.prompt_panel.cancel_page_rename();
    // 新建目标与旧暂存无关：丢弃，避免陈旧历史被同名目标误恢复。
    workspace_state.prompt_panel.clear_editor_stash();
    let id = workspace_state.prompt_panel.new_temp();
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(PromptEditing::Temp(id));
    // 新草稿与旧目标的块聚焦行无关，避免后续正文恰好占用同一行时误入块模式。
    state.prompt.exit_block_focus();
    state.prompt.set_text("");
    state.prompt_collapsed = false;
    super::focus_prompt(state);
    true
}

/// 清空指定工作区树视图的搜索词（不影响其余状态）。
pub(super) fn clear_search_for(state: &mut AppState, workspace: usize) {
    if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
        workspace_state.prompt_panel.search.clear();
    }
}

/// 保存成功回执：仅当编辑器仍显示对应（条目、页）时按修订确认保存点。
///
/// 旧页保存完成不得把新页的保存点误标为已保存；页内容与待确认标记照常清除。
pub fn mark_prompt_page_persisted(
    state: &mut AppState,
    id: i64,
    page_id: i64,
    content: &str,
    prompt: crate::layout::PaneId,
    revision: u64,
) -> bool {
    let mut found = false;
    for workspace in &mut state.workspaces {
        if let Some(page) = workspace
            .prompt_panel
            .tree
            .item_mut(id)
            .and_then(|item| item.page_mut(PromptPageId::Saved(page_id)))
        {
            page.content = content.to_string();
            found = true;
        }
        workspace.prompt_panel.clear_page_pending(id, page_id);
    }
    let editing_page = |workspace: &crate::app::state::Workspace| {
        workspace.prompt_panel.editing == Some(PromptEditing::Item(id))
            && workspace
                .prompt_panel
                .tree
                .item(id)
                .is_some_and(|item| item.active_page == PromptPageId::Saved(page_id))
    };
    if state.prompt.id() == prompt {
        let index = state.prompt_workspace();
        if state.workspaces.get(index).is_some_and(editing_page) {
            state.prompt.confirm_saved(revision);
        }
    } else {
        for workspace in &mut state.workspaces {
            let editor_matches = workspace
                .prompt
                .as_ref()
                .is_some_and(|draft| draft.editor.id() == prompt);
            if editor_matches
                && editing_page(workspace)
                && let Some(draft) = workspace.prompt.as_mut()
            {
                draft.editor.confirm_saved(revision);
            }
        }
    }
    found
}

/// 保存失败：重置对应编辑器的自动保存观察，等待下一轮静置重试。
pub fn reset_prompt_autosave(state: &mut AppState, prompt: crate::layout::PaneId) -> bool {
    if state.prompt.id() == prompt {
        state.prompt.reset_autosave_watch();
        return true;
    }
    for workspace in &mut state.workspaces {
        if let Some(draft) = workspace.prompt.as_mut()
            && draft.editor.id() == prompt
        {
            draft.editor.reset_autosave_watch();
            return true;
        }
    }
    false
}

/// 打开提示词条目右键菜单：编辑、重命名、复制与删除。
///
/// 右键不改动列表选中行（对齐工作区列表：右键只挂菜单，不切换当前项）；
/// 菜单命令按 target 里的 item_id 生效。
pub fn open_prompt_item_menu(state: &mut AppState, item_id: i64, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    if state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .is_none()
    {
        return;
    }
    let commands = vec![
        MenuCommand::EditPrompt,
        MenuCommand::RenamePrompt,
        MenuCommand::CopyPromptContent,
        MenuCommand::DeletePrompt,
    ];
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptItem { workspace, item_id },
        commands,
        selected: 0,
    }));
}

/// 打开临时提示词右键菜单：编辑、复制与删除（删除仅内存、立即生效，无确认浮层）。
///
/// 同条目菜单：右键不改动列表选中行。
pub fn open_prompt_temp_menu(state: &mut AppState, temp_id: u64, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let known = state
        .workspaces
        .get(workspace)
        .is_some_and(|workspace| workspace.prompt_panel.temp(temp_id).is_some());
    if !known {
        return;
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::TempPrompt { workspace, temp_id },
        commands: vec![
            MenuCommand::EditPrompt,
            MenuCommand::CopyPromptContent,
            MenuCommand::DeleteTemp,
        ],
        selected: 0,
    }));
}

/// 打开文件夹右键菜单：新建提示词、新建子文件夹（仅根层）、重命名与删除。
pub fn open_prompt_folder_menu(state: &mut AppState, folder_id: i64, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let Some(folder) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.folder(folder_id))
    else {
        return;
    };
    let mut commands = vec![MenuCommand::NewPrompt];
    if folder.parent_id.is_none() {
        commands.push(MenuCommand::NewSubFolder);
    }
    commands.push(MenuCommand::RenameFolder);
    commands.push(MenuCommand::DeleteFolder);
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptFolder {
            workspace,
            folder_id,
        },
        commands,
        selected: 0,
    }));
}

/// 打开树空白区域菜单：新建提示词与新建文件夹。
pub fn open_prompt_tree_menu(state: &mut AppState, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptTree { workspace },
        commands: vec![MenuCommand::NewPrompt, MenuCommand::NewFolder],
        selected: 0,
    }));
}

/// 显示中编辑视图是否绑定临时提示词。
pub fn prompt_temp_editing(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| {
            matches!(workspace.prompt_panel.editing, Some(PromptEditing::Temp(_)))
        })
}

/// 打开文件夹/条目的重命名浮层（预填当前名）。
pub fn open_prompt_rename(state: &mut AppState, target: RenameTarget) {
    let workspace = state.prompt_workspace();
    let name = state.workspaces.get(workspace).and_then(|workspace| {
        let panel = &workspace.prompt_panel;
        match target {
            RenameTarget::PromptFolder { folder_id, .. } => panel
                .tree
                .folder(folder_id)
                .map(|folder| folder.name.clone()),
            RenameTarget::PromptItem { item_id, .. } => {
                panel.tree.item(item_id).map(|item| item.name.clone())
            }
            _ => None,
        }
    });
    let Some(name) = name else {
        return;
    };
    state.overlay = Some(Overlay::Rename(Rename {
        target,
        input: TextInput::new(name),
    }));
}

/// 提交提示词树重命名；空名拒绝（浮层保持打开）。
pub fn commit_prompt_rename(state: &mut AppState, target: RenameTarget, name: String) {
    let workspace = state.prompt_workspace();
    let Some(cwd) = displayed_workspace_cwd(state) else {
        return;
    };
    match target {
        RenameTarget::PromptFolder { folder_id, .. } => {
            let renamed = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.prompt_panel.tree.folder_mut(folder_id))
                .map(|folder| folder.name.clone_from(&name))
                .is_some();
            if renamed {
                state.effects.push(Effect::RenameFolder {
                    workspace: cwd,
                    id: folder_id,
                    name,
                });
            }
        }
        RenameTarget::PromptItem { item_id, .. } => {
            let renamed = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.prompt_panel.tree.item_mut(item_id))
                .map(|item| item.name.clone_from(&name))
                .is_some();
            if renamed {
                state.effects.push(Effect::RenamePrompt {
                    workspace: cwd,
                    id: item_id,
                    name,
                });
            }
        }
        _ => {}
    }
}

/// 删除提示词条目：内存移除、必要时退出编辑视图，并登记删除。
pub fn delete_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    if workspace_state.prompt_panel.tree.item(item_id).is_none() {
        return;
    }
    workspace_state.prompt_panel.tree.delete_item(item_id);
    if workspace_state.prompt_panel.editing == Some(PromptEditing::Item(item_id)) {
        workspace_state.prompt_panel.editing = None;
        workspace_state.prompt_panel.mode = PromptMode::Tree;
    }
    workspace_state.prompt_panel.clamp_selection(visible);
    if workspace == state.prompt_workspace() {
        state.prompt.clear_panel();
    }
    state.effects.push(Effect::DeletePrompt {
        workspace: cwd,
        id: item_id,
    });
}

/// 删除文件夹（级联）：内存移除、必要时退出编辑视图，并登记删除。
pub fn delete_prompt_folder(state: &mut AppState, workspace: usize, folder_id: i64) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    if workspace_state
        .prompt_panel
        .tree
        .folder(folder_id)
        .is_none()
    {
        return;
    }
    let editing_removed = workspace_state
        .prompt_panel
        .editing
        .and_then(|editing| match editing {
            PromptEditing::Item(id) => Some(id),
            PromptEditing::Temp(_) => None,
        })
        .is_some_and(|id| {
            workspace_state
                .prompt_panel
                .tree
                .item(id)
                .is_none_or(|item| {
                    tree_view::removed_with_folder(
                        &workspace_state.prompt_panel.tree,
                        folder_id,
                        item.folder_id,
                    )
                })
        });
    workspace_state.prompt_panel.tree.delete_folder(folder_id);
    if editing_removed {
        workspace_state.prompt_panel.editing = None;
        workspace_state.prompt_panel.mode = PromptMode::Tree;
    }
    workspace_state.prompt_panel.clamp_selection(visible);
    if workspace == state.prompt_workspace() {
        state.prompt.clear_panel();
    }
    state.effects.push(Effect::DeleteFolder {
        workspace: cwd,
        id: folder_id,
    });
}

/// 复制提示词当前页正文到剪贴板（经效应出箱由事件层写入系统剪贴板）。
pub fn copy_prompt_content(state: &mut AppState, workspace: usize, item_id: i64) {
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .and_then(|item| item.active_page())
        .map(|page| page.content.clone())
    else {
        return;
    };
    state.effects.push(Effect::CopyToClipboard {
        text: content,
        anchor: None,
    });
}

/// 复制临时提示词当前页正文到剪贴板；不入库、无确认。
pub fn copy_prompt_temp_content(state: &mut AppState, workspace: usize, temp_id: u64) {
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.temp(temp_id))
        .and_then(|temp| temp.active_page())
        .map(|page| page.content.clone())
    else {
        return;
    };
    state.effects.push(Effect::CopyToClipboard {
        text: content,
        anchor: None,
    });
}

/// 删除临时提示词：仅内存移除并钳制选中；不入库、无确认浮层。
pub fn delete_prompt_temp(state: &mut AppState, workspace: usize, temp_id: u64) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    if !workspace_state.prompt_panel.remove_temp(temp_id) {
        return false;
    }
    workspace_state.prompt_panel.clamp_selection(visible);
    true
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
mod tests;
