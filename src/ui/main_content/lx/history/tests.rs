//! 历史渲染与布局测试：用户底色/前缀、assistant 无标签、markdown 正文、折叠 thinking、
//! 换行、宽窄窗格与命中矩形。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::*;
use crate::app::state::{AssistantStepView, ChatMessageView, ChatPartView, Lx, LxChat};
use crate::chat::events::PartKind;

/// 工具步骤的几何一致性测试（布局/渲染行数、滚动条、置顶行、thinking 命中）。
mod tool_geometry;

fn lx_with(messages: Vec<ChatMessageView>) -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages = messages;
    lx.chat = chat;
    lx
}

fn message(role: &str, status: &str, parts: Vec<ChatPartView>) -> ChatMessageView {
    ChatMessageView {
        id: format!("{role}-1"),
        role: role.into(),
        status: status.into(),
        turn_index: 0,
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
    let layout = layout(area, &lx);
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
    let layout = layout(area, &lx);
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
    let layout = layout(area, &lx);
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
        line0.starts_with("■ Thought"),
        "首个渲染行为方块 + 图标 + 折叠头：{line0}"
    );
    assert!(
        !line0.contains("assistant"),
        "不再显示 assistant 标签：{line0}"
    );
    assert!(line1.trim().is_empty(), "折叠头与正文间空一行：{line1}");
    assert!(!line2.contains("秘密思考"), "折叠时不显示内容");
    assert!(
        line2.starts_with("■ 可见正文"),
        "正文块同样以方块起头：{line2}"
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
    assert_eq!(rows[2], "■ Failed", "行首为方块，与步骤标记同款");
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
    assert_eq!(rows[2], "■ Failed");
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
    assert_eq!(rows[2], "■ Interrupted", "中断用黄色警告通知 + 方块标记");
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
    let layout = layout(Rect::new(0, 0, 0, 0), &lx);
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
    let layout = layout(area, &lx);
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
    let _layout = layout(area, &lx);
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
    let layout = layout(area, &lx);
    // 正文 1 + 块间空行 1 + 统计行 1 + 消息间空行 1 = 4。
    assert_eq!(layout.total_rows, 4);
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
fn thinking_header_uses_step_square_and_becomes_thought() {
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
    // 常态压暗（次要内容），悬停才恢复品红（用户要求恢复原来的 hover 交互）。
    let header = &buf[(0, 0)];
    assert_eq!(header.fg, Color::Reset, "常态压暗（默认前景 + DIM）");
    assert!(header.modifier.contains(Modifier::DIM), "常态压暗");
    assert!(
        !header.modifier.contains(Modifier::BOLD),
        "压暗档不加粗（悬停才恢复加粗品红）"
    );
    let label = &buf[(2, 0)];
    assert_eq!(label.fg, Color::Reset, "文案常态压暗");
    assert!(!label.modifier.contains(Modifier::BOLD), "文案不加粗");
    // 悬停整块：方块与文案恢复品红 218。
    streaming.chat.hover = Some(crate::app::state::CardTarget::Thinking {
        message_index: 0,
        step_index: 0,
    });
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    assert_eq!(buf[(0, 0)].fg, Color::Indexed(218), "悬停方块恢复品红");
    assert!(
        buf[(0, 0)].modifier.contains(Modifier::BOLD),
        "悬停时方块加粗（生成中的原色）"
    );
    assert_eq!(buf[(2, 0)].fg, Color::Indexed(218), "悬停文案恢复品红");
    streaming.chat.hover = None;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    let line0 = row_text(&buf, area, 0);
    assert!(
        line0.starts_with("■ Thinking"),
        "方块 + 文案（文案从第 2 列起）：{line0}"
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
    // 呼吸：进入空心相位后同一行换成 `□`（相位每 `STEP_BREATH_TICKS` 帧翻转）。
    streaming.chat.loading_frame = text::agent::STEP_BREATH_TICKS;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &streaming, 0);
    let line0 = row_text(&buf, area, 0);
    assert!(line0.starts_with("□ Thinking"), "空心相位：{line0}");

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
    assert_eq!(header.fg, Color::Reset, "完成方块常态压暗");
    assert!(
        !header.modifier.contains(Modifier::BOLD),
        "完成方块不加粗（与运行中的加粗呼吸方块区分）"
    );
    let label = &buf[(2, 0)];
    assert_eq!(label.fg, Color::Reset, "完成文案常态压暗");
    assert!(!label.modifier.contains(Modifier::BOLD), "完成文案不加粗");
    let line0 = row_text(&buf, area, 0);
    assert!(
        line0.starts_with("■ Thought · 6.3s"),
        "完成后显示思考耗时：{line0}"
    );
    // 终态与帧无关：奇数帧也是实心。
    done.chat.loading_frame = text::agent::STEP_BREATH_TICKS;
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &done, 0);
    assert!(
        row_text(&buf, area, 0).starts_with("■ Thought · 6.3s"),
        "完成后停为静态实心方块"
    );

    // 展开：文案不变（不再有 ▸/▾），正文缩进 2 列与方块对齐。
    done.chat.toggle_reasoning(0, 0);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &done, 0);
    let header = &buf[(0, 0)];
    assert_eq!(header.fg, Color::Reset, "展开后仍是常态压暗");
    assert!(
        !header.modifier.contains(Modifier::BOLD),
        "展开后仍是不加粗的完成态"
    );
    let line0 = row_text(&buf, area, 0);
    assert!(
        line0.starts_with("■ Thought · 6.3s"),
        "展开后头部不变（折叠标记已移除）：{line0}"
    );
    let line1 = row_text(&buf, area, 1);
    assert!(line1.starts_with("  旧思考"), "正文缩进对齐方块：{line1}");
    assert!(!line0.contains("Thinking"), "不再是 Thinking：{line0}");
}

