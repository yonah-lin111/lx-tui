//! 窗格与终端生命周期、PTY 喂入与 lx 视图交互。

use std::path::Path;

use ratatui::layout::Rect;

use crate::app::actions::LxInput;
use crate::app::state::{AppState, PaneKind, PaneView};
use crate::detect::PaneAgentSnapshot;
use crate::layout::{self, PaneId};

use super::prompt::cancel_page_rename;
use super::selection::{WHEEL_LINES, clear_selection, clear_terminal_selection};

/// 按方向滚动终端窗格回滚视口（负数向上、正数向下）；返回视口是否移动。
pub fn scroll_pane(state: &mut AppState, id: PaneId, direction: isize) -> bool {
    scroll_pane_lines(state, id, direction, WHEEL_LINES)
}

/// 按方向滚动终端窗格指定行数；返回视口是否移动。
pub(crate) fn scroll_pane_lines(
    state: &mut AppState,
    id: PaneId,
    direction: isize,
    lines: isize,
) -> bool {
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

/// 键盘或粘贴输入后把窗格视口吸回最新输出；返回视口是否移动。
pub fn reset_pane_scroll(state: &mut AppState, id: PaneId) -> bool {
    state
        .pane_mut_anywhere(id)
        .is_some_and(|pane| pane.kind == PaneKind::Terminal && pane.terminal.scroll_to_bottom())
}

/// 点击终端窗格：焦点回到窗格，prompt 失焦并关闭块命令面板与页重命名输入。
pub fn focus_pane(state: &mut AppState, id: PaneId) {
    state.prompt_focused = false;
    state.prompt.clear_panel();
    cancel_page_rename(state);
    state.active_tab_mut().layout.focus_pane(id);
}

/// 应用 lx 页输入命令到焦点窗格；非 lx 视图忽略。
///
/// md 面板打开时 `↑`/`↓` 移动高亮、`Tab` 采纳、`Esc` 关闭；其余按键走编辑。
pub fn apply_lx_input(state: &mut AppState, command: LxInput) {
    let Some(pane) = state.active_pane_mut() else {
        return;
    };
    if pane.view != PaneView::Lx {
        return;
    }
    let lx = &mut pane.lx;
    // 路径粘贴参照面板优先：与 md 块面板互斥；面板打开时其余按键按面板策略吞掉。
    if lx.paste_panel().is_some() {
        match command {
            LxInput::Up => {
                lx.paste_move(-1);
            }
            LxInput::Down => {
                lx.paste_move(1);
            }
            LxInput::Enter => match lx.paste_panel().and_then(|panel| panel.active_action()) {
                Some(action) => {
                    lx.paste_confirm(action);
                }
                None => {
                    lx.paste_cancel();
                }
            },
            LxInput::Escape => {
                lx.paste_cancel();
            }
            _ => {}
        }
        return;
    }
    let panel_open = lx.panel.is_some();
    match command {
        LxInput::Up if panel_open => lx.panel_move(-1),
        LxInput::Down if panel_open => lx.panel_move(1),
        LxInput::Tab if panel_open => {
            lx.panel_accept();
        }
        LxInput::Enter if panel_open => {
            lx.panel_accept();
        }
        LxInput::Escape if panel_open => lx.panel_close(),
        LxInput::Char(ch) => lx.insert_char(ch),
        LxInput::Newline => lx.newline(),
        LxInput::Backspace => lx.backspace(),
        LxInput::Delete => lx.delete(),
        LxInput::Left => lx.move_left(),
        LxInput::Right => lx.move_right(),
        LxInput::Up => lx.move_up(),
        LxInput::Down => lx.move_down(),
        LxInput::Home | LxInput::LineStart => lx.home(),
        LxInput::End | LxInput::LineEnd => lx.end(),
        LxInput::WordLeft => lx.word_left(),
        LxInput::WordRight => lx.word_right(),
        LxInput::DeleteToLineStart => lx.delete_to_line_start(),
        LxInput::DeleteToLineEnd => lx.delete_to_line_end(),
        LxInput::DeleteWordBackward => lx.delete_word_backward(),
        LxInput::DeleteWordForward => lx.delete_word_forward(),
        LxInput::Undo => lx.undo(),
        LxInput::Redo => lx.redo(),
        LxInput::Clear => lx.clear(),
        // 无面板时 Enter 预留给发送、Tab/Esc 吞掉。
        LxInput::Enter | LxInput::Tab | LxInput::Escape => {}
    }
}

/// 切换 lx 页选中入口（仅鼠标点击触发）；切换入口会取消在途粘贴面板。
pub fn set_lx_entry(state: &mut AppState, id: PaneId, entry: crate::app::state::LxEntry) {
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.paste_cancel();
        pane.lx.entry = entry;
    }
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
        // 切换视图时取消在途粘贴面板：隐藏的 lx 页不再持有悬空面板。
        pane.lx.paste_cancel();
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
            let (lx_cols, _) = layout::lx_input_size(*rect);
            pane.lx.resize(lx_cols);
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

/// 更新窗格 shell cwd（路径与标题标签）；任一变化返回 true（供置脏重绘）。
pub fn update_pane_cwd(state: &mut AppState, id: PaneId, cwd: &Path, label: String) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.cwd.as_deref() == Some(cwd) && pane.cwd_label.as_deref() == Some(label.as_str()) {
        return false;
    }
    pane.cwd = Some(cwd.to_path_buf());
    pane.cwd_label = Some(label);
    true
}

/// 更新窗格 Agent 快照（None 移除条目）；有变化返回 true（供置脏重绘）。
pub fn update_pane_agent(
    state: &mut AppState,
    id: PaneId,
    snapshot: Option<PaneAgentSnapshot>,
) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.agent == snapshot {
        return false;
    }
    pane.agent = snapshot;
    true
}

/// 点击 Agents 条目：切到所属标签、聚焦窗格、lx 视图切回终端、收回 prompt 焦点；返回是否成功。
pub fn focus_agent_pane(state: &mut AppState, pane: PaneId) -> bool {
    let Some(tab_index) = state
        .active_workspace()
        .tabs
        .iter()
        .position(|tab| tab.pane(pane).is_some())
    else {
        return false;
    };
    state.active_workspace_mut().active_tab = tab_index;
    if let Some(target) = state.active_tab_mut().pane_mut(pane)
        && target.view == PaneView::Lx
    {
        target.view = PaneView::Terminal;
    }
    focus_pane(state, pane);
    clear_selection(state);
    true
}

/// 把终端窗格视口移动到距内容顶部 `offset` 行（0 为最旧一屏）；返回视口是否移动。
pub fn set_terminal_scroll(state: &mut AppState, pane: PaneId, offset: usize) -> bool {
    state.pane_mut_anywhere(pane).is_some_and(|target| {
        target.kind == PaneKind::Terminal && target.terminal.scroll_to_content_offset(offset)
    })
}
