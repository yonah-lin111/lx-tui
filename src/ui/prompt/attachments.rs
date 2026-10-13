//! prompt 附件条渲染：类型标签、文件名、缺失状态与移除命中区；只读状态。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::prompt::Prompt;
use crate::layout;
use crate::ui::style;
use crate::ui::text;

/// 移除按钮固定宽度（列）。
const PROMPT_ATTACH_REMOVE_WIDTH: u16 = 3;
/// 类型标签与名称之间的间距（列）。
const PROMPT_ATTACH_TAG_GAP: u16 = 1;

/// 附件条条目命中区：附件 ID、整行与移除按钮矩形。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentHit {
    pub id: String,
    pub row: Rect,
    pub remove: Rect,
}

/// 附件条可见条目与命中区；渲染与鼠标命中共用（`scroll` 为顶部附件索引）。
pub fn attachment_hits(prompt: &Prompt, panel: Rect, scroll: usize) -> Vec<AttachmentHit> {
    let attachments = prompt.attachments();
    let Some(bar) = layout::prompt_attachment_rect(panel, attachments.len()) else {
        return Vec::new();
    };
    let max = usize::from(bar.height);
    let start = scroll.min(attachments.len().saturating_sub(max));
    attachments
        .iter()
        .skip(start)
        .take(max)
        .enumerate()
        .map(|(offset, attachment)| {
            let y = bar.y + offset as u16;
            AttachmentHit {
                id: attachment.id.clone(),
                row: Rect::new(bar.x, y, bar.width, 1),
                remove: Rect::new(
                    bar.right().saturating_sub(PROMPT_ATTACH_REMOVE_WIDTH),
                    y,
                    PROMPT_ATTACH_REMOVE_WIDTH,
                    1,
                ),
            }
        })
        .collect()
}

/// 附件条矩形；未显示附件时为 None。
pub fn attachment_bar_rect(prompt: &Prompt, panel: Rect) -> Option<Rect> {
    layout::prompt_attachment_rect(panel, prompt.attachments().len())
}

/// 绘制附件条：每行类型标签 + 名称 + 缺失状态 + 移除按钮；计数指示常显在末行右侧。
pub fn render_attachments(panel: Rect, buf: &mut Buffer, prompt: &Prompt, scroll: usize) {
    let hits = attachment_hits(prompt, panel, scroll);
    let Some(bar) = layout::prompt_attachment_rect(panel, prompt.attachments().len()) else {
        return;
    };
    let total = prompt.attachments().len();
    for hit in &hits {
        let Some(attachment) = prompt
            .attachments()
            .iter()
            .find(|attachment| attachment.id == hit.id)
        else {
            continue;
        };
        let tag = match attachment.kind {
            crate::app::prompt::AttachmentKind::File => text::PROMPT_ATTACH_FILE_TAG,
            crate::app::prompt::AttachmentKind::Image => text::PROMPT_ATTACH_IMAGE_TAG,
        };
        let available = prompt.attachment_available(&hit.id);
        let name_style = if available {
            style::text()
        } else {
            style::muted()
        };
        let mut x = bar.x + 1;
        for ch in tag.chars() {
            if let Some(cell) = buf.cell_mut((x, hit.row.y)) {
                cell.reset();
                cell.set_char(ch);
                cell.set_style(style::muted());
            }
            x = x.saturating_add(1);
        }
        x = x.saturating_add(PROMPT_ATTACH_TAG_GAP);
        let name_end = hit
            .remove
            .x
            .saturating_sub(if !available {
                text::PROMPT_ATTACH_MISSING.width() as u16 + 1
            } else {
                0
            })
            .saturating_sub(1);
        let width = usize::from(name_end.saturating_sub(x));
        let label = text::truncate_to_width(&attachment.name, width);
        for ch in label.chars() {
            let cell_width = ch.width().unwrap_or(0) as u16;
            if cell_width == 0 {
                continue;
            }
            if let Some(cell) = buf.cell_mut((x, hit.row.y)) {
                cell.reset();
                cell.set_char(ch);
                cell.set_style(name_style);
            }
            x = x.saturating_add(cell_width);
        }
        if !available {
            let missing = format!(" {}", text::PROMPT_ATTACH_MISSING);
            for ch in missing.chars() {
                let cell_width = ch.width().unwrap_or(0) as u16;
                if let Some(cell) = buf.cell_mut((x, hit.row.y)) {
                    cell.reset();
                    cell.set_char(ch);
                    cell.set_style(style::muted());
                }
                x = x.saturating_add(cell_width);
            }
        }
        // 计数指示：末行右端显示可见/总数（超出固定高度时提示可滚动）。
        if hit.row.y == bar.bottom().saturating_sub(1) && total > hits.len() {
            let label = text::attachment_count_label(hits.len(), total);
            let label_width = label.width() as u16;
            let mut x = hit.remove.x.saturating_sub(label_width).saturating_sub(1);
            for ch in label.chars() {
                if let Some(cell) = buf.cell_mut((x, hit.row.y)) {
                    cell.reset();
                    cell.set_char(ch);
                    cell.set_style(style::muted());
                }
                x = x.saturating_add(1);
            }
        }
        let mut remove_x = hit.remove.x;
        for ch in text::PROMPT_ATTACH_REMOVE.chars() {
            if let Some(cell) = buf.cell_mut((remove_x, hit.row.y)) {
                cell.reset();
                cell.set_char(ch);
                cell.set_style(style::muted());
            }
            remove_x = remove_x.saturating_add(1);
        }
    }
}
