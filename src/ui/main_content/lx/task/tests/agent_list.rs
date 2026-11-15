//! Agents 列表固定列与详情缓存：对齐、降级、省略号、共享快照、离屏合并。

use super::*;
use crate::app::state::{AgentsState, AppState, SubagentRun, TaskPage, TaskTab};
use crate::app::update;
use crate::ui::PaneCache;
use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::HistorySource;
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

fn draw_list_with_overflow(
    width: u16,
    height: u16,
    agents: &AgentsState,
    page: &TaskPage,
) -> (Vec<String>, Vec<String>) {
    // list 只占前 width 列，右侧留 8 列哨兵；任何越界写入都会出现在 overflow 里。
    let list = Rect::new(0, 0, width, height);
    let area = Rect::new(0, 0, width + 8, height);
    let mut grid = Grid::new(area);
    super::super::agents::draw_list(&mut grid, list, page, agents);
    let lines = (list.y..list.bottom())
        .map(|y| (list.x..list.right()).map(|x| grid.cell(x, y).ch).collect())
        .collect();
    let overflow = (list.y..list.bottom())
        .map(|y| {
            (list.right()..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect()
        })
        .collect();
    (lines, overflow)
}

fn draw_list_lines(width: u16, height: u16, agents: &AgentsState, page: &TaskPage) -> Vec<String> {
    draw_list_with_overflow(width, height, agents, page).0
}

fn assert_no_overflow(overflow: &[String], width: u16) {
    assert!(
        overflow.iter().all(|line| line.chars().all(|ch| ch == ' ')),
        "绘制越过了 list.right()={width}: {overflow:?}"
    );
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
        ..Default::default()
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
    let (lines, overflow) = draw_list_with_overflow(40, 1, &agents, &page);
    let line = &lines[0];
    assert_no_overflow(&overflow, 40);
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
        "ZebraTitle",
    ));
    let page = TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    };

    // 身份 17 + 状态 7 + 时长 2；各列起点：图标 2、身份 4、状态 23、时长 32、标题 36。
    for width in 8..=50 {
        let (lines, overflow) = draw_list_with_overflow(width, 1, &agents, &page);
        let line = &lines[0];
        assert_no_overflow(&overflow, width);
        assert!(
            line.contains('■'),
            "状态图标必须保留（{width} 列）：{line:?}"
        );
        if width >= 30 {
            assert!(
                line.contains("running"),
                "状态列应出现（{width} 列）：{line:?}"
            );
        } else {
            assert!(
                !line.contains("running"),
                "状态列应消失（{width} 列）：{line:?}"
            );
        }
        if width >= 34 {
            assert!(line.contains("0s"), "时长列应出现（{width} 列）：{line:?}");
        } else {
            assert!(!line.contains("0s"), "时长列应消失（{width} 列）：{line:?}");
        }
        // 33..=36：时长刚放不下但身份仍完整，标题绝不能回填到时长槽。
        if (33..=36).contains(&width) {
            assert!(
                !line.contains('…'),
                "标题不得在时长槽复活（{width} 列）：{line:?}"
            );
        }
        if width > 36 {
            let tail: String = line.chars().skip(36).collect();
            assert!(
                !tail.trim().is_empty(),
                "标题应在固定列出现（{width} 列）：{line:?}"
            );
        }
        // 身份本身放不下时用单字符省略号收尾。
        if width < 21 {
            assert!(
                line.contains('…'),
                "身份极端窄时用省略号（{width} 列）：{line:?}"
            );
        }
    }
}

