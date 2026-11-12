//! 后台任务页的按键路由：列表搜索/导航、Agents 追问与 Processes 停止。

use super::*;
use crate::app::update::chat;

/// `PageUp`/`PageDown` 对详情区的翻页量；其余键返回 None。
fn task_output_delta(command: LxInput) -> Option<isize> {
    match command {
        LxInput::PageUp => Some(-10),
        LxInput::PageDown => Some(10),
        _ => None,
    }
}

/// 当前 tab 的可见条目数（进程或筛选+搜索后的子 agent）。
fn task_active_len(state: &AppState, pane: PaneId) -> usize {
    state
        .pane_anywhere(pane)
        .and_then(|target| {
            let page = target.lx.task.as_ref()?;
            Some(match page.tab {
                crate::app::state::TaskTab::Processes => page.processes.len(),
                crate::app::state::TaskTab::Agents => {
                    page.agent_visible_indices(&target.lx.agents).len()
                }
                crate::app::state::TaskTab::Btw => 0,
            })
        })
        .unwrap_or(0)
}

/// 键盘/鼠标切换高亮后，详情贴回最新消息。
fn reset_active_agent_follow(state: &mut AppState, pane: PaneId) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    let Some(page) = target.lx.task.as_ref() else {
        return;
    };
    if page.tab != crate::app::state::TaskTab::Agents {
        return;
    }
    let Some(active) = page.selected_agent_index(&target.lx.agents) else {
        return;
    };
    let Some(run) = target.lx.agents.runs.get_mut(active) else {
        return;
    };
    run.follow = true;
    run.scroll = usize::MAX;
    target.lx.agents.selected_id = Some(run.agent_id.clone());
}

/// 把稳定 `selected_id` 映射到当前可见列表的高亮位置。
fn sync_agent_window(state: &mut AppState, pane: PaneId) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return;
    };
    if page.tab != crate::app::state::TaskTab::Agents {
        return;
    }
    let visible = target
        .lx
        .agents
        .visible_indices(page.agent_filter, page.agents_search.text());
    let position = target
        .lx
        .agents
        .selected_id
        .as_deref()
        .and_then(|selected| {
            visible.iter().position(|index| {
                target
                    .lx
                    .agents
                    .runs
                    .get(*index)
                    .is_some_and(|run| run.agent_id == selected)
            })
        })
        .unwrap_or(0);
    page.agents_window.active = position;
    page.agents_window.anchor = position;
    page.agents_window.viewport = None;
    page.agents_window.clamp(visible.len());
}

/// Agents 列表搜索框编辑；文本变化后按稳定 ID 恢复高亮。
fn edit_agents_search(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let page = target.lx.task.as_mut();
    let Some(page) = page else {
        return false;
    };
    let before = (
        page.agents_search.text().to_string(),
        page.agents_search.cursor(),
        page.agents_search.selection(),
    );
    match command {
        LxInput::Char(ch) => page.agents_search.insert_char(ch),
        LxInput::Backspace => page.agents_search.backspace(),
        LxInput::Delete => page.agents_search.delete(),
        LxInput::Left => page.agents_search.move_left(),
        LxInput::Right => page.agents_search.move_right(),
        LxInput::Home | LxInput::LineStart => page.agents_search.move_home(),
        LxInput::End | LxInput::LineEnd => page.agents_search.move_end(),
        LxInput::WordLeft => page.agents_search.move_word_left(),
        LxInput::WordRight => page.agents_search.move_word_right(),
        LxInput::DeleteToLineStart => page.agents_search.delete_to_line_start(),
        LxInput::DeleteToLineEnd => page.agents_search.delete_to_line_end(),
        LxInput::DeleteWordBackward => page.agents_search.delete_word_backward(),
        LxInput::DeleteWordForward => page.agents_search.delete_word_forward(),
        LxInput::SelectAll => page.agents_search.select_all(),
        LxInput::Undo => {
            page.agents_search.undo();
        }
        LxInput::Redo => {
            page.agents_search.redo();
        }
        _ => {}
    }
    let changed = before
        != (
            page.agents_search.text().to_string(),
            page.agents_search.cursor(),
            page.agents_search.selection(),
        );
    if changed {
        sync_agent_window(state, pane);
    }
    changed
}

