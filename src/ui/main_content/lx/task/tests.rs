//! 后台任务页渲染：整页接管窗格、tab 指示、列表行与空态。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::{ProcessEntry, TaskPage, TaskTab};

/// 把整个缓冲区拼成文本行。
fn buffer_lines(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect()
}

fn entry(session_id: u64, command: &str, cwd: &str) -> ProcessEntry {
    ProcessEntry {
        session_id,
        command: command.into(),
        cwd: cwd.into(),
        tty: false,
        started_at_ms: 0,
    }
}

/// 渲染给定页面并返回文本行。
fn render_page(page: &TaskPage, width: u16, height: u16) -> Vec<String> {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, page, true);
    buffer_lines(&buf, area)
}

fn page_with(entries: Vec<ProcessEntry>, now_ms: i64) -> TaskPage {
    let mut page = TaskPage::default();
    page.set_processes(entries, now_ms);
    page
}

#[test]
fn page_draws_tabs_and_footer_on_one_bottom_bar() {
    let page = page_with(vec![entry(1, "npm run dev", "/w/web")], 90_000);
    let text = render_page(&page, 70, 12).join("\n");
    assert!(
        text.contains(TaskTab::Processes.label()),
        "进程 tab：{text}"
    );
    assert!(text.contains(TaskTab::Agents.label()), "子代理 tab：{text}");
    assert!(text.contains(text::LX_TASK_PAGE_FOOTER), "键位提示：{text}");
}

#[test]
fn page_does_not_draw_its_own_frame() {
    // 用户报的 bug：页内整框与窗格边框叠成双重边框。
    // 页面只负责列表 + 底部一条横栏，绝不再画 `╭ ╮ ╰ ╯`。
    let page = page_with(vec![entry(1, "npm run dev", "/w/web")], 0);
    let lines = render_page(&page, 70, 12);
    let text = lines.join("\n");
    for corner in ['╭', '╮', '╰', '╯'] {
        assert!(
            !text.contains(corner),
            "页内不得再画圆角框（`{corner}` 属于窗格边框）：{text}"
        );
    }
}

#[test]
fn bottom_bar_joins_the_pane_side_borders() {
    // 横栏是 `├─…─┤`：两端落在窗格竖直边框所在列（内容区左右各外扩一列），
    // 因此读作"从边框长出来的一条分隔线"，而不是第二个框的顶边。
    let page = page_with(vec![entry(1, "npm run dev", "/w/web")], 0);
    // 内容区左右各外扩一列放接缝，所以缓冲区要比内容区宽 2 列。
    let area = Rect::new(1, 1, 68, 12);
    let mut buf = Buffer::empty(Rect::new(0, 0, 70, 14));
    render(area, &mut buf, &page, true);
    let bar_y = area.bottom() - 1;
    assert_eq!(
        buf[(area.x - 1, bar_y)].symbol(),
        "├",
        "左端接在窗格左边框上"
    );
    assert_eq!(
        buf[(area.right(), bar_y)].symbol(),
        "┤",
        "右端接在窗格右边框上"
    );
    // tab 标签紧跟在左接缝之后（角内侧留一列空白）。
    assert_eq!(buf[(area.x + 2, bar_y)].symbol(), "P", "tab 从接缝右侧起");
}

#[test]
fn process_rows_show_command_cwd_and_running_age() {
    let page = page_with(
        vec![entry(1, "npm run dev", "/workspace/web")],
        90_000, // 启动于 0，已运行 1m 30s
    );
    let text = render_page(&page, 70, 12).join("\n");
    assert!(text.contains("npm run dev"), "命令：{text}");
    assert!(text.contains("/workspace/web"), "cwd：{text}");
    // 时长带 `running ` 前缀，让"还在跑"这件事在列表里一眼可见。
    assert!(text.contains("running 1m 30s"), "运行时长：{text}");
}

#[test]
fn selected_row_carries_the_marker() {
    let mut page = page_with(
        vec![entry(1, "first", "/a"), entry(2, "second", "/b")],
        1_000,
    );
    page.window.move_active(1, 2);
    let lines = render_page(&page, 60, 12);
    let second = lines
        .iter()
        .find(|line| line.contains("second"))
        .expect("第二行");
    let first = lines
        .iter()
        .find(|line| line.contains("first"))
        .expect("第一行");
    assert!(
        second.contains(text::AGENT_SELECTED_MARKER),
        "高亮行带标记：{second}"
    );
    assert!(
        !first.contains(text::AGENT_SELECTED_MARKER),
        "未高亮行不带标记：{first}"
    );
}

#[test]
fn empty_process_tab_shows_the_empty_state() {
    let page = page_with(Vec::new(), 0);
    let text = render_page(&page, 60, 12).join("\n");
    assert!(text.contains(text::LX_TASK_PROCESSES_EMPTY), "空态：{text}");
}

#[test]
fn agents_tab_shows_its_placeholder_and_reserves_the_slot() {
    // 需求：这一页要预留出显示 subagent 的位置，本阶段先给占位提示。
    let mut page = page_with(vec![entry(1, "npm run dev", "/w")], 0);
    page.tab = TaskTab::Agents;
    let text = render_page(&page, 60, 12).join("\n");
    assert!(
        text.contains(text::LX_TASK_AGENTS_EMPTY),
        "子代理占位：{text}"
    );
    // 切到 Agents 后不再显示进程行：两个 tab 是互斥的列表。
    assert!(
        !text.contains("npm run dev"),
        "Agents tab 不显示进程：{text}"
    );
}

#[test]
fn narrow_pane_drops_cwd_before_truncating_command() {
    // 窄窗格里先牺牲 cwd，命令尽量保留（命令才是用户识别进程的依据）。
    let page = page_with(
        vec![entry(1, "npm run dev", "/very/long/workspace/path")],
        0,
    );
    let text = render_page(&page, 30, 12).join("\n");
    assert!(text.contains("npm run"), "命令优先保留：{text}");
    assert!(!text.contains("/very/long"), "cwd 先被牺牲：{text}");
}

#[test]
fn tiny_area_draws_nothing_instead_of_panicking() {
    // 窗格被压到极小：直接放弃绘制，不越界、不 panic。
    for (width, height) in [(0u16, 0u16), (1, 1), (3, 2)] {
        let page = page_with(vec![entry(1, "cmd", "/w")], 0);
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &page, true);
    }
}

#[test]
fn window_scrolls_to_keep_the_highlight_visible() {
    // 行数多于可视高度时，高亮滚到哪，窗口跟到哪。
    let entries = (1..=10)
        .map(|index| entry(index, &format!("cmd-{index}"), "/w"))
        .collect();
    let mut page = page_with(entries, 0);
    page.window.move_active(9, 10);
    let lines = render_page(&page, 50, 6);
    let text = lines.join("\n");
    assert!(text.contains("cmd-10"), "末项可见：{text}");
    assert!(!text.contains("cmd-1 "), "窗口已滚离首项：{text}");
}
