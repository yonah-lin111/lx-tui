//! Agents 列表固定列与详情缓存：对齐、降级、省略号、共享快照、离屏合并。

use super::*;
use crate::app::state::{AgentsState, SubagentRun, TaskPage, TaskTab};
use crate::ui::PaneCache;
use crate::ui::main_content::lx::Grid;
use ratatui::layout::Rect;
use std::time::Duration;

fn agent(id: &str, path: &str, nickname: Option<&str>, status: &str, title: &str) -> SubagentRun {
    let mut run = tree_agent(id, path, status, None);
    run.nickname = nickname.map(str::to_string);
    run.title = title.to_string();
    run.loaded = true;
    if status == "running" {
        run.completed_at_ms = None;
    }
    run
}

fn draw_list_lines(width: u16, height: u16, agents: &AgentsState, page: &TaskPage) -> Vec<String> {
    let area = Rect::new(0, 0, width, height);
    let mut grid = Grid::new(area);
    super::super::agents::draw_list(&mut grid, area, page, agents);
    (area.y..area.bottom())
        .map(|y| (area.x..area.right()).map(|x| grid.cell(x, y).ch).collect())
        .collect()
}

/// 按字符（不是字节）返回 needle 的起始列。
fn column(line: &str, needle: &str) -> usize {
    let byte = line.find(needle).expect("needle");
    line[..byte].chars().count()
}

fn detail_run() -> SubagentRun {
    let mut run = agent("a", "/root/a", Some("Turing"), "running", "task");
    run.chat.messages = (0..60)
        .map(|index| {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            text_message(
                &format!("m{index}"),
                role,
                &format!("line {index}\nsecond line\nthird line"),
            )
        })
        .collect();
    if let Some(message) = run.chat.messages.last_mut() {
        message.status = "streaming".into();
    }
    run
}

#[test]
fn columns_align_across_identity_widths_and_depths() {
    let agents = AgentsState {
        runs: vec![
            agent("parent", "/root/parent", None, "completed", "short"),
            agent(
                "child",
                "/root/parent/child",
                Some("Curie"),
                "running",
                "a much longer task title",
            ),
            agent(
                "grand",
                "/root/parent/child/grand",
                Some("Ada"),
                "completed",
                "title",
            ),
        ],
        cards_dirty: false,
    };
    let page = TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    };
    let lines = draw_list_lines(80, 3, &agents, &page);
    let parent = lines
        .iter()
        .find(|line| line.contains("executor") && line.contains("short"))
        .expect("parent");
    let child = lines
        .iter()
        .find(|line| line.contains("Curie"))
        .expect("child");
    let grand = lines
        .iter()
        .find(|line| line.contains("Ada"))
        .expect("grand");

    let identity = |line: &str| {
        if line.contains("Curie") {
            column(line, "Curie")
        } else if line.contains("Ada") {
            column(line, "Ada")
        } else {
            column(line, "executor")
        }
    };
    assert_eq!(identity(parent), identity(child), "身份列对齐");
    assert_eq!(identity(child), identity(grand));
    let status = |line: &str| {
        if line.contains("completed") {
            column(line, "completed")
        } else {
            column(line, "running")
        }
    };
    assert_eq!(status(parent), status(child), "状态列对齐");
    assert_eq!(status(child), status(grand));
    let title = |line: &str, needle: &str| column(line, needle);
    let title_x = title(parent, "short");
    assert_eq!(title_x, title(child, "a much longer"));
    assert_eq!(title_x, title(grand, "title"));
}

#[test]
fn long_title_truncates_with_ellipsis() {
    let mut agents = AgentsState::default();
    agents.runs.push(agent(
        "long",
        "/root/long",
        Some("Turing"),
        "running",
        "a very long task title that must not overflow the row",
    ));
    let page = TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    };
    let lines = draw_list_lines(40, 1, &agents, &page);
    let line = &lines[0];
    assert!(line.contains('…'), "超长标题用单字符省略号：{line:?}");
    assert!(line.chars().count() <= 40, "不得越界：{line:?}");
}

