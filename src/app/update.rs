//! 行为到状态的转换；几何信息由事件循环传入，保证逻辑纯且可测。

mod worktree;

pub use worktree::{
    apply_git_refresh, apply_worktree_list, apply_worktree_open_key, commit_worktree_open,
    open_worktree_dialog, request_git_refresh, set_worktree_open_selection, take_git_requests,
    toggle_workspace_group,
};

use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::layout::{Direction, Rect};

use crate::layout::{self, PaneId};

use super::actions::{Action, EditorCommand, OverlayKey};
use super::markdown::MentionEntry;
use super::overlay::{
    ConfirmClose, Menu, MenuCommand, Overlay, OverlayKind, OverlayTarget, Rename, RenameTarget,
    TextInput,
};
use super::selection::Selection;
use super::state::{
    AppState, AutoscrollDirection, PaneKind, PaneView, SelectionAutoscroll, Tab, Workspace,
    current_workspace_identity, home_dir, tab_label, unique_workspace_name, workspace_label,
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
                clear_selection(state);
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

/// 鼠标悬停提及条目：更新高亮；返回是否变化。
pub fn hover_mention(state: &mut AppState, index: usize) -> bool {
    state.prompt.mention_set_active(index)
}

/// 鼠标点选提及条目：确认插入。
pub fn select_mention(state: &mut AppState, index: usize) {
    state.prompt.mention_confirm_at(index);
}

/// 滚轮在提及面板上移动高亮：越界钳制不循环；返回是否变化。
pub fn scroll_mention(state: &mut AppState, delta: isize) -> bool {
    state.prompt.mention_scroll(delta)
}

/// 滚轮一格滚动的视觉行数；对齐 opencode 默认步长。
const WHEEL_LINES: isize = 3;

/// 边缘自动滚动每步的间隔。
const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(30);

/// lx 页动画帧间隔。
const LX_FRAME_INTERVAL: Duration = Duration::from_millis(200);

/// 按方向滚动 prompt 视口（负数向上、正数向下）；光标不动，编辑后自动吸回。
pub fn scroll_prompt(state: &mut AppState, direction: isize) {
    scroll_prompt_selection(state, direction, WHEEL_LINES);
}

/// 按方向滚动终端窗格回滚视口（负数向上、正数向下）；返回视口是否移动。
pub fn scroll_pane(state: &mut AppState, id: PaneId, direction: isize) -> bool {
    scroll_pane_lines(state, id, direction, WHEEL_LINES)
}

/// 按方向滚动终端窗格指定行数；返回视口是否移动。
fn scroll_pane_lines(state: &mut AppState, id: PaneId, direction: isize, lines: isize) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.kind != PaneKind::Terminal {
        return false;
    }
    // 仿真器的 delta 正数指向历史（视口上移），与方向的上下语义相反。
    pane.terminal
        .scroll_display(-(direction.signum() * lines) as i32)
}

/// 滚动 prompt 视口；选区是内容行坐标，视口滚动天然不影响选区；返回视口是否移动。
fn scroll_prompt_selection(state: &mut AppState, direction: isize, lines: isize) -> bool {
    let before = state.prompt.scroll();
    state.prompt.scroll_by(direction.signum() * lines);
    state.prompt.scroll() != before
}

/// prompt 拖拽选区时滚轮：滚动视口并保持选区锚点，终点跟到鼠标单元格；返回视口是否移动。
pub fn wheel_prompt_selection(state: &mut AppState, direction: isize, row: u16, col: u16) -> bool {
    if !scroll_prompt_selection(state, direction, WHEEL_LINES) {
        return false;
    }
    let prompt = state.prompt.id();
    drag_selection(state, prompt, row, col);
    place_prompt_cursor(state, row, col);
    true
}