#[test]
fn deep_tree_prefix_does_not_leak_into_scrollbar() {
    let agents = AgentsState {
        runs: vec![
            agent("parent", "/root/p", None, "completed", "parent"),
            agent("child", "/root/p/c", None, "running", "child"),
        ],
        cards_dirty: false,
        ..Default::default()
    };
    let page = TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    };
    for width in 6..=10 {
        let (lines, overflow) = draw_list_with_overflow(width, 2, &agents, &page);
        assert_no_overflow(&overflow, width);
        let child = &lines[1];
        if width < 7 {
            // 6 列：层级槽 + 状态图标都放不下，只保留选择标记，不得留裸连接符。
            assert!(
                !child.contains('├') && !child.contains('└'),
                "放不下状态图标时不得留下裸连接符（{width} 列）：{child:?}"
            );
        }
    }
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
    let (total, fresh) = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert!(total > 6, "内容必须溢出视口");
    assert!(fresh, "首次测量必须是新快照");
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
    let (_, fresh) = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert!(fresh, "follow=true 时立即可重建");
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
    let (total, fresh) = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert!(total > 6);
    assert!(fresh);
    let builds = cache.builds();

    if let Some(AssistantStepView::Part { part, .. }) = run
        .chat
        .messages
        .last_mut()
        .and_then(|message| message.steps.first_mut())
    {
        part.content.push_str("\nstreaming line");
    }
    let (_, fresh) = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert!(!fresh, "命中离屏合并窗口时快照不是最新");
    assert_eq!(cache.builds(), builds, "离屏流式窗口内不重建");

    cache.set_offscreen_coalesce(Duration::ZERO);
    let (_, fresh) = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert!(fresh, "窗口结束后重建");
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
    let (_, fresh) = super::super::agents::detail_rows(rect, &run, &mut cache);
    assert!(fresh, "贴底跟随时立即重建");
    assert_eq!(cache.builds(), builds + 1, "贴底跟随时立即重建");
}

fn wide_detail_run() -> SubagentRun {
    let mut run = detail_run();
    for message in &mut run.chat.messages {
        if let Some(AssistantStepView::Part { part, .. }) = message.steps.first_mut() {
            part.content = "wide words ".repeat(20);
        }
    }
    run
}

fn state_with_detail(run: SubagentRun) -> (AppState, crate::tiling::PaneId) {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = crate::app::state::PaneView::Lx;
    target.lx.agents.runs.push(run);
    target.lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    });
    (state, pane)
}

/// 详情区上滚阅读后，上方内容变高：滚动偏移按锚点补偿，阅读位置不动。
#[test]
fn history_source_owner_is_stable_across_run_reordering() {
    let first = agent("a", "/root/a", Some("A"), "completed", "first");
    let owner_before = HistorySource::from_run(&first).owner();
    let mut agents = AgentsState::default();
    agents.runs.push(first);
    agents
        .runs
        .push(agent("b", "/root/b", Some("B"), "completed", "second"));
    agents.sort_runs_tree_order();
    let first = agents
        .runs
        .iter()
        .find(|run| run.agent_id == "a")
        .expect("run a");
    assert_eq!(
        HistorySource::from_run(first).owner(),
        owner_before,
        "run 排序不能改变详情缓存 owner"
    );
}

#[test]
fn agent_composer_marker_and_badge_stay_inside_narrow_rect() {
    let mut lx = Lx::new();
    lx.agents.runs.push(agent(
        "a",
        "/root/turing",
        Some("Turing"),
        "running",
        "task",
    ));
    lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        agent_edit: Some("a".into()),
        ..TaskPage::default()
    });
    lx.agents.ensure_composer("a");
    for width in [9, 10, 11, 12, 20, 32] {
        let rect = Rect::new(0, 0, width, 3);
        let area = Rect::new(0, 0, width + 8, 3);
        let mut grid = Grid::new(area);
        assert!(super::super::agents::draw_edit(&mut grid, rect, &lx, true).is_some());
        let top: String = (rect.x..rect.right())
            .map(|x| grid.cell(x, rect.y).ch)
            .collect();
        let overflow: String = (rect.right()..area.right())
            .map(|x| grid.cell(x, rect.y).ch)
            .collect();
        assert!(
            overflow.trim().is_empty(),
            "width={width}: marker/badge overflowed composer: {overflow:?}"
        );
        if width >= 12 {
            assert!(top.contains("Subagent"), "width={width}: {top:?}");
        } else {
            assert!(
                !top.contains("Subagent"),
                "width={width}: marker must be hidden instead of clipped: {top:?}"
            );
        }
    }
}

#[test]
fn agents_filter_without_matches_does_not_claim_there_are_no_agents() {
    let agents = AgentsState {
        runs: vec![agent(
            "done",
            "/root/done",
            Some("Ada"),
            "completed",
            "done",
        )],
        ..Default::default()
    };
    let page = TaskPage {
        tab: TaskTab::Agents,
        agent_filter: crate::app::state::AgentFilter::Active,
        ..TaskPage::default()
    };
    let lines = draw_list_lines(80, 5, &agents, &page);
    let text = lines.join("\n");
    assert!(
        text.contains(crate::ui::text::LX_TASK_AGENTS_FILTER_EMPTY),
        "{text:?}"
    );
    assert!(
        !text.contains(crate::ui::text::LX_TASK_AGENTS_EMPTY),
        "{text:?}"
    );
}

