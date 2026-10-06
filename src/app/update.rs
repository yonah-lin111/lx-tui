//! 行为到状态的转换；几何信息由事件循环传入，保证逻辑纯且可测。

use std::path::Path;
use std::time::Instant;

use ratatui::layout::Rect;

use crate::layout::{self, PaneId};

use super::actions::{Action, EditorCommand, OverlayKey};
use super::markdown::MentionEntry;
use super::overlay::{
    ConfirmClose, Menu, MenuCommand, MenuTarget, Overlay, OverlayKind, Rename, TextInput,
};
use super::selection::Selection;
use super::state::{
    AppState, PaneKind, Workspace, current_workspace_identity, home_dir, unique_workspace_name,
    workspace_label,
};
use super::toast::Toast;

/// 应用行为。
pub fn apply(action: Action, state: &mut AppState) {
    match action {
        Action::Quit => state.should_quit = true,
        Action::ToggleSidebar => state.sidebar_collapsed = !state.sidebar_collapsed,
        Action::TogglePrompt => {
            state.prompt_collapsed = !state.prompt_collapsed;
            if state.prompt_collapsed {
                state.prompt_focused = false;
                state.selection = None;
                state.prompt.clear_panel();
            }
        }
        Action::ToggleAgents => state.agents_collapsed = !state.agents_collapsed,
    }
}

/// 按键产生的编辑命令；仅当 prompt 聚焦时由事件循环调用。
///
/// prompt 存在选区时：输入类命令用新内容替换选区，Backspace/Delete 删除选区，
/// 其余命令先清除选区再执行（选区删除/替换作为一步撤销）。
pub fn apply_editor(state: &mut AppState, command: EditorCommand) {
    sync_mention_root(state);
    if route_panel(state, &command) {
        return;
    }
    if let Some((start, end)) = prompt_selection_bounds(state) {
        let replaced = match &command {
            EditorCommand::InsertChar(ch) => {
                state.prompt.replace_range(start, end, &ch.to_string())
            }
            EditorCommand::InsertText(text) => state.prompt.replace_range(start, end, text),
            EditorCommand::Newline | EditorCommand::NewlineBelow => {
                state.prompt.replace_range(start, end, "\n")
            }
            EditorCommand::Backspace | EditorCommand::Delete => {
                state.prompt.replace_range(start, end, "")
            }
            _ => false,
        };
        state.selection = None;
        if replaced {
            return;
        }
    }
    match command {
        EditorCommand::InsertChar(ch) => state.prompt.insert_char(ch),
        EditorCommand::InsertText(text) => state.prompt.insert_str(&text),
        EditorCommand::Newline => state.prompt.newline(),
        EditorCommand::NewlineBelow => state.prompt.newline_below(),
        EditorCommand::Backspace => state.prompt.backspace(),
        EditorCommand::Delete => state.prompt.delete(),
        EditorCommand::Left => state.prompt.move_left(),
        EditorCommand::Right => state.prompt.move_right(),
        EditorCommand::Up => state.prompt.move_up(),
        EditorCommand::Down => state.prompt.move_down(),
        EditorCommand::Home => state.prompt.move_home(),
        EditorCommand::End => state.prompt.move_end(),
        EditorCommand::LineStart => state.prompt.move_line_start(),
        EditorCommand::LineEnd => state.prompt.move_line_end(),
        EditorCommand::WordLeft => state.prompt.move_word_backward(),
        EditorCommand::WordRight => state.prompt.move_word_forward(),
        EditorCommand::DeleteToLineStart => state.prompt.delete_to_line_start(),
        EditorCommand::DeleteToLineEnd => state.prompt.delete_to_line_end(),
        EditorCommand::DeleteWordBackward => state.prompt.delete_word_backward(),
        EditorCommand::DeleteWordForward => state.prompt.delete_word_forward(),
        EditorCommand::Indent => state.prompt.indent(),
        EditorCommand::Outdent => state.prompt.outdent(),
        EditorCommand::Undo => state.prompt.undo(),
        EditorCommand::Redo => state.prompt.redo(),
        EditorCommand::Escape => {}
    }
}

