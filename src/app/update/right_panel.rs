//! 右侧统一容器的面板切换与悬停状态。

use crate::app::state::{AppState, FocusedContainer, RightTab};

use super::prompt::finish_page_rename;
use super::selection::clear_selection;
use super::todo::commit_todo_edit;

/// 切换右侧容器面板。切走 Prompt 时提交页重命名并收起"指向可见位置的瞬时指针"；
/// Prompt 的正文、光标、滚动、撤销栈、页与视图模式**一律不动**。
pub fn select_right_tab(state: &mut AppState, tab: RightTab) -> bool {
    // 点 tab 就是「点了右侧容器」：**无论 tab 有没有变**都要把焦点交给它。
    // 先聚焦再判早退——否则「主区持焦点、右栏已停在 Todo」时再点 Todo tab
    // 会因同 tab 早退，高亮留在主区（用户报告的错位）。
    let changed = focus_right_tab(state, tab);
    if state.right_tab == tab {
        return changed;
    }
    finish_page_rename(state); // 非空写回并产出 Effect::RenamePage；空名保留旧名
    commit_todo_edit(state); // 切走 Todo 时提交进行中的编辑（与切工作区同口径）
    state.right_tab = tab;
    state.right_tab_hover = None;
    state.prompt_drag = None; // 拖拽现场不跨面板存活
    state.todo_hover = None;
    if tab != RightTab::Prompt {
        clear_selection(state); // prompt 选区
        state.prompt.clear_panel(); // 块/斜杠/提及/引用/粘贴面板
        state.prompt_tree_hover = None; // 列表悬停行
    }
    true
}

/// 把焦点交给右侧容器的指定面板；返回是否发生变化。
///
/// 右侧容器的两个面板各自是一个焦点目标：点 tab 之后焦点必须停在右栏，
/// 不能退回主区——否则会出现「切了 tab 主区高亮」的错位。
fn focus_right_tab(state: &mut AppState, tab: RightTab) -> bool {
    state.set_focus(match tab {
        RightTab::Prompt => FocusedContainer::Prompt,
        RightTab::Todo => FocusedContainer::Todo,
    })
}

/// 更新悬停的 tab；返回是否发生变化。
pub fn set_right_tab_hover(state: &mut AppState, hover: Option<RightTab>) -> bool {
    if state.right_tab_hover == hover {
        return false;
    }
    state.right_tab_hover = hover;
    true
}

/// prompt 面板当前是否可交互（面板可见且容器停在 Prompt）。
pub fn prompt_panel_active(state: &AppState) -> bool {
    state.right_tab == RightTab::Prompt && !state.prompt_collapsed
}
