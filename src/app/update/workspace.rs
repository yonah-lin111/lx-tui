//! 工作区与标签页生命周期、路径切换与工作区级 Prompt 打开。

use std::path::{Path, PathBuf};

use crate::app::overlay::{
    ConfirmSwitchCwd, ConfirmSyncWorkspaceCwd, NewWorkspace, Overlay, TextInput,
};
use crate::app::state::{
    AppState, PaneKind, PaneView, Tab, Workspace, current_workspace_identity, home_dir,
    unique_workspace_name, workspace_label,
};
use crate::layout::PaneId;

use super::todo::commit_todo_edit;
use crate::ui::text;

use super::editor::{
    focus_prompt, load_displayed_draft, release_prompt_pin, swap_display_prompt, sync_mention_root,
};
use super::prompt::commit_prompt_buffer;
use super::selection::clear_selection;
use super::worktree::request_git_refresh;

/// 打开新建工作区弹窗：清空滚动与拖拽状态，输入框初始为空。
pub fn open_new_workspace_dialog(state: &mut AppState) {
    state.workspace_scroll_drag = None;
    state.workspace_drag = None;
    state.workspace_dragging = false;
    state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
        input: TextInput::new(String::new()),
    }));
}

/// 解析用户输入的工作区路径：
/// 1. 空输入或纯引号包裹的空串返回 Ok(None)（表示继承默认工作区路径）；
/// 2. 剥离匹配的外层双引号/单引号；
/// 3. 支持 `~` / `~/...` 展开为当前用户 HOME 目录；
/// 4. 相对路径以基准路径（当前工作区路径或进程路径）展开；
/// 5. 校验目标路径必须在磁盘上存在且必须为目录。
pub fn parse_workspace_path(
    input: &str,
    base_cwd: Option<&Path>,
) -> Result<Option<PathBuf>, &'static str> {
    let mut trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
    {
        trimmed = trimmed[1..trimmed.len() - 1].trim();
    }
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = if trimmed == "~" {
        home_dir().ok_or("Cannot resolve home directory")?
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = home_dir().ok_or("Cannot resolve home directory")?;
        home.join(rest)
    } else {
        let p = Path::new(trimmed);
        if p.is_relative() {
            if let Some(base) = base_cwd {
                base.join(p)
            } else if let Ok(current) = std::env::current_dir() {
                current.join(p)
            } else {
                p.to_path_buf()
            }
        } else {
            p.to_path_buf()
        }
    };
    if !path.exists() {
        return Err(text::TOAST_DIRECTORY_NOT_FOUND);
    }
    if !path.is_dir() {
        return Err(text::TOAST_PATH_NOT_DIR);
    }
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    Ok(Some(path))
}

/// 新建工作区并激活：如果提供了目标 cwd 则以其为根路径，否则取当前工作区焦点终端的实时 cwd
/// 回退主终端 cwd 与进程 cwd；名字取目录末段（对齐 herdr），重名追加最小未用序号；
/// 立即登记 cwd 的 git 元数据查询；PTY 由事件循环按状态对齐启动。
pub fn create_workspace_with_cwd(state: &mut AppState, target_cwd: Option<PathBuf>) {
    let (cwd, base) = match target_cwd {
        Some(cwd) => {
            let base = workspace_label(&cwd, home_dir().as_deref());
            (Some(cwd), base)
        }
        None => match state
            .active_terminal_cwd()
            .or_else(|| state.active_workspace().cwd.clone())
        {
            Some(cwd) => {
                let base = workspace_label(&cwd, home_dir().as_deref());
                (Some(cwd), base)
            }
            None => current_workspace_identity(),
        },
    };
    let name = unique_workspace_name(&base, |candidate| {
        state
            .workspaces
            .iter()
            .any(|workspace| workspace.name == candidate)
    });
    state
        .workspaces
        .push(Workspace::single_terminal(name, cwd.clone()));
    let from = state.active_workspace;
    state.active_workspace = state.workspaces.len().saturating_sub(1);
    if state.prompt_pinned.is_none() {
        swap_display_prompt(state, from, state.active_workspace);
    }
    if let Some(cwd) = cwd.as_deref() {
        request_git_refresh(state, cwd);
    }
    clear_selection(state);
    state.prompt_focused = false;
}

/// 新建工作区（无指定路径，继承当前终端/工作区路径）。
pub fn create_workspace(state: &mut AppState) {
    create_workspace_with_cwd(state, None);
}

/// 在目标工作区追加终端标签并激活：初始 cwd 为工作区固定根路径，直接进入终端视图；
/// PTY 由事件循环按状态对齐启动。
pub fn create_terminal_in_workspace(state: &mut AppState, target: usize) {
    let Some(workspace) = state.workspaces.get_mut(target) else {
        return;
    };
    let mut tab = Tab::single_terminal();
    if let Some(id) = tab.root_pane()
        && let Some(pane) = tab.pane_mut(id)
    {
        pane.view = PaneView::Terminal;
    }
    workspace.tabs.push(tab);
    workspace.active_tab = workspace.tabs.len().saturating_sub(1);
    let from = state.active_workspace;
    state.active_workspace = target;
    if state.prompt_pinned.is_none() {
        swap_display_prompt(state, from, target);
    }
    clear_selection(state);
    state.prompt_focused = false;
}

