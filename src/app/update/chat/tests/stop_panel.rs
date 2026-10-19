//! `/stop` 面板：命令暴露、两级导航、进程列表回投与停止操作的效应。
//!
//! 交互口径与 `/model` 完全一致（↑↓ 移动、Enter 进入下一级、Esc 返回上一级），
//! 因此这些测试也照着 `/model` 的断言写法来写，两套面板的行为能逐条对照。

use super::*;
use crate::app::state::{ProcessEntry, SlashCommand, SlashPanel, StopPickerStage};

/// 三条后台进程（运行中）。
fn processes() -> Vec<ProcessEntry> {
    vec![
        ProcessEntry {
            session_id: 11,
            command: "npm run dev".into(),
            cwd: "/workspace/web".into(),
            tty: false,
            running: true,
            started_at_ms: 0,
        },
        ProcessEntry {
            session_id: 12,
            command: "cargo watch -x test".into(),
            cwd: "/workspace".into(),
            tty: false,
            running: true,
            started_at_ms: 0,
        },
        ProcessEntry {
            session_id: 13,
            command: "python worker.py".into(),
            cwd: "/workspace/py".into(),
            tty: true,
            running: true,
            started_at_ms: 0,
        },
    ]
}

/// 打开 `/stop` 并回投三条进程。
fn open_with_processes(state: &mut AppState, pane: PaneId) {
    assert!(open_stop_picker(state, pane));
    assert!(on_process_listed(state, pane, processes()));
}

/// 当前 `/stop` 面板的只读快照。
fn picker(state: &AppState, pane: PaneId) -> Box<crate::app::state::StopPicker> {
    let panel = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.slash.clone())
        .expect("面板已打开");
    match panel {
        SlashPanel::Stop(picker) => picker,
        other => panic!("期望 /stop 面板，实际是 {other:?}"),
    }
}

#[test]
fn stop_command_is_listed_with_kill_and_ps_aliases() {
    assert!(
        SlashCommand::ALL.contains(&SlashCommand::Stop),
        "`/stop` 必须出现在命令面板里"
    );
    assert_eq!(SlashCommand::Stop.text(), "/stop");
    // `kill` / `ps` 是用户会先想到的词（对齐 codex 的 /stop 与 /ps）。
    assert!(SlashCommand::Stop.aliases().contains(&"kill"));
    assert!(SlashCommand::Stop.aliases().contains(&"ps"));
    for query in ["/stop", "/kill", "/ps"] {
        assert!(
            SlashPanel::commands_matching(query).contains(&SlashCommand::Stop),
            "{query} 应命中断言 stop：{:?}",
            SlashPanel::commands_matching(query)
        );
    }
}

#[test]
fn opening_the_panel_requests_the_process_list() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_stop_picker(&mut state, pane));
    // 面板骨架先出现，列表由事件层回投（`app/` 不持有进程句柄）。
    assert!(matches!(
        state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()),
        Some(SlashPanel::Stop(_))
    ));
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::ListSessionProcesses { pane: p } if *p == pane
        )),
        "打开面板即请求列表：{:?}",
        state.effects
    );
}

#[test]
fn first_stage_lists_processes_with_age_tags() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    open_with_processes(&mut state, pane);
    let panel = picker(&state, pane);
    assert_eq!(panel.stage, StopPickerStage::Processes);
    assert_eq!(panel.processes.len(), 3);
    assert_eq!(panel.window.active, 0, "默认高亮第一条");
    // 时长标签是 `running Ns`（运行中才有）。
    assert!(
        panel.processes[0]
            .age_label(panel.now_ms)
            .starts_with("running "),
        "运行中的进程带时长"
    );
}

#[test]
fn enter_on_a_process_opens_the_action_stage_and_escape_goes_back() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    open_with_processes(&mut state, pane);
    // 清掉开面板时推入的列表效应，后面只看“采纳动作产生了什么”。
    state.effects.clear();
    // 高亮第二条 → Enter 进入二级。
    move_panel_active(&mut state, pane, 1);
    assert!(apply_slash_enter(&mut state, pane));
    let panel = picker(&state, pane);
    assert!(
        matches!(
            panel.stage,
            StopPickerStage::Actions {
                index: 1,
                session_id: 12,
                ..
            }
        ),
        "二级记住目标进程：{:?}",
        panel.stage
    );
    assert_eq!(panel.stage_len(), 1, "动作列表只有 stop 一项");

    // Esc 返回一级，并恢复进入前的进程高亮（与 `/model` 的 variant → 模型一致）。
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    let panel = picker(&state, pane);
    assert_eq!(panel.stage, StopPickerStage::Processes, "Esc 回到一级");
    assert_eq!(panel.window.active, 1, "恢复原高亮");
}

