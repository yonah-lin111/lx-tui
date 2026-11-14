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
    on_process_listed(&mut state, pane, Vec::new());
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
fn up_at_the_top_keeps_the_panel_open() {
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    assert_eq!(page(&state, pane).window.active, 0, "初始在首项");
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_some(),
        "列表边界 ↑ 不关闭面板"
    );
}

#[test]
fn up_on_an_empty_list_keeps_the_panel_open() {
    let (mut state, pane) = state_with_session("");
    assert!(open_task_page(&mut state, pane));
    on_process_listed(&mut state, pane, Vec::new());
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    assert!(
        state.pane_anywhere(pane).unwrap().lx.task.is_some(),
        "空列表 ↑ 不关闭面板"
    );
}

#[test]
fn up_at_the_top_keeps_the_draft_and_cursor() {
    let (mut state, pane) = state_with_session("hello draft");
    let before = {
        let lx = &state.pane_anywhere(pane).unwrap().lx;
        (lx.input.clone(), lx.cursor)
    };
    open_with_processes(&mut state, pane);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Up);
    let lx = &state.pane_anywhere(pane).unwrap().lx;
    assert!(lx.task.is_some());
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
fn page_keys_scroll_the_agents_detail_on_the_agents_tab() {
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    let mut run = crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: "a1".into(),
        nickname: None,
        agent_path: "/root/agent".into(),
        child_session_id: "s1".into(),
        parent_session_id: "sess-1".into(),
        spawn_message_id: None,
        role: crate::chat::agent::AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "running".into(),
        title: "investigate".into(),
        created_at_ms: 0,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    });
    run.visible = 10;
    run.total_rows = 100;
    run.scroll = 90;
    run.follow = true;
    crate::app::update::on_subagent_started(&mut state, pane, run);
    if let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.tab = TaskTab::Agents;
    }
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::PageUp);
    let run = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_session("s1")
        .expect("run");
    assert_eq!(run.scroll, 80);
    assert!(!run.follow, "上翻详情后停止跟随");
}

#[test]
fn escape_closes_the_page_and_returns_to_the_lx_page() {
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(state.pane_anywhere(pane).unwrap().lx.task.is_none());
}

#[test]
fn ctrl_d_on_agents_tab_stops_the_selected_subagent() {
    let (mut state, pane) = state_with_session("");
    assert!(open_task_page(&mut state, pane));
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .task
        .as_mut()
        .unwrap()
        .tab = TaskTab::Agents;
    let run =
        crate::app::state::lx::agent::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
            agent_id: "sub-a".into(),
            nickname: None,
            agent_path: "/root/agent".into(),
            child_session_id: "sess-a".into(),
            parent_session_id: "sess-1".into(),
            spawn_message_id: None,
            role: crate::chat::agent::AgentRole::Explorer,
            model_provider: String::new(),
            model_id: String::new(),
            model_variant: None,
            status: "running".into(),
            title: "investigate".into(),
            created_at_ms: 0,
            completed_at_ms: None,
            report_json: None,
            report_delivered: false,
        });
    assert!(crate::app::update::on_subagent_started(
        &mut state, pane, run
    ));
    let second =
        crate::app::state::lx::agent::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
            agent_id: "sub-b".into(),
            nickname: None,
            agent_path: "/root/agent".into(),
            child_session_id: "sess-b".into(),
            parent_session_id: "sess-1".into(),
            spawn_message_id: None,
            role: crate::chat::agent::AgentRole::Executor,
            model_provider: String::new(),
            model_id: String::new(),
            model_variant: None,
            status: "running".into(),
            title: "second".into(),
            created_at_ms: 0,
            completed_at_ms: None,
            report_json: None,
            report_delivered: false,
        });
    assert!(crate::app::update::on_subagent_started(
        &mut state, pane, second
    ));
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .task
        .as_mut()
        .unwrap()
        .agents_window
        .active = 1;
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::StopProcess);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::CancelSubagents { session_id, agent_id }
                if session_id == "sess-1" && agent_id.as_deref() == Some("sub-b")
        )),
        "Agents 面板 ^d 只停选中的子 agent：{:?}",
        state.effects
    );
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::StopSessionProcess { .. }
        )),
        "不能误停后台进程"
    );
}