/// 键盘或粘贴输入后把窗格视口吸回最新输出；返回视口是否移动。
pub fn reset_pane_scroll(state: &mut AppState, id: PaneId) -> bool {
    state
        .pane_mut_anywhere(id)
        .is_some_and(|pane| pane.kind == PaneKind::Terminal && pane.terminal.scroll_to_bottom())
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

/// 切换窗格主内容视图（lx 页 ↔ 终端）；返回是否变化。
///
/// 切到 lx 时清掉该窗格上的终端选区与滚动条拖拽：隐藏终端不接受交互。
pub fn toggle_pane_view(state: &mut AppState, id: PaneId) -> bool {
    let Some(view) = state.active_tab_mut().pane_mut(id).map(|pane| {
        pane.view = match pane.view {
            PaneView::Lx => PaneView::Terminal,
            PaneView::Terminal => PaneView::Lx,
        };
        pane.view
    }) else {
        return false;
    };
    if view == PaneView::Lx {
        if state.terminal_selection == Some(id) {
            clear_terminal_selection(state);
        }
        if state
            .terminal_scroll_drag
            .is_some_and(|(pane, _)| pane == id)
        {
            state.terminal_scroll_drag = None;
        }
    }
    true
}

/// 推进 lx 页动画：按帧间隔步进相位；无可见 lx 窗格时冻结；返回是否变化。
fn tick_lx_animation(state: &mut AppState, now: Instant) -> bool {
    if !state.lx_visible() || now.duration_since(state.lx_last_tick) < LX_FRAME_INTERVAL {
        return false;
    }
    state.lx_phase = state.lx_phase.wrapping_add(1);
    state.lx_last_tick = now;
    true
}

/// 按几何同步各窗格终端与 prompt 编辑器的尺寸。
///
/// prompt 文本区固定预留滚动条槽；尺寸变化会让视口选区坐标失效，此时清除选区。
pub fn resize_panes(state: &mut AppState, pane_rects: &[(PaneId, Rect)]) {
    for (id, rect) in pane_rects {
        if *id == state.prompt.id() {
            let (cols, rows) = layout::prompt_inner_size(*rect);
            if state.prompt.size() != (cols, rows) {
                clear_selection(state);
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

/// 更新窗格 cwd 标题标签；变化返回 true（供置脏重绘）。
pub fn update_pane_cwd(state: &mut AppState, id: PaneId, label: String) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.cwd_label.as_deref() == Some(label.as_str()) {
        return false;
    }
    pane.cwd_label = Some(label);
    true
}

/// 在 prompt 开始一次文本选择；与右栏拖拽、终端选区互斥。
pub fn begin_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    state.resizing_prompt = false;
    clear_terminal_selection(state);
    state.selection_autoscroll = None;
    let row = selection_content_row(state, pane, row);
    state.selection = Some(Selection::begin(pane, row, col));
}

/// 扩展当前 prompt 选区；窗格不一致时忽略。
pub fn drag_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    let row = selection_content_row(state, pane, row);
    if let Some(selection) = state.selection.as_mut()
        && selection.pane() == pane
    {
        selection.drag(row, col);
    }
}

/// 视口行换算成内容行：只有 prompt 视口会滚动，加回滚动量即可。
fn selection_content_row(state: &AppState, pane: PaneId, row: u16) -> i32 {
    if pane == state.prompt.id() {
        i32::from(row) + state.prompt.scroll() as i32
    } else {
        i32::from(row)
    }
}

/// 在终端窗格开始拖拽选择；选区存于仿真器，随输出滚动钉在内容上。
pub fn begin_terminal_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    state.selection = None;
    state.selection_autoscroll = None;
    clear_terminal_selection(state);
    let started = state.pane_mut_anywhere(pane).is_some_and(|target| {
        if target.kind != PaneKind::Terminal {
            return false;
        }
        target.terminal.start_selection(row, col);
        true
    });
    state.terminal_selection = started.then_some(pane);
}

/// 扩展终端选区到新的视口坐标；目标窗格不一致时忽略。
pub fn drag_terminal_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    if state.terminal_selection != Some(pane) {
        return;
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.terminal.update_selection(row, col);
    }
}

/// 结束终端选区并提取文本；空选区返回 None。
pub fn finish_terminal_selection(state: &mut AppState, pane: PaneId) -> Option<String> {
    state.terminal_selection = None;
    state.selection_autoscroll = None;
    let target = state.pane_mut_anywhere(pane)?;
    if target.kind != PaneKind::Terminal {
        return None;
    }
    // 全空白选区没有可复制的内容，与空选区一样不产生文本。
    target
        .terminal
        .take_selection_text()
        .filter(|text| !text.trim().is_empty())
}