#[test]
fn agents_composer_receives_pane_width_and_wraps() {
    let mut run = detail_run();
    run.loaded = true;
    let (mut state, pane) = state_with_detail(run);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.agents.selected_id = Some("a".into());
        target.lx.task.as_mut().expect("task").agent_edit = Some("a".into());
        target
            .lx
            .agents
            .ensure_composer("a")
            .editor_mut()
            .replace_input(&"word ".repeat(80));
    }
    let rect = Rect::new(0, 0, 48, 24);
    let mut cache = crate::ui::RenderCache::default();
    // 第一帧拿到 composer 实际宽度；第二帧据此重新量软换行行数。
    update::resolve_task_agents_visible(&mut state, &[(pane, rect)], &mut cache);
    update::resolve_task_agents_visible(&mut state, &[(pane, rect)], &mut cache);
    let target = state.pane_anywhere(pane).expect("pane");
    let editor = target.lx.agents.composer("a").expect("composer").editor();
    assert!(editor.input_width() < 100, "composer 必须按窗格宽度软换行");
    assert!(editor.visual_rows().len() > 1, "长草稿应产生多行");
    assert!(
        super::task_panel_height(&target.lx) > 3,
        "多行草稿应撑高 composer"
    );
}

#[test]
fn agents_detail_anchor_compensates_when_above_grows() {
    let mut run = detail_run();
    run.follow = false;
    run.scroll = 0;
    let (mut state, pane) = state_with_detail(run);
    let rect = Rect::new(0, 0, 80, 24);
    let mut cache = crate::ui::RenderCache::default();
    update::resolve_task_agents_visible(&mut state, &[(pane, rect)], &mut cache);
    {
        let run = &mut state.pane_mut_anywhere(pane).expect("pane").lx.agents.runs[0];
        run.scroll = 20;
        run.rows_dirty = true;
        // 助手正文不折叠：3 行 → 8 行，视口上方净增 5 行。
        let Some(AssistantStepView::Part { part, .. }) = run.chat.messages[1].steps.first_mut()
        else {
            panic!("第二条消息必须是正文步骤");
        };
        part.content = "0\n1\n2\n3\n4\n5\n6\n7".into();
    }
    update::resolve_task_agents_visible(&mut state, &[(pane, rect)], &mut cache);

    let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
    assert_eq!(run.scroll, 25, "顶部行跟着锚点块下移 5 行");
    assert!(!run.follow, "补偿不得把阅读态改回跟随");
}

/// 贴底展开详情卡片：先钉住上一帧顶部行，不把底部内容顶出视野。
#[test]
fn resizing_while_reading_does_not_restore_follow() {
    let mut run = detail_run();
    run.follow = false;
    run.scroll = 0;
    let (mut state, pane) = state_with_detail(run);
    let narrow = Rect::new(0, 0, 80, 24);
    let mut cache = crate::ui::RenderCache::default();
    update::resolve_task_agents_visible(&mut state, &[(pane, narrow)], &mut cache);
    let max = {
        let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
        run.total_rows.saturating_sub(run.visible)
    };
    {
        let run = &mut state.pane_mut_anywhere(pane).expect("pane").lx.agents.runs[0];
        run.scroll = max;
        run.follow = false;
        run.rows_dirty = true;
    }

    // 变宽后总行数/旧 max 变小；不得因 run.scroll 曾等于旧 max 而恢复跟随。
    let wide = Rect::new(0, 0, 120, 24);
    update::resolve_task_agents_visible(&mut state, &[(pane, wide)], &mut cache);
    let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
    assert!(!run.follow, "宽度变化不得把阅读态改回跟随");
    assert!(
        run.scroll <= run.total_rows.saturating_sub(run.visible),
        "scroll 必须钳在新上限内"
    );
}