#[test]
fn markdown_body_block_has_step_square_and_indented_continuation() {
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
        line0.starts_with("■ 第一行"),
        "markdown 正文同样以方块起头：{line0}"
    );
    assert!(
        line1.starts_with("  第二行"),
        "续行缩进 2 列与方块对齐：{line1}"
    );
    // 正文块的方块用**默认字体颜色**（不设前景），不加粗、不压暗。
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
    assert!(first.starts_with("■ 甲"), "已结束的块停为实心：{first}");
    assert!(last.starts_with("□ 乙"), "仍在流式的最后一块呼吸：{last}");
}

// ===== 置顶用户消息行（表现层，codex prompt_header 口径）=====

/// 构造一段够长、能溢出的历史：两轮问答，正文各若干行。
fn sticky_lx() -> Lx {
    let mut lx = lx_with(vec![
        message("user", "complete", vec![text_part("第一轮的问题")]),
        message("assistant", "complete", vec![text_part("A1\nA2\nA3")]),
        message("user", "complete", vec![text_part("第二轮的问题")]),
        message("assistant", "complete", vec![text_part("B1\nB2\nB3")]),
    ]);
    // 溢出必需的滚动空间：视口 6 行、内容远多于 6 行。
    lx.chat.follow_bottom = false;
    lx
}

/// 历史区（宽 24、高 8）：body 6 行。
fn sticky_area() -> Rect {
    Rect::new(0, 0, 24, 8)
}

#[test]
fn sticky_prompt_pins_the_turn_prompt_when_top_row_is_answer() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx);
    // 顶行落在第一轮回复里。
    let scroll = layout.messages[1].start;
    lx.chat.history_scroll = scroll;
    let computed = view(area, &lx);
    assert_eq!(computed.sticky, Some(0), "钉第一轮的 prompt");
    assert_eq!(computed.body.y, area.y + 1, "让位一行");
    assert_eq!(computed.body.height, area.height - 1);
    assert_eq!(computed.scroll, computed.scroll, "滚动偏移按 body 钳制");
}

#[test]
fn sticky_prompt_yields_when_top_row_is_the_prompt() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx);
    // 顶行就是第二条用户消息自己的行（含它前面的空行）→ 不钉。
    lx.chat.history_scroll = layout.messages[2].start;
    let computed = view(area, &lx);
    assert_eq!(computed.sticky, None, "用户消息自带上下文");
    assert_eq!(computed.body, area, "不占行");

    // 轮边界空行（归后一条消息）同样按用户消息自己的行处理。
    lx.chat.history_scroll = layout.messages[2].start.saturating_sub(1);
    let at_blank = view(area, &lx);
    assert_eq!(at_blank.sticky, None, "空行归后一条：仍算用户消息的行");
}

#[test]
fn sticky_prompt_revert_records_suppression_memory() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx);
    // 整块 body 下顶行 = 第一轮回复最后一行 → 需要钉；让位一行后顶行变成
    // 第二条用户消息的边界空行 → 不再需要 → 撤销让位并记记忆。
    let scroll = layout.messages[2].start.saturating_sub(1);
    lx.chat.history_scroll = scroll;
    let computed = view(area, &lx);
    assert_eq!(computed.sticky, None, "让位后不再需要 → 撤销");
    assert_eq!(computed.body, area, "撤销让位");
    let memory = computed.suppressed.expect("写入抑制记忆");
    assert_eq!(memory.viewport, area);
    assert_eq!(memory.message, 1, "记忆的是整块视口的顶行位置");
}

