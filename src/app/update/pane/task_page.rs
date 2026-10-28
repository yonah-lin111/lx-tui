//! 后台任务页的按键路由：`↑↓` 高亮、`←→` 切 tab、`^d` 停止、`PgUp/PgDn` 翻页。
//!
//! 面板占的是输入框矩形，两者不可能同时可见，因此「离开面板」就是把
//! `pane.lx.task` 置空——草稿、光标与历史滚动本来就没被销毁。

use super::*;
use crate::app::update::chat;

/// 后台任务页的按键：`↑/↓` 移动、`^d` 停止高亮进程、`←/→` 换 tab、`Esc` 返回 lx 页。
///
/// 页面整页接管输入（不回落到草稿编辑）：它是一次独立浏览，退出来后草稿与光标
/// 仍是进入前的样子。不可用的键直接吞掉，避免"按下去没反应但草稿里多了字符"。
/// 后台任务面板里 `PageUp`/`PageDown` 对**输出区**的翻页量；其余键返回 None。
///
/// `↑`/`↓` 不在此列——它们归面板列表的高亮（见 [`apply_task_page_input`]）。
/// 面板接管历史区后上方已经没有可见的消息列表，因此翻页键改翻输出区
/// （决策 11：让它继续有意义，而不是去翻一块看不见的内容）。
fn task_output_delta(command: LxInput) -> Option<isize> {
    match command {
        LxInput::PageUp => Some(-10),
        LxInput::PageDown => Some(10),
        _ => None,
    }
}

/// 当前 tab 的条目数（进程或子 agent）。
fn task_active_len(state: &AppState, pane: PaneId) -> usize {
    state
        .pane_anywhere(pane)
        .and_then(|target| {
            target
                .lx
                .task
                .as_ref()
                .map(|page| (page, &target.lx.agents))
        })
        .map(|(page, agents)| page.len(agents))
        .unwrap_or(0)
}

pub(super) fn apply_task_page_input(state: &mut AppState, pane: PaneId, command: LxInput) {
    // `PageUp`/`PageDown`：翻输出区（`↑`/`↓` 已经归任务列表高亮）。
    if let Some(delta) = task_output_delta(command) {
        let agents_tab = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
        if agents_tab {
            chat::scroll_agents_detail(state, pane, delta);
        } else if let Some(page) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.task.as_mut())
        {
            page.scroll_output(delta);
        }
        return;
    }
    match command {
        // `esc` 无条件退出：列表很长时不必连按 ↑ 到顶。
        LxInput::Escape => {
            chat::close_task_page(state, pane);
        }
        // `↑`：列表非空且不在顶部 → 上移高亮；已在顶部或列表为空 → 回到输入框。
        //
        // 「回到输入框」就是把面板关掉（`pane.lx.task = None`）：面板占的是输入框的
        // 矩形，两者不可能同时可见，所以没有"切焦点"的中间态；草稿、光标、历史滚动
        // 本来就没被销毁，关掉即原样恢复。
        LxInput::Up => {
            // 条目数先取好：随后要同时可变借窗格（`move_active_len` 不再借 `state`）。
            let len = task_active_len(state, pane);
            let moved = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| page.move_active_len(len, -1));
            if moved {
                crate::app::update::clear_lx_selection_for(state, pane);
            } else {
                chat::close_task_page(state, pane);
            }
        }
        LxInput::Down => {
            let len = task_active_len(state, pane);
            let moved = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| page.move_active_len(len, 1));
            if moved {
                crate::app::update::clear_lx_selection_for(state, pane);
            }
        }
        // ←/→ 换 tab（对齐 opencode composer 的左右切 tab）。
        LxInput::Left => {
            let cycled = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| page.cycle_tab(-1));
            if cycled {
                crate::app::update::clear_lx_selection_for(state, pane);
            }
        }
        LxInput::Right => {
            let cycled = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| page.cycle_tab(1));
            if cycled {
                crate::app::update::clear_lx_selection_for(state, pane);
            }
        }
        // `^d` 停止高亮进程 / 子 agent（对齐 opencode `composer.shell.kill`）。
        //
        // 只认 `StopProcess`（仅 Ctrl+D 映射）：裸 `Delete` 键在编辑器语义里是
        // 「向后删一个字符」，面板里把它当停止用会让用户「想删一行」时
        // SIGTERM 掉一个正在跑的服务，且不可撤销。
        LxInput::StopProcess => {
            let agents_tab = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.task.as_ref())
                .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
            if agents_tab {
                chat::stop_selected_subagent(state, pane);
            } else {
                chat::stop_selected_process(state, pane);
            }
        }
        // 其余键不属于面板：吞掉，不落到草稿。
        _ => {}
    }
}