#[test]
fn escape_at_the_first_stage_closes_the_panel() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    open_with_processes(&mut state, pane);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Escape);
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|t| t.lx.slash.clone())
            .is_none(),
        "一级 Esc 关闭面板"
    );
}

#[test]
fn confirming_the_action_stops_that_process_and_refreshes_the_list() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    open_with_processes(&mut state, pane);
    move_panel_active(&mut state, pane, 2);
    assert!(apply_slash_enter(&mut state, pane)); // 进入二级
    state.effects.clear();
    assert!(apply_slash_enter(&mut state, pane)); // 采纳动作
    // 停止效应带着正确的进程会话 ID。
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::StopSessionProcess {
                pane: p,
                process_session_id: 13
            } if *p == pane
        )),
        "按选中进程停止：{:?}",
        state.effects
    );
    // 不同步刷新列表（停止异步，同步重读会读到旧快照）：
    // 列表由进程终态事件驱动刷新。
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::ListSessionProcesses { pane: p } if *p == pane
        )),
        "不应在同一帧重拉列表：{:?}",
        state.effects
    );
    assert!(
        matches!(
            state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()),
            Some(SlashPanel::Stop(_))
        ),
        "停止后不关闭面板"
    );
}

#[test]
fn empty_list_shows_the_empty_state_and_cannot_be_selected() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(open_stop_picker(&mut state, pane));
    assert!(on_process_listed(&mut state, pane, Vec::new()));
    let panel = picker(&state, pane);
    assert_eq!(panel.stage_len(), 0, "没有进程");
    // Enter 不会进入二级（没有可选的条目）。
    assert!(apply_slash_enter(&mut state, pane));
    assert_eq!(
        picker(&state, pane).stage,
        StopPickerStage::Processes,
        "空列表不进二级"
    );
    // 高亮被钳到 0，不会越界。
    assert_eq!(picker(&state, pane).window.active, 0);
}

#[test]
fn listed_processes_clamp_the_highlight_when_the_list_shrinks() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    open_with_processes(&mut state, pane);
    move_panel_active(&mut state, pane, 2);
    // 进程在面板停留期间退出：列表缩短，高亮必须随之钳制。
    assert!(on_process_listed(
        &mut state,
        pane,
        processes()[..1].to_vec()
    ));
    let panel = picker(&state, pane);
    assert_eq!(panel.processes.len(), 1);
    assert_eq!(panel.window.active, 0, "高亮被钳到唯一一项");
    assert_eq!(panel.stage, StopPickerStage::Processes);
}

#[test]
fn listing_into_a_missing_panel_is_a_no_op() {
    // 面板不在时回投不 panic、也不改状态。
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    assert!(!on_process_listed(&mut state, pane, processes()));
}

#[test]
fn down_arrow_on_an_empty_draft_opens_the_panel() {
    // opencode 口径：↓ 在历史不接管时向**下层派发**——空草稿 + 光标在末尾即打开面板。
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    assert!(
        matches!(
            state.pane_anywhere(pane).and_then(|t| t.lx.slash.clone()),
            Some(SlashPanel::Stop(_))
        ),
        "空草稿按 ↓ 打开后台进程面板"
    );
}

#[test]
fn down_arrow_with_a_draft_still_moves_the_cursor() {
    // 有草稿时 ↓ 仍是光标移动：不能因为新增快捷键就让正在写的内容突然弹面板。
    let mut state = ready_state("写点东西");
    let pane = focus_pane(&state);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|t| t.lx.slash.clone())
            .is_none(),
        "有草稿时不弹面板"
    );
}

#[test]
fn down_arrow_does_not_steal_a_visible_panel() {
    // 面板已打开时，↓ 归面板（移动高亮），不再触发"打开面板"。
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    open_with_processes(&mut state, pane);
    crate::app::update::pane::apply_lx_input(&mut state, crate::app::actions::LxInput::Down);
    let panel = picker(&state, pane);
    assert_eq!(panel.window.active, 1, "↓ 移动面板高亮");
    assert_eq!(panel.processes.len(), 3, "没有重新拉列表覆盖现有条目");
}
