//! 沙箱权限切换提示（permission switch notice）的物理行布局与绘制。
//!
//! 系统事件步骤卡片（与 [`super::compact`] / [`super::model_switch`] / [`super::rewind`] 共用
//! [`super::system_event::SystemCard`]）：
//! - 标题行：菱形标记 `◆`（`StepKind::System`）+ `Permission switched`（accent）；
//! - 元信息默认可见：目标权限/时间、完整跃迁链（`Read-only -> Workspace -> Full`）；
//! - 正文为注入的 `<system-update>` 文本（折叠 / 展开同工具输出与模型切换卡片）。

use crate::app::state::PermissionSwitchNoticeView;
use crate::ui::main_content::lx::history::tool::Segment;
use crate::ui::main_content::lx::history::tool::tree::Node;
use crate::ui::main_content::lx::history::{StepKind, StepMark};
use crate::ui::{style, text};

use super::system_event::{SystemCard, body_width};

/// 完整跃迁链（首行元信息，不随正文折叠）：`switched A -> B -> C`。
fn transition_line(slice: &[PermissionSwitchNoticeView]) -> String {
    let Some(first) = slice.first() else {
        return String::new();
    };
    if slice.len() <= 1 {
        return first.transition_line();
    }
    let mut chain = format!("switched {}", first.from_short_label());
    for notice in slice {
        chain.push_str(" -> ");
        chain.push_str(notice.to_short_label());
    }
    chain
}

/// 最后一次注入的权限切换提示词（正文折叠项）。
fn injected_prompt(slice: &[PermissionSwitchNoticeView]) -> &str {
    slice
        .last()
        .map(|notice| notice.injected_prompt.trim())
        .unwrap_or("")
}

/// 卡片（折叠 / 展开同一构造器）。
pub(super) fn build(
    slice: &[PermissionSwitchNoticeView],
    width: u16,
    expanded: bool,
) -> SystemCard {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let header = vec![Segment::new(
        format!("{mark}{}", title(slice.len())),
        style::accent(),
    )];
    let meta = vec![
        Node::text(head_line(slice), style::muted()),
        Node::text(transition_line(slice), style::muted()),
    ];
    let prompt = super::tool::muted_markdown_lines(injected_prompt(slice), body_width(width));
    SystemCard::new(width, header, meta, prompt, expanded)
}

/// 标题（连续多条合并为 `Permission switched xN`）。
pub(super) fn title(count: usize) -> String {
    text::permission_switch_notice_title(count)
}

/// 目标权限模式与时间。
pub(super) fn head_line(slice: &[PermissionSwitchNoticeView]) -> String {
    let Some(last) = slice.last() else {
        return String::new();
    };
    text::permission_switch_notice_head(
        last.to_short_label(),
        &crate::platform::local_time_label(last.created_at_ms),
    )
}