/// 块命令面板打开时的按键优先：上下选择、回车确认、Esc 关闭；返回是否消费。
///
/// 文件提及面板优先于块命令面板；两者互斥，同时只可能有一个打开。
fn route_panel(state: &mut AppState, command: &EditorCommand) -> bool {
    if route_mention_panel(state, command) {
        return true;
    }
    match command {
        EditorCommand::Up => state.prompt.panel_move(-1),
        EditorCommand::Down => state.prompt.panel_move(1),
        EditorCommand::Newline => state.prompt.panel_confirm(),
        EditorCommand::Escape => state.prompt.panel_escape(),
        _ => false,
    }
}

/// 文件提及面板打开时的按键优先：上下选择、回车确认、Esc 关闭；返回是否消费。
fn route_mention_panel(state: &mut AppState, command: &EditorCommand) -> bool {
    match command {
        EditorCommand::Up => state.prompt.mention_move(-1),
        EditorCommand::Down => state.prompt.mention_move(1),
        EditorCommand::Newline => state.prompt.mention_confirm(),
        EditorCommand::Escape => state.prompt.mention_escape(),
        _ => false,
    }
}

/// 把活动工作区 cwd 同步为提及扫描根；根变化时 Prompt 内缓存失效。
fn sync_mention_root(state: &mut AppState) {
    let root = state.active_workspace().cwd.clone();
    state.prompt.set_mention_root(root);
}

/// 写入文件提及扫描结果；过期代号在 Prompt 内丢弃。
pub fn apply_mention_entries(state: &mut AppState, generation: u64, entries: Vec<MentionEntry>) {
    state.prompt.apply_mention_entries(generation, entries);
}

/// 滚轮一格滚动的视觉行数；对齐 opencode 默认步长。
const WHEEL_LINES: isize = 3;

/// 按方向滚动 prompt 视口（负数向上、正数向下）；光标不动，编辑后自动吸回。
pub fn scroll_prompt(state: &mut AppState, direction: isize) {
    state.prompt.scroll_by(direction.signum() * WHEEL_LINES);
}

/// 鼠标点击 prompt：把视口单元格映射为光标位置。
pub fn place_prompt_cursor(state: &mut AppState, row: u16, col: u16) {
    sync_mention_root(state);
    state.prompt.set_cursor_from_cell(row, col);
}

/// 点击 prompt：键盘焦点交给编辑器。
pub fn focus_prompt(state: &mut AppState) {
    sync_mention_root(state);
    state.prompt_focused = true;
}

/// 点击终端窗格：焦点回到窗格，prompt 失焦并关闭块命令面板。
pub fn focus_pane(state: &mut AppState, id: PaneId) {
    state.prompt_focused = false;
    state.prompt.clear_panel();
    state.active_tab_mut().layout.focus_pane(id);
}

/// 按几何同步各窗格终端与 prompt 编辑器的尺寸。
///
/// prompt 文本区固定预留滚动条槽；尺寸变化会让视口选区坐标失效，此时清除选区。
pub fn resize_panes(state: &mut AppState, pane_rects: &[(PaneId, Rect)]) {
    for (id, rect) in pane_rects {
        if *id == state.prompt.id() {
            let (cols, rows) = layout::prompt_inner_size(*rect);
            if state.prompt.size() != (cols, rows) {
                state.selection = None;
            }
            state.prompt.resize(cols, rows);
        } else if let Some(pane) = state.pane_mut_anywhere(*id) {
            let (cols, rows) = layout::pane_inner_size(*rect);
            pane.terminal.resize(cols, rows);
        }
    }
}

