//! 历史渲染与布局测试：用户底色/前缀、assistant 无标签、markdown 正文、折叠 thinking、
//! 换行、宽窄窗格与命中矩形。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::*;

/// 测试用缓存：每次调用独立，避免测试之间共享几何快照。
fn cache() -> PaneCache {
    PaneCache::default()
}
use crate::app::state::{AssistantStepView, ChatMessageView, ChatPartView, Lx, LxChat};
use crate::chat::events::PartKind;
use crate::ui::main_content::lx::{history_center_target, history_rect};

/// 工具步骤的几何一致性测试（布局/渲染行数、滚动条、置顶行、thinking 命中）。
mod tool_geometry;

// 按主题拆出的子模块（单文件行数上限）；构造器都在本模块，子模块用 `use super::*`。
mod cards;
mod compact;
mod rewind;
mod sticky;
mod transcript;

/// 滚动性能回归（`#[ignore]`）。
mod perf;

fn lx_with(messages: Vec<ChatMessageView>) -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages = messages;
    lx.chat = chat;
    lx
}

fn message(role: &str, status: &str, parts: Vec<ChatPartView>) -> ChatMessageView {
    ChatMessageView {
        sequence: 0,
        id: format!("{role}-1"),
        role: role.into(),
        status: status.into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: parts
            .into_iter()
            .enumerate()
            .map(|(index, part)| AssistantStepView::Part {
                position: index as i64,
                part,
            })
            .collect(),
        model_label: Some("P / M".into()),
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
    }
}

fn text_part(content: &str) -> ChatPartView {
    ChatPartView {
        kind: PartKind::Text,
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

/// 带起止时刻的 reasoning part（思考耗时展示用）。
fn timed_reasoning_part(content: &str, started_at_ms: i64, ended_at_ms: i64) -> ChatPartView {
    let mut part = reasoning_part(content);
    part.started_at_ms = Some(started_at_ms);
    part.ended_at_ms = Some(ended_at_ms);
    part
}

fn reasoning_part(content: &str) -> ChatPartView {
    ChatPartView {
        kind: PartKind::Reasoning,
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

/// 用一组内容块替换消息的步骤（保持位置连续；测试构造用）。
fn set_parts(message: &mut ChatMessageView, parts: Vec<ChatPartView>) {
    message.steps = parts
        .into_iter()
        .enumerate()
        .map(|(index, part)| AssistantStepView::Part {
            position: index as i64,
            part,
        })
        .collect();
}

#[test]
fn thinking_collapsed_after_completion_one_row_each() {
    let lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![reasoning_part("长思考\n两行"), text_part("正文")],
    )]);
    let area = Rect::new(0, 0, 40, 20);
    let layout = layout(area, &lx, &mut cache());
    // 完成态默认折叠：折叠头 1 + 块间空行 1 + 正文 1 + 消息间空行 1 = 4
    //（无用量数据不出统计行）。
    assert_eq!(layout.total_rows, 4);
    assert_eq!(layout.thinking.len(), 1);
    assert_eq!(layout.thinking[0].0, 0);
    assert_eq!(layout.thinking[0].1, 0);
}

#[test]
fn thinking_collapsed_by_default_while_streaming() {
    // 默认折叠：生成中也不自动展开（对齐 opencode 的 hide 模式，单击才展开）。
    let lx = lx_with(vec![message(
        "assistant",
        "streaming",
        vec![reasoning_part("l1\nl2"), text_part("")],
    )]);
    let area = Rect::new(0, 0, 40, 20);
    let layout = layout(area, &lx, &mut cache());
    assert!(!lx.chat.reasoning_expanded(0, 0), "生成中保持折叠");
    assert!(lx.chat.reasoning_streaming(0, 0), "仍在生成");
    // 折叠头 1 + 消息间空行 1 = 2（空的正文块不占行、也不引入空行）。
    assert_eq!(layout.total_rows, 2);
}

#[test]
fn expanded_thinking_survives_completion() {
    // 用户展开后不因生成结束被强制收起（状态完全由用户控制）。
    let mut lx = lx_with(vec![message(
        "assistant",
        "streaming",
        vec![reasoning_part("思考内容"), text_part("")],
    )]);
    assert!(!lx.chat.reasoning_expanded(0, 0), "默认折叠");
    lx.chat.toggle_reasoning(0, 0);
    assert!(lx.chat.reasoning_expanded(0, 0), "显式展开");
    lx.chat.messages[0].status = "complete".into();
    assert!(lx.chat.reasoning_expanded(0, 0), "完成后仍保持展开");
    lx.chat.toggle_reasoning(0, 0);
    assert!(!lx.chat.reasoning_expanded(0, 0), "再点收起");
}

#[test]
fn expanded_thinking_counts_content_rows() {
    let mut lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![reasoning_part("l1\nl2\nl3"), text_part("t")],
    )]);
    lx.chat.toggle_reasoning(0, 0);
    let area = Rect::new(0, 0, 40, 20);
    let layout = layout(area, &lx, &mut cache());
    // 展开头 1 + 3 内容行 + 块间空行 1 + 正文 1 + 消息间空行 1 = 7。
    assert_eq!(layout.total_rows, 7);
}

