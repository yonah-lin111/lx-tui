//! 临时提示词草稿行为：保存到文件夹选择器与新建节点命名浮层。

use crate::app::actions::OverlayKey;
use crate::app::effects::{Effect, PromptPageDraft};
use crate::app::overlay::{NewNode, NewNodeKind, Overlay, SaveDraft, SaveDraftEntry, TextInput};
use crate::app::prompt::{PromptEditing, PromptTree};
use crate::app::state::AppState;
use crate::ui::text;

use super::{clear_search_for, displayed_workspace_cwd, workspace_cwd};

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
        OverlayKey::Up
        | OverlayKey::Down
        | OverlayKey::PageUp
        | OverlayKey::PageDown
        | OverlayKey::EnterFolder
        | OverlayKey::Back
        | OverlayKey::Copy
        | OverlayKey::Remove => {}
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
        OverlayKey::EnterFolder
        | OverlayKey::Back
        | OverlayKey::Copy
        | OverlayKey::Remove
        | OverlayKey::PageUp
        | OverlayKey::PageDown => {}
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

/// 确认保存（`[save ↵]` 按钮 / Enter）：名称去空白后为空则拒绝；
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

/// 把临时提示词登记为正式条目（连页写入指定文件夹）；成功后由服务移除该 temp。
fn save_draft(state: &mut AppState, workspace: usize, folder_id: Option<i64>, name: String) {
    let Some(cwd) = workspace_cwd(state, workspace) else {
        return;
    };
    let Some((pages, active)) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| match workspace.prompt_panel.editing {
            Some(PromptEditing::Temp(id)) => workspace.prompt_panel.temp(id),
            _ => None,
        })
        .map(|temp| {
            (
                temp.pages
                    .iter()
                    .map(|page| PromptPageDraft {
                        name: page.name.clone(),
                        content: page.content.clone(),
                    })
                    .collect::<Vec<_>>(),
                temp.active_index(),
            )
        })
    else {
        return;
    };
    state.effects.push(Effect::SaveDraft {
        workspace: cwd,
        folder_id,
        name,
        pages,
        active,
    });
    state.overlay = None;
    clear_search_for(state, workspace);
}

/// 临时提示词保存成功后的收尾：移除已升格为条目的 temp 并清空过滤词
/// （新条目须在未过滤列表中可见）。
pub fn finish_draft_save(state: &mut AppState, workspace: usize) {
    if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
        if let Some(PromptEditing::Temp(id)) = workspace_state.prompt_panel.editing {
            workspace_state.prompt_panel.remove_temp(id);
        }
        workspace_state.prompt_panel.cancel_page_rename();
        workspace_state.prompt_panel.search.clear();
    }
}
