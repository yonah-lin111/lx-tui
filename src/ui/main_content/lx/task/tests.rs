//! 后台任务面板：只替换输入框容器，消息列表照旧，键位内嵌底边框。
//!
//! 断言口径来自用户要求：
//! 1. 不覆盖整个 pane——上方消息列表与普通聊天页**逐行一致**；
//! 2. 面板占输入框那一块，高度随条目数（封顶 `LIST_MAX`）；
//! 3. 底边框只显示 hints，**不出现**模型名 / 用量 / 进程计数；
//! 4. `↑` 在顶部或空列表时回到输入框。

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

/// 整块渲染并返回文本行（**含**底边框那一行）。
///
/// 底边框画在 `area.bottom()`，是内容区之外的一行（`status::render` 与面板 hints
/// 都在那里落笔），所以缓冲区要多留一行才看得到它。返回的行数 = `height + 1`。
fn render_lines(lx: &Lx, width: u16, height: u16) -> Vec<String> {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height + 1));
    super::super::render(area, &mut buf, lx, true, None);
    (0..=height)
        .map(|y| {
            (0..width)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// 带两条消息的 `lx`（面板关闭态）。
fn lx_with_messages() -> Lx {
    let mut lx = Lx::default();
    lx.chat.messages = vec![
        text_message("m1", "user", "你是谁"),
        text_message("m2", "assistant", "我是 Claude，一个 AI 助手。"),
    ];
    lx.chat.follow_bottom = true;
    lx
}

#[test]
fn panel_replaces_only_the_input_box() {
    // 用户要求：不要覆盖整个 pane。面板打开与关闭时，**上方消息行必须完全一致**。
    let mut lx = lx_with_messages();
    let before = render_lines(&lx, 70, 16);
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w/web")], 0));
    let after = render_lines(&lx, 70, 16);

    // 找到面板 tab 行，其上方的**消息行**应逐字相同（输入框那几行本就该被替换）。
    let tab_row = after
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("面板 tab 行");
    assert!(tab_row > 0, "面板上方必须有消息列表：{after:#?}");
    // 关闭态里消息占 1..tab_row；比对这段（不含标题/分割线）。
    let before_msg: Vec<&String> = before[..tab_row]
        .iter()
        .filter(|line| !line.trim().is_empty() && !line.contains('─'))
        .collect();
    let after_msg: Vec<&String> = after[..tab_row]
        .iter()
        .filter(|line| !line.trim().is_empty() && !line.contains('─'))
        .collect();
    assert!(!before_msg.is_empty(), "对照态应有消息行：{before:#?}");
    assert_eq!(
        before_msg, after_msg,
        "面板打开不得改动消息行：\n关闭 {before:#?}\n打开 {after:#?}"
    );
    // 消息确实渲染出来了（不是空区）。
    let joined = after[..tab_row].join("\n");
    assert!(joined.contains('你'), "消息列表可见：{joined}");
}

#[test]
fn panel_sits_where_the_input_box_was() {
    // 面板占输入框矩形：面板**沉底**、紧贴底边框，且占满固定高度（不贴顶）。
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w/web")], 0));
    let lines = render_lines(&lx, 70, 16);
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    let entry_row = lines
        .iter()
        .position(|line| line.contains("npm run dev"))
        .expect("进程行");
    // `render_lines` 返回 `height + 1` 行：最后一行是窗格底边框（hints）。
    // 面板高度固定 = TAB_ROW + LIST_MAX，因此 tab 行应正好落在该起点上。
    assert_eq!(
        tab_row + usize::from(TAB_ROW + LIST_MAX as u16),
        lines.len() - 1,
        "面板占满固定高度并紧贴底边框：{lines:#?}"
    );
    // 条目落在面板内部。
    assert!(entry_row > tab_row, "条目在 tab 行之下");
    assert!(entry_row < lines.len() - 1, "条目在底边框之上");
}

#[test]
fn footer_shows_hints_and_hides_model_name() {
    // 用户要求：快捷键内嵌 pane 底边框，且不显示模型名等数据。
    let mut lx = lx_with_messages();
    lx.chat.status.model_name = Some("deepseek-v4.1".into());
    lx.background_tasks = 2;
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w/web")], 0));
    let lines = render_lines(&lx, 70, 16);
    let bottom = lines.last().expect("底边框行");
    assert!(
        bottom.contains(text::LX_TASK_FOOTER_PROCESSES),
        "底边框是键位提示：{bottom}"
    );
    assert!(
        !bottom.contains("deepseek"),
        "面板打开时不显示模型名：{bottom}"
    );
    assert!(
        !bottom.contains(text::LX_TASK_COUNT_NOUN_MANY),
        "面板打开时不显示进程计数：{bottom}"
    );
    // 该行是窗格底边框：hints 从内侧起，行首仍是边线或其内侧空白
    // （角字符由窗格边框负责，渲染层不画）。
    assert!(
        bottom
            .trim_start()
            .starts_with(text::LX_TASK_FOOTER_PROCESSES),
        "hints 从左边框内侧起：{bottom}"
    );
}

#[test]
fn model_name_returns_when_the_panel_closes() {
    // `status::render` 与面板 hints 都写在 `area.bottom()` 那一行（内容区之外的
    // 窗格底边框），所以缓冲区要比内容区多留一行才看得到。
    let mut lx = lx_with_messages();
    lx.chat.status.model_name = Some("deepseek-v4.1".into());
    let border = render_lines(&lx, 70, 16).last().cloned().expect("底边框");
    assert!(
        border.contains("deepseek"),
        "面板关闭后模型名回来：{border}"
    );
}

#[test]
fn panel_height_is_fixed_regardless_of_entry_count() {
    // 用户要求：面板高度**固定**。0 条、2 条、9 条都必须一样高，
    // 否则打开面板时上方消息列表会被挤动一下再弹回来。
    let expected = TAB_ROW + LIST_MAX as u16;
    let mut page = page_with(Vec::new(), 0);
    assert_eq!(height(&page), expected, "空列表也是固定高度");

    for count in [1usize, 2, 5, 9] {
        let entries: Vec<ProcessEntry> = (1..=count)
            .map(|i| entry(i as u64, &format!("cmd-{i}"), "/w"))
            .collect();
        page = page_with(entries, 0);
        assert_eq!(height(&page), expected, "{count} 条时高度不变");
    }
    // 切 tab 也不改变高度（Agents 恒空）。
    page.tab = TaskTab::Agents;
    assert_eq!(height(&page), expected, "Agents tab 高度相同");
}

#[test]
fn empty_state_fills_the_fixed_list_area() {
    // 固定高度下，空态提示落在列表区首行，面板整体仍占满固定行数。
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(Vec::new(), 0));
    let lines = render_lines(&lx, 70, 20);
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    // 面板共 TAB_ROW + LIST_MAX 行，末行紧贴底边框上一行。
    assert_eq!(
        tab_row + usize::from(TAB_ROW + LIST_MAX as u16),
        lines.len() - 1,
        "面板占满固定行数：{lines:#?}"
    );
}

#[test]
fn process_rows_show_command_cwd_and_running_age() {
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(
        vec![entry(1, "npm run dev", "/workspace/web")],
        90_000, // 启动于 0，已运行 1m 30s
    ));
    let text = render_lines(&lx, 70, 16).join("\n");
    assert!(text.contains("npm run dev"), "命令：{text}");
    assert!(text.contains("/workspace/web"), "cwd：{text}");
    assert!(text.contains("running 1m 30s"), "运行时长：{text}");
}