#[test]
fn sticky_prompt_respects_suppression_memory_while_reading() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx);
    // 顶行在第一轮回复里：本会钉，但记忆一致 → 跳过重判、沿用抑制。
    lx.chat.history_scroll = layout.messages[1].start;
    let top = sticky::message_at(&layout.messages, layout.messages[1].start).expect("顶行");
    lx.chat.suppressed_prompt_header = Some(crate::app::state::lx::chat::StickySuppression {
        viewport: area,
        message: top.0,
        row: top.1,
    });
    let computed = view(area, &lx);
    assert_eq!(computed.sticky, None, "阅读态沿用抑制");
    assert_eq!(computed.body, area);
    assert_eq!(
        computed.suppressed, lx.chat.suppressed_prompt_header,
        "记忆保留"
    );

    // 视口一变化记忆即失效 → 重新判定（本会钉）。
    lx.chat.suppressed_prompt_header = Some(crate::app::state::lx::chat::StickySuppression {
        viewport: Rect::new(0, 0, 23, 8),
        ..computed.suppressed.expect("记忆")
    });
    let resized = view(area, &lx);
    assert_eq!(resized.sticky, Some(0), "视口变化使记忆失效");
}

#[test]
fn sticky_prompt_thresholds_hide_the_row() {
    let mut lx = sticky_lx();
    let layout = layout(sticky_area(), &lx);
    lx.chat.history_scroll = layout.messages[1].start;
    // 太矮：不显示（也不写记忆）。
    let short = view(Rect::new(0, 0, 24, 3), &lx);
    assert_eq!(short.sticky, None, "高度不足不显示");
    assert_eq!(short.suppressed, None);
    // 太窄：不显示。
    let narrow = view(Rect::new(0, 0, 15, 8), &lx);
    assert_eq!(narrow.sticky, None, "宽度不足不显示");
}

#[test]
fn sticky_text_collapses_sanitizes_and_marks_empty() {
    let mut message = message(
        "user",
        "complete",
        vec![text_part("  第一  行\n\t第二\r行  ")],
    );
    assert_eq!(
        sticky::sticky_text(&message).as_deref(),
        Some("第一 行 第二 行")
    );
    // 只有空白 → 不钉（None）。
    set_parts(&mut message, vec![text_part("   \n\t ")]);
    assert_eq!(sticky::sticky_text(&message), None);
    // 超长粘贴按 512 字符取样（限住每帧成本）。
    set_parts(&mut message, vec![text_part(&"字".repeat(1_000))]);
    let text = sticky::sticky_text(&message).expect("摘要");
    assert_eq!(text.chars().count(), sticky::SAMPLE_CHARS);
}

#[test]
fn sticky_row_is_painted_outside_the_grid() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx);
    lx.chat.history_scroll = layout.messages[1].start;
    let computed = view(area, &lx);
    let row = sticky_rect(&computed).expect("置顶行");
    assert_eq!(row.rect, Rect::new(area.x, area.y, area.width, 1));

    // 表现层：直接写 Buffer，网格那一行保持空白（→ 不进复制与选区）。
    let message = lx.chat.messages[row.message].clone();
    let mut buf = Buffer::empty(area);
    render_sticky_prompt(&mut buf, row, &message);
    let painted = row_text(&buf, area, 0);
    let squeezed = painted.replace(' ', "");
    assert!(
        squeezed.starts_with(">第一轮的问题"),
        "前缀与用户消息同款 `> `：{painted}"
    );

    let mut grid = Grid::new(area);
    render(&mut grid, computed.body, &lx, computed.scroll);
    let mut grid_row = String::new();
    for x in area.x..area.right() {
        grid_row.push(grid.cell(x, area.y).ch);
    }
    assert!(
        grid_row.trim().is_empty(),
        "网格置顶行保持空白：{grid_row:?}"
    );
}

#[test]
fn sticky_text_is_truncated_with_ellipsis_to_width() {
    let mut lx = sticky_lx();
    set_parts(
        &mut lx.chat.messages[0],
        vec![text_part("很长很长很长很长很长很长很长的问题")],
    );
    let area = sticky_area();
    let layout = layout(area, &lx);
    lx.chat.history_scroll = layout.messages[1].start;
    let computed = view(area, &lx);
    let row = sticky_rect(&computed).expect("置顶行");
    let message = lx.chat.messages[row.message].clone();
    let mut buf = Buffer::empty(area);
    render_sticky_prompt(&mut buf, row, &message);
    let painted = row_text(&buf, area, 0);
    assert!(
        painted.trim_end().ends_with('…'),
        "溢出以省略号收尾：{painted}"
    );
    // 截断函数本体：按显示列截断、宽字符按两列计。
    assert_eq!(truncate_with_ellipsis("abcdef", 4), "abc…");
    assert_eq!(truncate_with_ellipsis("中文中文", 5), "中文…");
    assert_eq!(truncate_with_ellipsis("abc", 3), "abc");
}

// ===== G7：真实工具步骤行（必须占行、不 panic、不与布局漂移）=====

