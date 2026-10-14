//! prompt 编辑器命令、二级面板路由、草稿切换与模板块操作。

use crate::app::actions::EditorCommand;
use crate::app::markdown::MentionEntry;
use crate::app::prompt::{PromptEditing, PromptMode};
use crate::app::state::AppState;
use crate::layout::PaneId;

use super::prompt::{
    activate_prompt_row, apply_prompt_search, commit_prompt_buffer, move_prompt_selection,
    prompt_tree_mode, prompt_tree_visible_rows, select_prompt_item, show_prompt_tree,
    toggle_prompt_panel_mode,
};
use super::selection::{
    WHEEL_LINES, clear_selection, prompt_selection_bounds, scroll_prompt_selection,
};

/// 终端粘贴的普通文本：与 `InsertText` 一致，但插入后压制补全面板直到下一次编辑/移动。
///
/// 粘贴不是“正在输入”：若粘贴文本尾部凑巧是候选触发词，面板一旦打开，
/// 紧跟的 Enter 会把这段正文替换成候选原文（把刚粘贴的路径吃掉）。
pub fn apply_pasted_text(state: &mut AppState, text: String) {
    apply_editor(state, EditorCommand::InsertText(text));
    if !prompt_tree_mode(state) {
        state.prompt.suppress_completion_panels();
    }
}

/// 按键产生的编辑命令；仅当 prompt 聚焦时由事件循环调用。
///
/// prompt 存在选区时：输入类命令用新内容替换选区，Backspace/Delete 删除选区，
/// 其余命令先清除选区再执行（选区删除/替换作为一步撤销）。
pub fn apply_editor(state: &mut AppState, command: EditorCommand) {
    // 树视图轻量接管按键：上下移动、回车激活、左右展开折叠，其余吞掉。
    if prompt_tree_mode(state) {
        route_prompt_tree(state, &command);
        return;
    }
    sync_mention_root(state);
    if route_panel(state, &command) {
        return;
    }
    // 编辑绑定条目时 Esc 返回树视图并提交内容；无绑定时保持既有行为。
    if command == EditorCommand::Escape && prompt_editing(state) {
        show_prompt_tree(state);
        return;
    }
    if let Some((start, end)) = prompt_selection_bounds(state) {
        let replaced = match &command {
            EditorCommand::InsertChar(ch) => {
                state.prompt.replace_range(start, end, &ch.to_string())
            }
            EditorCommand::InsertText(text) => state.prompt.replace_range(start, end, text),
            EditorCommand::Newline | EditorCommand::NewlineBelow | EditorCommand::EnterFolder => {
                state.prompt.replace_range(start, end, "\n")
            }
            EditorCommand::Backspace | EditorCommand::Delete => {
                state.prompt.replace_range(start, end, "")
            }
            _ => false,
        };
        clear_selection(state);
        if replaced {
            return;
        }
    }
    match command {
        EditorCommand::InsertChar(ch) => state.prompt.insert_char(ch),
        EditorCommand::InsertText(text) => state.prompt.insert_str(&text),
        EditorCommand::Newline => state.prompt.newline(),
        EditorCommand::NewlineBelow => state.prompt.newline_below(),
        EditorCommand::EnterFolder => state.prompt.newline_below(),
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
        EditorCommand::Outdent => {
            if !state.prompt.cycle_template_status_at_cursor() {
                state.prompt.outdent();
            }
        }
        EditorCommand::Undo => state.prompt.undo(),
        EditorCommand::Redo => state.prompt.redo(),
        EditorCommand::Escape => {}
    }
}