#[test]
fn toggling_detail_card_while_following_pins_viewport() {
    let mut run = detail_run();
    // 让视口下方的用户消息真正可折叠（>5 行）；否则 toggle 是行数不变的 no-op，
    // 测试即使 pin/补偿坏掉也会通过。
    let Some(AssistantStepView::Part { part, .. }) = run.chat.messages[58].steps.first_mut() else {
        panic!("用户消息必须含正文步骤");
    };
    part.content = (0..10)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    run.follow = true;
    run.scroll = 0;
    let (mut state, pane) = state_with_detail(run);
    let rect = Rect::new(0, 0, 80, 24);
    let mut cache = crate::ui::RenderCache::default();
    update::resolve_task_agents_visible(&mut state, &[(pane, rect)], &mut cache);
    let (total, visible) = {
        let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
        assert!(run.follow, "初始应贴底");
        (run.total_rows, run.visible)
    };
    assert!(total > visible, "夹具应可滚动");

    assert!(update::toggle_agent_card(
        &mut state,
        pane,
        crate::app::state::CardTarget::UserMessage {
            message_id: "m58".into(),
        }
    ));
    let pinned = total.saturating_sub(visible);
    {
        let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
        assert!(!run.follow, "展开即进入阅读态");
        assert_eq!(run.scroll, pinned, "钉在展开前的顶部行");
        assert!(run.rows_dirty);
    }

    update::resolve_task_agents_visible(&mut state, &[(pane, rect)], &mut cache);
    let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
    assert!(!run.follow, "补偿不得恢复跟随");
    assert_eq!(run.scroll, pinned, "展开内容在视口下方，顶部行不动");
    assert!(
        run.total_rows > total,
        "卡片展开必须真的增加行数（{} → {}）",
        total,
        run.total_rows
    );
}

#[test]
fn coalesced_measure_keeps_rows_dirty_until_fresh() {
    let mut run = detail_run();
    run.follow = false;
    run.scroll = 0;
    let (mut state, pane) = state_with_detail(run);
    let rect = Rect::new(0, 0, 80, 24);
    let mut cache = crate::ui::RenderCache::default();
    assert!(update::resolve_task_agents_visible(
        &mut state,
        &[(pane, rect)],
        &mut cache
    ));
    let old_total = {
        let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
        assert!(!run.rows_dirty);
        assert!(run.total_rows > run.visible);
        run.total_rows
    };
    // test profile 默认关闭离屏合并；这里显式打开，模拟生产 80ms 窗口。
    cache
        .pane(pane)
        .set_offscreen_coalesce(Duration::from_millis(80));

    {
        let run = &mut state.pane_mut_anywhere(pane).expect("pane").lx.agents.runs[0];
        if let Some(AssistantStepView::Part { part, .. }) = run
            .chat
            .messages
            .last_mut()
            .and_then(|message| message.steps.first_mut())
        {
            part.content.push_str("\nstreaming line");
        }
        run.rows_dirty = true;
    }
    update::resolve_task_agents_visible(&mut state, &[(pane, rect)], &mut cache);
    {
        let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
        assert!(run.rows_dirty, "合并窗口内不得清除 rows_dirty");
        assert_eq!(run.total_rows, old_total, "合并窗口内不得写旧快照的总行数");
    }

    cache.pane(pane).set_offscreen_coalesce(Duration::ZERO);
    update::resolve_task_agents_visible(&mut state, &[(pane, rect)], &mut cache);
    let run = &state.pane_anywhere(pane).expect("pane").lx.agents.runs[0];
    assert!(!run.rows_dirty, "窗口结束后必须重测");
    assert!(run.total_rows > old_total, "总行数应随新内容增长");
}

#[test]
fn geometry_invalidation_refreshes_detail_width() {
    let run = wide_detail_run();
    let (mut state, pane) = state_with_detail(run);
    let mut cache = crate::ui::RenderCache::default();
    let narrow = Rect::new(0, 0, 40, 24);
    update::resolve_task_agents_visible(&mut state, &[(pane, narrow)], &mut cache);
    let narrow_total = state.pane_anywhere(pane).expect("pane").lx.agents.runs[0].total_rows;

    update::invalidate_task_agent_rows(&mut state);
    let wide = Rect::new(0, 0, 80, 24);
    update::resolve_task_agents_visible(&mut state, &[(pane, wide)], &mut cache);
    let wide_total = state.pane_anywhere(pane).expect("pane").lx.agents.runs[0].total_rows;
    assert_ne!(narrow_total, wide_total, "宽度变化后必须重算总行数");
    assert!(
        !state.pane_anywhere(pane).expect("pane").lx.agents.runs[0].rows_dirty,
        "重算后复位"
    );
}

