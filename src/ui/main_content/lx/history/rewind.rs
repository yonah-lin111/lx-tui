//! 独立撤销提示（persisted tip）的物理行布局与绘制。
//!
//! 系统事件步骤卡片（与 [`super::compact`] / [`super::model_switch`] 共用
//! [`super::system_event::SystemCard`]）：
//! - 标题行：菱形标记 `◆`（`StepKind::System`）+ `Rewind`（accent）；
//! - 详情行：`  └─ <一句话描述>`（muted），窄屏按列折行、路径不拼进未净化文本。

use crate::app::state::RewindNoticeView;
use crate::ui::main_content::lx::history::tool::Segment;
use crate::ui::main_content::lx::history::tool::tree::Node;
use crate::ui::main_content::lx::history::{StepKind, StepMark};
use crate::ui::{style, text};

use super::system_event::SystemCard;

/// 卡片（描述是唯一详情，没有可折叠正文）。
pub(super) fn build(notice: &RewindNoticeView, count: usize, width: u16) -> SystemCard {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let header = vec![Segment::new(
        format!("{mark}{}", title(count)),
        style::accent(),
    )];
    let fixed = vec![Node::text(description(notice), style::muted())];
    SystemCard::new(width, header, fixed, Vec::new(), false)
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
