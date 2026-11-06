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

/// 键盘 `↑`/`↓` 切换高亮子 agent 后，与鼠标点选一致重置详情跟随与滚动。
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
    if let Some(run) = target.lx.agents.runs.get_mut(active) {
        run.follow = true;
        run.scroll = usize::MAX;
    }
}

/// 内联追问输入态：拦截全部按键，字符/编辑键委托给 `TextInput`。
fn apply_agent_reply_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let composing = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .is_some_and(|page| {
            page.tab == crate::app::state::TaskTab::Agents && page.agent_reply.is_some()
        });
    if !composing {
        return false;
    }
    match command {
        LxInput::Escape => {
            if let Some(page) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
            {
                page.agent_reply = None;
                page.agent_reply_target = None;
            }
        }
        LxInput::Enter => {
            submit_agent_reply(state, pane);
        }
        LxInput::Char(ch) => {
            edit_agent_reply(state, pane, |reply| reply.insert_char(ch));
        }
        LxInput::Backspace => edit_agent_reply(state, pane, |reply| reply.backspace()),
        LxInput::Delete => edit_agent_reply(state, pane, |reply| reply.delete()),
        LxInput::Left => edit_agent_reply(state, pane, |reply| reply.move_left()),
        LxInput::Right => edit_agent_reply(state, pane, |reply| reply.move_right()),
        LxInput::Home | LxInput::LineStart => {
            edit_agent_reply(state, pane, |reply| reply.move_home());
        }
        LxInput::End | LxInput::LineEnd => {
            edit_agent_reply(state, pane, |reply| reply.move_end());
        }
        LxInput::WordLeft => edit_agent_reply(state, pane, |reply| reply.move_word_left()),
        LxInput::WordRight => edit_agent_reply(state, pane, |reply| reply.move_word_right()),
        LxInput::DeleteWordBackward => {
            edit_agent_reply(state, pane, |reply| reply.delete_word_backward());
        }
        LxInput::DeleteToLineStart => {
            edit_agent_reply(state, pane, |reply| reply.delete_to_line_start());
        }
        LxInput::DeleteToLineEnd => {
            edit_agent_reply(state, pane, |reply| reply.delete_to_line_end());
        }
        LxInput::DeleteWordForward => {
            edit_agent_reply(state, pane, |reply| reply.delete_word_forward());
        }
        LxInput::SelectAll => {
            crate::app::update::clear_selection(state);
            edit_agent_reply(state, pane, |reply| reply.select_all());
        }
        LxInput::Undo => {
            edit_agent_reply(state, pane, |reply| {
                reply.undo();
            });
        }
        LxInput::Redo => {
            edit_agent_reply(state, pane, |reply| {
                reply.redo();
            });
        }
        // 输入态不响应列表/面板快捷键，全部吞掉。
        _ => {}
    }
    true
}

/// 修改内联追问输入框（输入态已经由调用方确认存在）。
fn edit_agent_reply(
    state: &mut AppState,
    pane: PaneId,
    edit: impl FnOnce(&mut crate::app::overlay::TextInput),
) {
    if let Some(reply) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
        .and_then(|page| page.agent_reply.as_mut())
    {
        edit(reply);
    }
}

/// 提交内联追问：非空文本按锁定的目标 `agent_id`（无锁定时回退当前选中 run）发出 `SendSubagentUserInput`。
fn submit_agent_reply(state: &mut AppState, pane: PaneId) -> bool {
    let mut closed = false;
    let effect = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        let Some(page) = target.lx.task.as_mut() else {
            return false;
        };
        let Some(text) = page
            .agent_reply
            .as_ref()
            .map(|input| input.text().trim().to_string())
        else {
            return false;
        };
        if text.is_empty() {
            return true;
        }
        let _ = page.agent_reply.take();
        let locked_target = page.agent_reply_target.take().map(|(id, _)| id);
        let target_agent_id = locked_target.or_else(|| {
            page.selected_agent_index(&target.lx.agents)
                .and_then(|index| target.lx.agents.runs.get(index))
                .map(|run| run.agent_id.clone())
        });
        let Some(agent_id) = target_agent_id else {
            return true;
        };
        if target
            .lx
            .agents
            .by_agent(&agent_id)
            .is_some_and(|run| run.status == "cancelled")
        {
            closed = true;
            None
        } else {
            target.lx.chat.session_id.clone().map(|session_id| {
                crate::app::effects::Effect::SendSubagentUserInput {
                    pane,
                    session_id,
                    agent_id,
                    text,
                }
            })
        }
    };
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
    if let Some(effect) = effect {
        state.effects.push(effect);
    }
    true
}