/// 列表态：搜索优先，`Shift+Enter` 进入编辑，`Enter` 不动作。
fn apply_agents_list_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    match command {
        LxInput::Char(_)
        | LxInput::Backspace
        | LxInput::Delete
        | LxInput::Home
        | LxInput::End
        | LxInput::LineStart
        | LxInput::LineEnd
        | LxInput::WordLeft
        | LxInput::WordRight
        | LxInput::DeleteToLineStart
        | LxInput::DeleteToLineEnd
        | LxInput::DeleteWordBackward
        | LxInput::DeleteWordForward
        | LxInput::SelectAll
        | LxInput::Undo
        | LxInput::Redo => edit_agents_search(state, pane, command),
        LxInput::Left | LxInput::Right => {
            let empty = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.task.as_ref())
                .is_some_and(|page| page.agents_search.text().is_empty());
            if empty {
                let delta = if command == LxInput::Left { -1 } else { 1 };
                cycle_task_tab(state, pane, delta)
            } else {
                edit_agents_search(state, pane, command)
            }
        }
        LxInput::Up | LxInput::Down => {
            let delta = if command == LxInput::Up { -1 } else { 1 };
            let len = task_active_len(state, pane);
            let moved = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| page.move_active_len(len, delta));
            if moved {
                reset_active_agent_follow(state, pane);
                sync_agent_window(state, pane);
                ensure_selected_agent_loaded(state, pane);
                crate::app::update::clear_lx_selection_for(state, pane);
            }
            true
        }
        LxInput::EnterFolder => enter_task_agent_edit(state, pane),
        LxInput::Enter => true,
        LxInput::Tab => {
            let cycled = cycle_agent_filter(state, pane);
            if cycled {
                ensure_selected_agent_loaded(state, pane);
                crate::app::update::clear_lx_selection_for(state, pane);
            }
            true
        }
        LxInput::PrevStep => true,
        LxInput::PageUp | LxInput::PageDown => {
            let delta = if command == LxInput::PageUp { -10 } else { 10 };
            chat::scroll_agents_detail(state, pane, delta);
            true
        }
        LxInput::StopProcess => {
            chat::stop_selected_subagent(state, pane);
            true
        }
        LxInput::Escape => {
            let cleared = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| {
                    if page.agents_search.text().is_empty() {
                        false
                    } else {
                        page.agents_search.clear();
                        true
                    }
                });
            if cleared {
                sync_agent_window(state, pane);
                true
            } else {
                chat::close_task_page(state, pane);
                true
            }
        }
        _ => true,
    }
}

/// 编辑态：Enter 发送、Esc 返回列表、PgUp/PgDn 翻详情。
fn apply_agent_edit_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    match command {
        LxInput::Escape => leave_agent_edit(state, pane),
        LxInput::Enter => submit_agent_edit(state, pane),
        LxInput::StopProcess => {
            chat::stop_selected_subagent(state, pane);
            true
        }
        LxInput::PageUp | LxInput::PageDown => {
            let delta = if command == LxInput::PageUp { -10 } else { 10 };
            chat::scroll_agents_detail(state, pane, delta);
            true
        }
        LxInput::Tab | LxInput::PrevStep => true,
        _ => edit_agent_composer(state, pane, command),
    }
}

fn edit_agent_composer(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_ref() else {
        return false;
    };
    let Some(agent_id) = page.agent_edit.clone() else {
        return false;
    };
    let Some(composer) = target.lx.agents.composer_mut(&agent_id) else {
        return false;
    };
    let before = composer.revision();
    super::apply_editor_edit(composer.editor_mut(), command);
    composer.revision() != before
}

/// `TaskPage` 打开时的粘贴：列表态单行化写入搜索框，编辑态写入 Agent composer。
pub fn apply_task_paste(state: &mut AppState, pane: PaneId, text: &str) -> bool {
    let tab = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref().map(|page| page.tab));
    match tab {
        Some(crate::app::state::TaskTab::Agents) => {
            let editing = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.task.as_ref())
                .and_then(|page| page.agent_edit.clone());
            if let Some(agent_id) = editing {
                let Some(target) = state.pane_mut_anywhere(pane) else {
                    return false;
                };
                let Some(composer) = target.lx.agents.composer_mut(&agent_id) else {
                    return false;
                };
                if text.is_empty() {
                    return false;
                }
                composer.editor_mut().insert_str(text);
                return true;
            }
            let single_line: String = text
                .chars()
                .map(|ch| if matches!(ch, '\n' | '\r') { ' ' } else { ch })
                .collect();
            if single_line.is_empty() {
                return false;
            }
            {
                let Some(page) = state
                    .pane_mut_anywhere(pane)
                    .and_then(|target| target.lx.task.as_mut())
                else {
                    return false;
                };
                for ch in single_line.chars() {
                    page.agents_search.insert_char(ch);
                }
            }
            sync_agent_window(state, pane);
            true
        }
        Some(crate::app::state::TaskTab::Btw) => {
            let editing = state
                .pane_anywhere(pane)
                .is_some_and(|target| target.lx.btw.editing);
            if editing {
                return false;
            }
            let single_line: String = text
                .chars()
                .map(|ch| if matches!(ch, '\n' | '\r') { ' ' } else { ch })
                .collect();
            if single_line.is_empty() {
                return false;
            }
            let Some(target) = state.pane_mut_anywhere(pane) else {
                return false;
            };
            for ch in single_line.chars() {
                target.lx.btw.search.insert_char(ch);
            }
            target.lx.btw.sync_selection_after_filter();
            true
        }
        _ => false,
    }
}

