//! 后台任务页的更新逻辑：进入/退出、列表回投、停止效应与 `↓` 入口。
//!
//! 这一页挂在 `pane.lx.task` 上（窗格内整页），因此这些测试同时锁住一件关键的事：
//! **进入页面不销毁 lx 页状态**——会话绑定、消息、草稿、光标全部原样保留。

use super::*;
use crate::app::state::{ProcessEntry, TaskTab};

/// 已绑定会话的 lx 窗格状态（面板只在有会话时能打开）。
fn state_with_session(draft: &str) -> (AppState, PaneId) {
    let mut state = ready_state(draft);
    let pane = focus_pane(&state);
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("sess-1".into());
    (state, pane)
}

fn entry(session_id: u64, command: &str) -> ProcessEntry {
    ProcessEntry {
        session_id,
        command: command.into(),
        cwd: "/workspace".into(),
        tty: false,
        started_at_ms: 0,
    }
}

fn processes() -> Vec<ProcessEntry> {
    vec![
        entry(11, "npm run dev"),
        entry(12, "cargo watch -x test"),
        entry(13, "python worker.py"),
    ]
}

/// 打开页面并回投三条进程。
fn open_with_processes(state: &mut AppState, pane: PaneId) {
    assert!(open_task_page(state, pane));
    assert!(on_process_listed(state, pane, processes()));
}

/// 当前页面（只读快照）。
fn page(state: &AppState, pane: PaneId) -> crate::app::state::TaskPage {
    state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.clone())
        .expect("后台任务页已打开")
}

#[test]
fn slash_command_list_no_longer_contains_stop() {
    // `/stop` 已移除：后台任务改由 `↓` 进入的独立页面管理。
    let names: Vec<&str> = crate::app::state::SlashCommand::ALL
        .iter()
        .map(|command| command.name())
        .collect();
    assert!(!names.contains(&"stop"), "命令表不应再有 stop：{names:?}");
}

#[test]
fn opening_the_page_requests_the_process_list() {
    let (mut state, pane) = state_with_session("x");
    assert!(open_task_page(&mut state, pane));
    // 页面骨架先出现，列表由事件层回投（`app/` 不持有进程句柄）。
    assert!(state.pane_anywhere(pane).unwrap().lx.task.is_some());
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::ListSessionProcesses { pane: p } if *p == pane
        )),
        "进入页面即请求列表：{:?}",
        state.effects
    );
}