/// 打开内联追问；`cancelled` 子 agent 不可再接收输入。
fn open_agent_reply(state: &mut AppState, pane: PaneId) -> bool {
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
        let label = run
            .nickname
            .as_deref()
            .filter(|nickname| !nickname.is_empty())
            .or_else(|| (!run.title.is_empty()).then_some(run.title.as_str()))
            .unwrap_or_else(|| run.role.as_str())
            .to_string();
        (run.status == "cancelled", run.agent_id.clone(), label)
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
    let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    else {
        return false;
    };
    page.agent_reply_target = Some((target_meta.1, target_meta.2));
    page.agent_reply = Some(crate::app::overlay::TextInput::new(""));
    true
}

/// `TaskPage` 打开时的粘贴处理：若处于内联追问输入态则单行化写入 `agent_reply`，
/// 否则直接吞掉粘贴事件，绝不穿透污染隐藏的主聊天草稿框。
pub fn apply_task_paste(state: &mut AppState, pane: PaneId, text: &str) -> bool {
    let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    else {
        return false;
    };
    let Some(reply) = page.agent_reply.as_mut() else {
        return false;
    };
    let single_line: String = text
        .chars()
        .map(|ch| if matches!(ch, '\n' | '\r') { ' ' } else { ch })
        .collect();
    if single_line.is_empty() {
        return false;
    }
    for ch in single_line.chars() {
        reply.insert_char(ch);
    }
    true
}

/// `Tab`：轮转 `all -> active -> done -> all` 并钳制高亮。
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
    page.agent_reply = None;
    page.agent_reply_target = None;
    let filtered = target.lx.agents.filtered_indices(page.agent_filter).len();
    page.agents_window.clamp(filtered);
    true
}

/// 按当前筛选后的可见条目数钳制 Agents 高亮（切 tab 后调用）。
fn clamp_agents_window(state: &mut AppState, pane: PaneId) {
    let filtered = state.pane_anywhere(pane).and_then(|target| {
        target
            .lx
            .task
            .as_ref()
            .map(|page| target.lx.agents.filtered_indices(page.agent_filter).len())
    });
    let Some(filtered) = filtered else {
        return;
    };
    if let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.agents_window.clamp(filtered);
    }
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
        page.agent_reply = None;
        page.agent_reply_target = None;
        let filtered = target.lx.agents.filtered_indices(filter).len();
        page.agents_window.clamp(filtered);
        true
    };
    if changed {
        ensure_selected_agent_loaded(state, pane);
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    changed
}

pub(super) fn apply_task_page_input(state: &mut AppState, pane: PaneId, command: LxInput) {
    if apply_agent_reply_input(state, pane, command) {
        return;
    }
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
                reset_active_agent_follow(state, pane);
                ensure_selected_agent_loaded(state, pane);
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
                reset_active_agent_follow(state, pane);
                ensure_selected_agent_loaded(state, pane);
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
                clamp_agents_window(state, pane);
                ensure_selected_agent_loaded(state, pane);
                crate::app::update::clear_lx_selection_for(state, pane);
            }
        }
        LxInput::Right => {
            let cycled = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| page.cycle_tab(1));
            if cycled {
                clamp_agents_window(state, pane);
                ensure_selected_agent_loaded(state, pane);
                crate::app::update::clear_lx_selection_for(state, pane);
            }
        }
        // `Tab`：Agents 页签轮转状态筛选（all -> active -> done）。
        LxInput::Tab => {
            let cycled = cycle_agent_filter(state, pane);
            if cycled {
                ensure_selected_agent_loaded(state, pane);
                crate::app::update::clear_lx_selection_for(state, pane);
            }
        }
        // `Enter`：Agents 页签打开选中子 agent 的内联追问输入框。
        LxInput::Enter => {
            let agents_tab = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.task.as_ref())
                .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
            if agents_tab {
                open_agent_reply(state, pane);
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
        // Agents 页签里的 `^a`：停止整棵根会话树下所有运行中的子 agent。
        //
        // 输入框的 `^a` 是行首（`LineStart`），但任务页整页接管输入、不存在草稿编辑，
        // 因此这里复用同键位，不新增映射。
        LxInput::LineStart => {
            let agents_tab = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.task.as_ref())
                .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
            if agents_tab {
                stop_all_subagents(state, pane);
            }
        }
        // 其余键不属于面板：吞掉，不落到草稿。
        _ => {}
    }
}

/// `^a`：停止根会话树下全部运行中的子 agent（事件层级联处理后代）。
fn stop_all_subagents(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let running = target.lx.agents.running_count();
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        return false;
    };
    if running == 0 {
        return false;
    }
    state
        .effects
        .push(crate::app::effects::Effect::CancelSubagents {
            session_id,
            agent_id: None,
        });
    crate::app::update::show_lx_toast(
        state,
        pane,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Default,
            crate::ui::text::stopping_subagents(running),
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}
