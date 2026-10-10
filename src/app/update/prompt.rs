//! 提示词树行为：视图模式切换、选中导航、右键菜单与编辑提交。
//!
//! DB 读写不在此层执行：变更登记为 `Effect` 出箱，由持久化服务执行并回报。

use crate::app::actions::{EditorCommand, OverlayKey};
use crate::app::effects::Effect;
use crate::app::overlay::{
    Menu, MenuCommand, NewNode, NewNodeKind, Overlay, OverlayTarget, Rename, RenameTarget,
    SaveDraft, SaveDraftEntry, TextInput,
};
use crate::app::prompt::{
    PromptEditing, PromptFolder, PromptItem, PromptItemStatus, PromptMode, PromptRow,
    PromptRowKind, PromptTree,
};
use crate::app::state::AppState;
use crate::db::{FolderRecord, PromptRecord};
use crate::ui::text;

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

/// 用 DB 记录整体替换指定工作区的树；保持已有折叠状态。
pub fn apply_prompt_tree(
    state: &mut AppState,
    workspace: usize,
    folders: Vec<FolderRecord>,
    prompts: Vec<PromptRecord>,
) -> bool {
    let visible = prompt_tree_visible_rows(state);
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    let folders: Vec<PromptFolder> = folders.into_iter().map(PromptFolder::from).collect();
    let prompts: Vec<PromptItem> = prompts.into_iter().map(PromptItem::from).collect();
    workspace_state.prompt_panel.tree.replace(folders, prompts);
    workspace_state.prompt_panel.loaded = true;
    workspace_state.prompt_panel.clamp_selection(visible);
    true
}

/// 切换树视图：提交编辑内容并清空编辑绑定。
pub fn show_prompt_tree(state: &mut AppState) {
    commit_editing_prompt(state);
    let index = state.prompt_workspace();
    if let Some(workspace) = state.workspaces.get_mut(index) {
        workspace.prompt_panel.mode = PromptMode::Tree;
        workspace.prompt_panel.editing = None;
    }
    state.prompt.clear_panel();
    state.selection = None;
}

/// 工具栏模式按钮：树视图进入选中条目编辑（无选中条目回退草稿），编辑视图切回树。
pub fn toggle_prompt_panel_mode(state: &mut AppState) {
    if prompt_tree_mode(state) {
        edit_selected_prompt(state);
    } else {
        show_prompt_tree(state);
    }
}

/// 提交编辑视图内容到绑定目标：条目立即更新并登记 DB 保存；草稿更新内存槽位。
pub fn commit_editing_prompt(state: &mut AppState) {
    let index = state.prompt_workspace();
    commit_prompt_buffer(state, index);
}

/// 提交指定工作区的编辑器缓冲（其为显示态时缓冲位于 `state.prompt`）。
pub fn commit_prompt_buffer(state: &mut AppState, workspace: usize) {
    let Some(workspace_state) = state.workspaces.get(workspace) else {
        return;
    };
    let Some(target) = workspace_state.prompt_panel.editing else {
        return;
    };
    let content = state.prompt.text().to_string();
    let PromptEditing::Item(id) = target else {
        if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
            workspace_state.prompt_panel.draft = content;
        }
        return;
    };
    let Some(cwd) = workspace_state.cwd.clone() else {
        return;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    let Some(item) = workspace_state.prompt_panel.tree.item_mut(id) else {
        return;
    };
    if item.content == content {
        return;
    }
    item.content.clone_from(&content);
    state.effects.push(Effect::SavePromptContent {
        workspace: cwd,
        id,
        content,
    });
}

/// 树上移/下移选中行；`visible` 为可见行数，选中始终保持在视口内。
pub fn move_prompt_selection(state: &mut AppState, delta: isize, visible: usize) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    let len = workspace.prompt_panel.rows().len();
    if len == 0 {
        return false;
    }
    let current = workspace.prompt_panel.selected;
    let next = (current as isize + delta).clamp(0, len as isize - 1) as usize;
    if next == current {
        return false;
    }
    workspace.prompt_panel.selected = next;
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
    true
}