#[test]
fn assistant_message_has_no_label_and_shows_body() {
    let lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![reasoning_part("秘密思考"), text_part("可见正文")],
    )]);
    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let line0 = row_text(&buf, area, 0);
    let line1 = row_text(&buf, area, 1);
    let line2 = row_text(&buf, area, 2);
    assert!(
        line0.starts_with("● Thought"),
        "首个渲染行为圆点 + 图标 + 折叠头：{line0}"
    );
    assert!(
        !line0.contains("assistant"),
        "不再显示 assistant 标签：{line0}"
    );
    assert!(line1.trim().is_empty(), "折叠头与正文间空一行：{line1}");
    assert!(!line2.contains("秘密思考"), "折叠时不显示内容");
    assert!(
        line2.starts_with("● 可见正文"),
        "正文块同样以圆点起头：{line2}"
    );
}

#[test]
fn streaming_message_has_no_list_marker() {
    // 流中不再在消息列表插入标记（改用输入框上方 loading 行）。
    let lx = lx_with(vec![message(
        "assistant",
        "streaming",
        vec![text_part("部分")],
    )]);
    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let joined = (0..3)
        .map(|row| row_text(&buf, area, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("[streaming]"), "流中不显示标记：{joined}");
    assert!(joined.contains("部分"), "正文仍绘制：{joined}");
}

#[test]
fn failed_message_shows_error_notice_with_code_at_bottom() {
    let mut message = message("assistant", "failed", vec![text_part("出错了")]);
    message.error_code = Some("http 503".into());
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    // 正文 → 空行（块间）→ `■ Failed` → `└─ 失败码`（旧 `[failed: …]` 标记不再出现在顶部）。
    let rows: Vec<String> = (0..4).map(|row| row_text(&buf, area, row)).collect();
    assert!(rows[0].contains("出错了"), "正文：{rows:?}");
    assert_eq!(rows[1], "", "与正文隔一行");
    assert_eq!(
        rows[2], "● Failed",
        "行首为圆点（文字类通知与步骤标记同款）"
    );
    assert_eq!(rows[3], "  └─ http 503");
    assert!(
        !rows.iter().any(|row| row.contains("[failed")),
        "不再有方括号标记：{rows:?}"
    );
}

#[test]
fn failed_message_without_code_uses_fallback_reason() {
    let lx = lx_with(vec![message(
        "assistant",
        "failed",
        vec![text_part("挂了")],
    )]);
    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rows: Vec<String> = (0..4).map(|row| row_text(&buf, area, row)).collect();
    assert_eq!(rows[2], "● Failed");
    assert_eq!(
        rows[3], "  └─ no error details provided",
        "无码时给一句话兜底"
    );
}

#[test]
fn interrupted_message_shows_warning_notice_above_stats_line() {
    let mut message = message("assistant", "interrupted", vec![text_part("半截回答")]);
    message.input_tokens = Some(1_200);
    message.output_tokens = Some(300);
    message.duration_ms = Some(2_700);
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rows: Vec<String> = (0..6).map(|row| row_text(&buf, area, row)).collect();
    assert!(rows[0].contains("半截回答"), "正文：{rows:?}");
    assert_eq!(rows[1], "", "通知与正文隔一行");
    assert_eq!(rows[2], "● Interrupted", "中断用黄色警告通知 + 圆点标记");
    assert_eq!(
        rows[3], "  └─ interrupted before completion",
        "固定一句话原因"
    );
    assert_eq!(rows[4], "", "通知与统计行之间空一行");
    assert!(
        rows[5].starts_with("P / M · 1.2k in · 300 out"),
        "统计行：{rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("[interrupted]")),
        "旧标记已移除：{rows:?}"
    );
    // 端到端：标题整行黄 + 粗（不再有灰色档）。
    let title = buf.cell((0, 2)).map(|cell| cell.style()).expect("标题格");
    assert_eq!(title.fg, style::agent_warn().fg, "中断通知标题用警告黄");
    assert!(
        title.add_modifier.contains(ratatui::style::Modifier::BOLD),
        "标题加粗"
    );
}

#[test]
fn user_message_has_prefix_and_background() {
    let lx = lx_with(vec![message("user", "complete", vec![text_part("你好")])]);
    let area = Rect::new(0, 0, 20, 4);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let line0 = row_text(&buf, area, 0);
    assert!(line0.starts_with("> "), "用户消息前缀：{line0}");
    assert!(line0.contains("你好"));
    // 整行铺用户底色（内容列末列带背景；最后一列为滚动条预留列）。
    let cell = buf.cell((18, 0)).expect("cell");
    assert_eq!(cell.style().bg, style::chat_user_bg().bg, "整块底色");
}

#[test]
fn assistant_markdown_collapses_markers() {
    let lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![text_part("## 标题\n\n- 项 **粗**")],
    )]);
    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let joined = (0..6)
        .map(|row| row_text(&buf, area, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("标题"), "标题正文：{joined}");
    assert!(!joined.contains("##"), "隐藏标题标记：{joined}");
    assert!(joined.contains("• 项 粗"), "列表 + 粗体折叠：{joined}");
    assert!(!joined.contains("**"), "隐藏粗体标记：{joined}");
}

