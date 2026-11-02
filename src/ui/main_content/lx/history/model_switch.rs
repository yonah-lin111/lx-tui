//! 模型切换提示（model switch notice）的物理行布局与绘制。
//!
//! 系统事件步骤卡片（与 [`super::compact`] / [`super::rewind`] 共用
//! [`super::system_event::SystemCard`]）：
//! - 标题行：菱形标记 `◆`（`StepKind::System`）+ `Model switched`（accent）；
//! - 元信息默认可见：目标模型/时间、指令源、完整跃迁链（`A -> B -> A` 不退化）；
//! - 正文只有注入的系统提示词是折叠项：折叠态保留前
//!   [`super::system_event::CONTENT_PREVIEW_ROWS`] 行 + `… N more lines`，
//!   展开后铺开全部（压暗 Markdown，与工具输出同一套交互）。
//!
//! 行数、可点击性与绘制都从 [`SystemCard`] 取。

use ratatui::layout::Rect;

use crate::app::state::ModelSwitchNoticeView;
use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::tool::Segment;
use crate::ui::main_content::lx::history::tool::tree::Node;
use crate::ui::main_content::lx::history::{StepKind, StepMark};
use crate::ui::{style, text};

use super::system_event::{SystemCard, body_width};

/// 合并区间内去重保序的变更指令源（中间切换的 `core/model` 不能丢）。
fn merged_sources(slice: &[ModelSwitchNoticeView]) -> Vec<String> {
    let mut sources: Vec<String> = Vec::new();
    for notice in slice {
        for source in &notice.updated_sources {
            if !sources.contains(source) {
                sources.push(source.clone());
            }
        }
    }
    sources
}

/// 完整跃迁链（首行元信息，不随正文折叠）：`switched A -> B -> A`。
fn transition_line(slice: &[ModelSwitchNoticeView]) -> String {
    let Some(last) = slice.last() else {
        return String::new();
    };
    if slice.len() <= 1 {
        return last.transition_line(None);
    }
    // 连续切换记录完整链：`A -> B -> A`，而不是退化成 `switched A -> A`。
    let mut chain = String::from("switched");
    if let Some(start) = slice[0]
        .from_label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
    {
        chain.push(' ');
        chain.push_str(start);
    }
    for notice in slice {
        chain.push_str(" -> ");
        chain.push_str(&notice.target_display_with_variant());
    }
    chain
}

/// 最后一次注入的系统提示词（正文折叠项）。
fn injected_prompt(slice: &[ModelSwitchNoticeView]) -> &str {
    slice
        .last()
        .map(|notice| notice.injected_prompt.trim())
        .unwrap_or("")
}

/// 卡片（折叠 / 展开同一构造器）。
pub(super) fn build(slice: &[ModelSwitchNoticeView], width: u16, expanded: bool) -> SystemCard {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let header = vec![Segment::new(
        format!("{mark}{}", title(slice.len())),
        style::accent(),
    )];
    let mut meta = vec![
        Node::text(head_line(slice), style::muted()),
        Node::text(transition_line(slice), style::muted()),
    ];
    if let Some(sources) = text::model_switch_notice_sources(&merged_sources(slice)) {
        meta.insert(1, Node::text(sources, style::muted()));
    }
    let prompt = super::tool::muted_markdown_lines(injected_prompt(slice), body_width(width));
    SystemCard::new(width, header, meta, prompt, expanded)
}

/// 标题（连续多条合并为 `Model switched xN`）。
pub(super) fn title(count: usize) -> String {
    text::model_switch_notice_title(count)
}

/// 目标模型 / 思考等级与时间。
pub(super) fn head_line(slice: &[ModelSwitchNoticeView]) -> String {
    let Some(last) = slice.last() else {
        return String::new();
    };
    let variant_only = slice.len() == 1 && last.is_variant_only();
    text::model_switch_notice_head(
        variant_only,
        &last.to_label,
        last.to_variant.as_deref(),
        &crate::platform::local_time_label(last.created_at_ms),
    )
}

/// 绘制模型切换提示并推进内容行号。
pub(super) fn draw(
    grid: &mut Grid,
    slice: &[ModelSwitchNoticeView],
    expanded: bool,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
) {
    if slice.is_empty() {
        return;
    }
    build(slice, content.width, expanded).draw(grid, content, scroll, row_index);
}