/// 放弃进行中的终端选区（清空仿真器高亮）。
pub fn clear_terminal_selection(state: &mut AppState) {
    if let Some(pane) = state.terminal_selection.take()
        && let Some(target) = state.pane_mut_anywhere(pane)
    {
        target.terminal.clear_selection();
    }
}

/// 清除 prompt 与终端选区。
pub fn clear_selection(state: &mut AppState) {
    state.selection = None;
    state.selection_autoscroll = None;
    clear_terminal_selection(state);
}

/// 松开鼠标：结束拖动；空选区（未拖动）直接清除，非空选区保留供复制或删除。
pub fn end_selection_drag(state: &mut AppState) {
    state.selection_autoscroll = None;
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

/// 拖拽选择时按鼠标相对窗格内容区的位置安排边缘自动滚动。
///
/// 鼠标越出上下边缘：立即按距离滚动并延伸选区，随后进入定时自动滚动；
/// 停在边缘行：只登记计划；回到内部：清除计划。
pub fn arm_selection_autoscroll(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    mouse: (u16, u16),
    now: Instant,
) {
    if !selection_dragging_on(state, pane) {
        state.selection_autoscroll = None;
        return;
    }
    let top = inner.y;
    let bottom = inner.bottom().saturating_sub(1);
    let (direction, distance) = if mouse.1 < top {
        (AutoscrollDirection::Up, top - mouse.1)
    } else if mouse.1 > bottom {
        (AutoscrollDirection::Down, mouse.1 - bottom)
    } else if mouse.1 == top {
        (AutoscrollDirection::Up, 0)
    } else if mouse.1 == bottom {
        (AutoscrollDirection::Down, 0)
    } else {
        state.selection_autoscroll = None;
        return;
    };
    if distance > 0 {
        autoscroll_step(
            state,
            pane,
            direction,
            edge_scroll_lines(distance),
            mouse,
            inner,
        );
    }
    state.selection_autoscroll = Some(SelectionAutoscroll {
        pane,
        direction,
        mouse,
        inner,
        next_at: now + AUTOSCROLL_INTERVAL,
    });
}

/// 停止边缘自动滚动。
pub fn stop_selection_autoscroll(state: &mut AppState) {
    state.selection_autoscroll = None;
}

/// 自动滚动到点后补一步；返回是否产生变化。
fn tick_selection_autoscroll(state: &mut AppState, now: Instant) -> bool {
    let Some(autoscroll) = state.selection_autoscroll else {
        return false;
    };
    if now < autoscroll.next_at {
        return false;
    }
    if !selection_dragging_on(state, autoscroll.pane) {
        state.selection_autoscroll = None;
        return false;
    }
    if !autoscroll_step(
        state,
        autoscroll.pane,
        autoscroll.direction,
        1,
        autoscroll.mouse,
        autoscroll.inner,
    ) {
        // 到达回滚边界或视口尽头，停止。
        state.selection_autoscroll = None;
        return false;
    }
    if let Some(autoscroll) = state.selection_autoscroll.as_mut() {
        autoscroll.next_at = now + AUTOSCROLL_INTERVAL;
    }
    true
}

/// 自动滚动一步：滚动若干行并把选区终点推进到鼠标位置；返回视口是否移动。
fn autoscroll_step(
    state: &mut AppState,
    pane: PaneId,
    direction: AutoscrollDirection,
    lines: isize,
    mouse: (u16, u16),
    inner: Rect,
) -> bool {
    let step = match direction {
        AutoscrollDirection::Up => -1,
        AutoscrollDirection::Down => 1,
    };
    let terminal = state.terminal_selection == Some(pane);
    let moved = if terminal {
        scroll_pane_lines(state, pane, step, lines)
    } else {
        scroll_prompt_selection(state, step, lines)
    };
    if !moved {
        return false;
    }
    let row = mouse.1.clamp(inner.y, inner.bottom().saturating_sub(1)) - inner.y;
    let col = mouse.0.clamp(inner.x, inner.right().saturating_sub(1)) - inner.x;
    if terminal {
        drag_terminal_selection(state, pane, row, col);
    } else {
        drag_selection(state, pane, row, col);
        place_prompt_cursor(state, row, col);
    }
    true
}

/// 该窗格上是否存在进行中的选择拖拽。
fn selection_dragging_on(state: &AppState, pane: PaneId) -> bool {
    if state.terminal_selection == Some(pane) {
        return true;
    }
    state
        .selection
        .is_some_and(|selection| selection.is_dragging() && selection.pane() == pane)
}

/// 边缘距离对应的立即滚动行数（对齐 herdr：距离 × 3，夹在 3..=15）。
fn edge_scroll_lines(distance: u16) -> isize {
    usize::from(distance).saturating_mul(3).clamp(3, 15) as isize
}

/// prompt 选区文本（保留选区，供 Ctrl/Cmd+C 复制）；空选区返回 None。
pub fn prompt_selection_text(state: &AppState) -> Option<String> {
    let (start, end) = prompt_selection_range(state)?;
    state.prompt.selection_text(start, end)
}

/// prompt 选区对应的内容行列范围；选区不在 prompt 或为空时返回 None。
fn prompt_selection_range(state: &AppState) -> Option<((u16, u16), (u16, u16))> {
    let selection = state.selection?;
    if selection.pane() != state.prompt.id() {
        return None;
    }
    let (start, end) = selection.range()?;
    let content_row = |row: i32| u16::try_from(row).ok();
    Some((
        (content_row(start.0)?, start.1),
        (content_row(end.0)?, end.1),
    ))
}

/// prompt 选区对应的字节范围；选区不在 prompt 或为空时返回 None。
fn prompt_selection_bounds(state: &AppState) -> Option<(usize, usize)> {
    let (start, end) = prompt_selection_range(state)?;
    state.prompt.selection_bounds(start, end)
}

/// 在 prompt 右栏分割线上开始拖拽；清除已有选区。
pub fn begin_prompt_resize(state: &mut AppState) {
    clear_selection(state);
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
    clear_selection(state);
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

/// 清除已过期的 toast、推进拖选边缘自动滚动与 lx 动画；返回是否发生变化（用于置脏重绘）。
pub fn tick(state: &mut AppState, now: Instant) -> bool {
    let expired = state
        .toast
        .as_ref()
        .is_some_and(|toast| toast.is_expired(now));
    if expired {
        state.toast = None;
    }
    let scrolled = tick_selection_autoscroll(state, now);
    let animated = tick_lx_animation(state, now);
    expired || scrolled || animated
}

/// 最近一次定时到期时间（toast 消失、自动滚动或 lx 动画帧）；事件循环据此安排唤醒。
pub fn next_deadline(state: &AppState) -> Option<Instant> {
    let toast = state.toast.as_ref().and_then(Toast::next_deadline);
    let autoscroll = state
        .selection_autoscroll
        .map(|autoscroll| autoscroll.next_at);
    let animation = state
        .lx_visible()
        .then_some(state.lx_last_tick + LX_FRAME_INTERVAL);
    [toast, autoscroll, animation].into_iter().flatten().min()
}

/// 新建工作区并激活：名字取当前 cwd 末段（对齐 herdr），重名追加最小未用序号；
/// 立即登记 cwd 的 git 元数据查询；PTY 由事件循环按状态对齐启动。
pub fn create_workspace(state: &mut AppState) {
    let (cwd, base) = current_workspace_identity();
    let name = unique_workspace_name(&base, |candidate| {
        state
            .workspaces
            .iter()
            .any(|workspace| workspace.name == candidate)
    });
    state
        .workspaces
        .push(Workspace::single_terminal(name, cwd.clone()));
    state.active_workspace = state.workspaces.len().saturating_sub(1);
    if let Some(cwd) = cwd.as_deref() {
        request_git_refresh(state, cwd);
    }
    clear_selection(state);
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
    clear_selection(state);
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
    if state
        .workspaces
        .get(target)
        .is_some_and(|workspace| workspace.git.is_some())
    {
        commands.push(MenuCommand::OpenWorktree);
    }
    if state.workspaces.len() > 1 {
        commands.push(MenuCommand::CloseWorkspace);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::Workspace(target),
        commands,
        selected: 0,
    }));
}