/// 喂入窗格输出；返回需要写回 PTY 的响应。
pub fn feed_pane(state: &mut AppState, id: PaneId, bytes: &[u8]) -> Vec<u8> {
    state
        .pane_mut_anywhere(id)
        .map(|pane| pane.terminal.feed(bytes))
        .unwrap_or_default()
}

/// 标记窗格进程已退出；保留最后一屏。
pub fn mark_pane_exited(state: &mut AppState, id: PaneId) {
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.exited = true;
    }
}

/// 在区域内容区开始一次文本选择；与右栏拖拽互斥。
pub fn begin_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    state.resizing_prompt = false;
    state.selection = Some(Selection::begin(pane, row, col));
}

/// 扩展当前选区；窗格不一致时忽略。
pub fn drag_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    if let Some(selection) = state.selection.as_mut()
        && selection.pane() == pane
    {
        selection.drag(row, col);
    }
}

/// 清除选区。
pub fn clear_selection(state: &mut AppState) {
    state.selection = None;
}

/// 松开鼠标：结束拖动；空选区（未拖动）直接清除，非空选区保留供复制或删除。
pub fn end_selection_drag(state: &mut AppState) {
    let keep = state
        .selection
        .as_ref()
        .is_some_and(|selection| selection.range().is_some());
    if keep {
        if let Some(selection) = state.selection.as_mut() {
            selection.finish();
        }
    } else {
        state.selection = None;
    }
}

/// prompt 选区文本（保留选区，供 Ctrl/Cmd+C 复制）；空选区返回 None。
pub fn prompt_selection_text(state: &AppState) -> Option<String> {
    let selection = state.selection?;
    if selection.pane() != state.prompt.id() {
        return None;
    }
    let (start, end) = selection.range()?;
    state.prompt.selection_text(start, end)
}

/// prompt 选区对应的字节范围；选区不在 prompt 或为空时返回 None。
fn prompt_selection_bounds(state: &AppState) -> Option<(usize, usize)> {
    let selection = state.selection?;
    if selection.pane() != state.prompt.id() {
        return None;
    }
    let (start, end) = selection.range()?;
    state.prompt.selection_bounds(start, end)
}

/// 结束选区并提取文本（选区保留高亮）；未拖动、prompt 空选区或空占位窗格返回 None。
pub fn finish_selection(state: &mut AppState) -> Option<String> {
    let selection = state.selection?;
    let (start, end) = selection.range()?;
    if selection.pane() == state.prompt.id() {
        return state.prompt.selection_text(start, end);
    }
    let pane = state.pane_mut_anywhere(selection.pane())?;
    if pane.kind != PaneKind::Terminal {
        return None;
    }
    pane.terminal
        .text_in_range(start, end)
        .filter(|text| !text.is_empty())
}

/// 在 prompt 右栏分割线上开始拖拽；清除已有选区。
pub fn begin_prompt_resize(state: &mut AppState) {
    state.selection = None;
    state.resizing_prompt = true;
}

/// 拖拽中更新右栏宽度；未处于拖拽时忽略。
pub fn drag_prompt(state: &mut AppState, width: u16) {
    if state.resizing_prompt {
        state.prompt_width = width;
    }
}

/// 结束右栏拖拽。
pub fn end_prompt_resize(state: &mut AppState) {
    state.resizing_prompt = false;
}

/// 在侧栏分割线上开始拖拽；清除已有选区。
pub fn begin_sidebar_resize(state: &mut AppState) {
    state.selection = None;
    state.resizing_sidebar = true;
}

/// 拖拽中更新侧栏宽度；未处于拖拽时忽略。
pub fn drag_sidebar(state: &mut AppState, width: u16) {
    if state.resizing_sidebar {
        state.sidebar_width = width;
    }
}

/// 结束侧栏拖拽。
pub fn end_sidebar_resize(state: &mut AppState) {
    state.resizing_sidebar = false;
}

/// 更新侧栏分割线悬停状态；返回是否发生变化。
pub fn set_sidebar_hover(state: &mut AppState, hover: bool) -> bool {
    if state.sidebar_hover == hover {
        return false;
    }
    state.sidebar_hover = hover;
    true
}

