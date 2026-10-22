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
        sequence: 0,
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
    // 输出容器从内容区最顶部开始（**没有**顶边栏，第一行就是占位/日志）。
    assert!(
        lines[0].contains(text::LX_TASK_OUTPUT_EMPTY),
        "输出容器贴内容区顶部且无边框：{lines:#?}"
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
        "item 列表里能看到高亮任务：{joined}"
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
fn focus_on_empty_list_keeps_the_request() {
    // 列表还没到（`[open]` 打开面板那一刻必然是空的）时**不能**消费请求：
    // 早先这里先 `take()` 再判空，请求被吃掉，高亮永远落在首项。
    let mut page = page_with(Vec::new());
    page.focus = Some(7);
    page.resolve_focus();
    assert!(page.selected().is_none(), "空列表下高亮无效");
    assert_eq!(page.focus, Some(7), "请求必须留到列表到达");

    // 列表到达后落地并清空。
    page.processes = vec![entry(11, "a"), entry(7, "b")];
    page.resolve_focus();
    assert_eq!(
        page.selected().map(|process| process.session_id),
        Some(7),
        "列表到达后定位到目标进程"
    );
    assert_eq!(page.focus, None, "定位是一次性的");
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

// ===== 输出容器：无边框 + 常驻滚动条槽 =====

#[test]
fn output_container_draws_no_border_lines() {
    // 用户圈红的两条整行横线（顶边栏 + 分隔行）必须消失：第一行直接是日志。
    let mut lx = lx_with_messages();
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    page.output = vec!["first log line".into(), "second log line".into()];
    lx.task = Some(page);
    let lines = render_lines(&lx, 70, 24);
    assert!(
        lines[0].contains("first log line"),
        "第一行就是日志正文，没有顶边栏：{lines:#?}"
    );
    // 输出区最后一行不能再是整行横线（它下面是 item 列表自己的顶边框）。
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    assert!(
        lines[tab_row - 1].contains('─'),
        "紧邻 tab 行上方是列表自己的顶边框（唯一的线）：{lines:#?}"
    );
    assert!(
        !lines[tab_row - 1].contains(text::LX_TASK_OUTPUT_TITLE),
        "那条线不再带 Output 标题：{lines:#?}"
    );
}

#[test]
fn output_scrollbar_appears_only_when_content_overflows() {
    let mut lx = lx_with_messages();
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    // 10 行输出、输出容器远高于它：没有滚动条。
    page.output = (0..10).map(|index| format!("line {index}")).collect();
    lx.task = Some(page);
    let rects = {
        let area = Rect::new(0, 0, 70, 40);
        crate::ui::main_content::lx::task_rects(area, &lx).expect("面板已打开")
    };
    assert!(
        output_scrollbar(rects.output, lx.task.as_ref().expect("page")).is_none(),
        "内容不溢出时没有滚动条"
    );

    // 200 行输出：必须出现滚动条，且贴最右一列、只画 thumb。
    if let Some(page) = lx.task.as_mut() {
        page.output = (0..200).map(|index| format!("line {index}")).collect();
        page.output_visible = usize::from(rects.output.height);
        page.pin_output_to_tail();
    }
    let bar = output_scrollbar(rects.output, lx.task.as_ref().expect("page"))
        .expect("溢出时必须有滚动条");
    assert_eq!(bar.track.x, rects.output.right() - 1, "贴最右一列");
    assert_eq!(
        bar.track.height, rects.output.height,
        "轨道覆盖输出容器整高"
    );

    let lines = render_lines(&lx, 70, 40);
    let thumb_rows: Vec<&String> = lines.iter().filter(|line| line.contains('▐')).collect();
    assert!(!thumb_rows.is_empty(), "滚动条要画出来：{lines:#?}");
    assert!(
        lines.iter().all(|line| !line.contains('▕')),
        "轨道不画竖线（只画 thumb）：{lines:#?}"
    );
}

#[test]
fn wheel_over_output_container_scrolls_it() {
    // 输出容器滚轮：翻输出视口，不穿透到下方 item 列表。
    let mut lx = lx_with_messages();
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    page.output = (0..200).map(|index| format!("line {index}")).collect();
    lx.task = Some(page);
    let area = Rect::new(0, 0, 70, 30);
    let rects = crate::ui::main_content::lx::task_rects(area, &lx).expect("面板已打开");
    assert!(
        rects.output.height > 3,
        "输出容器够高，可滚动：{:?}",
        rects.output
    );
}

// ===== bug 回归：输出刷新不得把用户拽回底部 =====

#[test]
fn refreshing_output_keeps_a_manually_scrolled_position() {
    // 真实 bug：面板每轮都重新拉输出，原来每次都贴尾 → 用户往上翻立刻被拽回底部。
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    page.output = (0..100).map(|index| format!("line {index}")).collect();
    page.output_visible = 10;
    page.pin_output_to_tail();
    assert_eq!(page.output_scroll, 90, "初始贴尾");

    // 用户主动上翻到 20。
    assert!(page.scroll_output(-70));
    assert_eq!(page.output_scroll, 20);

    // 后台又追加了几行（事件层回投新快照）：位置必须保持不变。
    let next: Vec<String> = (0..105).map(|index| format!("line {index}")).collect();
    assert!(page.set_output(next));
    assert_eq!(page.output_scroll, 20, "刷新输出不得改变用户手动上翻的位置");
}

#[test]
fn refreshing_output_still_follows_when_pinned_at_tail() {
    // 反过来：本来就在底部跟随最新时，新日志要照常推进视口。
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    page.output = (0..100).map(|index| format!("line {index}")).collect();
    page.output_visible = 10;
    page.pin_output_to_tail();
    let next: Vec<String> = (0..120).map(|index| format!("line {index}")).collect();
    assert!(page.set_output(next));
    assert_eq!(page.output_scroll, 110, "贴尾状态继续跟随最新输出");
}

#[test]
fn shrinking_output_clamps_but_keeps_the_read_position() {
    // 缓冲区被截断变短时只做越界钳制，不应跳回顶部或底部。
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    page.output = (0..100).map(|index| format!("line {index}")).collect();
    page.output_visible = 10;
    page.output_scroll = 50;
    let next: Vec<String> = (0..20).map(|index| format!("line {index}")).collect();
    page.set_output(next);
    assert_eq!(page.output_scroll, 10, "钳到新上限（20 − 10）");
}

// ===== bug 回归：面板里不得出现「↓ jump to bottom」 =====

#[test]
fn task_panel_hides_the_jump_to_bottom_bar() {
    // 真实 bug：面板打开时输出容器已接管历史区，聊天栏却还画着
    // `↓ jump to bottom`——那是消息列表的控件，此时点了只会滚动看不见的历史。
    let mut lx = lx_with_messages();
    // 构造"上滚未跟随底部"状态：平时正是它让聊天栏出现。
    lx.chat.follow_bottom = false;
    lx.chat.history_scroll = 0;
    let closed = render_lines(&lx, 70, 24).join("\n");
    assert!(
        closed.contains(text::LX_JUMP_BOTTOM_HINT),
        "前提：关闭态确实显示回到底部控件：{closed}"
    );

    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let opened = render_lines(&lx, 70, 24).join("\n");
    assert!(
        !opened.contains(text::LX_JUMP_BOTTOM_HINT),
        "面板打开时不得显示回到底部控件：{opened}"
    );

    // 关闭面板后恢复（历史区回来，控件也跟着回来）。
    lx.task = None;
    let restored = render_lines(&lx, 70, 24).join("\n");
    assert!(
        restored.contains(text::LX_JUMP_BOTTOM_HINT),
        "关闭面板后控件恢复：{restored}"
    );
}

#[test]
fn task_panel_hides_the_streaming_loading_bar() {
    // loading 行同理：它属于聊天历史区，面板打开时不显示。
    let mut lx = lx_with_messages();
    lx.chat.status.streaming = true;
    let closed = render_lines(&lx, 70, 24).join("\n");
    assert!(
        closed.contains(text::chat::LOADING_LABEL),
        "前提：流中确实显示 loading 行：{closed}"
    );

    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    let opened = render_lines(&lx, 70, 24).join("\n");
    assert!(
        !opened.contains(text::chat::LOADING_LABEL),
        "面板打开时不得显示聊天栏 loading：{opened}"
    );
}

// ===== review 回归：面板打开时历史相关命中必须一起失效 =====

#[test]
fn task_panel_kills_every_history_hit() {
    // 真实缺陷：面板打开后 `chat_hit` / 历史滚动条仍按**隐藏的**历史几何作答，
    // 于是输出区成了巨大的点击穿透面（点一下会跳看不见的历史、展开看不见的 thinking）。
    let mut lx = lx_with_messages();
    let area = Rect::new(0, 0, 70, 30);
    // 关闭态：这些入口在历史区域里是有命中的（先证明前提成立）。
    assert!(
        crate::ui::main_content::lx::history_scrollbar(area, &lx).is_some()
            || crate::ui::main_content::lx::history_wheel_region(area, &lx).is_some(),
        "前提：关闭态存在历史相关命中"
    );
    let history = crate::ui::main_content::lx::history_rect(area, &lx).expect("历史区");
    let probe = (history.x + 1, history.y + 1);

    lx.task = Some(page_with(vec![entry(1, "npm run dev")]));
    assert!(
        crate::ui::main_content::lx::chat_hit(area, &lx, probe.0, probe.1).is_none(),
        "面板打开时不得命中隐藏的历史行"
    );
    assert!(
        crate::ui::main_content::lx::history_scrollbar(area, &lx).is_none(),
        "面板打开时历史滚动条命中必须消失"
    );
    assert!(
        crate::ui::main_content::lx::history_wheel_region(area, &lx).is_none(),
        "面板打开时历史区滚轮不得接管输出区"
    );
    assert!(
        crate::ui::main_content::lx::sticky_prompt_jump(area, &lx).is_none(),
        "面板打开时置顶行跳转必须消失"
    );
}

#[test]
fn resizing_the_window_keeps_a_manually_scrolled_output_position() {
    // 真实缺陷：可见行数一变就无条件 `pin_output_to_tail()`，用户改窗口大小
    // （哪怕只多/少一行）就被从日志中间甩到最新一屏。
    let mut page = page_with(vec![entry(1, "npm run dev")]);
    page.output = (0..100).map(|index| format!("line {index}")).collect();
    page.output_visible = 10;
    page.pin_output_to_tail();
    assert_eq!(page.output_scroll, 90);
    // 用户上翻到 30。
    page.scroll_output(-60);
    assert_eq!(page.output_scroll, 30);

    // 窗口变矮：可见行数 10 → 8（max 90 → 92），阅读位置必须保持。
    page.resize_output_viewport(8);
    assert_eq!(page.output_scroll, 30, "改窗口不得把读者拽回底部");

    // 反过来：原本贴尾时继续跟随最新。
    page.pin_output_to_tail();
    assert_eq!(page.output_scroll, 92);
    page.resize_output_viewport(6);
    assert_eq!(page.output_scroll, 94, "贴尾状态下继续跟随底部");
}
