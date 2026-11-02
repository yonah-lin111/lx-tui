//! 独立撤销提示（persisted tip）的物理行布局与绘制。
//!
//! 系统事件步骤卡片：
//! - 折叠态为单行紧凑条目：行首菱形标记 `◆`（`StepKind::System`）+ 标题（accent）+
//!   ` · ` 分隔的描述（muted），如 `◆ Rewind · restored 3 files`；
//! - 窄屏按列换行，路径不拼进未净化文本。

use ratatui::layout::Rect;

use crate::app::state::RewindNoticeView;
use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::tool::Segment;
use crate::ui::main_content::lx::history::{StepKind, StepMark};
use crate::ui::{style, text};

use super::visible_row;

/// 组装头部单行分段。
fn header_segments(notice: &RewindNoticeView, count: usize) -> Vec<Segment> {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let title = title(count);
    let desc = description(notice);
    vec![
        Segment::new(format!("{mark}{title}"), style::accent()),
        Segment::new(format!(" · {desc}"), style::muted()),
    ]
}

/// 提示占用的行数（与 [`draw`] 同口径）；`count > 1` 表示合并后的连续提示。
pub(super) fn rows(notice: &RewindNoticeView, count: usize, width: u16) -> usize {
    super::tool::wrap_spans(&header_segments(notice, count), usize::from(width)).len()
}

/// 提示标题（连续提示合并为 `Rewind xN`）。
pub(super) fn title(count: usize) -> String {
    text::rewind_notice_title(count)
}

/// 提示描述。
pub(super) fn description(notice: &RewindNoticeView) -> String {
    text::rewind_notice(
        &notice.kind,
        notice.restored_file_count,
        notice.files_already_restored,
    )
}

/// 绘制提示并推进内容行号；`count` 为合并的连续提示数。
pub(super) fn draw(
    grid: &mut Grid,
    notice: &RewindNoticeView,
    count: usize,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
) {
    for line in super::tool::wrap_spans(&header_segments(notice, count), usize::from(content.width))
    {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            super::tool::put_segments(grid, content.x, y, content.width, &line);
        }
        *row_index += 1;
    }
}
