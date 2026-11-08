//! 消息尾部 recap 系统事件卡（`◆ Recap`）与生成中占位行。
//!
//! 复用 [`SystemCard`]：标题 + 常显元信息（模型/时间、Next）+ 可折叠摘要。
//! 摘要走思考块同款压暗 Markdown；生成中只画一行 `◇ Generating recap... · Ns`。

use crate::app::state::lx::{MessageRecapView, RecapPending};
use crate::ui::main_content::lx::history::tool::Segment;
use crate::ui::main_content::lx::history::tool::tree::Node;
use crate::ui::main_content::lx::history::{StepKind, StepMark};
use crate::ui::{style, text};

use super::system_event::{SystemCard, body_width};

/// 已完成 recap 卡。
pub(super) fn build(recap: &MessageRecapView, width: u16, expanded: bool) -> SystemCard {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let header = vec![Segment::new(
        format!("{mark}{}", text::LX_RECAP_TITLE),
        style::accent(),
    )];
    let mut fixed = vec![Node::text(
        format!(
            "{}{} · {}",
            text::LX_RECAP_MODEL_PREFIX,
            recap.model_label,
            crate::platform::local_time_label(recap.created_at_ms)
        ),
        style::muted(),
    )];
    if !recap.next_action.trim().is_empty() {
        fixed.push(Node::new(vec![
            Segment::new(text::LX_RECAP_NEXT_PREFIX, style::muted()),
            Segment::new(recap.next_action.clone(), style::chat_tool_param()),
        ]));
    }
    let summary = super::tool::muted_markdown_lines(&recap.summary, body_width(width));
    SystemCard::new(width, header, fixed, summary, expanded)
}

/// 生成中占位行（单行；`elapsed_ms` 由 tick 每秒更新）。
pub(super) fn running_line(pending: &RecapPending) -> String {
    // 强制空心相：占位行不是「已完成」，不能画实心菱形。
    let mark = StepMark::Running.segment(StepKind::System, text::agent::STEP_BREATH_TICKS, true);
    format!(
        "{mark}{} · {}s",
        text::LX_RECAP_RUNNING,
        pending.elapsed_ms / 1000
    )
}