#[test]
fn render_expanded_shows_reasoning_content() {
    let mut lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![reasoning_part("展开后的思考"), text_part("正文")],
    )]);
    lx.chat.toggle_reasoning(0, 0);
    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let joined = (0..4)
        .map(|row| row_text(&buf, area, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("展开后的思考"), "展开后显示内容：{joined}");
}

#[test]
fn wrap_counts_wide_characters_as_two_columns() {
    // 20 列宽、中文每字 2 列 → 每行最多 10 字。
    let lines = wrap("一二三四五六七八九十一二", 20);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].chars().count(), 10);
    assert_eq!(lines[1], "一二");
}

#[test]
fn wrap_preserves_explicit_newlines() {
    let lines = wrap("a\nb\n", 10);
    assert_eq!(lines, vec!["a", "b", ""]);
}

#[test]
fn layout_handles_zero_area() {
    let lx = lx_with(vec![message("user", "complete", vec![text_part("x")])]);
    let layout = layout(Rect::new(0, 0, 0, 0), &lx, &mut cache());
    assert_eq!(layout.total_rows, 0);
    assert!(layout.thinking.is_empty());
}

#[test]
fn thinking_header_rect_tracks_row_position() {
    let lx = lx_with(vec![
        message("user", "complete", vec![text_part("问题")]),
        message("assistant", "complete", vec![reasoning_part("思考")]),
    ]);
    let area = Rect::new(3, 5, 40, 20);
    let layout = layout(area, &lx, &mut cache());
    // 第一条：正文(5，用户前缀在首行) + 空行(6)；第二条 thinking 头(7)。
    assert_eq!(layout.thinking.len(), 1);
    let (_, _, header) = layout.thinking[0];
    assert_eq!(header.y, 7);
    assert_eq!(header.x, 3);
    assert_eq!(header.width, 40 - super::SCROLLBAR_COLUMNS);
}

/// 读取缓冲区某行文本（跳过宽字符占位格）。
fn row_text(buf: &Buffer, area: Rect, row: u16) -> String {
    let mut out = String::new();
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell((x, area.y + row)) {
            if cell.diff_option == ratatui::buffer::CellDiffOption::Skip {
                continue;
            }
            let symbol = cell.symbol();
            if symbol == "\0" {
                continue;
            }
            out.push_str(symbol);
        }
    }
    out.trim_end().to_string()
}

