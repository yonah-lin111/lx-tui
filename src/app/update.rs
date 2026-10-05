//! 行为到状态的转换；几何信息由事件循环传入，保证逻辑纯且可测。

use ratatui::layout::Rect;

use crate::layout::{self, PaneId};

use super::actions::Action;
use super::selection::Selection;
use super::state::{AppState, PaneKind};

/// 应用行为；窗格几何仅为方向导航所需，其余行为忽略。
pub fn apply(action: Action, state: &mut AppState, pane_rects: &[(PaneId, Rect)]) {
    match action {
        Action::Quit => state.should_quit = true,
        Action::FocusNextPane => state.active_tab_mut().layout.focus_next(),
        Action::FocusPrevPane => state.active_tab_mut().layout.focus_prev(),
        Action::MoveFocus(direction) => {
            let current = state.active_tab().layout.focus();
            if let Some(target) = layout::pane_in_direction(pane_rects, current, direction) {
                state.active_tab_mut().layout.focus_pane(target);
            }
        }
        Action::NextTab => {
            let workspace = state.active_workspace_mut();
            workspace.active_tab = (workspace.active_tab + 1) % workspace.tabs.len();
        }
        Action::PrevTab => {
            let workspace = state.active_workspace_mut();
            workspace.active_tab =
                (workspace.active_tab + workspace.tabs.len() - 1) % workspace.tabs.len();
        }
        Action::SelectWorkspace(index) => {
            if index < state.workspaces.len() {
                state.active_workspace = index;
            }
        }
        Action::ToggleSidebar => state.sidebar_collapsed = !state.sidebar_collapsed,
        Action::TogglePrompt => toggle_prompt(state),
    }
}

/// 切换当前标签 prompt 占位窗格的折叠状态；无 prompt 窗格时无操作。
fn toggle_prompt(state: &mut AppState) {
    let tab = state.active_tab_mut();
    let Some(id) = tab.prompt_pane() else {
        return;
    };
    let collapsed = tab.layout.collapsed() == Some(id);
    tab.layout.set_collapsed(id, !collapsed);
}

