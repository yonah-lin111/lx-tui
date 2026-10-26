//! 容器焦点：哪一侧持有键盘焦点由 [`FocusedContainer`] 单一真源决定。
//!
//! 这里只做"把焦点挪到某容器"的收尾动作（提交编辑、清瞬时面板/选区）；
//! 谁该高亮、谁该拿硬件光标全部由 `state.focus()` 派生，不在这里另存状态。

use crate::app::state::{AppState, FocusedContainer};
use crate::layout::PaneId;

use super::prompt::cancel_page_rename;
use super::selection::clear_selection;
use super::todo::commit_todo_edit;

/// 聚焦侧栏：键盘此后不写进任何输入框（侧栏没有键盘交互，路由层把它们全部吞掉）。
///
/// 与 [`focus_pane`] 同一套收尾：提交 todo 编辑、清选区与 prompt 瞬态面板。
pub fn focus_sidebar(state: &mut AppState) -> bool {
    commit_todo_edit(state);
    clear_selection(state);
    state.prompt.clear_panel();
    state.set_focus(FocusedContainer::Sidebar)
}

/// 把焦点交回主区，并先提交进行中的 todo 编辑。
///
/// 这是「离开 Todo」的唯一收口：切标签、开新标签、切工作区、分屏、打开 worktree
/// 等所有把焦点交给主区的路径都必须走这里。逐处手写 `set_focus(Panes)` 漏一处，
/// 就会留下一个悬空的编辑现场（草稿既没提交也没取消，后续行为取决于中间做过什么）。
pub fn focus_panes(state: &mut AppState) -> bool {
    commit_todo_edit(state);
    state.set_focus(FocusedContainer::Panes)
}

/// 点击终端窗格：焦点回到窗格，prompt 失焦并关闭块命令面板与页重命名输入。
pub fn focus_pane(state: &mut AppState, id: PaneId) {
    focus_panes(state);
    state.prompt.clear_panel();
    cancel_page_rename(state);
    state.active_tab_mut().layout.focus_pane(id);
}
