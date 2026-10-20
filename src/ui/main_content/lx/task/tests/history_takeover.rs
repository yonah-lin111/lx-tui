//! 后台任务面板两段式布局的回归测试。
//!
//! 用户明确要求的版式：
//!
//! ```text
//! ┌────────────────────────────┐
//! │ 显示后台任务的容器            │ ← 原**消息列表**的位置
//! │ （高亮任务的输出）            │
//! ├────────────────────────────┤
//! ┌────────────────────────────┐
//! │ Processes · Agents  [close]│ ← 原**输入框**的位置（item 列表）
//! │ ❯ cmd  cwd        running  │
//! └────────────────────────────┘
//! ```
//!
//! 两个区块**同时存在**：输出容器吃掉历史区，item 列表继续占输入框矩形。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    AssistantStepView, ChatMessageView, ChatPartView, Lx, ProcessEntry, TaskPage, TaskTab,
};

/// 构造一条消息（单个 text part）。
fn text_message(id: &str, role: &str, content: &str) -> ChatMessageView {
    ChatMessageView {
        id: id.into(),
        role: role.into(),
        status: "complete".into(),
        turn_index: 0,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
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

fn entry(session_id: u64, command: &str) -> ProcessEntry {
    ProcessEntry {
        session_id,
        command: command.into(),
        cwd: "/workspace/lx".into(),
        tty: false,
        started_at_ms: 0,
    }
}

fn page_with(entries: Vec<ProcessEntry>) -> TaskPage {
    let mut page = TaskPage::default();
    page.set_processes(entries, 0);
    page
}

fn lx_with_messages() -> Lx {
    let mut lx = Lx::default();
    lx.chat.messages = vec![
        text_message("m1", "user", "你是谁"),
        text_message("m2", "assistant", "我是 Claude，一个 AI 助手。"),
    ];
    lx.chat.follow_bottom = true;
    lx
}

/// 整块渲染并返回文本行（**含**底边框那一行）。
fn render_lines(lx: &Lx, width: u16, height: u16) -> Vec<String> {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height + 1));
    crate::ui::main_content::lx::render(area, &mut buf, lx, true, None);
    (0..=height)
        .map(|y| {
            (0..width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

// ===== 输出容器：占原消息列表位置 =====

#[test]
fn output_container_takes_over_the_message_list_area() {
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let lines = render_lines(&lx, 70, 24);
    let joined = lines.join("\n");
    // 消息正文不再出现（那块矩形被输出容器接管）。
    assert!(
        !joined.contains("我是 Claude"),
        "输出容器占用历史区后不再画消息正文：{lines:#?}"
    );
    // 输出容器从内容区最顶部开始（顶边栏 + 标题）。
    assert!(
        lines[0].contains(text::LX_TASK_OUTPUT_TITLE),
        "输出容器贴内容区顶部：{lines:#?}"
    );
}

#[test]
fn output_container_shows_the_selected_process_output() {
    let mut lx = lx_with_messages();
    let mut page = page_with(vec![entry(1, "npm run dev"), entry(2, "cargo watch")]);
    page.output = vec![
        "ready in 320ms".into(),
        "heartbeat 19:39:36".into(),
        "heartbeat 19:39:46".into(),
    ];
    lx.task = Some(page);
    let joined = render_lines(&lx, 70, 24).join("\n");
    assert!(
        joined.contains("heartbeat 19:39:36"),
        "输出正文必须显示：{joined}"
    );
    assert!(
        joined.contains("npm run dev"),
        "顶边栏标题带上高亮任务：{joined}"
    );
}

#[test]
fn empty_output_shows_an_explicit_placeholder() {
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let joined = render_lines(&lx, 70, 24).join("\n");
    assert!(
        joined.contains(text::LX_TASK_OUTPUT_EMPTY),
        "空输出要有占位而不是留白：{joined}"
    );
}

// ===== item 列表：仍在原输入框位置 =====

#[test]
fn item_list_stays_where_the_input_box_was() {
    let mut lx = lx_with_messages();
    let closed = render_lines(&lx, 70, 24);
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let opened = render_lines(&lx, 70, 24);

    // 关闭态里输入框所在行（`> Ask anything…` 上方那条分隔线）。
    let input_row = closed
        .iter()
        .rposition(|line| {
            line.trim_start()
                .starts_with(text::LX_INPUT_PROMPT.trim_end())
        })
        .expect("关闭态应有输入框");
    // 打开态里 tab 行必须落在输入框那一块（而不是贴内容区顶部）。
    let tab_row = opened
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    assert!(
        tab_row > opened.len() / 2,
        "item 列表在窗格下半部（原输入框位置）：{opened:#?}"
    );
    assert!(
        tab_row <= input_row,
        "item 列表不高于原输入框位置：tab {tab_row} / 输入框 {input_row}\n{opened:#?}"
    );
    // 任务条目也在下半部（命令名同样出现在输出容器顶边栏，只在 tab 行之下找）。
    let entry_row = tab_row
        + opened[tab_row..]
            .iter()
            .position(|line| line.contains("npm run dev"))
            .expect("任务行");
    assert!(entry_row > tab_row, "任务行在 tab 行之下");
}

#[test]
fn both_blocks_are_visible_at_the_same_time() {
    // 用户强调：消息列表位置变成输出容器，**同时**输入框位置是 item 列表。
    let mut lx = lx_with_messages();
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    page.output = vec!["heartbeat".into()];
    lx.task = Some(page);
    let lines = render_lines(&lx, 70, 24);
    let output_row = lines
        .iter()
        .position(|line| line.contains("heartbeat"))
        .expect("输出容器显示输出");
    // 命令名也出现在输出容器顶边栏，只在 tab 行之下找真正的 item 行。
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    let list_row = tab_row
        + lines[tab_row..]
            .iter()
            .position(|line| line.contains("npm run dev"))
            .expect("item 列表显示任务");
    assert!(
        output_row < list_row,
        "输出容器在上、item 列表在下：输出 @{output_row} 列表 @{list_row}"
    );
}

// ===== 关闭面板后恢复 =====

#[test]
fn closing_the_panel_restores_the_message_list_verbatim() {
    let mut lx = lx_with_messages();
    let before = render_lines(&lx, 70, 24);
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let _opened = render_lines(&lx, 70, 24);
    lx.task = None;
    let after = render_lines(&lx, 70, 24);
    assert_eq!(before, after, "关闭面板后整屏必须与打开前一致");
}

// ===== 决策 9：`[open]` 定位高亮 =====

#[test]
fn focus_selects_the_requested_process() {
    let mut page = page_with(vec![
        entry(1, "first"),
        entry(2, "second"),
        entry(3, "third"),
    ]);
    page.focus = Some(3);
    page.resolve_focus();
    assert_eq!(
        page.selected().map(|process| process.session_id),
        Some(3),
        "高亮必须落到 [open] 指定的那条"
    );
    assert_eq!(page.focus, None, "定位是一次性的，落地后清空");
}

#[test]
fn focus_falls_back_to_first_when_the_process_is_gone() {
    let mut page = page_with(vec![entry(1, "first"), entry(2, "second")]);
    page.focus = Some(99);
    page.resolve_focus();
    assert_eq!(page.window.active, 0, "进程已结束则回落首项");
    assert_eq!(page.focus, None, "失效定位也要清空");
}

#[test]
fn focus_on_empty_list_does_not_panic() {
    let mut page = page_with(Vec::new());
    page.focus = Some(7);
    page.resolve_focus();
    assert!(page.selected().is_none(), "空列表下高亮无效");
    assert_eq!(page.focus, None);
}

// ===== 决策 11：PageUp/PageDown 翻输出区 =====

#[test]
fn output_scroll_moves_within_bounds() {
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    page.output = (0..50).map(|index| format!("line {index}")).collect();
    page.output_visible = 10;
    page.output_scroll = 3;
    assert!(page.scroll_output(-5), "可以向上翻");
    assert_eq!(page.output_scroll, 0, "已在顶部时钳制");
    assert!(!page.scroll_output(-1), "顶部再向上不动");
    assert!(page.scroll_output(5));
    assert_eq!(page.output_scroll, 5);
    assert!(page.scroll_output(isize::MAX), "可以一次翻到底");
    assert_eq!(page.output_scroll, 40, "上限 = 总行数 - 可见行数");
    assert!(!page.scroll_output(1), "到底后不再移动");
}

#[test]
fn changing_selection_pins_output_to_the_tail() {
    let mut page = page_with(vec![entry(1, "first"), entry(2, "second")]);
    page.output = (0..50).map(|index| format!("line {index}")).collect();
    page.output_visible = 10;
    page.output_scroll = 3;
    assert!(page.move_active(1), "高亮移到第二条");
    assert_eq!(page.output_scroll, 40, "换任务后输出贴尾部（最新输出）");
}

// ===== 输出容器的 `[close esc]` 可点 =====

#[test]
fn output_container_close_button_is_clickable() {
    let area = Rect::new(0, 0, 70, 24);
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let rects = crate::ui::main_content::lx::task_rects(area, &lx).expect("面板已打开");
    let close = output_close_rect(rects.output).expect("输出容器有 [close esc]");
    assert_eq!(
        hit_in(
            rects.list,
            Some(rects.output),
            lx.task.as_ref().expect("page"),
            close.x,
            close.y
        ),
        Some(TaskHit::Close),
        "输出容器右端的 [close esc] 可点"
    );
    // item 列表的 tab 仍可点，两段命中互不干扰。
    let tab_row = rects.list.y.saturating_add(TOP_BORDER);
    assert!(
        matches!(
            hit_in(
                rects.list,
                Some(rects.output),
                lx.task.as_ref().expect("page"),
                rects.list.x,
                tab_row
            ),
            Some(TaskHit::Tab(0))
        ),
        "item 列表左上角 tab 仍可点"
    );
}
