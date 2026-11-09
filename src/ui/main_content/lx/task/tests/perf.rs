//! Agents 详情性能回归（`#[ignore]`，release 下手动跑）。
//!
//! 口径：6 个并行 subagent × 每个 200 条消息；连续 120 帧的
//! `refresh + resolve + layout + render` P95 ≤ 16ms。
//!
//! ```text
//! cargo test --release -- --ignored perf_agents_panel
//! ```

use super::*;
use crate::app::state::{AppState, SubagentRun, TaskPage, TaskTab};
use crate::app::update;
use crate::ui::main_content::lx::Grid;
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

const RUNS: usize = 6;
const MESSAGES_PER_RUN: usize = 200;
const FRAMES: usize = 120;

fn streaming_run(id: &str) -> SubagentRun {
    let mut run = tree_agent(id, &format!("/root/{id}"), "running", None);
    run.title = id.to_string();
    run.loaded = true;
    run.completed_at_ms = None;
    run.follow = true;
    run.chat.messages = (0..MESSAGES_PER_RUN)
        .map(|index| {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            text_message(
                &format!("{id}-{index}"),
                role,
                &format!("message {index} line one\nline two with some words\nline three"),
            )
        })
        .collect();
    run
}

fn append_stream_line(run: &mut SubagentRun, frame: usize) {
    if let Some(AssistantStepView::Part { part, .. }) = run
        .chat
        .messages
        .last_mut()
        .and_then(|message| message.steps.first_mut())
    {
        part.content.push_str(&format!("\nframe {frame}"));
    }
    run.rows_dirty = true;
}

#[test]
#[ignore = "性能回归：cargo test --release -- --ignored perf_agents_panel"]
fn perf_agents_panel() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.agents.runs = (0..RUNS)
            .map(|index| streaming_run(&format!("a{index}")))
            .collect();
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Agents,
            ..TaskPage::default()
        });
    }
    let area = Rect::new(0, 0, 80, 24);
    let mut cache = crate::ui::RenderCache::default();
    let mut samples = Vec::with_capacity(FRAMES);
    let builds_before = cache.pane(pane).builds();

    for frame in 0..FRAMES {
        {
            let target = state.pane_mut_anywhere(pane).expect("pane");
            for run in &mut target.lx.agents.runs {
                append_stream_line(run, frame);
            }
        }
        let started = Instant::now();
        update::refresh_subagent_cards(&mut state, pane);
        update::resolve_task_agents_visible(&mut state, &[(pane, area)], &mut cache);
        let Some(rect) = state.pane_anywhere(pane).and_then(|target| {
            crate::ui::main_content::lx::task_output_inner(
                crate::layout::pane_inner_rect(area),
                &target.lx,
            )
        }) else {
            panic!("Agents 详情区几何");
        };
        let mut grid = Grid::new(rect);
        {
            let target = state.pane_anywhere(pane).expect("pane");
            super::super::draw_agents_detail(&mut grid, rect, &target.lx, cache.pane(pane));
        }
        samples.push(started.elapsed());
        cache.pane(pane).advance_frame();
    }

    let builds = cache.pane(pane).builds().saturating_sub(builds_before);
    assert!(
        builds <= FRAMES as u64,
        "同一帧最多重建一次：{builds} builds / {FRAMES} frames"
    );
    samples.sort_unstable();
    let p95 = samples[samples.len() * 95 / 100];
    assert!(
        p95 <= Duration::from_millis(16),
        "P95 {p95:?} > 16ms（6 × 200，{FRAMES} 帧）"
    );
}