/// `Tab`：轮转 `all -> active -> done -> all`，保留稳定选中。
fn cycle_agent_filter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    if page.tab != crate::app::state::TaskTab::Agents {
        return false;
    }
    page.agent_filter = page.agent_filter.next();
    page.agents_focus = None;
    sync_agent_window(state, pane);
    state.task_list_scroll_drag = None;
    true
}

/// 按当前筛选 + 搜索后的可见条目数钳制 Agents 高亮。
fn clamp_agents_window(state: &mut AppState, pane: PaneId) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return;
    };
    let visible = target
        .lx
        .agents
        .visible_indices(page.agent_filter, page.agents_search.text())
        .len();
    crate::app::state::lx::task::clamp_task_window_visible(
        &mut page.agents_window,
        visible,
        crate::ui::main_content::lx::task::LIST_MAX_SEARCH,
    );
}

/// 已加载的选中 run 刷新访问序；未加载则请求从 DB 懒加载。
fn ensure_selected_agent_loaded(state: &mut AppState, pane: PaneId) {
    let pending = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return;
        };
        let Some(page) = target.lx.task.as_ref() else {
            return;
        };
        if page.tab != crate::app::state::TaskTab::Agents {
            return;
        }
        let Some(index) = page.selected_agent_index(&target.lx.agents) else {
            return;
        };
        let Some(run) = target.lx.agents.runs.get_mut(index) else {
            return;
        };
        let agent_id = run.agent_id.clone();
        let child_session = run.child_session_id.clone();
        let loaded = run.loaded;
        if loaded {
            run.touch_access();
        }
        target.lx.agents.evict_excess_idle_chats(Some(&agent_id));
        (!loaded).then_some((agent_id, child_session))
    };
    if let Some((agent_id, child_session)) = pending {
        state
            .effects
            .push(crate::app::effects::Effect::LoadSubagentSession {
                pane,
                agent_id,
                child_session,
            });
    }
}

/// 鼠标点击筛选胶囊（事件层入口）。
pub fn select_task_agent_filter(
    state: &mut AppState,
    pane: PaneId,
    filter: crate::app::state::AgentFilter,
) -> bool {
    let changed = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        let Some(page) = target.lx.task.as_mut() else {
            return false;
        };
        if page.tab != crate::app::state::TaskTab::Agents || page.agent_filter == filter {
            return false;
        }
        page.agent_filter = filter;
        page.agents_focus = None;
        true
    };
    if changed {
        sync_agent_window(state, pane);
        state.task_list_scroll_drag = None;
        ensure_selected_agent_loaded(state, pane);
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    changed
}

/// 轮转底部 tab 并清理上一页的拖拽/高亮状态；返回是否切换。
pub fn cycle_task_tab(state: &mut AppState, pane: PaneId, delta: isize) -> bool {
    let cycled = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
        .is_some_and(|page| page.cycle_tab(delta));
    if cycled {
        state.task_scroll_drag = None;
        state.task_list_scroll_drag = None;
        state.task_agents_scroll_drag = None;
        state.task_btw_scroll_drag = None;
        state.task_list_click = None;
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.btw.editing = false;
            target.lx.btw.search.clear();
            target.lx.btw.sync_selection_after_filter();
        }
        clamp_agents_window(state, pane);
        ensure_selected_agent_loaded(state, pane);
        crate::app::update::clear_lx_selection_for(state, pane);
        crate::app::update::chat::btw::ensure_loaded(state, pane);
        ensure_agents_loaded(state, pane);
        ensure_agent_selection(state, pane);
        sync_agent_window(state, pane);
    }
    cycled
}

/// 首次进入 Agents 页时把稳定选中落到第一项。
fn ensure_agent_selection(state: &mut AppState, pane: PaneId) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    if !target.lx.is_agents_tab() || target.lx.agents.selected_id.is_some() {
        return;
    }
    let Some(page) = target.lx.task.as_ref() else {
        return;
    };
    let first = target
        .lx
        .agents
        .visible_indices(page.agent_filter, page.agents_search.text())
        .first()
        .copied()
        .and_then(|index| target.lx.agents.runs.get(index))
        .map(|run| run.agent_id.clone());
    if let Some(agent_id) = first {
        target.lx.agents.selected_id = Some(agent_id);
        sync_agent_window(state, pane);
    }
}