/// 打开标签右键菜单；仅剩一个标签时不提供关闭项。
pub fn open_tab_menu(state: &mut AppState, tab: usize, anchor: (u16, u16)) {
    let workspace_index = state.active_workspace;
    let Some(workspace) = state.workspaces.get(workspace_index) else {
        return;
    };
    if tab >= workspace.tabs.len() {
        return;
    }
    state.workspace_scroll_drag = None;
    let mut commands = vec![MenuCommand::NewTab, MenuCommand::RenameTab];
    if workspace.tabs.len() > 1 {
        commands.push(MenuCommand::CloseTab);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::Tab {
            workspace: workspace_index,
            tab,
        },
        commands,
        selected: 0,
    }));
}

/// 打开窗格右键菜单；仅剩一个窗格时不提供关闭项。
pub fn open_pane_menu(state: &mut AppState, pane: PaneId, anchor: (u16, u16)) {
    let workspace = state.active_workspace;
    let tab = state.active_workspace().active_tab;
    let view = state.active_tab().pane(pane).map(|target| target.view);
    let Some(view) = view else {
        return;
    };
    state.workspace_scroll_drag = None;
    state.terminal_scroll_drag = None;
    let mut commands = vec![MenuCommand::SplitRight, MenuCommand::SplitDown];
    commands.push(match view {
        PaneView::Lx => MenuCommand::SwitchToTerminal,
        PaneView::Terminal => MenuCommand::SwitchToLx,
    });
    if state.active_tab().layout.pane_ids().len() > 1 {
        commands.push(MenuCommand::ClosePane);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::Pane {
            workspace,
            tab,
            pane,
        },
        commands,
        selected: 0,
    }));
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