#[test]
fn narrow_width_table_is_not_deformed() {
    let src = "| 序号 | 名称 | 类别 | 特点 |\n| --- | --- | --- | --- |\n| 1 | DeepSeek | AI助手 | 免费、支持长上下文 |\n| 2 | Python | 编程语言 | 简洁、生态丰富 |";
    let lx = lx_with(vec![message("assistant", "complete", vec![text_part(src)])]);
    for width in [30u16, 40, 50] {
        let area = Rect::new(0, 0, width, 20);
        let mut buf = Buffer::empty(area);
        crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
        // 每行不超过内容宽度（扣除滚动条预留列），且表格边界字符不会跨行断裂。
        let content_width = width - SCROLLBAR_COLUMNS;
        for row in 0..area.height {
            let line = row_text(&buf, area, row);
            let drawn: usize = line
                .chars()
                .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1))
                .sum();
            assert!(
                drawn <= usize::from(content_width),
                "宽度 {width} 第 {row} 行超宽：{line}"
            );
        }
        // 右下角边界与左上角边界都在（方框未被折断）。
        let joined = (0..area.height)
            .map(|row| row_text(&buf, area, row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains('┌'), "宽度 {width} 缺左上边界");
        assert!(joined.contains('┘'), "宽度 {width} 缺右下边界");
    }
}

#[test]
fn history_layout_reserves_scrollbar_column() {
    let lx = lx_with(vec![message(
        "user",
        "complete",
        vec![text_part("你好世界你好世界")],
    )]);
    let area = Rect::new(0, 0, 20, 10);
    let _layout = layout(area, &lx, &mut cache());
    // thinking 头宽度应扣除滚动条预留列。
    assert_eq!(super::content_rect(area).width, 20 - SCROLLBAR_COLUMNS);
}

#[test]
fn completed_assistant_shows_turn_stats_line() {
    let mut completed = message("assistant", "complete", vec![text_part("正文")]);
    completed.model_label = Some("deepseek-v4.1-flash".into());
    completed.input_tokens = Some(2_916);
    completed.output_tokens = Some(307);
    completed.cache_read_tokens = Some(2_048);
    completed.duration_ms = Some(2_655);
    let line = stats_line(&completed).expect("统计行");
    assert!(line.starts_with("deepseek-v4.1-flash"), "{line}");
    assert!(line.contains("2.9k in"), "{line}");
    assert!(line.contains("307 out"), "{line}");
    assert!(
        line.contains("70% cached"),
        "缓存命中 = cached/prompt：{line}"
    );
    assert!(line.contains("115.6 tok/s"), "tok/s = 输出 / 耗时：{line}");
    assert!(line.ends_with("2.7s"), "总耗时：{line}");
}

#[test]
fn stats_line_row_is_counted_in_layout() {
    let mut completed = message("assistant", "complete", vec![text_part("答")]);
    completed.input_tokens = Some(100);
    completed.output_tokens = Some(20);
    completed.duration_ms = Some(1_000);
    let lx = lx_with(vec![completed]);
    // 统计行单行放得下（60 列）。
    let area = Rect::new(0, 0, 60, 20);
    let layout = layout(area, &lx, &mut cache());
    // 正文 1 + 块间空行 1 + 统计行 1 + 消息间空行 1 = 4。
    assert_eq!(layout.total_rows, 4);
}

/// 后面还有可见报告批次时，这轮只是中间态：统计行留给报告回答。
#[test]
fn stats_line_is_hidden_before_a_visible_report_batch() {
    let mut assistant = message("assistant", "complete", vec![text_part("等待返回")]);
    assistant.input_tokens = Some(47_000);
    assistant.output_tokens = Some(2_300);
    assistant.duration_ms = Some(252_000);
    let mut report_row = message("user", "complete", Vec::new());
    report_row.turn_index = assistant.turn_index;
    report_row.reports = vec![report("sub-1", "host flow", "completed")];
    let lx = lx_with(vec![assistant, report_row]);
    // 正文 1 + 消息间空行 1 + 汇报块 4 + 呼吸位 1 = 7；统计行被隐藏。
    let area = Rect::new(0, 0, 80, 20);
    let mut cache = cache();
    assert_eq!(layout(area, &lx, &mut cache).total_rows, 7);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rendered: String = (0..7).map(|row| row_text(&buf, area, row)).collect();
    assert!(
        !rendered.contains("47k in"),
        "报告链路中间轮不能先出统计行：{rendered:?}"
    );
}