/// 更新右栏分割线悬停状态；返回是否发生变化。
pub fn set_prompt_hover(state: &mut AppState, hover: bool) -> bool {
    if state.prompt_hover == hover {
        return false;
    }
    state.prompt_hover = hover;
    true
}

/// 展示 toast；单条替换并重置倒计时。
pub fn show_toast(state: &mut AppState, toast: Toast) {
    state.toast = Some(toast);
}

/// 关闭当前 toast。
pub fn dismiss_toast(state: &mut AppState) {
    state.toast = None;
}

/// 更新 toast 悬停状态；返回是否发生变化。
pub fn set_toast_hover(state: &mut AppState, hovered: bool) -> bool {
    let Some(toast) = state.toast.as_mut() else {
        return false;
    };
    if toast.hovered() == hovered {
        return false;
    }
    toast.set_hovered(hovered);
    true
}

/// 清除已过期的 toast；返回是否发生变化（用于置脏重绘）。
pub fn tick(state: &mut AppState, now: Instant) -> bool {
    let expired = state
        .toast
        .as_ref()
        .is_some_and(|toast| toast.is_expired(now));
    if expired {
        state.toast = None;
    }
    expired
}

/// 最近一次 toast 到期时间；事件循环据此安排唤醒。
pub fn next_deadline(state: &AppState) -> Option<Instant> {
    state.toast.as_ref().and_then(Toast::next_deadline)
}

/// 新建工作区并激活：名字取当前 cwd 末段（对齐 herdr），重名追加最小未用序号；
/// PTY 由事件循环按状态对齐启动。
pub fn create_workspace(state: &mut AppState) {
    let (cwd, base) = current_workspace_identity();
    let name = unique_workspace_name(&base, |candidate| {
        state
            .workspaces
            .iter()
            .any(|workspace| workspace.name == candidate)
    });
    state.workspaces.push(Workspace::single_terminal(name, cwd));
    state.active_workspace = state.workspaces.len().saturating_sub(1);
    state.selection = None;
    state.prompt_focused = false;
}

/// 跟踪窗格 cwd 变化：自动命名工作区跟随 cwd 改名，手动命名工作区忽略；返回是否变化。
pub fn update_workspace_cwd(state: &mut AppState, index: usize, cwd: &Path) -> bool {
    let Some(workspace) = state.workspaces.get(index) else {
        return false;
    };
    if workspace.name_is_manual || workspace.cwd.as_deref() == Some(cwd) {
        return false;
    }
    let base = workspace_label(cwd, home_dir().as_deref());
    let name = unique_workspace_name(&base, |candidate| {
        state
            .workspaces
            .iter()
            .enumerate()
            .any(|(other, workspace)| other != index && workspace.name == candidate)
    });
    let Some(workspace) = state.workspaces.get_mut(index) else {
        return false;
    };
    workspace.cwd = Some(cwd.to_path_buf());
    workspace.name = name;
    true
}

/// 切换当前工作区；越界忽略。
pub fn switch_workspace(state: &mut AppState, index: usize) {
    if index >= state.workspaces.len() {
        return;
    }
    state.active_workspace = index;
    state.selection = None;
    state.prompt_focused = false;
}

/// 打开工作区右键菜单；仅剩一个工作区时不提供关闭项。
pub fn open_workspace_menu(state: &mut AppState, target: usize, anchor: (u16, u16)) {
    if target >= state.workspaces.len() {
        return;
    }
    // 打开模态时结束可能残留的滚动条拖拽。
    state.workspace_scroll_drag = None;
    let mut commands = vec![MenuCommand::RenameWorkspace];
    if state.workspaces.len() > 1 {
        commands.push(MenuCommand::CloseWorkspace);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: MenuTarget::Workspace(target),
        commands,
        selected: 0,
    }));
}