#[test]
fn narrow_width_drops_title_then_age_then_status() {
    let mut agents = AgentsState::default();
    agents.runs.push(agent(
        "narrow",
        "/root/narrow",
        Some("Turing"),
        "running",
        "task",
    ));
    let page = TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    };

    // 18 列：身份放得下，状态/时长/标题整列丢弃；状态图标保留。
    let lines = draw_list_lines(18, 1, &agents, &page);
    let line = &lines[0];
    assert!(line.contains('■'), "状态图标必须保留：{line:?}");
    assert!(line.contains("Turing"), "身份必须保留：{line:?}");
    assert!(!line.contains("running"), "先丢状态文字：{line:?}");
    assert!(!line.contains("task"), "再丢标题：{line:?}");

    // 8 列：连身份都放不下时用省略号收尾，状态图标仍在。
    let lines = draw_list_lines(8, 1, &agents, &page);
    let line = &lines[0];
    assert!(line.contains('■'), "状态图标必须保留：{line:?}");
    assert!(line.contains('…'), "身份极端窄时用省略号：{line:?}");
}

#[test]
fn filter_pills_keep_gap_before_close() {
    let rect = Rect::new(0, 0, 80, 20);
    let page = TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    };
    let (_, close) = super::super::tab_row_rects(rect);
    let close = close.expect("close button");
    let pills = super::super::filter_row_rects(rect, &page);
    let (_, x, width) = pills.last().copied().expect("filter pills");
    assert_eq!(
        close.x.saturating_sub(x + width),
        2,
        "筛选胶囊与 [esc close] 之间保留 2 列"
    );
}

#[test]
fn detail_measure_and_draw_share_one_snapshot() {
    let run = detail_run();
    let rect = Rect::new(0, 0, 40, 6);
    let mut cache = PaneCache::default();
    let total = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert!(total > 6, "内容必须溢出视口");
    assert_eq!(cache.builds(), 1, "测量只排版一次");

    let mut grid = Grid::new(rect);
    super::super::agents::draw_detail(&mut grid, rect, &run, 0, 0, &mut cache);
    assert_eq!(cache.builds(), 1, "绘制复用测量快照");

    let mut run = run;
    if let Some(AssistantStepView::Part { part, .. }) = run
        .chat
        .messages
        .last_mut()
        .and_then(|message| message.steps.first_mut())
    {
        part.content.push_str("\nnew line");
    }
    let _ = super::super::agents::detail_rows(rect, &run, &mut cache);
    let mut grid = Grid::new(rect);
    super::super::agents::draw_detail(&mut grid, rect, &run, 0, 0, &mut cache);
    assert_eq!(cache.builds(), 2, "内容变化后每帧只重建一次");
}

#[test]
fn detail_offscreen_stream_coalesces() {
    let mut run = detail_run();
    run.follow = false;
    run.scroll = 0;
    let rect = Rect::new(0, 0, 40, 6);
    let mut cache = PaneCache::default();
    cache.set_offscreen_coalesce(Duration::from_millis(80));
    let total = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert!(total > 6);
    let builds = cache.builds();

    if let Some(AssistantStepView::Part { part, .. }) = run
        .chat
        .messages
        .last_mut()
        .and_then(|message| message.steps.first_mut())
    {
        part.content.push_str("\nstreaming line");
    }
    let _ = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert_eq!(cache.builds(), builds, "离屏流式窗口内不重建");

    cache.set_offscreen_coalesce(Duration::ZERO);
    let _ = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert_eq!(cache.builds(), builds + 1, "窗口结束后重建");
}

#[test]
fn detail_follow_bottom_never_coalesces() {
    let mut run = detail_run();
    run.follow = true;
    let rect = Rect::new(0, 0, 40, 6);
    let mut cache = PaneCache::default();
    cache.set_offscreen_coalesce(Duration::from_millis(80));
    let _ = super::super::agents::detail_rows(rect, &run, &mut cache);
    let builds = cache.builds();

    if let Some(AssistantStepView::Part { part, .. }) = run
        .chat
        .messages
        .last_mut()
        .and_then(|message| message.steps.first_mut())
    {
        part.content.push_str("\nstreaming line");
    }
    let _ = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert_eq!(cache.builds(), builds + 1, "贴底跟随时立即重建");
}
