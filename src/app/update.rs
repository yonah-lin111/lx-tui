//! 行为到状态的转换；几何信息由事件循环传入，保证逻辑纯且可测。

use ratatui::layout::Rect;

use crate::layout::{self, PaneId};

use super::actions::Action;
use super::state::{AppState, Mode};

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
        Action::ToggleHelp => {
            state.mode = if state.mode == Mode::Help {
                Mode::Normal
            } else {
                Mode::Help
            };
        }
        Action::CloseOverlay => {
            if state.mode != Mode::Normal {
                state.mode = Mode::Normal;
            }
        }
    }
}

/// 按几何同步活动标签内各窗格的仿真尺寸。
pub fn resize_panes(state: &mut AppState, pane_rects: &[(PaneId, Rect)]) {
    let tab = state.active_tab_mut();
    for (id, rect) in pane_rects {
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
    fn help_toggles_and_escape_closes() {
        let mut state = AppState::demo();
        apply(Action::ToggleHelp, &mut state, &[]);
        assert_eq!(state.mode, Mode::Help);
        apply(Action::CloseOverlay, &mut state, &[]);
        assert_eq!(state.mode, Mode::Normal);
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
            (ids[1], Rect::new(10, 0, 10, 5)),
            (ids[2], Rect::new(10, 5, 10, 5)),
        ];
        state.active_tab_mut().layout.focus_pane(ids[0]);
        apply(Action::MoveFocus(NavDirection::Right), &mut state, &rects);
        assert_eq!(state.active_tab().layout.focus(), ids[1]);
        apply(Action::MoveFocus(NavDirection::Down), &mut state, &rects);
        assert_eq!(state.active_tab().layout.focus(), ids[2]);
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
}