/// 汇报之后真正收尾的 assistant 仍然显示统计行。
#[test]
fn stats_line_returns_after_the_last_report() {
    let mut report_row = message("user", "complete", Vec::new());
    report_row.reports = vec![crate::chat::agent::SubagentReportView {
        agent_id: "sub-1".into(),
        title: "host flow".into(),
        role: crate::chat::agent::AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 0,
        duration_ms: Some(1_000),
        body: "done".into(),
        delivered_at_ms: 1,
    }];
    let mut assistant = message("assistant", "complete", vec![text_part("总结")]);
    assistant.input_tokens = Some(100);
    assistant.output_tokens = Some(20);
    assistant.duration_ms = Some(1_000);
    let lx = lx_with(vec![report_row, assistant]);
    // 汇报块 4 + 消息间空行 1 + 正文 1 + 块间空行 1 + 统计 1 + 末尾呼吸位 1 = 9。
    let area = Rect::new(0, 0, 80, 20);
    let mut cache = cache();
    assert_eq!(layout(area, &lx, &mut cache).total_rows, 9);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rendered: String = (0..9).map(|row| row_text(&buf, area, row)).collect();
    assert!(
        !rendered.contains("Agent completed"),
        "报告后已有 assistant 收尾时不再补结束行：{rendered:?}"
    );
}

fn report(agent_id: &str, title: &str, status: &str) -> crate::chat::agent::SubagentReportView {
    crate::chat::agent::SubagentReportView {
        agent_id: agent_id.into(),
        title: title.into(),
        role: crate::chat::agent::AgentRole::Explorer,
        status: status.into(),
        files_changed: 0,
        duration_ms: Some(1_000),
        body: "done".into(),
        delivered_at_ms: 1,
    }
}

#[test]
fn report_batch_cards_render_before_the_assistant_answer() {
    let mut reports = message("user", "complete", Vec::new());
    reports.turn_index = 0;
    reports.reports = vec![
        report("sub-1", "host flow", "completed"),
        report("sub-2", "cancel path", "completed"),
    ];
    let mut assistant = message("assistant", "complete", vec![text_part("汇总")]);
    assistant.turn_index = 0;
    assistant.input_tokens = Some(100);
    assistant.output_tokens = Some(20);
    assistant.duration_ms = Some(1_000);
    let lx = lx_with(vec![reports, assistant]);
    let area = Rect::new(0, 0, 80, 30);
    let mut cache = cache();
    let total = layout(area, &lx, &mut cache).total_rows;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rendered: String = (0..total)
        .map(|row| row_text(&buf, area, u16::try_from(row).expect("row fits")))
        .collect();
    let first_card = rendered.find("Subagent report").expect("report card");
    let answer = rendered.find("汇总").expect("assistant answer");
    let stats = rendered.find("100 in").expect("stats line");
    assert!(first_card < answer, "报告卡必须在回答之前：{rendered:?}");
    assert!(answer < stats, "统计行属于报告轮回答：{rendered:?}");
}