#[test]
fn ctrl_d_stops_the_highlighted_process() {
    // Ctrl+D 映射为 `StopProcess`（对齐 opencode 的 `composer.shell.kill`）。
    // **不能**复用裸 `Delete`：那是行内删字符，面板里当停止用会误杀服务。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    state.effects.clear();
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::StopProcess);
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

// ===== 卡片 `[open]` 打开面板并定位高亮（决策 4/9）=====

#[test]
fn open_from_card_focuses_that_process() {
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    assert_eq!(page(&state, pane).window.active, 0, "初始高亮在首项");

    // 卡片指向第三条进程：打开面板后高亮必须落到它。
    assert!(open_task_page_at(&mut state, pane, Some(13)));
    let page = page(&state, pane);
    assert_eq!(
        page.selected().map(|process| process.session_id),
        Some(13),
        "高亮定位到卡片对应的进程"
    );
    assert_eq!(page.focus, None, "定位是一次性的");
}

#[test]
fn open_from_card_falls_back_when_the_process_is_gone() {
    // 卡片会留在历史里，但进程可能早就退出了：回落首项而不是空高亮。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    assert!(open_task_page_at(&mut state, pane, Some(999)));
    let page = page(&state, pane);
    assert_eq!(page.window.active, 0, "失效定位回落首项");
    assert!(page.selected().is_some(), "面板照常显示首项，不是空高亮");
}

#[test]
fn open_without_a_process_keeps_the_current_highlight() {
    // `[open]` 没带进程 id（理论上不出现）：只打开面板，不动高亮。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    assert!(open_task_page_at(&mut state, pane, None));
    assert_eq!(page(&state, pane).window.active, 0, "高亮保持当前值");
}

#[test]
fn open_from_card_is_refused_without_a_session() {
    // 与普通打开同一门槛：未绑定会话时拒绝。
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    assert!(!open_task_page_at(&mut state, pane, Some(11)));
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.task.is_none(),
        "未绑定会话不打开面板"
    );
}

// ===== 输出回投与翻页（决策 8/11）=====

#[test]
fn task_output_is_stored_and_pinned_to_the_tail() {
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    let output: Vec<String> = (0..50).map(|index| format!("line {index}")).collect();
    assert!(on_task_output(&mut state, pane, output.clone()));
    let page = page(&state, pane);
    assert_eq!(page.output, output, "输出尾部写入页面状态");
    assert_eq!(
        page.output_scroll,
        output.len().saturating_sub(page.output_visible),
        "新输出贴到尾（最新输出）"
    );
    // 内容不变不置脏。
    assert!(!on_task_output(&mut state, pane, output));
}

#[test]
fn page_keys_scroll_the_output_area_not_the_history() {
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    on_task_output(
        &mut state,
        pane,
        (0..60).map(|index| format!("line {index}")).collect(),
    );
    if let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.output_visible = 10;
        page.pin_output_to_tail();
    }
    // PageUp 前翻输出区；上方历史（面板接管后不可见）不再被翻动。
    let history_before = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .history_scroll;
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::PageUp);
    let page = page(&state, pane);
    assert!(
        page.output_scroll < 50,
        "PageUp 上翻输出区：{}",
        page.output_scroll
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .history_scroll,
        history_before,
        "面板接管历史区后不再翻动上方历史"
    );
}

// ===== review 回归：会话级状态复位与 tab 切换 =====

#[test]
fn switching_tabs_keeps_the_process_highlight() {
    // 真实缺陷：`clamp(self.len())` 按**当前 tab** 的条目数算，Agents 恒为 0，
    // 于是切到 Agents 就把 active/anchor/viewport 全抹成 0，切回 Processes 时
    // 用户原来选的那项已经丢了，`^d` 会停错进程。
    let (mut state, pane) = state_with_session("");
    open_with_processes(&mut state, pane);
    state
        .pane_mut_anywhere(pane)
        .unwrap()
        .lx
        .task
        .as_mut()
        .unwrap()
        .window
        .active = 2;
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Right);
    assert_eq!(page(&state, pane).tab, TaskTab::Agents);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Left);
    assert_eq!(page(&state, pane).tab, TaskTab::Processes);
    assert_eq!(
        page(&state, pane).window.active,
        2,
        "切 tab 往返后高亮必须还是原来那项"
    );
}

