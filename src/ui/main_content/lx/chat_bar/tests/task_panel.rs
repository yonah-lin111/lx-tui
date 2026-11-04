//! 后台任务面板（`Processes` · `Agents`）与 `ask` 等待期间的聊天栏：
//! 「回到底部」显隐、命中与点击回落，以及 loading 的抑制。

use super::*;

/// 通用窗格内容区（够高够宽，聊天栏与 `ask` 面板都能排下）。
fn area() -> Rect {
    Rect::new(0, 0, 80, 24)
}

/// `Processes` 页签：10 行输出、可见 4 行（滚动上限 6），偏移由调用方给定。
fn processes_page(scroll: usize) -> crate::app::state::TaskPage {
    crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Processes,
        output: (0..10).map(|index| format!("line {index}")).collect(),
        output_visible: 4,
        output_scroll: scroll,
        ..Default::default()
    }
}

/// `Agents` 页签：一条带消息的子 agent（`visible` 10 / 总计 100 行，尾部偏移 90）。
fn agent_run(follow: bool) -> crate::app::state::SubagentRun {
    let mut run = crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: "a1".into(),
        nickname: None,
        child_session_id: "s1".into(),
        parent_session_id: "sess-1".into(),
        spawn_message_id: None,
        role: crate::chat::agent::AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "completed".into(),
        title: "investigate".into(),
        created_at_ms: 0,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    });
    run.chat
        .messages
        .push(assistant("a1", "complete", vec![text_step("done")]));
    run.visible = 10;
    run.total_rows = 100;
    run.scroll = 40;
    run.follow = follow;
    run
}

#[test]
fn processes_page_shows_the_button_only_when_scrolled_up() {
    let area = area();
    let mut lx = Lx::new();
    lx.task = Some(processes_page(6));
    assert!(!visible(&lx), "贴尾时不画聊天栏");
    assert!(jump_bottom_rect(area, &lx).is_none());

    lx.task = Some(processes_page(3));
    assert!(visible(&lx), "上翻后必须给出回到底部入口");
    assert!(jump_bottom_rect(area, &lx).is_some());
}

#[test]
fn clicking_the_button_pins_the_process_output_tail() {
    let mut state = crate::app::state::AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.pane_mut_anywhere(pane).expect("pane").lx.task = Some(processes_page(3));

    assert!(crate::app::update::jump_lx_history_bottom(&mut state, pane));
    let page = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .task
        .clone()
        .expect("task");
    assert_eq!(page.output_scroll, page.output_max_scroll(), "回到尾部");
    assert!(!visible(&state.pane_anywhere(pane).expect("pane").lx));
    assert!(jump_bottom_rect(area(), &state.pane_anywhere(pane).expect("pane").lx).is_none());
}

#[test]
fn agents_page_shows_the_button_only_when_not_following() {
    let area = area();
    let mut lx = Lx::new();
    lx.agents.runs.push(agent_run(true));
    lx.task = Some(crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Agents,
        ..Default::default()
    });
    assert!(!visible(&lx), "跟随底部时不画聊天栏");
    assert!(jump_bottom_rect(area, &lx).is_none());

    lx.agents.runs[0].follow = false;
    assert!(visible(&lx), "子 agent 详情上翻后必须给出入口");
    assert!(jump_bottom_rect(area, &lx).is_some());
}