#[test]
fn legacy_wait_report_rows_without_assistant_are_hidden() {
    let mut assistant = message("assistant", "complete", vec![text_part("最终总结")]);
    assistant.turn_index = 0;
    assistant.input_tokens = Some(47_000);
    assistant.output_tokens = Some(2_300);
    assistant.duration_ms = Some(252_000);
    let mut legacy = message("user", "complete", Vec::new());
    legacy.turn_index = 1;
    legacy.reports = vec![report("sub-1", "host flow", "completed")];
    let lx = lx_with(vec![assistant, legacy]);
    let area = Rect::new(0, 0, 80, 20);
    // 正文 1 + 空行 1 + 统计 1 + 呼吸位 1；legacy 报告不占行也不挡统计。
    assert_eq!(layout(area, &lx, &mut cache()).total_rows, 4);
    // 隐藏 legacy 行不能压缩消息索引：sticky/居中/命中仍按原始下标查。
    let snapshot = cache().snapshot(80, &lx);
    assert_eq!(snapshot.messages.len(), 2);
    assert_eq!(snapshot.messages[1].start, 0);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rendered: String = (0..4).map(|row| row_text(&buf, area, row)).collect();
    assert!(!rendered.contains("Subagent report"), "{rendered:?}");
    assert!(
        rendered.contains("47k in"),
        "legacy 报告不能挡住宿主统计：{rendered:?}"
    );
}

/// 多轮报告链路：运行中全隐藏；全部结束后只留最后一个 assistant 的统计。
#[test]
fn stats_follow_the_final_assistant_of_a_report_chain() {
    let mut main = message("assistant", "complete", vec![text_part("主轮")]);
    main.turn_index = 0;
    main.input_tokens = Some(100);
    main.output_tokens = Some(20);
    main.duration_ms = Some(1_000);
    let mut report1 = message("user", "complete", Vec::new());
    report1.turn_index = 1;
    report1.reports = vec![report("sub-1", "first", "completed")];
    let mut answer1 = message("assistant", "complete", vec![text_part("报告轮一")]);
    answer1.turn_index = 1;
    answer1.input_tokens = Some(100);
    answer1.output_tokens = Some(20);
    answer1.duration_ms = Some(1_000);
    let mut report2 = message("user", "complete", Vec::new());
    report2.turn_index = 2;
    report2.reports = vec![report("sub-2", "second", "completed")];
    let mut answer2 = message("assistant", "complete", vec![text_part("报告轮二")]);
    answer2.turn_index = 2;
    answer2.input_tokens = Some(100);
    answer2.output_tokens = Some(20);
    answer2.duration_ms = Some(1_000);
    let mut lx = lx_with(vec![main, report1, answer1, report2, answer2]);
    lx.agents
        .runs
        .push(crate::app::state::SubagentRun::from_record(
            &crate::db::chat::SubagentRecord {
                agent_id: "sub-3".into(),
                child_session_id: "child".into(),
                parent_session_id: "parent".into(),
                spawn_message_id: None,
                role: crate::chat::agent::AgentRole::Explorer,
                status: "running".into(),
                title: "third".into(),
                created_at_ms: 0,
                completed_at_ms: None,
                report_json: None,
                report_delivered: false,
            },
        ));
    let area = Rect::new(0, 0, 80, 60);
    let mut cache = cache();
    let total = layout(area, &lx, &mut cache).total_rows;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rendered: String = (0..total)
        .map(|row| row_text(&buf, area, u16::try_from(row).expect("row fits")))
        .collect();
    assert!(
        !rendered.contains("100 in"),
        "子 agent 运行中不能收尾：{rendered:?}"
    );
    lx.agents.runs[0].status = "completed".into();
    let total = layout(area, &lx, &mut cache).total_rows;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rendered: String = (0..total)
        .map(|row| row_text(&buf, area, u16::try_from(row).expect("row fits")))
        .collect();
    assert_eq!(
        rendered.matches("100 in").count(),
        1,
        "只在报告链最后一个 assistant 显示统计：{rendered:?}"
    );
}

/// 子 agent 仍在运行时，任何中间 assistant 都不显示统计行：任务还没收尾。
#[test]
fn stats_line_is_hidden_while_a_subagent_is_running() {
    let mut assistant = message("assistant", "complete", vec![text_part("等待返回")]);
    assistant.input_tokens = Some(100);
    assistant.output_tokens = Some(20);
    assistant.duration_ms = Some(1_000);
    let mut lx = lx_with(vec![assistant]);
    lx.agents
        .runs
        .push(crate::app::state::SubagentRun::from_record(
            &crate::db::chat::SubagentRecord {
                agent_id: "sub-1".into(),
                child_session_id: "child".into(),
                parent_session_id: "parent".into(),
                spawn_message_id: None,
                role: crate::chat::agent::AgentRole::Explorer,
                status: "running".into(),
                title: "host flow".into(),
                created_at_ms: 0,
                completed_at_ms: None,
                report_json: None,
                report_delivered: false,
            },
        ));
    let area = Rect::new(0, 0, 80, 20);
    // 正文 1 + 末尾呼吸位 1；统计行被隐藏。
    assert_eq!(layout(area, &lx, &mut cache()).total_rows, 2);
    lx.agents.runs[0].status = "completed".into();
    assert_eq!(
        layout(area, &lx, &mut cache()).total_rows,
        4,
        "最后一个子 agent 结束后统计行才出现"
    );
}