#[test]
fn new_session_clears_the_previous_plan() {
    // 真实缺陷：`/new` 没清 `plan`，而 `start_stream` 会把它拼进本轮 system
    // 上下文——新会话的模型会看到上一个会话的待办清单。
    let (mut state, pane) = state_with_session("");
    {
        let chat = &mut state.pane_mut_anywhere(pane).unwrap().lx.chat;
        chat.plan = Some(crate::app::state::PlanSnapshotView {
            revision: 1,
            steps: vec![("old step".into(), crate::app::state::PlanStepView::Pending)],
            explanation: None,
        });
        chat.selected_tool = Some(("msg-1".into(), "call-1".into()));
    }
    crate::app::update::chat::new_session(&mut state, pane);
    let chat = &state.pane_anywhere(pane).unwrap().lx.chat;
    assert!(chat.plan.is_none(), "新会话不得继承上一会话的计划");
    assert!(chat.selected_tool.is_none(), "选中态也要清");
}

/// 运行中的子 agent 记录（记录里带规范路径）。
fn running_agent(agent_id: &str, child_session: &str) -> crate::app::state::SubagentRun {
    crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: agent_id.into(),
        nickname: None,
        agent_path: format!("/root/{agent_id}"),
        child_session_id: child_session.into(),
        parent_session_id: "sess-1".into(),
        spawn_message_id: None,
        role: crate::chat::agent::AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "running".into(),
        title: "investigate".into(),
        created_at_ms: 0,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    })
}

/// 指定状态的 agent 记录（列表筛选测试用）。
fn agent_with_status(
    agent_id: &str,
    child_session: &str,
    status: &str,
) -> crate::app::state::SubagentRun {
    let mut run = running_agent(agent_id, child_session);
    run.status = status.into();
    if status != "running" {
        run.completed_at_ms = Some(1_000);
    }
    run
}

/// 打开 Agents 页并回投三条记录（running / idle / completed）。
fn open_agents_with_statuses(state: &mut AppState, pane: PaneId) -> Vec<&'static str> {
    assert!(open_task_page(state, pane));
    let mut running = agent_with_status("sub-run", "sess-run", "running");
    running.agent_path = "/root/run".into();
    let mut idle = agent_with_status("sub-idle", "sess-idle", "idle");
    idle.agent_path = "/root/idle".into();
    let mut done = agent_with_status("sub-done", "sess-done", "completed");
    done.agent_path = "/root/done".into();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.agents.runs = vec![running, idle, done];
    let page = target.lx.task.as_mut().expect("task");
    page.tab = TaskTab::Agents;
    page.agents_window.active = 2;
    vec!["sub-run", "sub-idle", "sub-done"]
}

#[test]
fn tab_cycles_agent_filter_and_clamps_selection() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    let input = crate::app::update::pane::apply_lx_input;

    input(&mut state, crate::app::actions::LxInput::Tab);
    let filtered = page(&state, pane);
    assert_eq!(
        filtered.agent_filter,
        crate::app::state::AgentFilter::Active,
        "All -> Active"
    );
    assert_eq!(
        filtered.len(&state.pane_anywhere(pane).unwrap().lx.agents),
        2
    );
    assert_eq!(filtered.agents_window.active, 0, "稳定选中保持在首个可见项");

    input(&mut state, crate::app::actions::LxInput::Tab);
    let filtered = page(&state, pane);
    assert_eq!(filtered.agent_filter, crate::app::state::AgentFilter::Done);
    assert_eq!(filtered.agents_window.active, 0, "Done 只剩一条");

    input(&mut state, crate::app::actions::LxInput::Tab);
    let filtered = page(&state, pane);
    assert_eq!(filtered.agent_filter, crate::app::state::AgentFilter::All);
    assert_eq!(filtered.agents_window.active, 0, "回到 All 后保持钳制结果");
}

#[test]
fn selected_agent_index_maps_filtered_position_to_real_run() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    let page = target.lx.task.as_mut().expect("task");
    page.agent_filter = crate::app::state::AgentFilter::Done;
    page.agents_window.active = 0;
    assert_eq!(
        page.selected_agent_index(&target.lx.agents),
        Some(2),
        "Done 视图第一项对应 runs[2]"
    );
    assert_eq!(
        page.selected_agent(&target.lx.agents)
            .map(|run| run.agent_id.as_str()),
        Some("sub-done")
    );
}