/// 关闭当前浮层。
pub fn close_overlay(state: &mut AppState) {
    state.overlay = None;
}

/// 菜单高亮按步长循环移动。
pub fn move_menu_selection(state: &mut AppState, step: isize) {
    let Some(Overlay::Menu(menu)) = state.overlay.as_mut() else {
        return;
    };
    if menu.commands.is_empty() {
        return;
    }
    let len = menu.commands.len() as isize;
    menu.selected = (menu.selected as isize + step).rem_euclid(len) as usize;
}

/// 菜单悬停高亮；索引越界忽略；返回是否变化。
pub fn set_menu_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::Menu(menu)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= menu.commands.len() || menu.selected == index {
        return false;
    }
    menu.selected = index;
    true
}

/// 执行菜单当前项：重命名打开输入浮层，关闭打开确认浮层。
pub fn activate_menu(state: &mut AppState) {
    let Some(Overlay::Menu(menu)) = state.overlay.take() else {
        return;
    };
    let MenuTarget::Workspace(target) = menu.target;
    match menu.commands.get(menu.selected) {
        Some(MenuCommand::RenameWorkspace) => {
            let Some(workspace) = state.workspaces.get(target) else {
                return;
            };
            state.overlay = Some(Overlay::Rename(Rename {
                target,
                input: TextInput::new(workspace.name.clone()),
            }));
        }
        Some(MenuCommand::CloseWorkspace) => {
            state.overlay = Some(Overlay::ConfirmClose(ConfirmClose { target }));
        }
        None => {}
    }
}

/// 浮层按键分派；输入层已按浮层种类过滤。
pub fn apply_overlay_key(state: &mut AppState, key: OverlayKey) {
    match key {
        OverlayKey::Esc => close_overlay(state),
        OverlayKey::Up => move_menu_selection(state, -1),
        OverlayKey::Down => move_menu_selection(state, 1),
        OverlayKey::Enter => match state.overlay.as_ref().map(Overlay::kind) {
            Some(OverlayKind::Menu) => activate_menu(state),
            Some(OverlayKind::Rename) => commit_rename(state),
            Some(OverlayKind::ConfirmClose) => confirm_close(state),
            None => {}
        },
        OverlayKey::Char(ch) => edit_rename(state, |input| input.insert_char(ch)),
        OverlayKey::Clear => edit_rename(state, TextInput::clear),
        OverlayKey::Backspace => edit_rename(state, TextInput::backspace),
        OverlayKey::Delete => edit_rename(state, TextInput::delete),
        OverlayKey::Left => edit_rename(state, TextInput::move_left),
        OverlayKey::Right => edit_rename(state, TextInput::move_right),
        OverlayKey::Home => edit_rename(state, TextInput::move_home),
        OverlayKey::End => edit_rename(state, TextInput::move_end),
    }
}

/// 对重命名输入执行一次编辑；其他浮层忽略。
fn edit_rename(state: &mut AppState, edit: impl FnOnce(&mut TextInput)) {
    if let Some(Overlay::Rename(rename)) = state.overlay.as_mut() {
        edit(&mut rename.input);
    }
}

/// 提交重命名；空名不保存且浮层保持打开。
fn commit_rename(state: &mut AppState) {
    let Some(Overlay::Rename(rename)) = state.overlay.as_ref() else {
        return;
    };
    let name = rename.input.text().trim().to_string();
    if name.is_empty() {
        return;
    }
    let target = rename.target;
    let Some(workspace) = state.workspaces.get_mut(target) else {
        state.overlay = None;
        return;
    };
    workspace.name = name;
    workspace.name_is_manual = true;
    state.overlay = None;
}

/// 确认关闭工作区：至少保留一个；关闭当前工作区后焦点落到同索引，越界回退末项。
fn confirm_close(state: &mut AppState) {
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.take() else {
        return;
    };
    if state.workspaces.len() <= 1 || confirm.target >= state.workspaces.len() {
        return;
    }
    let removed_active = confirm.target == state.active_workspace;
    state.workspaces.remove(confirm.target);
    if removed_active {
        state.active_workspace = state.active_workspace.min(state.workspaces.len() - 1);
    } else if confirm.target < state.active_workspace {
        state.active_workspace -= 1;
    }
    state.selection = None;
}

