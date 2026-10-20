//! 后台任务面板接管消息列表区域的回归测试。
//!
//! 用户要求（Grill Me 决策 4/8/9/10/11）：
//! 1. 点卡片的 `[open]` 后，面板占用**原消息列表位置**，输入框仍在原位；
//! 2. 面板上半是任务列表（`❯` + `^d`），下半是高亮任务的输出；
//! 3. `[open]` 携带 session_id 定位高亮，进程已结束则回落首项；
//! 4. 面板期间历史滚动冻结，关闭后消息列表位置与内容原样恢复。

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

// ===== 决策 4：面板接管历史区，输入框原位 =====

#[test]
fn panel_takes_over_the_history_region_not_the_input_box() {
    let mut lx = lx_with_messages();
    let closed = render_lines(&lx, 70, 20);
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let opened = render_lines(&lx, 70, 20);

    // 消息内容必须从消息列表区域消失（被面板接管）。
    let joined = opened.join("\n");
    assert!(
        !joined.contains("我是 Claude"),
        "面板占用历史区后不应再画消息正文：{opened:#?}"
    );
    // 输入框仍在原位（lx 输入框是上下 `─` 分割线 + `> ` 前缀，没有圆角）。
    let input_row_closed = closed.iter().rposition(|line| {
        line.trim_start()
            .starts_with(text::LX_INPUT_PROMPT.trim_end())
    });
    assert!(input_row_closed.is_some(), "关闭态应有输入框：{closed:#?}");
    // tab 行必须出现在历史区起点附近（而不是贴底）。
    let tab_row = opened
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("面板 tab 行");
    assert!(
        tab_row < opened.len() / 2,
        "面板必须从历史区顶部开始，而不是贴底：{opened:#?}"
    );
}

#[test]
fn input_box_stays_at_its_original_position() {
    let mut lx = lx_with_messages();
    let closed = render_lines(&lx, 70, 20);
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let opened = render_lines(&lx, 70, 20);
    // 底边框行（最后一行）两侧都必须是同一套键位/状态渲染位置：
    // 输入框的下边框在关闭态倒数第二行，打开面板后该行仍然是输入框下边框。
    let closed_bottom = closed[closed.len() - 2].clone();
    let opened_bottom = opened[opened.len() - 2].clone();
    assert!(
        closed_bottom.contains('─') && opened_bottom.contains('─'),
        "输入框样式区域仍在原位：关闭 {closed_bottom:?} 打开 {opened_bottom:?}"
    );
}

#[test]
fn closing_the_panel_restores_the_message_list_verbatim() {
    // 决策 10：关闭后消息列表位置与内容必须原样恢复。
    let mut lx = lx_with_messages();
    let before = render_lines(&lx, 70, 20);
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let _opened = render_lines(&lx, 70, 20);
    lx.task = None;
    let after = render_lines(&lx, 70, 20);
    assert_eq!(before, after, "关闭面板后整屏必须与打开前一致");
}

// ===== 决策 8：列表 + 输出区 =====

#[test]
fn panel_shows_process_list_and_selected_output() {
    let mut lx = lx_with_messages();
    let mut page = page_with(vec![entry(1, "npm run dev"), entry(2, "cargo watch")]);
    page.output = vec![
        "ready in 320ms".into(),
        "heartbeat 19:39:36".into(),
        "heartbeat 19:39:46".into(),
    ];
    lx.task = Some(page);
    let lines = render_lines(&lx, 70, 24);
    let joined = lines.join("\n");
    assert!(joined.contains("npm run dev"), "任务列表可见：{joined}");
    assert!(joined.contains("cargo watch"), "另一条任务也可见：{joined}");
    assert!(
        joined.contains("heartbeat 19:39:36"),
        "高亮任务的输出必须显示：{joined}"
    );
}

#[test]
fn output_area_renders_nothing_alarming_when_empty() {
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let lines = render_lines(&lx, 70, 24);
    let joined = lines.join("\n");
    assert!(
        joined.contains(text::LX_TASK_OUTPUT_EMPTY),
        "空输出要有占位而不是空白：{joined}"
    );
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
    // 卡片会留在历史里，但进程可能早就退出了：此时优雅降级到首项。
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
    // 用户要求：高亮变了就贴输出尾部（最新输出）。
    let mut page = page_with(vec![entry(1, "first"), entry(2, "second")]);
    page.output = (0..50).map(|index| format!("line {index}")).collect();
    page.output_visible = 10;
    page.output_scroll = 3;
    assert!(page.move_active(1), "高亮移到第二条");
    assert_eq!(page.output_scroll, 40, "换任务后输出贴尾部（最新输出）");
}