#[test]
fn enter_is_noop_in_the_agent_list() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Enter);
    assert!(page(&state, pane).agent_edit.is_none());
    assert_eq!(page(&state, pane).tab, TaskTab::Agents);
}

#[test]
fn shift_enter_opens_agent_composer_and_submit_keeps_editing() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    state.effects.clear();
    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    assert_eq!(page(&state, pane).agent_edit.as_deref(), Some("sub-done"));
    input(&mut state, crate::app::actions::LxInput::Char('h'));
    input(&mut state, crate::app::actions::LxInput::Char('i'));
    input(&mut state, crate::app::actions::LxInput::Enter);
    assert!(page(&state, pane).agent_edit.is_some(), "发送后留在编辑态");
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .agents
            .composer("sub-done")
            .is_some_and(|composer| composer.text().is_empty()),
        "发送成功后清空草稿"
    );
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput {
                pane: p,
                session_id,
                agent_id,
                text,
                notify_parent,
            } if *p == pane && session_id == "sess-1" && agent_id == "sub-done" && text == "hi" && !*notify_parent
        )),
        "Enter 提交追问：{:?}",
        state.effects
    );
}

#[test]
fn escape_returns_to_list_and_keeps_the_agent_draft() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    input(&mut state, crate::app::actions::LxInput::Char('h'));
    input(&mut state, crate::app::actions::LxInput::Char('i'));
    input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(page(&state, pane).agent_edit.is_none());
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    assert_eq!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .agents
            .composer("sub-done")
            .map(|composer| composer.text()),
        Some("hi")
    );
}

#[test]
fn cancelled_agent_cannot_enter_composer() {
    let (mut state, pane) = state_with_session("");
    assert!(open_task_page(&mut state, pane));
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.agents.runs = vec![agent_with_status("sub-dead", "sess-dead", "cancelled")];
    let task_page = target.lx.task.as_mut().expect("task");
    task_page.tab = TaskTab::Agents;
    task_page.agents_window.active = 0;
    task_page.agents_loading = false;
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::EnterFolder);
    assert!(page(&state, pane).agent_edit.is_none());
    let toast = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .toast
        .as_ref()
        .expect("warning toast");
    assert_eq!(toast.message, crate::ui::text::LX_TASK_AGENT_REPLY_CLOSED);
}

#[test]
fn agent_search_filters_and_restores_stable_selection() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.agents.selected_id = Some("sub-idle".into());
    {
        let task = target.lx.task.as_mut().expect("task");
        task.agents_window.active = 1;
    }
    for ch in "done".chars() {
        crate::app::update::pane::apply_lx_input(
            &mut state,
            crate::app::actions::LxInput::Char(ch),
        );
    }
    let task = page(&state, pane);
    assert_eq!(task.agents_search.text(), "done");
    assert_eq!(
        task.agent_visible_indices(&state.pane_anywhere(pane).unwrap().lx.agents),
        vec![2]
    );
    assert_eq!(task.agents_window.active, 0, "高亮落到过滤后的首项");
    assert_eq!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .agents
            .selected_id
            .as_deref(),
        Some("sub-idle"),
        "稳定选中不因过滤丢失"
    );
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(page(&state, pane).agents_search.text().is_empty());
    assert_eq!(
        page(&state, pane).agents_window.active,
        1,
        "清空搜索恢复原高亮"
    );
}

#[test]
fn subagents_slash_command_opens_the_agents_tab() {
    assert_eq!(
        crate::app::state::SlashPanel::commands_matching("/agents").first(),
        Some(&crate::app::state::SlashCommand::Subagents),
        "`/agents` 是 `/subagents` 的别名"
    );
    let (mut state, pane) = state_with_session("");
    state.effects.clear();
    assert!(crate::app::update::chat::apply_slash_command(
        &mut state,
        pane,
        crate::app::state::SlashCommand::Subagents
    ));
    let page = page(&state, pane);
    assert_eq!(page.tab, TaskTab::Agents);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::ListSubagents { pane: p, parent_session }
                if *p == pane && parent_session == "sess-1"
        )),
        "打开面板即请求子 agent 列表：{:?}",
        state.effects
    );
}

