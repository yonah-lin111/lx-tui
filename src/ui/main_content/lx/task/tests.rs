//! 后台任务面板：接管历史区、输入框原位，键位内嵌底边框。
//!
//! 断言口径来自用户要求：
//! 1. 面板占用**原消息列表矩形**（`[open]` 打开后在这里看任务与输出）；
//! 2. 输入框仍在原位，退出面板后消息列表原样恢复；
//! 3. 底边框只显示 hints，**不出现**模型名 / 用量 / 进程计数；
//! 4. `↑` 在顶部或空列表时回到输入框。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

mod history_takeover;

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
        kind: "chat".into(),
        shell: None,
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
fn panel_replaces_messages_with_output_and_keeps_the_input_slot() {
    // 两段式：输出容器吃历史区（消息正文消失），item 列表接替输入框位置。
    let mut lx = lx_with_messages();
    let before = render_lines(&lx, 70, 16);
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w/web")], 0));
    let after = render_lines(&lx, 70, 16);

    // 历史区被输出容器接管：消息正文不再出现。
    let joined = after.join("\n");
    assert!(!joined.contains('你'), "历史区已被输出容器接管：{joined}");
    // 输出容器（无边框）从内容区顶部开始。
    assert!(
        after[0].contains(text::LX_TASK_OUTPUT_EMPTY),
        "输出容器贴内容区顶部：{after:#?}"
    );
    // item 列表在窗格下半部，顶边框 + tab 行 + 固定列表行。
    let tab_row = after
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("面板 tab 行");
    assert_eq!(
        tab_row,
        after.len() - 1 - usize::from(TAB_ROW + LIST_MAX as u16),
        "item 列表占满固定高度并紧贴底边框：{after:#?}"
    );
    // 关闭态里输入框所在的那一块，打开后由 item 列表占用（行数一致，不挤动布局）。
    let input_row = before
        .iter()
        .rposition(|line| {
            line.trim_start()
                .starts_with(text::LX_INPUT_PROMPT.trim_end())
        })
        .expect("关闭态应有输入框");
    assert!(
        tab_row <= input_row,
        "item 列表不高于原输入框位置：tab {tab_row} / 输入框 {input_row}"
    );
}