/// 工作区列表最大滚动偏移；列表放得下时恒为 0。
pub fn workspace_scroll_max(state: &AppState, visible: usize) -> usize {
    state.workspaces.len().saturating_sub(visible.max(1))
}

/// 滚动工作区列表；越界钳制；返回是否变化。
pub fn scroll_workspace_list(state: &mut AppState, delta: isize, visible: usize) -> bool {
    let max = workspace_scroll_max(state, visible);
    let target = (state.workspace_scroll.min(max) as isize).saturating_add(delta);
    let offset = target.clamp(0, max as isize) as usize;
    if offset == state.workspace_scroll {
        return false;
    }
    state.workspace_scroll = offset;
    true
}

/// 直接设置滚动偏移（滚动条点击/拖拽）；越界钳制；返回是否变化。
pub fn set_workspace_scroll(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let offset = offset.min(workspace_scroll_max(state, visible));
    if offset == state.workspace_scroll {
        return false;
    }
    state.workspace_scroll = offset;
    true
}

/// 直接设置 prompt 视口偏移（滚动条点击/拖拽）；越界钳制；返回是否变化。
pub fn set_prompt_scroll(state: &mut AppState, offset: usize) -> bool {
    let before = state.prompt.scroll();
    state.prompt.scroll_to(offset);
    state.prompt.scroll() != before
}

/// 开始拖动排序：记录被拖工作区索引；按下时已切换激活。
pub fn begin_workspace_drag(state: &mut AppState, index: usize) {
    if index < state.workspaces.len() {
        state.workspace_drag = Some(index);
    }
}

/// 拖动排序：把被拖工作区移动到 `target` 索引并钳制；激活项跟随其新位置；返回是否变化。
pub fn drag_workspace_to(state: &mut AppState, target: usize) -> bool {
    let Some(from) = state.workspace_drag else {
        return false;
    };
    let len = state.workspaces.len();
    if len == 0 || from >= len {
        return false;
    }
    let to = target.min(len - 1);
    if to == from {
        return false;
    }
    let workspace = state.workspaces.remove(from);
    state.workspaces.insert(to, workspace);
    state.workspace_drag = Some(to);
    let active = state.active_workspace;
    if active == from {
        state.active_workspace = to;
    } else if from < active && active <= to {
        state.active_workspace -= 1;
    } else if to <= active && active < from {
        state.active_workspace += 1;
    }
    true
}

/// 结束拖动排序。
pub fn end_workspace_drag(state: &mut AppState) {
    state.workspace_drag = None;
}

/// 保证当前工作区可见并钳制偏移；列表长度或可见行变化后调用；返回是否变化。
pub fn ensure_workspace_visible(state: &mut AppState, visible: usize) -> bool {
    let visible = visible.max(1);
    let max = workspace_scroll_max(state, visible);
    let active = state
        .active_workspace
        .min(state.workspaces.len().saturating_sub(1));
    let mut offset = state.workspace_scroll.min(max);
    if active < offset {
        offset = active;
    } else if active >= offset.saturating_add(visible) {
        offset = active + 1 - visible;
    }
    if offset == state.workspace_scroll {
        return false;
    }
    state.workspace_scroll = offset;
    true
}

/// 仅把滚动偏移钳制到合法范围（窗口缩放后调用，不强制跟随 active）；返回是否变化。
pub fn clamp_workspace_scroll(state: &mut AppState, visible: usize) -> bool {
    let max = workspace_scroll_max(state, visible);
    if state.workspace_scroll <= max {
        return false;
    }
    state.workspace_scroll = max;
    true
}

#[cfg(test)]
mod tests;