#[test]
fn clicking_the_button_restores_agent_follow() {
    let mut state = crate::app::state::AppState::demo();
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.agents.runs.push(agent_run(false));
    target.lx.task = Some(crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Agents,
        ..Default::default()
    });

    assert!(crate::app::update::jump_lx_history_bottom(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    let run = &target.lx.agents.runs[0];
    assert!(run.follow, "恢复跟随");
    assert_eq!(run.scroll, 90, "滚动偏移钉到尾部");
}

#[test]
fn the_button_stays_clickable_while_the_task_page_is_open() {
    let area = area();
    let mut lx = Lx::new();
    lx.task = Some(processes_page(3));
    let button = jump_bottom_rect(area, &lx).expect("按钮矩形");
    let mut cache = crate::ui::main_content::lx::PaneCache::default();
    assert_eq!(
        crate::ui::main_content::lx::chat_hit(area, &lx, button.x, button.y, &mut cache),
        Some(crate::ui::main_content::lx::LxChatHit::JumpBottom),
        "任务面板打开时历史几何不可用，但按钮必须能点"
    );
}

#[test]
fn ask_waiting_hides_the_loading_row_but_keeps_the_button() {
    let mut lx = streaming_lx("a1", vec![tool_step(0, ToolStatus::Running)]);
    lx.chat.follow_bottom = false;
    lx.ask = Some(crate::app::state::AskState::new(
        RequestKey {
            pane: PaneId::alloc(),
            session: "session".into(),
            generation: 1,
        },
        "call-ask".into(),
        Vec::new(),
    ));

    assert!(!loading_visible(&lx), "等待作答期间不计时、不显示 loading");
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(!row.contains(text::chat::RUNNING_TOOLS_LABEL), "{row:?}");
    assert!(
        row.contains(text::LX_JUMP_BOTTOM),
        "回到底部仍保留：{row:?}"
    );
    assert!(
        jump_bottom_rect(area(), &lx).is_some(),
        "`ask` 面板替换了输入框，按钮命中区仍要跟着栏位"
    );
}

#[test]
fn agents_detail_recovers_follow_on_boundary_scroll_and_keyboard_switch() {
    let mut state = crate::app::state::AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let mut r0 = agent_run(false);
        r0.scroll = 89; // 距 max (90) 仅差 1 行（即 chat_bar 占用的 1 行）
        let mut r1 = agent_run(false);
        r1.agent_id = "a2".into();
        r1.scroll = 40;
        target.lx.agents.runs.push(r0);
        target.lx.agents.runs.push(r1);
        target.lx.task = Some(crate::app::state::TaskPage {
            tab: crate::app::state::TaskTab::Agents,
            ..Default::default()
        });
    }
    // 向下滚轮跨越 1 行边界时吸回 max 并恢复 follow = true。
    assert!(crate::app::update::scroll_agents_detail(
        &mut state, pane, 1
    ));
    assert!(state.pane_anywhere(pane).expect("pane").lx.agents.runs[0].follow);

    // 已经钳在 max 但 follow == false 时，向下滚轮仍恢复 follow = true。
    {
        let run = &mut state.pane_mut_anywhere(pane).expect("pane").lx.agents.runs[0];
        run.follow = false;
        run.scroll = 90;
    }
    assert!(crate::app::update::scroll_agents_detail(
        &mut state, pane, 3
    ));
    assert!(state.pane_anywhere(pane).expect("pane").lx.agents.runs[0].follow);

    // 键盘 Down 切换到第 2 个子 agent 时，同样恢复其 follow = true。
    crate::app::update::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    let run1 = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[1];
    assert!(run1.follow, "键盘切换子 agent 高亮恢复跟随");
}

#[test]
fn reloaded_message_subtracts_ask_tool_waiting_duration() {
    let mut state = crate::app::state::AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .runs
        .push(crate::app::state::SubagentRun::from_record(
            &crate::db::chat::SubagentRecord {
                agent_id: "a1".into(),
                nickname: None,
                child_session_id: "s1".into(),
                parent_session_id: "sess-1".into(),
                spawn_message_id: None,
                role: crate::chat::agent::AgentRole::Explorer,
                model_provider: String::new(),
                model_id: String::new(),
                model_variant: None,
                status: "completed".into(),
                title: "investigate".into(),
                created_at_ms: 0,
                completed_at_ms: None,
                report_json: None,
                report_delivered: false,
            },
        ));
    let message = crate::db::chat::ChatMessage {
        external_id: "m1".into(),
        sequence: 1,
        turn_index: 0,
        kind: "chat".into(),
        user_shell: None,
        subagent_reports: None,
        role: "assistant".into(),
        status: "complete".into(),
        model_provider: None,
        model_id: None,
        variant: None,
        stop_reason: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        created_at_ms: 1_000,
        completed_at_ms: Some(15_000),
        parts: Vec::new(),
        tools: vec![crate::db::chat::ToolCallRecord {
            call_id: "c-ask".into(),
            message_id: "m1".into(),
            name: crate::chat::tools::ask::ASK_TOOL.into(),
            arguments_json: "{}".into(),
            position: 0,
            status: "completed".into(),
            result_json: None,
            error_text: None,
            started_at_ms: 2_000,
            completed_at_ms: Some(12_000),
            duration_ms: Some(10_000),
        }],
        attachments: Vec::new(),
    };
    assert!(crate::app::update::on_subagent_loaded(
        &mut state,
        pane,
        "a1",
        &[message]
    ));
    let view = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0]
        .chat
        .messages[0];
    assert_eq!(
        view.duration_ms,
        Some(4_000),
        "重载会话的回合总耗时应扣除 ask 等待时长"
    );
}