#[test]
fn selected_row_carries_the_marker() {
    let mut lx = lx_with_messages();
    let mut page = page_with(
        vec![entry(1, "first", "/a"), entry(2, "second", "/b")],
        1_000,
    );
    page.window.active = 1;
    lx.task = Some(page);
    let lines = render_lines(&lx, 60, 16);
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
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(Vec::new(), 0));
    let text = render_lines(&lx, 60, 16).join("\n");
    assert!(text.contains(text::LX_TASK_PROCESSES_EMPTY), "空态：{text}");
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
        text.contains(text::LX_TASK_FOOTER_AGENTS),
        "Agents tab 的键位：{text}"
    );
}

#[test]
fn tab_row_shows_esc_on_the_right() {
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w")], 0));
    let lines = render_lines(&lx, 70, 16);
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
fn panel_draws_no_frame_of_its_own() {
    // 页内不画圆角框：窗格边框就是唯一一层框。
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w")], 0));
    let text = render_lines(&lx, 70, 16).join("\n");
    for corner in ['╭', '╮', '╰', '╯'] {
        assert!(
            !text.contains(corner),
            "面板不得画框（`{corner}` 属于窗格边框）：{text}"
        );
    }
}

#[test]
fn narrow_pane_drops_cwd_before_truncating_command() {
    // 窄窗格里先牺牲 cwd，命令尽量保留（命令才是用户识别进程的依据）。
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(
        vec![entry(1, "npm run dev", "/very/long/workspace/path")],
        0,
    ));
    let text = render_lines(&lx, 30, 16).join("\n");
    assert!(text.contains("npm run"), "命令优先保留：{text}");
    assert!(!text.contains("/very/long"), "cwd 先被牺牲：{text}");
}

#[test]
fn tiny_area_draws_nothing_instead_of_panicking() {
    // 窗格被压到极小：不越界、不 panic。
    for (width, height) in [(0u16, 0u16), (1, 1), (3, 2)] {
        let mut lx = Lx::default();
        lx.task = Some(page_with(vec![entry(1, "cmd", "/w")], 0));
        let _ = render_lines(&lx, width, height);
    }
}

#[test]
fn window_scrolls_to_keep_the_highlight_visible() {
    // 条目多于可视行数时，高亮滚到哪，窗口跟到哪。
    let entries: Vec<ProcessEntry> = (1..=10)
        .map(|index| entry(index, &format!("cmd-{index}"), "/w"))
        .collect();
    let mut lx = lx_with_messages();
    let mut page = page_with(entries, 0);
    page.window.active = 9;
    lx.task = Some(page);
    let text = render_lines(&lx, 50, 20).join("\n");
    assert!(text.contains("cmd-10"), "末项可见：{text}");
    assert!(!text.contains("cmd-1 "), "窗口已滚离首项：{text}");
}
