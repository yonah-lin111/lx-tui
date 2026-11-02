//! 最近修改页面抽屉：分割线、双行条目、状态圆点与悬停高亮；只读状态。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::TemplateStatus;
use crate::app::prompt::RecentPageRow;
use crate::ui::{style, text};

use super::tree;

/// 抽屉渲染输入：几何、条目与交互态。
#[derive(Clone, Copy)]
pub struct RecentRender<'a> {
    pub panel: Rect,
    pub divider: Rect,
    pub area: Rect,
    pub rows: &'a [RecentPageRow],
    pub scroll: usize,
    pub visible_entries: usize,
    pub focused: bool,
    pub hover: Option<usize>,
}

/// 绘制 `├─ recent ───┤` 分割线、空态或双行条目列表。
pub fn render(buf: &mut Buffer, data: &RecentRender<'_>) {
    let RecentRender {
        panel,
        divider,
        area,
        rows,
        scroll,
        visible_entries,
        focused,
        hover,
    } = *data;
    super::render_rule(panel, divider.y, buf, focused);
    let label = text::PROMPT_RECENT_SECTION_LABEL;
    let label_x = divider.x.saturating_add(2);
    if label_x.saturating_add(label.width() as u16) < divider.right() {
        buf.set_stringn(
            label_x,
            divider.y,
            label,
            label.width(),
            style::muted_on_border(),
        );
    }
    if area.width == 0 || area.height == 0 {
        return;
    }
    if rows.is_empty() || visible_entries == 0 {
        tree::paint_text(
            buf,
            area.x.saturating_add(2),
            area.y,
            text::PROMPT_RECENT_EMPTY,
            style::muted(),
            area.right(),
        );
        return;
    }
    let start = scroll.min(rows.len().saturating_sub(visible_entries));
    for (offset, row) in rows.iter().enumerate().skip(start).take(visible_entries) {
        let y = area.y + (offset - start) as u16 * 2;
        if y + 1 >= area.bottom() {
            break;
        }
        paint_entry(buf, area, y, row, hover == Some(offset));
    }
}

/// 绘制一条双行记录：两行共用悬停底色。
fn paint_entry(buf: &mut Buffer, area: Rect, y: u16, row: &RecentPageRow, hovered: bool) {
    let highlight = if hovered {
        tree::fill_row(buf, area, y);
        tree::fill_row(buf, area, y + 1);
        style::selection()
    } else {
        Style::default()
    };
    let base = style::text().patch(highlight);
    let marker = if row.current {
        format!("{} ", text::AGENT_SELECTED_MARKER)
    } else {
        "  ".to_string()
    };
    let x = tree::paint_text(buf, area.x, y, &marker, base, area.right());
    let time_width = row.time_label.width();
    let time_x = area.right().saturating_sub(time_width as u16);
    if time_x > x {
        tree::paint_text(
            buf,
            time_x,
            y,
            &row.time_label,
            style::muted().patch(highlight),
            area.right(),
        );
    }
    let name_right = time_x.saturating_sub(1).max(x);
    let name_style = if row.current {
        style::strong().patch(highlight)
    } else {
        base
    };
    tree::paint_name(
        buf,
        x,
        y,
        &row.prompt_name,
        name_style,
        usize::from(name_right.saturating_sub(x)),
        name_right,
    );
    let title_x = tree::paint_text(buf, area.x, y + 1, "  ", base, area.right());
    let used_right = if let Some(status) = row.status {
        let (dot, dot_style) = match status {
            TemplateStatus::Todo => (text::PAGE_STATUS_OPEN, style::muted().patch(highlight)),
            TemplateStatus::InProgress => (
                text::PAGE_STATUS_DOT,
                base.patch(style::template_status(TemplateStatus::InProgress)),
            ),
            TemplateStatus::Done => (
                text::PAGE_STATUS_DOT,
                base.patch(style::template_status(TemplateStatus::Done)),
            ),
        };
        let dot_width = u16::try_from(dot.width()).unwrap_or(1);
        let dot_x = area.right().saturating_sub(dot_width);
        if dot_x >= title_x {
            tree::paint_text(buf, dot_x, y + 1, dot, dot_style, area.right());
            dot_x.saturating_sub(1)
        } else {
            area.right()
        }
    } else {
        area.right()
    };
    tree::paint_name(
        buf,
        title_x,
        y + 1,
        &row.page_title,
        style::muted().patch(highlight),
        usize::from(used_right.saturating_sub(title_x)),
        used_right,
    );
}

#[cfg(test)]
mod tests;