#[test]
fn opening_the_page_keeps_every_other_lx_field() {
    // 这是「不要清理 lx 页」的核心断言：页面只是 Lx 的一个字段。
    let (mut state, pane) = state_with_session("草稿内容");
    {
        let target = state.pane_mut_anywhere(pane).unwrap();
        target.lx.chat.session_id = Some("sess-keep".into());
        target.lx.chat.follow_bottom = false;
        target.lx.chat.history_scroll = 7;
    }
    let before_input = state.pane_anywhere(pane).unwrap().lx.input.clone();
    let before_cursor = state.pane_anywhere(pane).unwrap().lx.cursor;

    assert!(open_task_page(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.session_id.as_deref(), Some("sess-keep"));
    assert!(!target.lx.chat.follow_bottom, "滚动跟随状态保留");
    assert_eq!(target.lx.chat.history_scroll, 7, "滚动偏移保留");
    assert_eq!(target.lx.input, before_input, "草稿保留");
    assert_eq!(target.lx.cursor, before_cursor, "光标保留");
}

#[test]
fn reopening_the_page_keeps_tab_and_highlight() {
    let (mut state, pane) = state_with_session("x");
    open_with_processes(&mut state, pane);
    {
        let page = state
            .pane_mut_anywhere(pane)
            .unwrap()
            .lx
            .task
            .as_mut()
            .unwrap();
        page.tab = TaskTab::Agents;
        page.window.move_active(2, 3);
    }
    // 再次进入（例如又按了一次 ↓）只刷新列表，不重置浏览位置。
    assert!(open_task_page(&mut state, pane));
    let page = page(&state, pane);
    assert_eq!(page.tab, TaskTab::Agents, "tab 未被重置");
    assert_eq!(page.window.active, 2, "高亮未被重置");
}

#[test]
fn closing_the_page_drops_only_the_page() {
    let (mut state, pane) = state_with_session("草稿");
    open_with_processes(&mut state, pane);
    assert!(close_task_page(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(target.lx.task.is_none(), "页面已关闭");
    assert_eq!(target.lx.input, "草稿", "草稿仍在");
    // 未打开时再关闭是 no-op。
    assert!(!close_task_page(&mut state, pane));
}

#[test]
fn listed_processes_fill_the_page_and_clamp_the_highlight() {
    let (mut state, pane) = state_with_session("x");
    open_with_processes(&mut state, pane);
    assert_eq!(page(&state, pane).processes.len(), 3);
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .task
        .as_mut()
        .unwrap()
        .window
        .move_active(2, 3);
    // 进程在页面停留期间退出：列表缩短，高亮必须随之钳制。
    assert!(on_process_listed(
        &mut state,
        pane,
        processes()[..1].to_vec()
    ));
    let page = page(&state, pane);
    assert_eq!(page.processes.len(), 1);
    assert_eq!(page.window.active, 0, "高亮被钳到唯一一项");
}

#[test]
fn listing_into_a_closed_page_is_a_no_op() {
    let (mut state, pane) = state_with_session("x");
    assert!(!on_process_listed(&mut state, pane, processes()));
}

#[test]
fn stopping_emits_the_effect_for_the_highlighted_process() {
    let (mut state, pane) = state_with_session("x");
    open_with_processes(&mut state, pane);
    state.effects.clear();
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .task
        .as_mut()
        .unwrap()
        .window
        .move_active(1, 3);
    assert!(stop_selected_process(&mut state, pane));
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::StopSessionProcess {
                pane: p,
                process_session_id: 12
            } if *p == pane
        )),
        "按高亮进程停止：{:?}",
        state.effects
    );
    // 页面不被关闭：常常要连停多个。
    assert!(state.pane_anywhere(pane).unwrap().lx.task.is_some());
}

#[test]
fn stopping_with_an_empty_list_does_nothing() {
    let (mut state, pane) = state_with_session("x");
    assert!(open_task_page(&mut state, pane));
    assert!(on_process_listed(&mut state, pane, Vec::new()));
    state.effects.clear();
    assert!(!stop_selected_process(&mut state, pane));
    assert!(state.effects.is_empty(), "没有可停的进程就不发效应");
}

#[test]
fn agents_tab_has_nothing_to_stop() {
    let (mut state, pane) = state_with_session("x");
    open_with_processes(&mut state, pane);
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .task
        .as_mut()
        .unwrap()
        .tab = TaskTab::Agents;
    state.effects.clear();
    assert!(!stop_selected_process(&mut state, pane));
    assert!(state.effects.is_empty(), "预留 tab 不产生停止效应");
}

#[test]
fn down_arrow_on_an_empty_draft_opens_the_page() {
    // opencode 口径：↓ 在历史不接管时向**下层派发**——空草稿 + 光标在末尾即换页。
    let (mut state, pane) = state_with_session("");
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_some(),
        "空草稿按 ↓ 进入后台任务页"
    );
}

#[test]
fn down_arrow_with_a_draft_still_moves_the_cursor() {
    // 有草稿时 ↓ 仍是光标移动：不能因为新增入口就让正在写的内容突然换页。
    let (mut state, pane) = state_with_session("写点东西");
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    assert!(state.pane_anywhere(pane).unwrap().lx.task.is_none());
}

#[test]
fn page_keys_do_not_leak_into_the_draft() {
    // 页面整页接管按键：字符不会被写进草稿。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    let before = state.pane_anywhere(pane).unwrap().lx.input.clone();
    for key in [
        crate::app::actions::LxInput::Char('x'),
        crate::app::actions::LxInput::Newline,
        crate::app::actions::LxInput::Enter,
    ] {
        crate::app::update::pane::apply_lx_input(&mut state, key);
    }
    assert_eq!(
        state.pane_anywhere(pane).unwrap().lx.input,
        before,
        "页面里的按键不进草稿"
    );
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_some(),
        "页面仍在"
    );
}