/// 执行菜单当前项：新建立即生效；重命名打开输入浮层，关闭打开确认浮层。
pub fn activate_menu(state: &mut AppState) {
    let Some(Overlay::Menu(menu)) = state.overlay.take() else {
        return;
    };
    match (menu.commands.get(menu.selected), menu.target) {
        (Some(MenuCommand::NewTab), OverlayTarget::Tab { .. }) => create_tab(state),
        (Some(MenuCommand::RenameWorkspace), OverlayTarget::Workspace(target)) => {
            let Some(workspace) = state.workspaces.get(target) else {
                return;
            };
            state.overlay = Some(Overlay::Rename(Rename {
                target: RenameTarget::Workspace(target),
                input: TextInput::new(workspace.name.clone()),
            }));
        }
        (Some(MenuCommand::OpenWorktree), OverlayTarget::Workspace(target)) => {
            open_worktree_dialog(state, target);
        }
        (Some(MenuCommand::CloseWorkspace), OverlayTarget::Workspace(_)) => {
            state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
                target: menu.target,
            }));
        }
        (Some(MenuCommand::RenameTab), OverlayTarget::Tab { workspace, tab }) => {
            let Some(label) = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.tabs.get(tab))
                .map(|tab_state| tab_label(tab, tab_state.name.as_deref()))
            else {
                return;
            };
            state.overlay = Some(Overlay::Rename(Rename {
                target: RenameTarget::Tab { workspace, tab },
                input: TextInput::new(label),
            }));
        }
        (Some(MenuCommand::CloseTab), OverlayTarget::Tab { .. }) => {
            state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
                target: menu.target,
            }));
        }
        (
            Some(MenuCommand::SplitRight),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            split_pane(state, workspace, tab, pane, Direction::Horizontal);
        }
        (
            Some(MenuCommand::SplitDown),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            split_pane(state, workspace, tab, pane, Direction::Vertical);
        }
        (
            Some(MenuCommand::SwitchToTerminal | MenuCommand::SwitchToLx),
            OverlayTarget::Pane { pane, .. },
        ) => {
            toggle_pane_view(state, pane);
        }
        (Some(MenuCommand::ClosePane), OverlayTarget::Pane { .. }) => {
            state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
                target: menu.target,
            }));
        }
        _ => {}
    }
}