/// 按几何同步活动标签内各窗格的仿真尺寸。
///
/// 折叠窗格保持折叠前尺寸：alacritty 在 1 列宽 + 宽字符内容下 reflow 会死循环，
/// 且折叠窗格不渲染内容，展开时再按新几何同步。
pub fn resize_panes(state: &mut AppState, pane_rects: &[(PaneId, Rect)]) {
    let tab = state.active_tab_mut();
    let collapsed = tab.layout.collapsed();
    for (id, rect) in pane_rects {
        if Some(*id) == collapsed {
            continue;
        }
        let (cols, rows) = layout::pane_inner_size(*rect);
        if let Some(pane) = tab.pane_mut(*id) {
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

/// 在窗格内容区开始一次文本选择。
pub fn begin_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
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

/// 结束选区并提取文本（选区保留高亮）；未拖动或空占位窗格返回 None。
pub fn finish_selection(state: &mut AppState) -> Option<String> {
    let selection = state.selection?;
    let (start, end) = selection.range()?;
    let pane = state.pane_mut_anywhere(selection.pane())?;
    if !matches!(pane.kind, PaneKind::Terminal | PaneKind::Prompt) {
        return None;
    }
    pane.terminal
        .text_in_range(start, end)
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::NavDirection;
    use crate::terminal::GridSize;

    #[test]
    fn quit_sets_flag() {
        let mut state = AppState::demo();
        apply(Action::Quit, &mut state, &[]);
        assert!(state.should_quit);
    }

    #[test]
    fn select_workspace_ignores_out_of_range() {
        let mut state = AppState::demo();
        apply(Action::SelectWorkspace(9), &mut state, &[]);
        assert_eq!(state.active_workspace, 0);
        apply(Action::SelectWorkspace(1), &mut state, &[]);
        assert_eq!(state.active_workspace, 1);
    }

    #[test]
    fn tab_switching_wraps() {
        let mut state = AppState::demo();
        apply(Action::PrevTab, &mut state, &[]);
        assert_eq!(state.active_workspace().active_tab, 1);
        apply(Action::NextTab, &mut state, &[]);
        assert_eq!(state.active_workspace().active_tab, 0);
    }

    #[test]
    fn move_focus_uses_geometry() {
        let mut state = AppState::demo();
        let ids = state.active_tab().layout.pane_ids();
        let rects = vec![
            (ids[0], Rect::new(0, 0, 10, 10)),
            (ids[1], Rect::new(10, 0, 10, 10)),
        ];
        state.active_tab_mut().layout.focus_pane(ids[0]);
        apply(Action::MoveFocus(NavDirection::Right), &mut state, &rects);
        assert_eq!(state.active_tab().layout.focus(), ids[1]);
        apply(Action::MoveFocus(NavDirection::Left), &mut state, &rects);
        assert_eq!(state.active_tab().layout.focus(), ids[0]);
    }

    #[test]
    fn move_focus_skips_collapsed_pane() {
        let mut state = AppState::demo();
        let prompt = state.active_tab().prompt_pane();
        assert!(prompt.is_some());
        let Some(prompt) = prompt else { return };
        let shell = state.active_tab().layout.focus();
        let rects = vec![
            (shell, Rect::new(0, 0, 10, 10)),
            (prompt, Rect::new(10, 0, 10, 10)),
        ];
        apply(Action::TogglePrompt, &mut state, &[]);
        apply(Action::MoveFocus(NavDirection::Right), &mut state, &rects);
        assert_eq!(state.active_tab().layout.focus(), shell);
    }

    #[test]
    fn toggle_prompt_collapses_and_restores() {
        let mut state = AppState::demo();
        let prompt = state.active_tab().prompt_pane();
        assert!(prompt.is_some());
        let Some(prompt) = prompt else { return };
        assert_eq!(state.active_tab().layout.collapsed(), None);
        apply(Action::TogglePrompt, &mut state, &[]);
        assert_eq!(state.active_tab().layout.collapsed(), Some(prompt));
        apply(Action::TogglePrompt, &mut state, &[]);
        assert_eq!(state.active_tab().layout.collapsed(), None);
    }

    #[test]
    fn resize_syncs_terminal_size() {
        let mut state = AppState::demo();
        let ids = state.active_tab().layout.pane_ids();
        let rects = vec![(ids[0], Rect::new(0, 0, 12, 6))];
        resize_panes(&mut state, &rects);
        let pane = state.active_tab().pane(ids[0]);
        assert_eq!(
            pane.map(|pane| pane.terminal.size()),
            Some(GridSize { cols: 10, rows: 4 })
        );
    }

    #[test]
    fn collapsed_pane_keeps_terminal_size() {
        let mut state = AppState::demo();
        let prompt = state.active_tab().prompt_pane();
        let shell = state.active_tab().layout.focus();
        assert!(prompt.is_some());
        let Some(prompt) = prompt else { return };
        let before = state
            .active_tab()
            .pane(prompt)
            .map(|pane| pane.terminal.size());
        assert_eq!(before, Some(GridSize { cols: 80, rows: 24 }));

        apply(Action::TogglePrompt, &mut state, &[]);
        resize_panes(
            &mut state,
            &[
                (shell, Rect::new(24, 1, 73, 28)),
                (prompt, Rect::new(97, 1, 3, 28)),
            ],
        );
        assert_eq!(
            state
                .active_tab()
                .pane(prompt)
                .map(|pane| pane.terminal.size()),
            before
        );

        apply(Action::TogglePrompt, &mut state, &[]);
        resize_panes(&mut state, &[(prompt, Rect::new(62, 1, 38, 28))]);
        assert_eq!(
            state
                .active_tab()
                .pane(prompt)
                .map(|pane| pane.terminal.size()),
            Some(GridSize { cols: 36, rows: 26 })
        );
    }

    #[test]
    fn feed_and_exit_mark_pane() {
        let mut state = AppState::demo();
        let id = state.active_tab().layout.focus();
        let responses = feed_pane(&mut state, id, b"\x1b[6n");
        assert_eq!(String::from_utf8_lossy(&responses), "\x1b[1;1R");
        mark_pane_exited(&mut state, id);
        assert_eq!(
            state.active_tab().pane(id).map(|pane| pane.exited),
            Some(true)
        );
    }

    #[test]
    fn selection_finish_extracts_terminal_text() {
        let mut state = AppState::demo();
        let logs = state.workspaces[0].tabs[1].layout.pane_ids()[0];
        feed_pane(&mut state, logs, b"hello");
        begin_selection(&mut state, logs, 0, 0);
        drag_selection(&mut state, logs, 0, 4);
        assert_eq!(finish_selection(&mut state).as_deref(), Some("hello"));
        assert!(state.selection.is_some());
    }

    #[test]
    fn selection_ignores_non_terminal_pane() {
        let mut state = AppState::demo();
        let focus = state.active_tab().layout.focus();
        begin_selection(&mut state, focus, 0, 0);
        drag_selection(&mut state, focus, 0, 3);
        assert_eq!(finish_selection(&mut state), None);
        assert!(state.selection.is_some());
    }

    #[test]
    fn selection_extracts_prompt_text() {
        let mut state = AppState::demo();
        let prompt = state.active_tab().prompt_pane();
        assert!(prompt.is_some());
        let Some(prompt) = prompt else { return };
        begin_selection(&mut state, prompt, 0, 0);
        drag_selection(&mut state, prompt, 0, 3);
        assert_eq!(finish_selection(&mut state).as_deref(), Some("Drag"));
    }

    #[test]
    fn selection_drag_ignores_other_pane() {
        let mut state = AppState::demo();
        let focus = state.active_tab().layout.focus();
        let logs = state.workspaces[0].tabs[1].layout.pane_ids()[0];
        begin_selection(&mut state, logs, 0, 0);
        drag_selection(&mut state, focus, 2, 2);
        assert_eq!(finish_selection(&mut state), None);
    }
}
