//! 置顶用户消息行：判定、矩形与绘制。
//!
//! 从 `history.rs` 拆出（单文件行数上限）：这里的函数都是纯计算或纯 `Buffer` 绘制，
//! 不读文件、不写状态。**表现层**：置顶行不进网格 → 不参与复制与聊天选区。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::ui::style;

use super::{HistoryView, MessageRows, USER_PREFIX, content_rect};

/// 置顶行的最小可用视口（高度 / 宽度，对齐 codex）。
pub(super) const MIN_HEIGHT: u16 = 4;
pub(super) const MIN_WIDTH: u16 = 16;
/// 置顶行摘要的最大取样字符数（限住超长粘贴的每帧成本，对齐 codex 的 512）。
pub(super) const SAMPLE_CHARS: usize = 512;

/// 置顶行绘制目标：矩形 + 被钉的消息索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StickyRow {
    pub rect: Rect,
    pub message: usize,
}

/// 顶行所属消息与消息内行偏移；空历史返回 None。
pub(super) fn message_at(rows: &[MessageRows], scroll: usize) -> Option<(usize, usize)> {
    rows.iter()
        .enumerate()
        .find(|(_, range)| scroll >= range.start && scroll < range.end)
        .map(|(index, range)| (index, scroll - range.start))
}

/// 置顶候选：顶行落在用户消息自己的行上 → None（自带上下文）；
/// 否则取它前面最近一条**有正文**的用户消息。
pub(super) fn sticky_candidate(
    messages: &[crate::app::state::ChatMessageView],
    rows: &[MessageRows],
    scroll: usize,
) -> Option<usize> {
    let (index, _) = message_at(rows, scroll)?;
    if rows.get(index)?.is_user {
        return None;
    }
    (0..index)
        .rev()
        .find(|candidate| rows[*candidate].is_user && sticky_text(&messages[*candidate]).is_some())
}

/// 置顶行摘要：剥离有效引用标记后的用户消息首个正文块的前 512 字符 → 控制字符转空格 → 空白折叠为单行；
/// 若剥离后无文本但包含图片/文件引用，回退为图片占位文案或首个引用名；全空白返回 None（该条不钉）。
pub(super) fn sticky_text(message: &crate::app::state::ChatMessageView) -> Option<String> {
    let part = message.text_parts().next()?;
    let stripped = crate::chat::strip_references_for_display(&part.content, &message.attachments);
    let display = if stripped.is_empty() && !message.attachments.is_empty() {
        let image_count = message
            .attachments
            .iter()
            .filter(|att| att.kind == crate::db::chat::MessageAttachmentKind::Image)
            .count();
        if image_count > 0 {
            crate::ui::text::chat::image_message_preview(image_count).to_string()
        } else {
            message
                .attachments
                .first()
                .map(|att| att.label.clone())
                .unwrap_or_default()
        }
    } else {
        stripped
    };
    let sample: String = display.chars().take(SAMPLE_CHARS).collect();
    let single: String = sample
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let collapsed = single.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// 置顶行矩形（历史整区第 0 行整宽）；不显示时为 None。
pub(crate) fn sticky_rect(view: &HistoryView) -> Option<StickyRow> {
    view.sticky.map(|message| StickyRow {
        rect: Rect::new(view.area.x, view.area.y, view.area.width, 1),
        message,
    })
}

/// 绘制置顶行（**表现层**：直接写 `Buffer`、不进网格 → 不参与复制与选区）；
/// 整宽 `style::chat_user_bg` 底 + `› ` 前缀（accent）+ 摘要（默认色，按列截断加 `…`）。
pub(crate) fn render_sticky_prompt(
    buf: &mut Buffer,
    row: StickyRow,
    message: &crate::app::state::ChatMessageView,
) {
    let Some(text) = sticky_text(message) else {
        return;
    };
    let bg = style::chat_user_bg();
    let content = content_rect(row.rect);
    for x in content.x..content.right() {
        if let Some(cell) = buf.cell_mut((x, row.rect.y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(bg);
        }
    }
    let mut x = put_buffer_line(
        buf,
        content.x,
        row.rect.y,
        USER_PREFIX,
        style::accent().patch(bg),
        content.right(),
    );
    let remaining = usize::from(content.right().saturating_sub(x));
    if remaining > 0 {
        let text = truncate_with_ellipsis(&text, remaining);
        x = put_buffer_line(buf, x, row.rect.y, &text, bg, content.right());
    }
    // 尾部留白仍铺底（整宽一致）。
    for tail in x..content.right() {
        if let Some(cell) = buf.cell_mut((tail, row.rect.y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(bg);
        }
    }
}

/// 逐字符写 `Buffer`（宽字符占两列、不跨 `limit` 半写）；返回写入后的列。
fn put_buffer_line(
    buf: &mut Buffer,
    mut x: u16,
    y: u16,
    content: &str,
    style: Style,
    limit: u16,
) -> u16 {
    for ch in content.chars() {
        let width = ch.width().unwrap_or(0) as u16;
        if width == 0 {
            continue;
        }
        if x.saturating_add(width) > limit {
            break;
        }
        for offset in 0..width.max(1) {
            if let Some(cell) = buf.cell_mut((x.saturating_add(offset), y)) {
                cell.reset();
                cell.set_char(if offset == 0 { ch } else { ' ' });
                cell.set_style(style);
            }
        }
        x = x.saturating_add(width);
    }
    x
}

/// 按显示列截断，溢出以 `…` 收尾（宽字符按两列计）。
pub(crate) fn truncate_with_ellipsis(value: &str, max: usize) -> String {
    if value.width() <= max {
        return value.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0;
    for ch in value.chars() {
        let cell = ch.width().unwrap_or(0);
        if width + cell > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        width += cell;
    }
    out.push('…');
    out
}