#[test]
fn ready_snapshot_does_not_cross_pages() {
    let lx = lx_with_messages();
    let mut cache = PaneCache::default();
    let main = cache.snapshot_source(40, HistorySource::from(&lx));
    let builds = cache.builds();
    let run = detail_run();
    let detail = cache.snapshot_ready_source(40, HistorySource::from_chat(&run.chat, &run.compact));
    assert!(
        cache.builds() > builds,
        "主历史与详情宽度相同也不能复用快照"
    );
    assert_ne!(detail.total_rows, main.total_rows);
}

#[test]
fn zwj_emoji_identity_keeps_columns_aligned() {
    let agents = AgentsState {
        runs: vec![
            agent(
                "emoji",
                "/root/emoji",
                Some("👨‍👩‍👧👨‍👩‍👧"),
                "running",
                "emoji title 👨‍👩‍👧",
            ),
            agent(
                "ascii",
                "/root/ascii",
                Some("Turing"),
                "completed",
                "ascii title",
            ),
        ],
        cards_dirty: false,
        ..Default::default()
    };
    let page = TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    };
    let (lines, overflow) = draw_list_with_overflow(80, 2, &agents, &page);
    assert_no_overflow(&overflow, 80);
    let emoji = lines
        .iter()
        .find(|line| line.contains("👨"))
        .expect("emoji");
    let ascii = lines
        .iter()
        .find(|line| line.contains("Turing"))
        .expect("ascii");
    let status = |line: &str| {
        if line.contains("running") {
            column(line, "running")
        } else {
            column(line, "completed")
        }
    };
    assert_eq!(
        status(emoji),
        status(ascii),
        "emoji 身份宽度必须与逐字符 Grid 口径一致：{emoji:?} / {ascii:?}"
    );
    assert!(emoji.contains('■'));
    assert!(ascii.contains('■'));
}

#[test]
fn tree_sibling_parent_comparison_is_case_insensitive() {
    let agents = AgentsState {
        runs: vec![
            agent("parent", "/root/P", None, "completed", "parent"),
            // 同级兄弟的 parent 拼写大小写不同；不能把前者误判成末项。
            agent("c1", "/root/P/C1", None, "completed", "c1"),
            agent("c2", "/root/p/c2", None, "completed", "c2"),
        ],
        cards_dirty: false,
        ..Default::default()
    };
    let page = TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    };
    let lines = draw_list_lines(60, 3, &agents, &page);
    let c1 = lines.iter().find(|line| line.contains("c1")).expect("c1");
    let c2 = lines.iter().find(|line| line.contains("c2")).expect("c2");
    assert!(
        c1.contains("├─"),
        "大小写不同的同级兄弟仍应使用分支符：{c1:?}"
    );
    assert!(c2.contains("└─"), "最后一个兄弟收口：{c2:?}");
}

#[test]
fn agents_tab_shows_its_placeholder_and_reserves_the_slot() {
    // 需求：这一页要预留出显示 subagent 的位置，本阶段先给占位提示。
    let mut lx = lx_with_messages();
    let mut page = page_with(vec![entry(1, "npm run dev", "/w")], 0);
    page.tab = TaskTab::Agents;
    lx.task = Some(page);
    let text = render_lines(&lx, 60, 16).join("\n");
    assert!(
        text.contains(text::LX_TASK_AGENTS_EMPTY),
        "子代理占位：{text}"
    );
    assert!(
        !text.contains("npm run dev"),
        "Agents tab 不显示进程：{text}"
    );
    // 键位提示随 tab 变化。
    assert!(
        text.contains(&text::LX_TASK_HINTS_AGENTS[0].label()),
        "Agents tab 的键位：{text}"
    );
}

#[test]
fn agents_filter_pills_render_and_hit() {
    let mut lx = lx_with_messages();
    lx.agents
        .runs
        .push(tree_agent("a", "/root/a", "running", None));
    lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        agent_filter: crate::app::state::AgentFilter::Active,
        ..TaskPage::default()
    });
    let lines = render_lines(&lx, 80, 20);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("all · active · done")),
        "Agents 页签右侧渲染筛选胶囊：{lines:?}"
    );

    let rect = Rect::new(0, 0, 80, 20);
    let page = lx.task.as_ref().expect("task");
    let pills = filter_row_rects(rect, page);
    assert_eq!(pills.len(), 3, "三个筛选胶囊");
    let (filter, x, _) = pills[1];
    assert_eq!(filter, crate::app::state::AgentFilter::Active);
    assert_eq!(
        hit(rect, &lx, x, rect.y + TOP_BORDER),
        Some(TaskHit::AgentFilter(filter)),
        "点击活动筛选胶囊命中"
    );
}

