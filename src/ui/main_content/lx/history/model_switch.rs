//! 模型切换提示（model switch notice）的物理行布局与绘制。
//!
//! 与 [`super::compact`] 同一套口径：
//! - 第 1 行（`style::accent()`）：`-- Model switched --`（连续多条合并为 `-- Model switched xN --`）；
//! - 第 2 行（`style::muted()`）：目标模型（或思考等级）、时间与合并区间内全部变更指令源；
//! - 展开态：思考块同款的 `  └─ ` 树形连接符（[`text::agent::TREE_LEAF`]）+ 5 列正文缩进
//!   （[`super::thinking_body_indent`]）+ 压暗 Markdown（[`super::tool::muted_markdown_lines`]），
//!   渲染完整跃迁链（`A -> B -> A` 不退化）与最后一次注入的系统提示词。
//!
//! 行数计算 [`rows`] 与绘制 [`draw`] 严格共用同一套换行与 Markdown 渲染逻辑。

use ratatui::layout::Rect;

use crate::app::state::ModelSwitchNoticeView;
use crate::ui::main_content::lx::Grid;
use crate::ui::{style, text};

use super::visible_row;

/// 展开正文几何：树形前缀占满 [`super::thinking_body_indent`] 列，正文按剩余宽度换行。
fn tree_body_geometry(width: u16) -> (u16, u16) {
    let indent = super::thinking_body_indent(width);
    let body_width = width.saturating_sub(indent).max(1);
    (indent, body_width)
}

/// 展开正文首个物理行前的树形连接符（与思考块同款 `  └─ `）。
fn put_tree_leaf(grid: &mut Grid, content: Rect, y: u16) {
    grid.put_clipped(
        content.x,
        y,
        text::agent::TREE_LEAF,
        style::chat_notice_connector(),
        content.width,
    );
}

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

/// 提示占用的物理行数（与 [`draw`] 严格同口径）。
pub(super) fn rows(slice: &[ModelSwitchNoticeView], width: u16, expanded: bool) -> usize {
    if slice.is_empty() {
        return 0;
    }
    let mut total = super::wrap(&title(slice.len()), width).len();
    total += super::wrap(&description(slice), width).len();
    if expanded {
        let (_, body_width) = tree_body_geometry(width);
        total += super::tool::muted_markdown_lines(&expanded_markdown(slice), body_width).len();
    }
    total
}

/// 标题（连续多条合并为 `-- Model switched xN --`）。
pub(super) fn title(count: usize) -> String {
    text::model_switch_notice_title(count)
}

/// 头部描述：模型/思考等级、时间与区间内去重保序的全部变更指令源。
pub(super) fn description(slice: &[ModelSwitchNoticeView]) -> String {
    let Some(last) = slice.last() else {
        return String::new();
    };
    let variant_only = slice.len() == 1 && last.is_variant_only();
    text::model_switch_notice_desc(
        variant_only,
        &last.to_label,
        last.to_variant.as_deref(),
        &crate::platform::local_time_label(last.created_at_ms),
        &merged_sources(slice),
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
    for line in super::wrap(&title(slice.len()), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::accent(), content.width);
        }
        *row_index += 1;
    }
    for line in super::wrap(&description(slice), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::muted(), content.width);
        }
        *row_index += 1;
    }
    if !expanded {
        return;
    }
    let (indent, body_width) = tree_body_geometry(content.width);
    let tree = super::thinking_tree(content.width);
    // 只有展开态的第一个物理行画连接符；首行被滚出视口时不再补画。
    let mut first_expanded_row = true;
    for segments in super::tool::muted_markdown_lines(&expanded_markdown(slice), body_width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            if first_expanded_row && tree {
                put_tree_leaf(grid, content, y);
            }
            super::tool::put_segments(
                grid,
                content.x.saturating_add(indent),
                y,
                body_width,
                &segments,
            );
        }
        first_expanded_row = false;
        *row_index += 1;
    }
}