#[test]
fn ctrl_a_in_agent_search_moves_cursor_without_stopping_agents() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    let input = crate::app::update::pane::apply_lx_input;
    for ch in "done".chars() {
        input(&mut state, crate::app::actions::LxInput::Char(ch));
    }
    state.effects.clear();
    input(&mut state, crate::app::actions::LxInput::LineStart);
    let page = page(&state, pane);
    assert_eq!(page.agents_search.cursor(), 0, "Ctrl+A 回到行首");
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::CancelSubagents { .. })),
        "Ctrl+A 不再停止全部 agent：{:?}",
        state.effects
    );
}

/// 进入 Agents 追问编辑态，并把 `@` 面板驱动到有候选。
fn open_agent_mention(state: &mut AppState, pane: PaneId) {
    // 主编辑器先拿到扫描根；进入编辑态时 composer 会继承它。
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .set_mention_root(Some(std::path::PathBuf::from("/work/demo")));
    let input = crate::app::update::pane::apply_lx_input;
    input(state, crate::app::actions::LxInput::EnterFolder);
    assert!(page(state, pane).agent_edit.is_some(), "进入编辑态");
    input(state, crate::app::actions::LxInput::Char('@'));
    let (_, owner, generation, root) = crate::app::update::take_lx_mention_scan_requests(state)
        .into_iter()
        .find(|(_, owner, ..)| matches!(owner, crate::app::update::MentionScanOwner::Agent(_)))
        .expect("Agents composer 发起扫描");
    assert_eq!(
        owner,
        crate::app::update::MentionScanOwner::Agent("sub-done".into())
    );
    crate::app::update::apply_lx_mention_entries(
        state,
        pane,
        owner,
        generation,
        &root,
        vec![crate::app::markdown::MentionEntry {
            path: "src/main.rs".into(),
            is_directory: false,
        }],
    );
}

#[test]
fn mention_enter_confirms_candidate_instead_of_sending() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    open_agent_mention(&mut state, pane);
    state.effects.clear();

    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::Enter);
    let text = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .composer("sub-done")
        .map(|composer| composer.text().to_string());
    assert_eq!(text.as_deref(), Some("@src/main.rs "), "Enter 采纳候选");
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput { .. }
        )),
        "面板打开时 Enter 不发送：{:?}",
        state.effects
    );

    input(&mut state, crate::app::actions::LxInput::Enter);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput { text, .. }
                if text == "@src/main.rs"
        )),
        "面板关闭后发送同一段朴素文本（提交时裁剪尾随空格）：{:?}",
        state.effects
    );
}

#[test]
fn escape_closes_mention_panel_before_leaving_the_composer() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    open_agent_mention(&mut state, pane);

    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(page(&state, pane).agent_edit.is_some(), "第一下只关面板");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .composer("sub-done")
            .is_some_and(|composer| !composer.editor().mention_open()),
        "面板已关闭"
    );
    input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(page(&state, pane).agent_edit.is_none(), "第二下退出编辑态");
}

#[test]
fn late_mention_result_is_dropped_after_switching_agent() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .set_mention_root(Some(std::path::PathBuf::from("/work/demo")));
    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    input(&mut state, crate::app::actions::LxInput::Char('@'));
    let (_, owner, generation, root) =
        crate::app::update::take_lx_mention_scan_requests(&mut state)
            .into_iter()
            .find(|(_, owner, ..)| matches!(owner, crate::app::update::MentionScanOwner::Agent(_)))
            .expect("Agents composer 发起扫描");
    // 结果到达前切到另一个 agent：旧归属的迟到回执必须被丢弃。
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .task
        .as_mut()
        .expect("task")
        .agent_edit = Some("sub-idle".into());
    crate::app::update::apply_lx_mention_entries(
        &mut state,
        pane,
        owner,
        generation,
        &root,
        vec![crate::app::markdown::MentionEntry {
            path: "stale.rs".into(),
            is_directory: false,
        }],
    );
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .composer("sub-done")
            .is_some_and(|composer| composer.editor().mention_open()),
        "迟到结果不写入已切换的 agent"
    );
    // 丢弃回执时必须清掉原 composer 的在途标记，否则 Files Tab 永久 Loading。
    let retry = state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .composer_mut("sub-done")
        .expect("composer")
        .editor_mut()
        .take_mention_scan_request();
    assert!(retry.is_some(), "迟到结果丢弃后应允许重新发起扫描");
}