/// 打开 Prompt 并绑定工作区路径：切换激活目标工作区，展开并聚焦；
/// 提及扫描根与顶栏路径名随显式绑定根显示。
pub fn open_prompt_for_workspace(state: &mut AppState, target: usize) {
    let Some(root) = state
        .workspaces
        .get(target)
        .map(|workspace| workspace.cwd.clone())
    else {
        return;
    };
    release_prompt_pin(state);
    switch_workspace(state, target);
    state.prompt_root = root;
    state.prompt_collapsed = false;
    focus_prompt(state);
}

/// 打开 Prompt 并绑定窗格 cwd：保持当前工作区，展开并聚焦；
/// 窗格尚未轮询到 cwd 时回退工作区路径。
pub fn open_prompt_for_pane(state: &mut AppState, pane: PaneId) {
    let root = state
        .pane_anywhere(pane)
        .and_then(|target| target.cwd.clone())
        .or_else(|| state.active_workspace().cwd.clone());
    release_prompt_pin(state);
    state.prompt_root = root;
    state.prompt_collapsed = false;
    focus_prompt(state);
}

/// 打开切换工作区路径的二次确认浮层；工作区无固定路径时忽略。
pub(crate) fn open_confirm_switch_cwd(
    state: &mut AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
) {
    let Some(path) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.cwd.clone())
    else {
        return;
    };
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace,
        tab,
        pane,
        path,
    }));
}

/// 切换工作区路径的 shell 命令：双引号包裹并转义 `\`、`"`、`$` 与反引号，杜绝命令注入。
pub(crate) fn switch_cwd_command(path: &Path) -> Vec<u8> {
    let mut command = String::from("cd \"");
    for ch in path.to_string_lossy().chars() {
        if matches!(ch, '\\' | '"' | '$' | '`') {
            command.push('\\');
        }
        command.push(ch);
    }
    command.push_str("\"\n");
    command.into_bytes()
}

/// 确认切换工作区路径：关闭浮层并返回写入目标窗格的 `cd` 命令；
/// 目标窗格已不存在或非终端时返回 None。
pub(crate) fn confirm_switch_cwd(state: &mut AppState) -> Option<(PaneId, Vec<u8>)> {
    let Some(Overlay::ConfirmSwitchCwd(confirm)) = state.overlay.take() else {
        return None;
    };
    let pane = state
        .workspaces
        .get(confirm.workspace)?
        .tabs
        .get(confirm.tab)?
        .pane(confirm.pane)?;
    (pane.kind == PaneKind::Terminal).then(|| (confirm.pane, switch_cwd_command(&confirm.path)))
}

/// 打开同步当前工作区路径到终端路径的二次确认浮层；终端无有效路径时忽略。
pub(crate) fn open_confirm_sync_workspace_cwd(
    state: &mut AppState,
    workspace: usize,
    pane: PaneId,
) {
    let Some(path) = state
        .workspaces
        .get(workspace)
        .and_then(|ws| ws.tabs.iter().find_map(|t| t.pane(pane)))
        .and_then(|target| target.cwd.clone())
    else {
        return;
    };
    state.overlay = Some(Overlay::ConfirmSyncWorkspaceCwd(ConfirmSyncWorkspaceCwd {
        workspace,
        pane,
        path,
    }));
}

/// 确认同步当前工作区路径到终端路径：关闭浮层，更新工作区 cwd、自动命名与 git 查询，
/// 并同步活跃 prompt 根目录。
pub(crate) fn confirm_sync_workspace_cwd(state: &mut AppState) {
    let Some(Overlay::ConfirmSyncWorkspaceCwd(confirm)) = state.overlay.take() else {
        return;
    };
    let new_cwd = confirm.path;
    let target_ws = confirm.workspace;
    let name_is_manual = state
        .workspaces
        .get(target_ws)
        .is_some_and(|ws| ws.name_is_manual);
    let new_name = if !name_is_manual {
        let base = workspace_label(&new_cwd, home_dir().as_deref());
        Some(unique_workspace_name(&base, |candidate| {
            state
                .workspaces
                .iter()
                .enumerate()
                .any(|(i, ws)| i != target_ws && ws.name == candidate)
        }))
    } else {
        None
    };
    let Some(workspace) = state.workspaces.get_mut(target_ws) else {
        return;
    };
    workspace.cwd = Some(new_cwd.clone());
    if let Some(name) = new_name {
        workspace.name = name;
    }
    workspace.git = None;
    request_git_refresh(state, &new_cwd);
    if state.prompt_workspace() == target_ws {
        state.prompt_root = Some(new_cwd);
        sync_mention_root(state);
    } else if let Some(draft) = state
        .workspaces
        .get_mut(target_ws)
        .and_then(|ws| ws.prompt.as_mut())
    {
        draft.root = Some(new_cwd);
    }
}

