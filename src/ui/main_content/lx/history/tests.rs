//! 历史渲染与布局测试：用户底色/前缀、assistant 无标签、markdown 正文、折叠 thinking、
//! 换行、宽窄窗格与命中矩形。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::{ChatMessageView, ChatPartView, Lx, LxChat};
use crate::chat::events::PartKind;

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
        parts,
        model_label: Some("P / M".into()),
        error_code: None,
    }
}

fn text_part(content: &str) -> ChatPartView {
    ChatPartView {
        kind: PartKind::Text,
        content: content.into(),
        provider_metadata_json: None,
    }
}

fn reasoning_part(content: &str) -> ChatPartView {
    ChatPartView {
        kind: PartKind::Reasoning,
        content: content.into(),
        provider_metadata_json: None,
    }
}

#[test]
fn thinking_collapsed_by_default_one_row_each() {
    let lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![reasoning_part("长思考\n两行"), text_part("正文")],
    )]);
    let area = Rect::new(0, 0, 40, 20);
    let layout = layout(area, &lx);
    // 1 折叠头 + 1 正文行 + 1 空行 = 3（reasoning 折叠不展开内容；完成态无标题）。
    assert_eq!(layout.total_rows, 3);
    assert_eq!(layout.thinking.len(), 1);
    assert_eq!(layout.thinking[0].0, 0);
    assert_eq!(layout.thinking[0].1, 0);
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
    // 展开头 1 + 3 内容行 + 正文 1 + 空行 1 = 6。
    assert_eq!(layout.total_rows, 7 - 1);
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
    assert!(line0.contains("thinking"), "首个渲染行为折叠头：{line0}");
    assert!(
        !line0.contains("assistant"),
        "不再显示 assistant 标签：{line0}"
    );
    assert!(!line1.contains("秘密思考"), "折叠时不显示内容");
    assert!(line1.contains("可见正文"), "正文绘制：{line1}");
}

#[test]
fn streaming_status_marker_is_shown() {
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
    assert!(joined.contains("[streaming]"), "流中显示标记：{joined}");
}

#[test]
fn failed_message_shows_error_marker_with_code() {
    let mut message = message("assistant", "failed", vec![text_part("")]);
    message.error_code = Some("http 503".into());
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let line0 = row_text(&buf, area, 0);
    assert!(line0.contains("[failed: http 503]"), "失败标记：{line0}");
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