/// 块命令面板打开时的按键优先：上下选择、回车确认、Esc 关闭；返回是否消费。
///
/// 优先级：粘贴选择 > 文件提及 > 字母快捷 > 斜杠命令 > 块命令；互斥面板同一时刻只可能
/// 打开一个；粘贴面板打开时其余按键按面板策略吞掉。
fn route_panel(state: &mut AppState, command: &EditorCommand) -> bool {
    if route_paste_panel(state, command) {
        return true;
    }
    if route_mention_panel(state, command) {
        return true;
    }
    if route_refer_panel(state, command) {
        return true;
    }
    if route_slash_panel(state, command) {
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

/// 粘贴二级面板按键：上下移动、回车确认、Esc 取消；面板打开时其余命令吞掉。
///
/// 确认时按高亮可用项执行（引用改写占位文本 / 路径保持）；取消回滚占位插入。
fn route_paste_panel(state: &mut AppState, command: &EditorCommand) -> bool {
    if state.prompt.paste_panel().is_none() {
        return false;
    }
    match command {
        EditorCommand::Up => {
            state.prompt.paste_move(-1);
        }
        EditorCommand::Down => {
            state.prompt.paste_move(1);
        }
        EditorCommand::Newline => {
            if let Some(action) = state
                .prompt
                .paste_panel()
                .and_then(|panel| panel.active_action())
            {
                state.prompt.paste_confirm(action);
            } else {
                state.prompt.paste_cancel();
            }
        }
        EditorCommand::Escape => {
            state.prompt.paste_cancel();
        }
        _ => {}
    }
    true
}

/// 字母快捷面板按键：上下移动、回车确认、Esc 关闭；返回是否消费。
fn route_refer_panel(state: &mut AppState, command: &EditorCommand) -> bool {
    match command {
        EditorCommand::Up => state.prompt.refer_move(-1),
        EditorCommand::Down => state.prompt.refer_move(1),
        EditorCommand::Newline => state.prompt.refer_confirm(),
        EditorCommand::Escape => state.prompt.refer_escape(),
        _ => false,
    }
}

/// 斜杠命令面板打开时的按键优先：上下选择、回车插入、Esc 关闭；返回是否消费。
fn route_slash_panel(state: &mut AppState, command: &EditorCommand) -> bool {
    match command {
        EditorCommand::Up => state.prompt.slash_move(-1),
        EditorCommand::Down => state.prompt.slash_move(1),
        EditorCommand::Newline => state.prompt.slash_confirm(),
        EditorCommand::Escape => state.prompt.slash_escape(),
        _ => false,
    }
}

/// 文件提及面板打开时的按键优先：上下选择、回车确认、Esc 关闭；返回是否消费。
///
/// `Shift+Enter` 在面板打开时由面板接管（高亮非目录或无子项目录 no-op）；面板未打开时
/// 回落为 `NewlineBelow`。目录回退不设专用键，走编辑器撤销（进入目录是一次可撤销编辑）。
fn route_mention_panel(state: &mut AppState, command: &EditorCommand) -> bool {
    match command {
        EditorCommand::Up => state.prompt.mention_move(-1),
        EditorCommand::Down => state.prompt.mention_move(1),
        EditorCommand::Newline => state.prompt.mention_confirm(),
        EditorCommand::Escape => state.prompt.mention_escape(),
        EditorCommand::EnterFolder => {
            if !state.prompt.mention_active() {
                return false;
            }
            state.prompt.mention_enter_folder();
            true
        }
        _ => false,
    }
}

/// 提示词树视图按键路由：上下移动选中、回车激活、Esc 返回编辑视图；
/// 其余可映射按键进入搜索框（与模态单行输入一致：字符/移动/删除/清空，
/// 清空过滤词走 Ctrl+C）；无法映射时吞掉。
fn route_prompt_tree(state: &mut AppState, command: &EditorCommand) {
    let visible = prompt_tree_visible_rows(state);
    match command {
        EditorCommand::Up => {
            move_prompt_selection(state, -1, visible);
        }
        EditorCommand::Down => {
            move_prompt_selection(state, 1, visible);
        }
        EditorCommand::Newline => {
            activate_prompt_row(state);
        }
        EditorCommand::Escape => {
            toggle_prompt_panel_mode(state);
        }
        _ => {
            apply_prompt_search(state, command);
        }
    }
}

/// 显示中工作区是否处于已绑定条目的编辑视图。
pub(crate) fn prompt_editing(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| workspace.prompt_panel.editing.is_some())
}

/// 显示中工作区是否处于已绑定条目的编辑视图（临时提示词不参与自动保存）。
pub(crate) fn prompt_item_editing(state: &AppState) -> bool {
    state
        .workspaces
        .get(state.prompt_workspace())
        .is_some_and(|workspace| {
            matches!(workspace.prompt_panel.editing, Some(PromptEditing::Item(_)))
        })
}

/// 把 prompt 的上下文根同步为提及扫描根与 worktree 仓库根。
///
/// 提及根在 `MentionState` 内按光标所在块优先取 `{wt:}` 绑定，这里只提供
/// 显式绑定路径（回退显示中工作区 cwd）与仓库根；worktree 根取工作区 git
/// 元数据（非 git 工作区不提供 `/gitWorkTree` 候选）。
pub(crate) fn sync_mention_root(state: &mut AppState) {
    let root = state
        .prompt_root
        .clone()
        .or_else(|| state.prompt_owner().cwd.clone());
    let repo = state
        .prompt_owner()
        .git
        .as_ref()
        .map(|git| git.repo_root.clone());
    state.prompt.set_mention_root(root);
    let _ = state.prompt.set_worktree_repo(repo);
}

/// 切换显示中的 prompt 草稿：`from` 为当前显示工作区，`to` 为目标工作区。
///
/// 目标草稿移入 `state.prompt` / `state.prompt_root`，旧草稿停回 `from` 槽位；
/// 槽位缺失（不变量被破坏）时兜底新建，避免 panic；
/// 切换前提交 `from` 编辑中的提示词内容并清除空白 temp 绑定
/// （空白 default temp 保留在列表，缓冲随之回到已保存态）；
/// 编辑绑定随工作区保留，切回时由 `restore_prompt_view` 恢复。
pub(crate) fn swap_display_prompt(state: &mut AppState, from: usize, to: usize) {
    if from == to {
        return;
    }
    // 工作区切换取消在途粘贴提案。
    state.prompt.paste_cancel();
    commit_prompt_buffer(state, from);
    if let Some(workspace) = state.workspaces.get_mut(from) {
        workspace.prompt_panel.drop_blank_temp_binding();
        workspace.prompt_panel.cancel_page_rename();
    }
    state.prompt.mark_saved();
    let mut next = state
        .workspaces
        .get_mut(to)
        .and_then(|workspace| workspace.prompt.take())
        .unwrap_or_default();
    std::mem::swap(&mut state.prompt, &mut next.editor);
    std::mem::swap(&mut state.prompt_root, &mut next.root);
    if let Some(workspace) = state.workspaces.get_mut(from) {
        workspace.prompt = Some(next);
    }
    restore_prompt_view(state, to);
    sync_mention_root(state);
}

/// 恢复目标工作区的 prompt 视图：已绑定条目/临时提示词（且仍在）保持编辑；
/// 否则绑定该工作区恒保留的空白 default temp（缺失时新建）；树视图下缓冲内容已过期，重载为空正文。
pub(crate) fn restore_prompt_view(state: &mut AppState, index: usize) {
    let Some(workspace) = state.workspaces.get(index) else {
        return;
    };
    let panel = &workspace.prompt_panel;
    if panel.mode == PromptMode::Editor {
        match panel.editing {
            Some(PromptEditing::Item(id)) if panel.tree.item(id).is_some() => {
                select_prompt_item(state, index, id);
                return;
            }
            Some(PromptEditing::Temp(id)) if panel.temp(id).is_some() => return,
            _ => {}
        }
    }
    if let Some(workspace) = state.workspaces.get_mut(index) {
        workspace.prompt_panel.mode = PromptMode::Editor;
        let id = workspace.prompt_panel.ensure_blank_temp();
        workspace.prompt_panel.editing = Some(PromptEditing::Temp(id));
    }
    state.prompt.set_text("");
}

/// 把激活工作区的挂起缓冲装入显示态（丢弃当前显示草稿）；槽位缺失时兜底新建。
///
/// 仅用于显示中工作区被关闭：其草稿随工作区销毁，显示切到新激活工作区；
/// 视图按目标工作区的编辑绑定恢复（条目或临时提示词）。
pub(crate) fn load_displayed_draft(state: &mut AppState) {
    let index = state.active_workspace;
    state.prompt.paste_cancel();
    let next = state
        .workspaces
        .get_mut(index)
        .and_then(|workspace| workspace.prompt.take())
        .unwrap_or_default();
    state.prompt = next.editor;
    state.prompt_root = next.root;
    restore_prompt_view(state, index);
    sync_mention_root(state);
}

/// 切换 prompt 钉住：未钉住时钉住当前显示的工作区，已钉住时释放。
pub fn toggle_prompt_pin(state: &mut AppState) {
    match state.prompt_pinned {
        None => state.prompt_pinned = Some(state.active_workspace),
        Some(_) => release_prompt_pin(state),
    }
}

/// 释放钉住：显示切回激活工作区草稿并清除选区；未钉住时 no-op。
pub fn release_prompt_pin(state: &mut AppState) {
    let Some(pinned) = state.prompt_pinned.take() else {
        return;
    };
    swap_display_prompt(state, pinned, state.active_workspace);
    clear_selection(state);
}

/// 写入文件提及扫描结果；按编辑器标识路由到显示中或挂起的草稿，工作区已关则丢弃。
pub fn apply_mention_entries(
    state: &mut AppState,
    prompt: PaneId,
    generation: u64,
    entries: Vec<MentionEntry>,
) {
    if state.prompt.id() == prompt {
        state.prompt.apply_mention_entries(generation, entries);
        return;
    }
    for workspace in &mut state.workspaces {
        if let Some(draft) = workspace.prompt.as_mut()
            && draft.editor.id() == prompt
        {
            draft.editor.apply_mention_entries(generation, entries);
            return;
        }
    }
}

/// 鼠标悬停提及条目：更新高亮；返回是否变化。
pub fn hover_mention(state: &mut AppState, index: usize) -> bool {
    state.prompt.mention_set_active(index)
}

/// 鼠标悬停字母快捷候选：更新高亮；返回是否变化。
pub fn hover_refer(state: &mut AppState, index: usize) -> bool {
    state.prompt.refer_set_active(index)
}

/// 鼠标点选字母快捷候选：确认插入完整原文。
pub fn select_refer(state: &mut AppState, index: usize) {
    state.prompt.refer_set_active(index);
    state.prompt.refer_confirm();
}

/// 鼠标悬停粘贴面板操作项：仅接受可用项；返回是否变化。
pub fn hover_paste(state: &mut AppState, index: usize) -> bool {
    state.prompt.paste_set_active(index)
}

/// 鼠标点选粘贴面板操作项：按该项执行。
pub fn select_paste(state: &mut AppState, index: usize) {
    if let Some(action) = crate::app::prompt::PasteAction::ALL.get(index).copied() {
        state.prompt.paste_confirm(action);
    }
}

/// 鼠标点选提及条目：确认插入。
pub fn select_mention(state: &mut AppState, index: usize) {
    state.prompt.mention_confirm_at(index);
}

/// 滚轮在提及面板上滚动可见窗口：`base` 为当前窗口起点；高亮不动。
pub fn scroll_mention(state: &mut AppState, delta: isize, base: usize) -> bool {
    state.prompt.mention_scroll(delta, base)
}

/// 鼠标悬停块命令条目：只提高亮、不动窗口锚点；返回是否变化。
pub fn hover_panel(state: &mut AppState, index: usize) -> bool {
    state.prompt.panel_set_active(index)
}

/// 鼠标点选块命令条目：确认插入。
pub fn select_panel(state: &mut AppState, index: usize) {
    state.prompt.panel_confirm_at(index);
}

/// 滚轮在块命令面板上滚动可见窗口：`base` 为当前窗口起点；高亮不动。
pub fn scroll_panel(state: &mut AppState, delta: isize, base: usize) -> bool {
    state.prompt.panel_scroll(delta, base)
}

/// 鼠标悬停斜杠命令条目：只提高亮、不动窗口锚点；返回是否变化。
pub fn hover_slash(state: &mut AppState, index: usize) -> bool {
    state.prompt.slash_set_active(index)
}

/// 鼠标点选斜杠命令条目：确认插入模板。
pub fn select_slash(state: &mut AppState, index: usize) {
    state.prompt.slash_confirm_at(index);
}

/// 滚轮在斜杠命令面板上滚动可见窗口：`base` 为当前窗口起点；高亮不动。
pub fn scroll_slash(state: &mut AppState, delta: isize, base: usize) -> bool {
    state.prompt.slash_scroll(delta, base)
}

/// 复制模板块正文（对齐 lx-agent：剔除标题占位行/补充子块/记录块标记/注释与空项）；
/// 返回待写入剪贴板的文本。
pub fn copy_template_block(state: &mut AppState, start_line: usize) -> Option<String> {
    state
        .prompt
        .template_block_body(start_line)
        .map(|body| crate::app::markdown::copy_template_content(&body))
}

/// 复制补充块正文（同模板块复制语义）；返回待写入剪贴板的文本。
pub fn copy_supple_block(state: &mut AppState, start_line: usize) -> Option<String> {
    state
        .prompt
        .supple_block_body(start_line)
        .map(|body| crate::app::markdown::copy_template_content(&body))
}

/// 复制光标所在模板块或补充块正文（Cmd/Ctrl+Shift+C）；不在任何块内返回 None。
pub fn copy_template_block_at_cursor(state: &mut AppState) -> Option<String> {
    state.prompt.copy_text_at_cursor()
}

/// 清理模板块内未填写的空项（单步撤销）；返回是否产生变更。
pub fn clean_template_block(state: &mut AppState, start_line: usize) -> bool {
    state.prompt.clean_template_block(start_line)
}

/// 删除整个模板块（单步撤销）；返回是否产生变更。
pub fn delete_template_block(state: &mut AppState, start_line: usize) -> bool {
    state.prompt.delete_template_block(start_line)
}

/// 切换模板块状态 todo → in_progress → done（单步撤销）；返回是否产生变更。
pub fn toggle_template_status(state: &mut AppState, start_line: usize) -> bool {
    state.prompt.toggle_template_status(start_line)
}

/// 清理补充块内未填写的空项（单步撤销）；返回是否产生变更。
pub fn clean_supple_block(state: &mut AppState, start_line: usize) -> bool {
    state.prompt.clean_supple_block(start_line)
}

/// 删除整个补充块（单步撤销）；返回是否产生变更。
pub fn delete_supple_block(state: &mut AppState, start_line: usize) -> bool {
    state.prompt.delete_supple_block(start_line)
}

/// 按方向滚动 prompt 视口（负数向上、正数向下）；光标不动，编辑后自动吸回。
pub fn scroll_prompt(state: &mut AppState, direction: isize) {
    scroll_prompt_selection(state, direction, WHEEL_LINES);
}

/// 鼠标点击 prompt：把视口单元格映射为光标位置。
///
/// 光标落到别的行会让二级 worktree 选择器失配关闭（面板重算时按命令行锚定同步，
/// 对齐 lx-agent）；落在原命令行上则保持打开且不改文本。
pub fn place_prompt_cursor(state: &mut AppState, row: u16, col: u16) {
    sync_mention_root(state);
    state.prompt.set_cursor_from_cell(row, col);
}

/// 点击 prompt：键盘焦点交给编辑器。
pub fn focus_prompt(state: &mut AppState) {
    sync_mention_root(state);
    state.prompt_focused = true;
}