#[test]
fn streaming_assistant_has_no_stats_line() {
    let mut streaming = message("assistant", "streaming", vec![text_part("部分")]);
    streaming.input_tokens = Some(10);
    streaming.output_tokens = Some(2);
    streaming.duration_ms = Some(100);
    assert_eq!(stats_line(&streaming), None, "流中不出统计行");
    // 无任何可用数据（如失败且未返回 usage）时也不出。
    let empty = message("assistant", "failed", vec![text_part("")]);
    assert_eq!(stats_line(&empty), None);
}

#[test]
fn token_and_duration_formatting_matches_agent_conventions() {
    assert_eq!(text::token_count(999), "999");
    assert_eq!(text::token_count(1_200), "1.2k");
    assert_eq!(text::token_count(12_345), "12k");
    assert_eq!(text::token_count(1_250_000), "1.2M");
    assert_eq!(format_duration(820), "820ms");
    assert_eq!(format_duration(4_200), "4.2s");
    assert_eq!(format_duration(38_400), "38s");
    assert_eq!(format_duration(72_000), "1m 12s");
}

#[test]
fn thinking_header_uses_step_dot_and_becomes_thought() {
    // 生成中：行首方块（呼吸第 0 帧为实心、翡翠绿加粗）+ `Thinking`。
    let mut streaming = lx_with(vec![message(
        "assistant",
        "streaming",
        vec![reasoning_part("流式思考"), text_part("")],
    )]);
    // 闪烁只在动画推进时发生（`status.streaming`）：这是「动画停止恒实心」的前提。
    streaming.chat.status.streaming = true;
    let area = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    // 折叠态用**浅粉 182**（用户要求「没展开时不要灰色，用浅一点的粉色」），
    // 只有展开才升为亮粉 218；悬停不改变颜色。
    let header = &buf[(0, 0)];
    assert_eq!(header.fg, Color::Indexed(182), "折叠态方块浅粉");
    assert!(
        header.modifier.contains(Modifier::BOLD),
        "生成中的方块保留呼吸加粗"
    );
    let label = &buf[(2, 0)];
    assert_eq!(label.fg, Color::Indexed(182), "折叠态文案浅粉");
    assert!(!label.modifier.contains(Modifier::BOLD), "文案不加粗");
    // 悬停整块：颜色不变化（只有展开才升亮粉）。
    streaming.chat.hover = Some(crate::app::state::CardTarget::Thinking {
        message_index: 0,
        step_index: 0,
    });
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    assert_eq!(buf[(0, 0)].fg, Color::Indexed(182), "悬停方块仍为折叠浅粉");
    assert!(
        buf[(0, 0)].modifier.contains(Modifier::BOLD),
        "悬停不影响生成中的呼吸加粗"
    );
    assert_eq!(buf[(2, 0)].fg, Color::Indexed(182), "悬停文案仍为折叠浅粉");
    streaming.chat.hover = None;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    let line0 = row_text(&buf, area, 0);
    assert!(
        line0.starts_with("● Thinking"),
        "圆点 + 文案（文案从第 2 列起）：{line0}"
    );
    assert!(
        !line0.contains('▸') && !line0.contains('▾'),
        "折叠标记已移除：{line0}"
    );
    assert!(
        !line0
            .chars()
            .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
        "不再使用 braille 转轮：{line0}"
    );
    // 呼吸：进入空心相位后同一行换成 `○`（相位每 `STEP_BREATH_TICKS` 帧翻转）。
    streaming.chat.loading_frame = text::agent::STEP_BREATH_TICKS;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    let line0 = row_text(&buf, area, 0);
    assert!(line0.starts_with("○ Thinking"), "空心相位：{line0}");

    // 完成后（带起止时刻）：静态实心方块、品牌天蓝加粗（不压暗）、`Thought · 6.3s`。
    let mut done = lx_with(vec![message(
        "assistant",
        "complete",
        vec![
            timed_reasoning_part("旧思考", 1_000, 7_300),
            text_part("正文"),
        ],
    )]);
    done.chat.loading_frame = 0;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &done, 0);
    let header = &buf[(0, 0)];
    assert_eq!(header.fg, Color::Indexed(182), "完成方块折叠态浅粉");
    assert!(
        !header.modifier.contains(Modifier::BOLD),
        "完成方块不加粗（与运行中的加粗呼吸方块区分）"
    );
    let label = &buf[(2, 0)];
    assert_eq!(label.fg, Color::Indexed(182), "完成文案折叠态浅粉");
    assert!(!label.modifier.contains(Modifier::BOLD), "完成文案不加粗");
    let line0 = row_text(&buf, area, 0);
    assert!(
        line0.starts_with("● Thought · 6.3s"),
        "完成后显示思考耗时：{line0}"
    );
    // 终态与帧无关：奇数帧也是实心。
    done.chat.loading_frame = text::agent::STEP_BREATH_TICKS;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &done, 0);
    assert!(
        row_text(&buf, area, 0).starts_with("● Thought · 6.3s"),
        "完成后停为静态实心圆点"
    );

    // 展开：文案不变（不再有 ▸/▾），正文缩进 2 列与方块对齐。
    done.chat.toggle_reasoning(0, 0);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &done, 0);
    let header = &buf[(0, 0)];
    assert_eq!(header.fg, Color::Indexed(218), "展开后升为亮粉");
    assert!(
        !header.modifier.contains(Modifier::BOLD),
        "展开后仍是不加粗的完成态"
    );
    let label = &buf[(2, 0)];
    assert_eq!(label.fg, Color::Indexed(218), "展开后文案升为亮粉");
    let line0 = row_text(&buf, area, 0);
    assert!(
        line0.starts_with("● Thought · 6.3s"),
        "展开后头部不变（折叠标记已移除）：{line0}"
    );
    let line1 = row_text(&buf, area, 1);
    assert!(line1.starts_with("  └─ 旧思考"), "正文带树体收口：{line1}");
    assert!(!line0.contains("Thinking"), "不再是 Thinking：{line0}");
}