#[test]
fn panel_navigation_moves_the_highlight_and_cycles_tabs() {
    // `↑`/`↓` 移列表高亮；`←/→` 换 tab。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    assert_eq!(page(&state, pane).window.active, 1, "↓ 移动高亮");
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    assert_eq!(page(&state, pane).window.active, 0, "↑ 移动高亮");
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Right);
    assert_eq!(page(&state, pane).tab, TaskTab::Agents, "→ 换 tab");
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Left);
    assert_eq!(page(&state, pane).tab, TaskTab::Processes, "← 换回");
}

#[test]
fn up_at_the_top_returns_to_the_input_box() {
    // 用户要求：列表顶部（或空列表）按 ↑ 回到输入框 = 关闭面板。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    assert_eq!(page(&state, pane).window.active, 0, "初始在首项");
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_none(),
        "已在顶部时 ↑ 关闭面板、回到输入框"
    );
}

#[test]
fn up_on_an_empty_list_returns_to_the_input_box() {
    let (mut state, pane) = state_with_session("");
    assert!(open_task_page(&mut state, pane));
    assert!(on_process_listed(&mut state, pane, Vec::new()));
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_none(),
        "空列表时 ↑ 关闭面板"
    );
}

#[test]
fn closing_via_up_keeps_the_draft_and_cursor() {
    // 「回到原来的输入框」：草稿与光标必须原样。
    let (mut state, pane) = state_with_session("hello draft");
    let before = {
        let lx = &state.pane_anywhere(pane).unwrap().lx;
        (lx.input.clone(), lx.cursor)
    };
    open_with_processes(&mut state, pane);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    let lx = &state.pane_anywhere(pane).unwrap().lx;
    assert!(lx.task.is_none(), "面板已关闭");
    assert_eq!(lx.input, before.0, "草稿原样");
    assert_eq!(lx.cursor, before.1, "光标原样");
}

#[test]
fn page_keys_scroll_the_message_list_without_moving_the_highlight() {
    // `PageUp`/`PageDown` 是留给上方消息列表的辅助键：列表高亮不动，面板也不关。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    let before = page(&state, pane).window.active;
    for key in [
        crate::app::actions::LxInput::PageDown,
        crate::app::actions::LxInput::PageUp,
    ] {
        crate::app::update::pane::apply_lx_input(&mut state, key);
    }
    assert_eq!(
        page(&state, pane).window.active,
        before,
        "翻页键不动列表高亮"
    );
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_some(),
        "翻页键不关闭面板"
    );
}

#[test]
fn escape_closes_the_page_and_returns_to_the_lx_page() {
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(state.pane_anywhere(pane).unwrap().lx.task.is_none());
}

#[test]
fn delete_key_stops_the_highlighted_process() {
    // `^d` 在 lx 输入里映射为 `Delete`（对齐 opencode 的 `composer.shell.kill`）。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Delete);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::StopSessionProcess {
                pane: p,
                process_session_id: 11
            } if *p == pane
        )),
        "^d 停止高亮进程：{:?}",
        state.effects
    );
}

// ===== 无会话时不得打开面板 =====

#[test]
fn open_task_page_is_refused_without_a_session() {
    // 未绑定会话的空白草稿：没有可列的进程，也没有对应的进程运行实例。
    // 开了只会显示一个永远空着的列表，让人误以为"后台什么都没跑"。
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .session_id
            .is_none(),
        "前置：该窗格没有会话"
    );
    state.effects.clear();
    assert!(!open_task_page(&mut state, pane), "无会话时拒绝打开");
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_none(),
        "面板没有被创建"
    );
    assert!(
        state.effects.is_empty(),
        "也不该发出 ListSessionProcesses：{:?}",
        state.effects
    );
}

#[test]
fn down_arrow_does_nothing_without_a_session() {
    // 空草稿 `↓` 是面板入口；无会话时按键应当无声无息，不留下任何状态。
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .session_id
            .is_none()
    );
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_none(),
        "无会话时 ↓ 不打开面板"
    );
}

#[test]
fn open_task_page_works_once_a_session_exists() {
    // 对照：有会话时照常打开（避免上一条测试把功能整个关掉）。
    let (mut state, pane) = state_with_session("");
    assert!(open_task_page(&mut state, pane));
    assert!(state.pane_anywhere(pane).unwrap().lx.task.is_some());
}
