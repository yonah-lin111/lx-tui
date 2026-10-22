//! 独立撤销提示（persisted tip）的物理行布局与绘制。
//!
//! 两行通知带：`-- Rewind --`（accent）+ 描述（muted）。不是 user/assistant 消息，
//! 不进入复制选区与模型上下文；窄屏按列换行，路径不拼进未净化文本。

use ratatui::layout::Rect;

use crate::app::state::RewindNoticeView;
use crate::ui::{style, text};

use super::visible_row;
use crate::ui::main_content::lx::Grid;

/// 提示占用的行数（与 [`draw`] 同口径）。
pub(super) fn rows(notice: &RewindNoticeView, width: u16) -> usize {
    super::wrap(text::LX_REWIND_NOTICE_TITLE, width).len()
        + super::wrap(&description(notice), width).len()
}

/// 分布式提示描述。
pub(super) fn description(notice: &RewindNoticeView) -> String {
    text::rewind_notice(
        &notice.kind,
        notice.restored_file_count,
        notice.files_already_restored,
    )
}

/// 绘制提示并推进内容行号。
pub(super) fn draw(
    grid: &mut Grid,
    notice: &RewindNoticeView,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
) {
    for line in super::wrap(text::LX_REWIND_NOTICE_TITLE, content.width) {
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
}
