//! 上下文压缩提示（compact checkpoint）的物理行布局与绘制。
//!
//! 与 [`super::rewind`] 同一套口径：标题带 `-- Context compacted --`，描述行给出
//! 实际摘要模型与时间；默认折叠只显示两行头部，展开后追加覆盖范围与摘要正文。
//! 不是 user/assistant 消息，不进入复制选区与模型上下文；窄屏按列换行。
//!
//! 行数计算 [`rows`] 与绘制 [`draw`] 必须使用同一份 `wrap`，否则滚动条与视口锚点会漂移。

use ratatui::layout::Rect;

use crate::app::state::CompactNoticeView;
use crate::ui::style;
use crate::ui::text;

use super::visible_row;
use crate::ui::main_content::lx::Grid;

/// 提示占用的行数（与 [`draw`] 同口径）。
pub(super) fn rows(notice: &CompactNoticeView, count: usize, width: u16, expanded: bool) -> usize {
    let mut total = super::wrap(&title(count), width).len();
    total += super::wrap(&description(notice), width).len();
    if expanded {
        total += super::wrap(&range_line(notice), width).len();
        total += super::wrap(&notice.summary, width).len();
    }
    total
}

/// 标题（连续多条合并为 `-- Context compacted xN --`）。
pub(super) fn title(count: usize) -> String {
    text::compact_notice_title(count)
}

/// 头部描述：模型与时间；已撤销的条目明确标注。
pub(super) fn description(notice: &CompactNoticeView) -> String {
    let base = text::compact_notice(
        &notice.model_label,
        &crate::platform::local_time_label(notice.created_at_ms),
    );
    if notice.active {
        base
    } else {
        format!("{base} · reverted")
    }
}

/// 展开后的覆盖范围行。
fn range_line(notice: &CompactNoticeView) -> String {
    let range = if notice.auto {
        text::LX_COMPACT_AUTO_RANGE.to_string()
    } else {
        notice.range_label()
    };
    format!("{} {range}", text::LX_COMPACT_RANGE_LABEL)
}

/// 绘制提示并推进内容行号；`count` 为合并的连续提示数。
pub(super) fn draw(
    grid: &mut Grid,
    notice: &CompactNoticeView,
    count: usize,
    expanded: bool,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
) {
    for line in super::wrap(&title(count), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::accent(), content.width);
        }
        *row_index += 1;
    }
    for line in super::wrap(&description(notice), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::muted(), content.width);
        }
        *row_index += 1;
    }
    if !expanded {
        return;
    }
    for line in super::wrap(&range_line(notice), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::muted(), content.width);
        }
        *row_index += 1;
    }
    for line in super::wrap(&notice.summary, content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::muted(), content.width);
        }
        *row_index += 1;
    }
}