/// 切换当前工作区；越界忽略。
pub fn switch_workspace(state: &mut AppState, index: usize) {
    if index >= state.workspaces.len() {
        return;
    }
    let from = state.active_workspace;
    state.active_workspace = index;
    if state.prompt_pinned.is_none() {
        swap_display_prompt(state, from, index);
    }
    clear_selection(state);
    // 编辑中切工作区走提交；todo 的列表/选中/滚动/焦点**不随工作区变化**（全局列表）。
    commit_todo_edit(state);
    state.prompt_focused = false;
    // Agents 列表随工作区切换：滚动与悬停复位，避免旧槽位错位。
    state.agent_scroll = 0;
    state.agent_hover = None;
}

/// 新建标签并激活：追加自动命名标签；PTY 由事件循环按状态对齐启动。
pub fn create_tab(state: &mut AppState) {
    let workspace = state.active_workspace_mut();
    workspace.tabs.push(Tab::single_terminal());
    workspace.active_tab = workspace.tabs.len().saturating_sub(1);
    clear_selection(state);
    state.prompt_focused = false;
}

/// 切换当前工作区的标签；越界忽略。
pub fn switch_tab(state: &mut AppState, index: usize) {
    let workspace = state.active_workspace_mut();
    if index >= workspace.tabs.len() {
        return;
    }
    workspace.active_tab = index;
    clear_selection(state);
    state.prompt_focused = false;
}

/// 标签栏滚动最大偏移；由调用方按当前几何计算并钳制。
pub fn set_tab_scroll(state: &mut AppState, offset: usize, max: usize) -> bool {
    let offset = offset.min(max);
    if offset == state.tab_scroll {
        return false;
    }
    state.tab_scroll = offset;
    true
}

/// 按步长滚动标签栏；越界钳制；返回是否变化。
pub fn scroll_tab_bar(state: &mut AppState, delta: isize, max: usize) -> bool {
    let target = (state.tab_scroll.min(max) as isize).saturating_add(delta);
    set_tab_scroll(state, target.clamp(0, max as isize) as usize, max)
}

/// 关闭工作区：至少保留一个；关闭当前工作区后焦点落到同索引，越界回退末项。
pub(crate) fn close_workspace(state: &mut AppState, target: usize) {
    if state.workspaces.len() <= 1 || target >= state.workspaces.len() {
        return;
    }
    let removed_active = target == state.active_workspace;
    let display_removed = target == state.prompt_workspace();
    // 被关闭工作区若在显示态，其编辑缓冲先提交落库再销毁。
    if display_removed {
        commit_prompt_buffer(state, target);
    }
    state.workspaces.remove(target);
    if removed_active {
        state.active_workspace = state.active_workspace.min(state.workspaces.len() - 1);
    } else if target < state.active_workspace {
        state.active_workspace -= 1;
    }
    match state.prompt_pinned {
        Some(pinned) if pinned == target => state.prompt_pinned = None,
        Some(pinned) if pinned > target => state.prompt_pinned = Some(pinned - 1),
        _ => {}
    }
    if display_removed {
        load_displayed_draft(state);
    }
    clear_selection(state);
}

/// 关闭标签：至少保留一个；关闭当前标签后焦点落到同索引，越界回退末项。
pub(crate) fn close_tab(state: &mut AppState, workspace: usize, tab: usize) {
    let Some(workspace_state) = state.workspaces.get_mut(workspace) else {
        return;
    };
    if workspace_state.tabs.len() <= 1 || tab >= workspace_state.tabs.len() {
        return;
    }
    workspace_state.tabs.remove(tab);
    let last = workspace_state.tabs.len() - 1;
    if tab < workspace_state.active_tab {
        workspace_state.active_tab -= 1;
    } else if tab == workspace_state.active_tab {
        workspace_state.active_tab = workspace_state.active_tab.min(last);
    }
    clear_selection(state);
}

/// 关闭窗格：至少保留一个；被关闭窗格是焦点时，焦点落到提升兄弟子树的首个窗格。
pub(crate) fn close_pane(state: &mut AppState, workspace: usize, tab: usize, pane: PaneId) {
    let Some(tab_state) = state
        .workspaces
        .get_mut(workspace)
        .and_then(|workspace| workspace.tabs.get_mut(tab))
    else {
        return;
    };
    if !tab_state.remove_pane(pane) {
        return;
    }
    if state.terminal_selection == Some(pane) {
        state.terminal_selection = None;
    }
    if state.terminal_scroll_drag.is_some_and(|(id, _)| id == pane) {
        state.terminal_scroll_drag = None;
    }
    if state
        .selection_autoscroll
        .is_some_and(|plan| plan.pane == pane)
    {
        state.selection_autoscroll = None;
    }
}