#[test]
fn markdown_body_block_has_step_dot_and_indented_continuation() {
    let lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![text_part("第一行\n第二行")],
    )]);
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let line0 = row_text(&buf, area, 0);
    let line1 = row_text(&buf, area, 1);
    assert!(
        line0.starts_with("● 第一行"),
        "markdown 正文同样以圆点起头：{line0}"
    );
    assert!(
        line1.starts_with("  第二行"),
        "续行缩进 2 列与方块对齐：{line1}"
    );
    // 正文块的圆点用**默认字体颜色**（不设前景），不加粗、不压暗。
    let marker = &buf[(0, 0)];
    assert_eq!(
        marker.fg,
        ratatui::style::Color::Reset,
        "正文方块 = 默认前景"
    );
    assert!(!marker.modifier.contains(ratatui::style::Modifier::BOLD));
    assert!(!marker.modifier.contains(ratatui::style::Modifier::DIM));
}

#[test]
fn only_the_active_last_step_breathes() {
    // 消息仍在流式：前一个正文块已结束（静态实心），最后一块仍在输出（空心呼吸）。
    let mut lx = lx_with(vec![message(
        "assistant",
        "streaming",
        vec![text_part("甲"), text_part("乙")],
    )]);
    lx.chat.loading_frame = text::agent::STEP_BREATH_TICKS;
    // 动画推进中才有呼吸（动画一停就恒实心，见 `StepMark::glyph`）。
    lx.chat.status.streaming = true;
    let area = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let first = row_text(&buf, area, 0);
    let last = row_text(&buf, area, 2);
    assert!(first.starts_with("● 甲"), "已结束的块停为实心：{first}");
    assert!(last.starts_with("○ 乙"), "仍在流式的最后一块呼吸：{last}");
}