/// 构造工具步骤（`read` / Pending，带路径与行范围摘要）。
fn tool_step(position: i64) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: crate::app::state::ToolCallView {
            call_id: format!("call-{position}"),
            name: "read".into(),
            position,
            arguments_json: r#"{"path":"src/a.rs","offset":3,"limit":5}"#.into(),
            status: crate::app::state::ToolStatus::Pending,
            summary: Some(Box::new(crate::app::state::ToolSummary {
                path: Some("src/a.rs".into()),
                range: Some((3, 7)),
                ..Default::default()
            })),
            error: None,
            result_json: None,
            detail: None,
        },
    }
}

/// 工具步骤占两行（`■ 动词` + `  └─ 详情`）：布局行数与渲染行数一致，
/// 且文本可见（不吞掉、不错位）。
#[test]
fn tool_step_occupies_two_rows_and_renders_summary() {
    let mut message = message("assistant", "complete", vec![text_part("正文")]);
    message.steps.push(tool_step(1));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 20);
    let computed = layout(area, &lx);
    // 正文 1 + 块间空行 1 + 工具 2 行 + 消息间空行 1 = 5。
    assert_eq!(computed.total_rows, 5);

    let mut grid = Grid::new(area);
    render(&mut grid, area, &lx, 0);
    let mut rows: Vec<String> = Vec::new();
    for y in 0..7u16 {
        let mut row = String::new();
        for x in 0..area.right() {
            row.push(grid.cell(x, y).ch);
        }
        rows.push(row);
    }
    let painted = rows.join("\n");
    let squeezed: String = painted
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '\0')
        .collect();
    assert!(squeezed.contains("正文"), "正文仍渲染：{painted}");
    assert!(
        squeezed.contains(text::agent::VERB_TOOL_READ),
        "工具行动词可见：{painted}"
    );
    // 第 1 行只有方块 + 动词（无状态文字、无路径）。
    let header_row = rows
        .iter()
        .find(|row| row.contains(text::agent::VERB_TOOL_READ))
        .expect("工具标题行");
    assert_eq!(
        header_row.trim_end(),
        format!(
            "{} {}",
            text::agent::STEP_MARK_FILLED,
            text::agent::VERB_TOOL_READ
        ),
        "标题行只有方块 + 动词：{header_row:?}"
    );
    // 第 2 行是 `└─` + 参数 + 状态词兜底（pending 没有参数摘要之外的状态字段，
    // 这里 pending 有摘要 → 只显示参数与范围）。
    let detail_row = rows
        .iter()
        .find(|row| row.contains("src/a.rs"))
        .expect("详情行");
    assert!(
        detail_row
            .trim_start()
            .starts_with(text::agent::MARKER_TREE_LEAF),
        "详情行以 └─ 起头：{detail_row:?}"
    );
    assert!(squeezed.contains("3-7"), "行范围可见：{painted}");
}

/// 仅工具步骤的消息也占行（不会因“没有 text part”而整条消失）。
#[test]
fn tool_only_message_still_occupies_rows() {
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(tool_step(0));
    let lx = lx_with(vec![message]);
    let computed = layout(Rect::new(0, 0, 40, 10), &lx);
    // 工具 2 行 + 消息间空行/呼吸行 1 = 3。
    assert_eq!(computed.total_rows, 3);
}

#[test]
fn transcript_lays_out_step_square_with_two_column_gutter() {
    // 一份完整观感锁定：用户消息 `> `、每个步骤的方块标记与 2 列内容缩进
    // （思考头 / 思考正文 / 正文块 / 工具行）、块间空行。
    let mut assistant = message(
        "assistant",
        "streaming",
        vec![
            reasoning_part("思考甲\n思考乙"),
            text_part("正文第一行\n正文第二行"),
        ],
    );
    assistant.steps.push(tool_step(2));
    let mut lx = lx_with(vec![
        message("user", "complete", vec![text_part("用户问题")]),
        assistant,
    ]);
    lx.chat.toggle_reasoning(1, 0);
    lx.chat.loading_frame = text::agent::STEP_BREATH_TICKS; // 呼吸进入空心相位
    // 动画推进中才呼吸（停止后恒实心，见 `StepMark::glyph`）。
    lx.chat.status.streaming = true;
    let area = Rect::new(0, 0, 56, 12);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    // 10 行内容 + 1 行呼吸位：工具行改成两行后多出 1 行。
    let rows: Vec<String> = (0..11).map(|row| row_text(&buf, area, row)).collect();
    assert_eq!(
        rows,
        vec![
            "> 用户问题",
            "",
            "□ Thinking",
            "  思考甲",
            "  思考乙",
            "",
            "■ 正文第一行",
            "  正文第二行",
            "",
            "□ Read",
            "  └─ src/a.rs · 3-7",
        ],
        "步骤方块、2 列缩进与两行式工具行的整块观感"
    );
}
