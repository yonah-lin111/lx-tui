//! 后台任务面板渲染：上方历史 + 沉底控制面板 + 底边框键位。
//!
//! 断言口径：面板**不再整页接管**，也不再画自己的框；上半区是普通聊天历史，
//! 输入框被面板替换，键位提示落在窗格底边框上。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::{Lx, ProcessEntry, TaskPage, TaskTab};

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

fn page_with(entries: Vec<ProcessEntry>, now_ms: i64) -> TaskPage {
    let mut page = TaskPage::default();
    page.set_processes(entries, now_ms);
    page
}

/// 渲染给定页面并返回文本行（`lx` 不带消息：只测面板本身）。
fn render_page(page: &TaskPage, width: u16, height: u16) -> Vec<String> {
    let lx = Lx::default();
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, page, true);
    buffer_lines(&buf, area)
}

/// 渲染页面并返回文本行（带消息的 `lx`）。
fn render_page_with(lx: &Lx, page: &TaskPage, width: u16, height: u16) -> Vec<String> {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, lx, page, true);
    buffer_lines(&buf, area)
}

#[test]
fn page_draws_tabs_in_the_panel_and_hints_on_the_bottom_border() {
    let page = page_with(vec![entry(1, "npm run dev", "/w/web")], 90_000);
    let lines = render_page(&page, 70, 14);
    let text = lines.join("\n");
    assert!(
        text.contains(TaskTab::Processes.label()),
        "进程 tab：{text}"
    );
    assert!(text.contains(TaskTab::Agents.label()), "子代理 tab：{text}");
    // 键位提示在**最后一行**（窗格底边框），不在正文里。
    let last = lines.last().expect("最后一行");
    assert!(
        last.contains(text::LX_TASK_FOOTER_PROCESSES),
        "键位提示落在底边框：{last}"
    );
}

#[test]
fn panel_sits_at_the_bottom_not_the_top() {
    // 用户要求：任务列表沉底，中间内容区留给消息列表。
    let page = page_with(vec![entry(1, "npm run dev", "/w/web")], 0);
    let lines = render_page(&page, 70, 14);
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    // tab 行必须在面板区（靠下），不能贴在最顶部。
    assert!(
        tab_row >= lines.len() / 2,
        "tab 行应沉底（第 {tab_row} 行 / 共 {} 行）：{lines:#?}",
        lines.len()
    );
    // 列表在 tab 行之下。
    let list_row = lines
        .iter()
        .position(|line| line.contains("npm run dev"))
        .expect("进程行");
    assert!(list_row > tab_row, "列表在 tab 之后");
}

#[test]
fn page_does_not_draw_its_own_frame() {
    // 用户报过：页内整框与窗格边框叠成双重边框。
    // 页面只负责面板 + 底边框键位，绝不画 `╭ ╮ ╰ ╯`。
    let page = page_with(vec![entry(1, "npm run dev", "/w/web")], 0);
    let text = render_page(&page, 70, 14).join("\n");
    for corner in ['╭', '╮', '╰', '╯'] {
        assert!(
            !text.contains(corner),
            "页内不得再画圆角框（`{corner}` 属于窗格边框）：{text}"
        );
    }
}

#[test]
fn history_is_rendered_above_the_panel() {
    // 中间内容区默认显示消息列表：面板打开时历史仍然可见。
    let mut lx = Lx::default();
    lx.chat.messages = vec![text_message("msg-1", "user", "你是谁")];
    let page = page_with(vec![entry(1, "npm run dev", "/w/web")], 0);
    let lines = render_page_with(&lx, &page, 70, 16);
    let text = lines.join("\n");
    assert!(text.contains("你"), "历史消息应当可见：{text}");
    // 消息在面板之上。
    let history_row = lines
        .iter()
        .position(|line| line.contains('你'))
        .expect("历史行");
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    assert!(history_row < tab_row, "历史在面板之上");
}

#[test]
fn process_rows_show_command_cwd_and_running_age() {
    let page = page_with(
        vec![entry(1, "npm run dev", "/workspace/web")],
        90_000, // 启动于 0，已运行 1m 30s
    );
    let text = render_page(&page, 70, 14).join("\n");
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
    let lines = render_page(&page, 60, 14);
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
    let text = render_page(&page, 60, 14).join("\n");
    assert!(text.contains(text::LX_TASK_PROCESSES_EMPTY), "空态：{text}");
}

#[test]
fn agents_tab_shows_its_placeholder_and_reserves_the_slot() {
    // 需求：这一页要预留出显示 subagent 的位置，本阶段先给占位提示。
    let mut page = page_with(vec![entry(1, "npm run dev", "/w")], 0);
    page.tab = TaskTab::Agents;
    let text = render_page(&page, 60, 14).join("\n");
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
fn panel_shows_esc_on_the_tab_row() {
    // 对齐 opencode composer：右上角提示 `esc` 关闭。
    let page = page_with(vec![entry(1, "npm run dev", "/w")], 0);
    let lines = render_page(&page, 70, 14);
    let tab_row = lines
        .iter()
        .find(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    assert!(
        tab_row.contains(text::LX_TASK_ESC),
        "tab 行右端有 esc 提示：{tab_row}"
    );
}

#[test]
fn narrow_pane_drops_cwd_before_truncating_command() {
    // 窄窗格里先牺牲 cwd，命令尽量保留（命令才是用户识别进程的依据）。
    let page = page_with(
        vec![entry(1, "npm run dev", "/very/long/workspace/path")],
        0,
    );
    let text = render_page(&page, 30, 14).join("\n");
    assert!(text.contains("npm run"), "命令优先保留：{text}");
    assert!(!text.contains("/very/long"), "cwd 先被牺牲：{text}");
}

#[test]
fn tiny_area_draws_nothing_instead_of_panicking() {
    // 窗格被压到极小：直接放弃绘制，不越界、不 panic。
    let lx = Lx::default();
    for (width, height) in [(0u16, 0u16), (1, 1), (3, 2)] {
        let page = page_with(vec![entry(1, "cmd", "/w")], 0);
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &lx, &page, true);
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
    let text = render_page(&page, 50, 14).join("\n");
    assert!(text.contains("cmd-10"), "末项可见：{text}");
    assert!(!text.contains("cmd-1 "), "窗口已滚离首项：{text}");
}

/// 构造一条消息（单个 text part）。
fn text_message(id: &str, role: &str, content: &str) -> crate::app::state::ChatMessageView {
    crate::app::state::ChatMessageView {
        id: id.into(),
        role: role.into(),
        status: "complete".into(),
        turn_index: 0,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: content.into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}
