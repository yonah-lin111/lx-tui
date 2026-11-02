//! 模型切换提示（model switch notice）的物理行布局与绘制。
//!
//! 系统事件步骤卡片（与 [`super::compact`] / [`super::rewind`] 共用
//! [`super::system_event::SystemCard`]）：
//! - 标题行：菱形标记 `◆`（`StepKind::System`）+ `Model switched`（accent）；
//! - 折叠态：`  ├─ model: X (level) · 16:59` + `  └─ instructions: core/model, …`；
//! - 展开态：末尾追加完整跃迁链（`A -> B -> A` 不退化）与最后一次注入的系统提示词，
//!   正文采用思考块同款的压暗 Markdown。
//!
//! 行数计算 [`rows`] 与绘制 [`draw`] 都从 [`SystemCard`] 取。

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

/// 构造展开态的 Markdown 源文本（首行为完整跃迁链，其后为最后一次注入的提示词）。
fn expanded_markdown(slice: &[ModelSwitchNoticeView]) -> String {
    let Some(last) = slice.last() else {
        return String::new();
    };
    let transition = if slice.len() <= 1 {
        last.transition_line(None)
    } else {
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
    };
    if last.injected_prompt.trim().is_empty() {
        transition
    } else {
        format!("{transition}\n\n{}", last.injected_prompt.trim())
    }
}

/// 卡片（折叠 / 展开同一构造器）。
fn card(slice: &[ModelSwitchNoticeView], width: u16, expanded: bool) -> SystemCard {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let header = vec![Segment::new(
        format!("{mark}{}", title(slice.len())),
        style::accent(),
    )];
    let mut nodes = vec![Node::text(head_line(slice), style::muted())];
    if let Some(sources) = text::model_switch_notice_sources(&merged_sources(slice)) {
        nodes.push(Node::text(sources, style::muted()));
    }
    if expanded {
        nodes.push(Node::block(super::tool::muted_markdown_lines(
            &expanded_markdown(slice),
            body_width(width),
        )));
    }
    SystemCard::new(width, header, nodes)
}

/// 提示占用的物理行数（与 [`draw`] 严格同口径）。
pub(super) fn rows(slice: &[ModelSwitchNoticeView], width: u16, expanded: bool) -> usize {
    if slice.is_empty() {
        return 0;
    }
    card(slice, width, expanded).rows()
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
    card(slice, content.width, expanded).draw(grid, content, scroll, row_index);
}