/// 激活选中行：草稿行打开草稿编辑，文件夹切换折叠，条目进入编辑视图。
pub fn activate_prompt_row(state: &mut AppState) -> bool {
    let index = state.prompt_workspace();
    let Some(row) = selected_row(state, index) else {
        return false;
    };
    match row.kind {
        PromptRowKind::Folder => toggle_prompt_folder(state, index, row),
        PromptRowKind::Item => edit_prompt_row(state, index, row),
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

/// 鼠标点击行：选中；文件夹行同时切换折叠，草稿行直接打开草稿编辑。
pub fn click_prompt_row(state: &mut AppState, row: usize) -> bool {
    let changed = select_prompt_row(state, row);
    let index = state.prompt_workspace();
    let Some(row_data) = state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.prompt_panel.rows().get(row).copied())
    else {
        return changed;
    };
    match row_data.kind {
        PromptRowKind::Folder => toggle_prompt_folder(state, index, row_data) || changed,
        PromptRowKind::Item => changed,
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

/// 进入指定条目的编辑视图并聚焦面板（服务在新建后调用；工作区须为显示中）。
pub fn edit_prompt_item(state: &mut AppState, workspace: usize, item_id: i64) -> bool {
    if workspace != state.prompt_workspace() || !select_prompt_item(state, workspace, item_id) {
        return false;
    }
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .map(|item| item.content.clone())
    else {
        return false;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return false;
    };
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(PromptEditing::Item(item_id));
    state.prompt.set_text(&content);
    state.prompt_collapsed = false;
    super::focus_prompt(state);
    true
}

/// 打开临时草稿的编辑视图并聚焦面板；草稿内容只存内存。
pub fn edit_prompt_draft(state: &mut AppState) -> bool {
    let index = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get_mut(index) else {
        return false;
    };
    let draft = workspace_state.prompt_panel.draft.clone();
    workspace_state.prompt_panel.mode = PromptMode::Editor;
    workspace_state.prompt_panel.editing = Some(PromptEditing::Draft);
    state.prompt.set_text(&draft);
    state.prompt_collapsed = false;
    super::focus_prompt(state);
    true
}

/// 打开选中条目的编辑视图（`[edit]` 按钮 / 树视图回车）；
/// 选中行非条目时回退打开临时草稿，保证 `[edit]` 永不无响应。
pub fn edit_selected_prompt(state: &mut AppState) -> bool {
    let index = state.prompt_workspace();
    let Some(item_id) = state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.prompt_panel.selected_prompt_id())
    else {
        return edit_prompt_draft(state);
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
            scroll_selection_into_view(panel, visible);
        }
    }
    true
}

/// 清空显示中树视图的搜索词（Ctrl+C 清空语义）；返回是否被消费。
pub fn clear_prompt_search(state: &mut AppState) -> bool {
    apply_prompt_search(state, &EditorCommand::Escape)
}

/// 工具栏 [temp prompt]：提交当前编辑内容并打开临时草稿编辑视图。
pub fn open_temp_prompt(state: &mut AppState) -> bool {
    commit_editing_prompt(state);
    edit_prompt_draft(state)
}

/// 打开草稿保存的文件夹选择浮层；文件夹按树层级顺序列出（忽略折叠）；
/// 名称预填默认名，列表末尾为“新建文件夹”动作行。
pub fn open_save_draft(state: &mut AppState) {
    let workspace = state.prompt_workspace();
    let Some(workspace_state) = state.workspaces.get(workspace) else {
        return;
    };
    let entries = save_draft_entries(&workspace_state.prompt_panel.tree);
    state.overlay = Some(Overlay::SaveDraft(SaveDraft {
        workspace,
        name: TextInput::new(text::PROMPT_DEFAULT_NAME),
        entries,
        selected: 0,
    }));
}

/// 选择器条目：根层 `/` + 文件夹树形展示（根层与子层）+ 末尾动作行。
fn save_draft_entries(tree: &PromptTree) -> Vec<SaveDraftEntry> {
    let folders = tree.folders();
    let mut entries = vec![SaveDraftEntry::Root];
    for root in folders.iter().filter(|folder| folder.parent_id.is_none()) {
        entries.push(SaveDraftEntry::Folder {
            id: root.id,
            depth: 0,
            name: root.name.clone(),
        });
        for sub in folders
            .iter()
            .filter(|folder| folder.parent_id == Some(root.id))
        {
            entries.push(SaveDraftEntry::Folder {
                id: sub.id,
                depth: 1,
                name: sub.name.clone(),
            });
        }
    }
    entries.push(SaveDraftEntry::AddFolder);
    entries
}

/// 选择器条目刷新：新建文件夹成功后由服务调用；重建条目并选中新文件夹。
pub fn refresh_save_draft(state: &mut AppState, folder_id: i64) {
    let Some(Overlay::SaveDraft(dialog)) = state.overlay.as_ref() else {
        return;
    };
    let workspace = dialog.workspace;
    let Some(workspace_state) = state.workspaces.get(workspace) else {
        return;
    };
    let entries = save_draft_entries(&workspace_state.prompt_panel.tree);
    let Some(Overlay::SaveDraft(dialog)) = state.overlay.as_mut() else {
        return;
    };
    if let Some(position) = entries
        .iter()
        .position(|entry| matches!(entry, SaveDraftEntry::Folder { id, .. } if *id == folder_id))
    {
        dialog.selected = position;
    }
    dialog.entries = entries;
}

/// 打开新建文件夹命名浮层；父级为根层（None）或指定文件夹；输入不预填。
pub fn open_new_folder(state: &mut AppState, parent_id: Option<i64>) {
    state.overlay = Some(Overlay::NewNode(NewNode {
        kind: NewNodeKind::Folder { parent_id },
        name: TextInput::new(""),
        resume: None,
    }));
}

/// 打开新建提示词命名浮层；输入不预填；确认创建后选中条目不进入编辑视图。
pub fn open_new_prompt(state: &mut AppState, folder_id: Option<i64>) {
    state.overlay = Some(Overlay::NewNode(NewNode {
        kind: NewNodeKind::Prompt { folder_id },
        name: TextInput::new(""),
        resume: None,
    }));
}

/// 从草稿保存选择器打开新建文件夹浮层：取消或确认后恢复选择器。
pub fn open_new_folder_from_draft(state: &mut AppState) {
    let Some(Overlay::SaveDraft(dialog)) = state.overlay.take() else {
        return;
    };
    state.overlay = Some(Overlay::NewNode(NewNode {
        kind: NewNodeKind::Folder { parent_id: None },
        name: TextInput::new(""),
        resume: Some(Box::new(dialog)),
    }));
}

/// 取消新建节点：来自选择器时恢复选择器，否则直接关闭。
pub fn cancel_new_node(state: &mut AppState) {
    let Some(Overlay::NewNode(dialog)) = state.overlay.take() else {
        return;
    };
    state.overlay = dialog.resume.map(|dialog| Overlay::SaveDraft(*dialog));
}

/// 确认新建节点：名称去空白后为空则拒绝；来自选择器时先恢复选择器再登记创建
/// （服务创建成功后刷新条目并选中新节点）。
pub fn commit_new_node(state: &mut AppState) {
    let Some(Overlay::NewNode(dialog)) = state.overlay.as_ref() else {
        return;
    };
    let name = dialog.name.text().trim().to_string();
    if name.is_empty() {
        return;
    }
    let kind = dialog.kind;
    let Some(Overlay::NewNode(dialog)) = state.overlay.take() else {
        return;
    };
    state.overlay = dialog.resume.map(|dialog| Overlay::SaveDraft(*dialog));
    let Some(cwd) = displayed_workspace_cwd(state) else {
        return;
    };
    match kind {
        NewNodeKind::Folder { parent_id } => {
            state.effects.push(Effect::CreateFolder {
                workspace: cwd,
                parent_id,
                name,
            });
        }
        NewNodeKind::Prompt { folder_id } => {
            // 新条目须在未过滤列表中可见：先清搜索，保证服务端选中成功。
            clear_search_for(state, state.prompt_workspace());
            state.effects.push(Effect::CreatePrompt {
                workspace: cwd,
                folder_id,
                name,
            });
        }
    }
}

/// 新建节点浮层按键：回车确认、Esc 取消、其余为名称输入编辑。
pub fn apply_new_node_key(state: &mut AppState, key: OverlayKey) {
    match key {
        OverlayKey::Esc => cancel_new_node(state),
        OverlayKey::Enter => commit_new_node(state),
        OverlayKey::Char(_)
        | OverlayKey::Clear
        | OverlayKey::Backspace
        | OverlayKey::Delete
        | OverlayKey::Left
        | OverlayKey::Right
        | OverlayKey::Home
        | OverlayKey::End => {
            let Some(Overlay::NewNode(dialog)) = state.overlay.as_mut() else {
                return;
            };
            let input = &mut dialog.name;
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
        OverlayKey::Up | OverlayKey::Down => {}
    }
}

/// 草稿保存选择器按键：上下移动、回车确认、Esc 关闭；字符输入归属顶部提示词名称。
pub fn apply_save_draft_key(state: &mut AppState, key: OverlayKey) {
    match key {
        OverlayKey::Esc => state.overlay = None,
        OverlayKey::Up => {
            move_save_draft_selection(state, -1);
        }
        OverlayKey::Down => {
            move_save_draft_selection(state, 1);
        }
        OverlayKey::Enter => commit_save_draft(state),
        OverlayKey::Char(_)
        | OverlayKey::Clear
        | OverlayKey::Backspace
        | OverlayKey::Delete
        | OverlayKey::Left
        | OverlayKey::Right
        | OverlayKey::Home
        | OverlayKey::End => edit_save_draft_input(state, key),
    }
}

/// 选择器字符输入：编辑顶部提示词名称。
fn edit_save_draft_input(state: &mut AppState, key: OverlayKey) {
    let Some(Overlay::SaveDraft(dialog)) = state.overlay.as_mut() else {
        return;
    };
    let input = &mut dialog.name;
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

/// 移动草稿保存选择器高亮；返回是否有变化。
pub fn move_save_draft_selection(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::SaveDraft(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    let len = dialog.entries.len();
    if len == 0 {
        return false;
    }
    let next = (dialog.selected as isize + delta).clamp(0, len as isize - 1) as usize;
    if next == dialog.selected {
        return false;
    }
    dialog.selected = next;
    true
}

/// 悬停/点击选中草稿保存目标；越界忽略；返回是否变化。
pub fn set_save_draft_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::SaveDraft(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= dialog.entries.len() || dialog.selected == index {
        return false;
    }
    dialog.selected = index;
    true
}

/// 确认保存（[save enter] 按钮 / Enter）：名称去空白后为空则拒绝；
/// 选中动作行时打开新建文件夹浮层；否则按选中目标登记保存。
pub fn commit_save_draft(state: &mut AppState) {
    let Some(Overlay::SaveDraft(dialog)) = state.overlay.as_ref() else {
        return;
    };
    let workspace = dialog.workspace;
    let prompt_name = dialog.name.text().trim().to_string();
    if prompt_name.is_empty() {
        return;
    }
    let Some(entry) = dialog.entries.get(dialog.selected) else {
        return;
    };
    match entry {
        SaveDraftEntry::AddFolder => open_new_folder_from_draft(state),
        SaveDraftEntry::Root => {
            save_draft(state, workspace, None, prompt_name);
        }
        SaveDraftEntry::Folder { id, .. } => {
            save_draft(state, workspace, Some(*id), prompt_name);
        }
    }
}

/// 把草稿登记为正式条目（写入指定文件夹）；成功后由服务清空草稿。
fn save_draft(state: &mut AppState, workspace: usize, folder_id: Option<i64>, name: String) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let content = state
        .workspaces
        .get(workspace)
        .map(|workspace| workspace.prompt_panel.draft.clone())
        .unwrap_or_default();
    state.effects.push(Effect::SaveDraft {
        workspace: cwd,
        folder_id,
        name,
        content,
    });
    state.overlay = None;
    clear_search_for(state, workspace);
}

/// 草稿保存成功后的收尾：清空草稿槽位与过滤词（新条目须在未过滤列表中可见）。
pub fn finish_draft_save(state: &mut AppState, workspace: usize) {
    if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
        workspace_state.prompt_panel.draft.clear();
        workspace_state.prompt_panel.search.clear();
    }
}

/// 清空指定工作区树视图的搜索词（不影响其余状态）。
fn clear_search_for(state: &mut AppState, workspace: usize) {
    if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
        workspace_state.prompt_panel.search.clear();
    }
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

/// 打开提示词条目右键菜单：编辑、状态切换、重命名、复制与删除。
pub fn open_prompt_item_menu(state: &mut AppState, item_id: i64, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let Some(item) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
    else {
        return;
    };
    let commands = vec![
        MenuCommand::EditPrompt,
        MenuCommand::ChangePromptStatus(item.status.next()),
        MenuCommand::RenamePrompt,
        MenuCommand::CopyPromptContent,
        MenuCommand::DeletePrompt,
    ];
    select_prompt_item(state, workspace, item_id);
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptItem { workspace, item_id },
        commands,
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
    select_prompt_folder(state, workspace, folder_id);
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

/// 显示中编辑视图是否绑定临时草稿。
pub fn prompt_draft_editing(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| workspace.prompt_panel.editing == Some(PromptEditing::Draft))
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
            PromptEditing::Draft => None,
        })
        .is_some_and(|id| {
            workspace_state
                .prompt_panel
                .tree
                .item(id)
                .is_none_or(|item| {
                    removed_with_folder(
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

/// 切换提示词状态，登记状态更新。
pub fn change_prompt_status(
    state: &mut AppState,
    workspace: usize,
    item_id: i64,
    status: PromptItemStatus,
) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    let Some(item) = workspace_state.prompt_panel.tree.item_mut(item_id) else {
        return;
    };
    if item.status == status {
        return;
    }
    item.status = status;
    state.effects.push(Effect::SetPromptStatus {
        workspace: cwd,
        id: item_id,
        status,
    });
}

/// 复制提示词正文到剪贴板（经效应出箱由事件层写入系统剪贴板）。
pub fn copy_prompt_content(state: &mut AppState, workspace: usize, item_id: i64) {
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .map(|item| item.content.clone())
    else {
        return;
    };
    state.effects.push(Effect::CopyToClipboard(content));
}

/// 显示中工作区的 cwd。
fn displayed_workspace_cwd(state: &AppState) -> Option<std::path::PathBuf> {
    workspace_cwd(state, state.prompt_workspace())
}

fn workspace_cwd(state: &AppState, workspace: usize) -> Option<std::path::PathBuf> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.cwd.clone())
}

/// 指定工作区的选中行。
fn selected_row(state: &AppState, workspace: usize) -> Option<PromptRow> {
    state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.selected_row())
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
fn removed_with_folder(tree: &PromptTree, folder_id: i64, item_folder: Option<i64>) -> bool {
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

#[cfg(test)]
mod tests;