#[test]
fn agent_composer_renders_target_badge_and_draft() {
    let mut lx = lx_with_messages();
    let mut run = tree_agent("a", "/root/turing", "completed", None);
    run.nickname = Some("Turing".into());
    lx.agents.runs.push(run);
    lx.agents.selected_id = Some("a".into());
    lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        agent_edit: Some("a".into()),
        ..TaskPage::default()
    });
    lx.agents
        .ensure_composer("a")
        .editor_mut()
        .replace_input("hello");
    let lines = render_lines(&lx, 80, 20);
    assert!(
        lines.iter().any(|line| line.contains("Subagent")),
        "composer 左侧标记：{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Turing [executor]")),
        "composer 右侧目标：{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Turing [executor] · private")),
        "默认私聊模式：{lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("To:")),
        "composer 右侧目标不再带 To: 前缀：{lines:?}"
    );
    assert!(lines.iter().any(|line| line.contains("hello")));

    // 运行中的 agent 显示 steer，而不是 private。
    lx.agents.runs[0].status = "running".into();
    lx.agents.runs[0]
        .chat
        .pending_steers
        .push(crate::app::state::lx::chat::steer::PendingSteer {
            text: "steer me".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        });
    let lines = render_lines(&lx, 80, 20);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Turing [executor] · steer 1")),
        "运行中显示待注入 steer：{lines:?}"
    );
}

#[test]
fn agents_filter_hit_maps_to_real_run_index() {
    let rect = Rect::new(0, 0, 70, 10);
    let mut agents = crate::app::state::AgentsState::default();
    agents
        .runs
        .push(tree_agent("done", "/root/done", "completed", None));
    agents
        .runs
        .push(tree_agent("live", "/root/live", "running", None));
    let page = TaskPage {
        tab: TaskTab::Agents,
        agent_filter: crate::app::state::AgentFilter::Active,
        ..TaskPage::default()
    };
    let mut lx = Lx::new();
    lx.task = Some(page);
    lx.agents = agents;
    let list_y = rect.y + TOP_BORDER + TAB_ROW + SEARCH_ROW;
    assert_eq!(
        hit(rect, &lx, rect.x, list_y),
        Some(TaskHit::Agent(1)),
        "筛选后的第一行映射回 runs[1]"
    );
}

#[test]
fn agents_list_mode_has_search_and_five_rows() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        ..Default::default()
    });
    let height = task_panel_height(&lx);
    assert_eq!(height, 8, "Agents 列表态固定 8 行");
    let panel = Rect::new(0, 0, 80, height);
    let search = search_rect(panel);
    let rows = list_rows_rect(panel, TaskTab::Agents);
    assert_eq!(search.height, 1);
    assert_eq!(rows.height, 5);
    assert_eq!(search.y + 1, rows.y);
}

#[test]
fn agent_composer_mention_panel_lists_four_tabs_without_subagents() {
    let mut lx = lx_with_messages();
    let mut run = tree_agent("a", "/root/turing", "running", None);
    run.nickname = Some("Turing".into());
    lx.agents.runs.push(run);
    lx.agents.selected_id = Some("a".into());
    lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        agent_edit: Some("a".into()),
        ..TaskPage::default()
    });
    let composer = lx.agents.ensure_composer("a").editor_mut();
    composer.set_mention_root(Some(std::path::PathBuf::from("/work/demo")));
    composer.insert_char('@');
    let (generation, root) = composer.take_mention_scan_request().expect("scan");
    composer.apply_mention_entries(
        generation,
        &root,
        vec![crate::app::markdown::MentionEntry {
            path: "main.rs".into(),
            is_directory: false,
        }],
    );

    let text = render_lines(&lx, 80, 24).join("\n");
    for label in ["Files", "refDir", "Skills", "MCP"] {
        assert!(text.contains(label), "缺少 Tab {label}：{text}");
    }
    assert!(
        !text.contains("Subagents"),
        "Agents 追问不显示 Subagents：{text}"
    );
    assert!(text.contains("main.rs"), "候选可见：{text}");
}