#[test]
fn item_list_sits_at_the_input_box_position() {
    // 用户要求：输出容器占消息列表位置，**item 列表仍在输入框位置**（窗格下半部）。
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w/web")], 0));
    let lines = render_lines(&lx, 70, 16);
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    assert!(
        tab_row > lines.len() / 2,
        "item 列表在窗格下半部（原输入框位置）：{lines:#?}"
    );
    let entry_row = lines
        .iter()
        .skip(tab_row)
        .position(|line| line.contains("npm run dev"))
        .map(|offset| tab_row + offset)
        .expect("进程行");
    assert!(entry_row > tab_row, "条目在 tab 行之下");
    // 输出容器（无边框）在上半部，占原消息列表位置。
    assert!(
        lines[0].contains(text::LX_TASK_OUTPUT_EMPTY),
        "输出容器贴内容区顶部：{lines:#?}"
    );
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
    // 每一项都是 `[功能 键]`；首项 `[select ↑↓]`。
    assert!(
        bottom.contains(&text::LX_TASK_HINTS_PROCESSES[0].label()),
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
            .starts_with(&text::LX_TASK_HINTS_PROCESSES[0].label()),
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
fn list_height_is_fixed_regardless_of_entry_count() {
    // item 列表占原输入框那一段，高度**固定**：0 条和 9 条一样高，
    // 开关面板时上方输出容器不会被挤动一下。
    let expected = TOP_BORDER + TAB_ROW + LIST_MAX as u16;
    assert_eq!(list_height(), expected, "列表高度固定");
}

#[test]
fn output_container_has_no_chrome_rows() {
    // 用户要求移除多余横线：输出容器**没有**顶边栏与分隔行，整块高度都给日志。
    for height in [1u16, 2, 3, 10, 30] {
        let rect = Rect::new(0, 0, 70, height);
        assert_eq!(
            output_rows(rect),
            usize::from(height),
            "高 {height} 的输出容器高度全部可用"
        );
    }
}

#[test]
fn output_content_reserves_the_scrollbar_column_always() {
    // 常驻预留最右 1 列：滚动条出现/消失不改变日志宽度（与 prompt 编辑器同规则）。
    assert_eq!(output_content_width(Rect::new(0, 0, 70, 10)), 69);
    assert_eq!(output_content_width(Rect::new(0, 0, 2, 10)), 1);
    assert_eq!(
        output_content_width(Rect::new(0, 0, 1, 10)),
        1,
        "宽度不足 2 列时无处预留"
    );
}

#[test]
fn empty_state_sits_at_the_top_of_the_list_area() {
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(Vec::new(), 0));
    let lines = render_lines(&lx, 70, 20);
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    assert!(
        lines
            .get(tab_row + 1)
            .is_some_and(|line| line.contains(text::LX_TASK_PROCESSES_EMPTY)),
        "空态紧跟 tab 行：{lines:#?}"
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
    // 命令名也会出现在输出容器顶边栏（`Output <命令>`），所以只在 **item 列表区**
    // （tab 行之下）找任务行，避免命中顶边栏。
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    let lookup = |needle: &str| {
        lines[tab_row..]
            .iter()
            .find(|line| line.contains(needle))
            .expect("任务行")
    };
    let second = lookup("second");
    let first = lookup("first");
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
        text.contains(&text::LX_TASK_HINTS_AGENTS[0].label()),
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
        tab_row.contains(&text::LX_TASK_CLOSE.label()),
        "tab 行右端是关闭按钮：{tab_row}"
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

// ===== 顶部边框、tab 配色与固定高度 =====

#[test]
fn panel_draws_a_top_border_above_the_tab_row() {
    // 面板顶边框把面板与上方消息列表分开；它画在面板矩形**内部**，不是第二个窗格框。
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w")], 0));
    let lines = render_lines(&lx, 70, 20);
    let tab_row = lines
        .iter()
        .position(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    let border = &lines[tab_row - 1];
    assert!(
        border.trim().chars().all(|ch| ch == '─'),
        "tab 行上方应是一条纯横线：{border:?}"
    );
}

#[test]
fn active_tab_uses_the_header_tab_bar_style() {
    // 用户要求：右上角 tab 用**头部 tab 栏的背景颜色** = `style::selected_item()`
    // （青底黑字 + BOLD），与顶部标签栏激活态完全一致。
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w")], 0));
    let area = Rect::new(0, 0, 70, 20);
    let mut buf = Buffer::empty(Rect::new(0, 0, 70, 21));
    super::super::render(area, &mut buf, &lx, true, None);
    let mut found = None;
    for y in 0..21u16 {
        for x in 0..70u16 {
            if buf[(x, y)].symbol() == "P" {
                found = Some((x, y));
            }
        }
    }
    let (x, y) = found.expect("Processes 标签");
    let style = buf[(x, y)].style();
    assert_eq!(
        style.bg,
        Some(ratatui::style::Color::Cyan),
        "当前 tab 用头部标签栏的青底"
    );
    assert_eq!(
        style.fg,
        Some(ratatui::style::Color::Black),
        "青底配黑字，与顶部标签栏一致"
    );
    assert!(
        style.add_modifier.contains(ratatui::style::Modifier::BOLD),
        "与头部标签栏一样加粗"
    );
}

#[test]
fn panel_is_taller_than_five_list_rows() {
    // 用户要求"高度再高一点"：列表区已从 5 行提到 6 行。
    const { assert!(LIST_MAX > 5) };
}

// ===== 键位提示：`[功能 键]` 语法 + tab/esc 可点击 =====

#[test]
fn hints_use_action_then_key_inside_brackets() {
    // 用户要求：`[功能名称 快捷键]` —— 功能在前、键在后，方括号包整条。
    assert_eq!(text::hint("select", "↑↓").label(), "[select ↑↓]");
    assert_eq!(text::LX_TASK_CLOSE.label(), "[close esc]");
    let labels: Vec<String> = text::LX_TASK_HINTS_PROCESSES
        .iter()
        .map(|hint| hint.label())
        .collect();
    assert_eq!(
        labels,
        vec![
            "[select ↑↓]".to_string(),
            "[stop ^d]".to_string(),
            "[tabs ←→]".to_string(),
            "[back esc]".to_string(),
        ],
        "进程 tab 的四项提示"
    );
}

#[test]
fn close_button_uses_the_same_action_key_syntax() {
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w")], 0));
    let lines = render_lines(&lx, 70, 20);
    let tab_row = lines
        .iter()
        .find(|line| line.contains(TaskTab::Processes.label()))
        .expect("tab 行");
    assert!(
        tab_row.contains("[close esc]"),
        "右上角是 [close esc]：{tab_row}"
    );
}

#[test]
fn footer_renders_every_hint_as_action_key() {
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w")], 0));
    let bottom = render_lines(&lx, 70, 20).last().cloned().expect("底边框");
    for hint in text::LX_TASK_HINTS_PROCESSES {
        assert!(
            bottom.contains(&hint.label()),
            "底边框缺少 `{}`：{bottom}",
            hint.label()
        );
    }
}

#[test]
fn narrow_pane_drops_whole_hints_instead_of_truncating() {
    // 窄窗格整项丢弃，绝不把 `[back esc]` 截成半截。
    let mut lx = lx_with_messages();
    lx.task = Some(page_with(vec![entry(1, "npm run dev", "/w")], 0));
    let bottom = render_lines(&lx, 34, 20).last().cloned().expect("底边框");
    let kept = text::LX_TASK_HINTS_PROCESSES
        .iter()
        .filter(|hint| bottom.contains(&hint.label()))
        .count();
    assert!(kept >= 1, "至少保留第一项：{bottom}");
    assert!(kept < 4, "窄窗格应丢掉后面的项：{bottom}");
    // 保留的每一项都完整（方括号闭合）。
    assert_eq!(
        bottom.matches('[').count(),
        bottom.matches(']').count(),
        "不得出现半个方括号：{bottom}"
    );
}

#[test]
fn clicking_a_tab_hits_it() {
    let rect = Rect::new(0, 10, 70, 8);
    // tab 行 = rect.y + TOP_BORDER。
    let tab_y = rect.y + TOP_BORDER;
    // 第一个 tab 标签首字符。
    assert_eq!(
        hit(rect, rect.x, tab_y),
        Some(TaskHit::Tab(0)),
        "点第一个 tab"
    );
    // `Agents` 标签首字符位置：跳过 "Processes" + 分隔符。
    let sep = u16::try_from(text::LX_TASK_TAB_SEP.width()).unwrap();
    let first_w = u16::try_from(TaskTab::Processes.label().width()).unwrap();
    let agents_x = rect.x + first_w + sep;
    assert_eq!(
        hit(rect, agents_x, tab_y),
        Some(TaskHit::Tab(1)),
        "点第二个 tab"
    );
}

#[test]
fn clicking_esc_hits_close() {
    let rect = Rect::new(0, 10, 70, 8);
    let tab_y = rect.y + TOP_BORDER;
    let esc_w = u16::try_from(text::LX_TASK_CLOSE.label().width()).unwrap();
    let esc_x = rect.right() - esc_w;
    assert_eq!(
        hit(rect, esc_x, tab_y),
        Some(TaskHit::Close),
        "点 [close esc] 关闭"
    );
}

#[test]
fn clicking_elsewhere_in_the_panel_misses() {
    let rect = Rect::new(0, 10, 70, 8);
    // 列表区 / 顶边框 / 面板之外都不该命中 tab 或 esc。
    assert_eq!(hit(rect, rect.x, rect.y), None, "顶边框不命中");
    assert_eq!(
        hit(rect, rect.x, rect.y + TOP_BORDER + 1),
        None,
        "列表区不命中"
    );
}