/// 在目标窗格处按方向分割并聚焦新窗格；新窗格继承源窗格视图。
fn split_pane(
    state: &mut AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
    direction: Direction,
) -> bool {
    let Some(tab_state) = state
        .workspaces
        .get_mut(workspace)
        .and_then(|workspace| workspace.tabs.get_mut(tab))
    else {
        return false;
    };
    if tab_state.split_pane(pane, direction).is_none() {
        return false;
    }
    state.prompt_focused = false;
    state.prompt.clear_panel();
    clear_selection(state);
    true
}

/// 浮层按键分派；输入层已按浮层种类过滤。
pub fn apply_overlay_key(state: &mut AppState, key: OverlayKey) {
    if matches!(state.overlay, Some(Overlay::WorktreeOpen(_))) {
        apply_worktree_open_key(state, key);
        return;
    }
    match key {
        OverlayKey::Esc => close_overlay(state),
        OverlayKey::Up => move_menu_selection(state, -1),
        OverlayKey::Down => move_menu_selection(state, 1),
        OverlayKey::Enter => match state.overlay.as_ref().map(Overlay::kind) {
            Some(OverlayKind::Menu) => activate_menu(state),
            Some(OverlayKind::Rename) => commit_rename(state),
            Some(OverlayKind::ConfirmClose) => confirm_close(state),
            Some(OverlayKind::WorktreeOpen) | None => {}
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
    match target {
        RenameTarget::Workspace(index) => {
            let Some(workspace) = state.workspaces.get_mut(index) else {
                state.overlay = None;
                return;
            };
            workspace.name = name;
            workspace.name_is_manual = true;
        }
        RenameTarget::Tab { workspace, tab } => {
            let Some(tab_state) = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.tabs.get_mut(tab))
            else {
                state.overlay = None;
                return;
            };
            tab_state.name = Some(name);
        }
    }
    state.overlay = None;
}

/// 确认关闭目标：工作区、标签或窗格；工作区与标签都至少保留一个。
fn confirm_close(state: &mut AppState) {
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.take() else {
        return;
    };
    match confirm.target {
        OverlayTarget::Workspace(target) => close_workspace(state, target),
        OverlayTarget::Tab { workspace, tab } => close_tab(state, workspace, tab),
        OverlayTarget::Pane {
            workspace,
            tab,
            pane,
        } => close_pane(state, workspace, tab, pane),
    }
}

/// 关闭工作区：至少保留一个；关闭当前工作区后焦点落到同索引，越界回退末项。
fn close_workspace(state: &mut AppState, target: usize) {
    if state.workspaces.len() <= 1 || target >= state.workspaces.len() {
        return;
    }
    let removed_active = target == state.active_workspace;
    state.workspaces.remove(target);
    if removed_active {
        state.active_workspace = state.active_workspace.min(state.workspaces.len() - 1);
    } else if target < state.active_workspace {
        state.active_workspace -= 1;
    }
    clear_selection(state);
}

/// 关闭标签：至少保留一个；关闭当前标签后焦点落到同索引，越界回退末项。
fn close_tab(state: &mut AppState, workspace: usize, tab: usize) {
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
fn close_pane(state: &mut AppState, workspace: usize, tab: usize, pane: PaneId) {
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

/// 工作区列表最大滚动偏移（按可见行计）；列表放得下时恒为 0。
pub fn workspace_scroll_max(state: &AppState, visible: usize) -> usize {
    state.workspace_rows().len().saturating_sub(visible.max(1))
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

/// 把终端窗格视口移动到距内容顶部 `offset` 行（0 为最旧一屏）；返回视口是否移动。
pub fn set_terminal_scroll(state: &mut AppState, pane: PaneId, offset: usize) -> bool {
    state.pane_mut_anywhere(pane).is_some_and(|target| {
        target.kind == PaneKind::Terminal && target.terminal.scroll_to_content_offset(offset)
    })
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
    let rows = state.workspace_rows();
    let active = rows
        .iter()
        .position(|row| row.index == state.active_workspace)
        .unwrap_or(0);
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
