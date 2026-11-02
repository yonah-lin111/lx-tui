//! 独立撤销提示（persisted tip）的物理行布局与绘制。
//!
//! 系统事件步骤卡片（与 [`super::compact`] / [`super::model_switch`] 共用
//! [`super::system_event::SystemCard`]）：
//! - 标题行：菱形标记 `◆`（`StepKind::System`）+ `Rewind`（accent）；
//! - 详情行：`  └─ <一句话描述>`（muted），窄屏按列折行、路径不拼进未净化文本。

use ratatui::layout::Rect;

use crate::app::state::RewindNoticeView;
use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::tool::Segment;
use crate::ui::main_content::lx::history::tool::tree::{Node, Section};
use crate::ui::main_content::lx::history::{StepKind, StepMark};
use crate::ui::{style, text};

use super::system_event::SystemCard;

/// 卡片（描述是唯一详情，没有可折叠正文）。
fn card(notice: &RewindNoticeView, count: usize, width: u16) -> SystemCard {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let header = vec![Segment::new(
        format!("{mark}{}", title(count)),
        style::accent(),
    )];
    let sections = vec![Section::fixed(vec![Node::text(
        description(notice),
        style::muted(),
    )])];
    SystemCard::new(width, header, sections, false)
}

/// 提示占用的行数（与 [`draw`] 同口径）；`count > 1` 表示合并后的连续提示。
pub(super) fn rows(notice: &RewindNoticeView, count: usize, width: u16) -> usize {
    card(notice, count, width).rows()
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
    card(notice, count, content.width).draw(grid, content, scroll, row_index);
}
