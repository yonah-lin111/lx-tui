//! 历史渲染与布局测试：折叠 thinking、换行、宽窄窗格与命中矩形。

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
    // 1 消息标题 + 1 折叠头 + 1 正文行 + 1 空行 = 4（reasoning 折叠不展开内容）。
    assert_eq!(layout.total_rows, 4);
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
    // 标题 1 + 展开头 1 + 3 内容行 + 正文 1 + 空行 1 = 7。
    assert_eq!(layout.total_rows, 7);
}

#[test]
fn render_writes_message_text_and_thinking_label() {
    let lx = lx_with(vec![message(
        "assistant",
        "streaming",
        vec![reasoning_part("秘密思考"), text_part("可见正文")],
    )]);
    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let line0 = row_text(&buf, area, 0);
    let line1 = row_text(&buf, area, 1);
    let line2 = row_text(&buf, area, 2);
    assert!(line0.contains("streaming"), "标题含状态：{line0}");
    assert!(line1.contains("thinking"), "折叠头显示 thinking：{line1}");
    assert!(!line1.contains("秘密思考"), "折叠时不显示内容");
    assert!(line2.contains("可见正文"), "正文绘制：{line2}");
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
    // 第一条消息：标题(5) + 正文(6) + 空行(7)；第二条标题(8) + thinking 头(9)。
    assert_eq!(layout.thinking.len(), 1);
    let (_, _, header) = layout.thinking[0];
    assert_eq!(header.y, 9);
    assert_eq!(header.x, 3);
    assert_eq!(header.width, 40);
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