#[test]
fn shift_tab_switches_agent_reply_mode_and_report_submit_carries_the_flag() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    let mode = |state: &AppState| {
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .reply_mode("sub-done")
    };
    assert_eq!(
        mode(&state),
        crate::app::state::lx::agent::AgentReplyMode::Private,
        "默认私聊"
    );
    input(&mut state, crate::app::actions::LxInput::PrevStep);
    assert_eq!(
        mode(&state),
        crate::app::state::lx::agent::AgentReplyMode::Report,
        "Shift+Tab 切到汇报"
    );
    state.effects.clear();
    for ch in "hi".chars() {
        input(&mut state, crate::app::actions::LxInput::Char(ch));
    }
    input(&mut state, crate::app::actions::LxInput::Enter);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput {
                agent_id,
                text,
                notify_parent: true,
                ..
            } if agent_id == "sub-done" && text == "hi"
        )),
        "汇报模式发送 notify_parent=true：{:?}",
        state.effects
    );
}

#[test]
fn enter_while_agent_running_emits_a_steer_instead_of_a_new_turn() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.task.as_mut().expect("task").agents_window.active = 0;
    target.lx.agents.selected_id = Some("sub-run".into());
    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    assert_eq!(page(&state, pane).agent_edit.as_deref(), Some("sub-run"));
    state.effects.clear();
    for ch in "hi".chars() {
        input(&mut state, crate::app::actions::LxInput::Char(ch));
    }
    input(&mut state, crate::app::actions::LxInput::Enter);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SteerSubagent {
                agent_id,
                text,
                notify_parent: false,
                ..
            } if agent_id == "sub-run" && text == "hi"
        )),
        "运行中的 agent 发送 steer：{:?}",
        state.effects
    );
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentUserInput { .. }
        )),
        "steer 不应落成新的排队输入：{:?}",
        state.effects
    );
}

/// Shift+Enter 在 @ 面板高亮普通文件时必须回落为换行，不能吞键。
#[test]
fn shift_enter_falls_back_to_newline_when_mention_target_is_not_a_directory() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    open_agent_mention(&mut state, pane);
    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    let text = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .composer("sub-done")
        .map(|composer| composer.text().to_string());
    assert_eq!(text.as_deref(), Some("@\n"), "Shift+Enter 应插入换行");
}

/// `/` 在 Agents composer 中必须只写入字符，不能打开任何编辑浮层。
#[test]
fn agent_composer_slash_does_not_open_panels() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    input(&mut state, crate::app::actions::LxInput::Char('/'));
    let editor = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .composer("sub-done")
        .expect("composer")
        .editor();
    assert_eq!(editor.input, "/");
    assert!(editor.slash.is_none(), "不打开斜杠面板");
    assert!(editor.panel.is_none(), "不打开 markdown 块面板");
    assert!(editor.paste_panel().is_none(), "不打开粘贴面板");
    assert!(editor.refer.is_none(), "不打开快捷引用面板");
}

/// steer 达到上限时提交侧必须保留草稿，不能等事件层拒绝后才发现文本已清空。
#[test]
fn full_steer_queue_keeps_the_agent_draft() {
    let (mut state, pane) = state_with_session("");
    open_agents_with_statuses(&mut state, pane);
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .task
        .as_mut()
        .expect("task")
        .agents_window
        .active = 0; // sub-run
    let input = crate::app::update::pane::apply_lx_input;
    input(&mut state, crate::app::actions::LxInput::EnterFolder);
    {
        let run = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .by_agent_mut("sub-run")
            .expect("run");
        for index in 0..crate::app::state::lx::chat::steer::MAX_PENDING_STEERS {
            run.chat
                .pending_steers
                .push(crate::app::state::lx::chat::steer::PendingSteer {
                    text: format!("steer-{index}"),
                    attachments: Vec::new(),
                    model_images: Vec::new(),
                    model: crate::db::chat::ModelSnapshot {
                        provider: "p".into(),
                        model: "m".into(),
                        variant: None,
                    },
                });
        }
        state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .composer_mut("sub-run")
            .expect("composer")
            .editor_mut()
            .replace_input("too many");
    }
    input(&mut state, crate::app::actions::LxInput::Enter);
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::SteerSubagent { .. })),
        "满队列不得再发 steer：{:?}",
        state.effects
    );
    let draft = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .composer("sub-run")
        .map(|composer| composer.text().to_string());
    assert_eq!(draft.as_deref(), Some("too many"), "草稿必须保留");
}