/// 打开/切回 Agents 页时确保列表已请求；列表为空才显示 loading。
fn ensure_agents_loaded(state: &mut AppState, pane: PaneId) {
    let Some((is_agents, empty, session)) = state.pane_anywhere(pane).map(|target| {
        (
            target.lx.is_agents_tab(),
            target.lx.agents.runs.is_empty(),
            target.lx.chat.session_id.clone(),
        )
    }) else {
        return;
    };
    if !is_agents || !empty {
        return;
    }
    let Some(session) = session else {
        return;
    };
    if let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.agents_loading = true;
    }
    state
        .effects
        .push(crate::app::effects::Effect::ListSubagents {
            pane,
            parent_session: session,
        });
}

/// 打开选中 agent 的追问 composer；cancelled 不可输入。
pub fn enter_task_agent_edit(state: &mut AppState, pane: PaneId) -> bool {
    let target_meta = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        let Some(page) = target.lx.task.as_ref() else {
            return false;
        };
        let Some(run) = page.selected_agent(&target.lx.agents) else {
            return false;
        };
        (run.status == "cancelled", run.agent_id.clone())
    };
    if target_meta.0 {
        crate::app::update::show_lx_toast(
            state,
            pane,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Warning,
                crate::ui::text::LX_TASK_AGENT_REPLY_CLOSED,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return false;
    }
    let agent_id = target_meta.1;
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.agents.ensure_composer(&agent_id);
    target.lx.agents.selected_id = Some(agent_id.clone());
    if let Some(page) = target.lx.task.as_mut() {
        page.agent_edit = Some(agent_id);
    }
    true
}

fn leave_agent_edit(state: &mut AppState, pane: PaneId) -> bool {
    let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    else {
        return false;
    };
    if page.agent_edit.take().is_none() {
        return false;
    }
    true
}

/// 提交追问：成功后清空草稿但留在编辑态。
fn submit_agent_edit(state: &mut AppState, pane: PaneId) -> bool {
    let (agent_id, text, closed, session_id) = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        let Some(page) = target.lx.task.as_ref() else {
            return false;
        };
        let Some(agent_id) = page.agent_edit.clone() else {
            return false;
        };
        let text = target
            .lx
            .agents
            .composer(&agent_id)
            .map(|composer| composer.text().trim().to_string())
            .unwrap_or_default();
        let closed = target
            .lx
            .agents
            .by_agent(&agent_id)
            .is_some_and(|run| run.status == "cancelled");
        (agent_id, text, closed, target.lx.chat.session_id.clone())
    };
    if text.is_empty() {
        return true;
    }
    if closed {
        crate::app::update::show_lx_toast(
            state,
            pane,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Warning,
                crate::ui::text::LX_TASK_AGENT_REPLY_CLOSED,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return true;
    }
    let Some(session_id) = session_id else {
        return false;
    };
    state
        .effects
        .push(crate::app::effects::Effect::SendSubagentUserInput {
            pane,
            session_id,
            agent_id: agent_id.clone(),
            text,
        });
    if let Some(composer) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.agents.composer_mut(&agent_id))
    {
        composer.clear();
    }
    true
}

pub(super) fn apply_task_page_input(state: &mut AppState, pane: PaneId, command: LxInput) {
    let Some(tab) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref().map(|page| page.tab))
    else {
        return;
    };
    if tab == crate::app::state::TaskTab::Btw {
        crate::app::update::chat::btw::apply_input(state, pane, command);
        return;
    }
    if tab == crate::app::state::TaskTab::Agents {
        let editing = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .is_some_and(|page| page.agent_edit.is_some());
        if editing {
            apply_agent_edit_input(state, pane, command);
        } else {
            apply_agents_list_input(state, pane, command);
        }
        return;
    }
    // Processes：保留原有行为。
    if let Some(delta) = task_output_delta(command) {
        if let Some(page) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.task.as_mut())
        {
            page.scroll_output(delta);
        }
        return;
    }
    match command {
        LxInput::Escape => {
            chat::close_task_page(state, pane);
        }
        LxInput::Up => {
            let len = task_active_len(state, pane);
            let moved = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| page.move_active_len(len, -1));
            if moved {
                crate::app::update::clear_lx_selection_for(state, pane);
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
        LxInput::Left => {
            cycle_task_tab(state, pane, -1);
        }
        LxInput::Right => {
            cycle_task_tab(state, pane, 1);
        }
        LxInput::StopProcess => {
            chat::stop_selected_process(state, pane);
        }
        _ => {}
    }
}
