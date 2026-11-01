//! 模型切换提示（model switch notice）的物理行布局与绘制。
//!
//! 与 [`super::compact`] 同一套口径：
//! - 第 1 行（`style::accent()`）：`-- Model switched --`（连续多条合并为 `-- Model switched xN --`）；
//! - 第 2 行（`style::muted()`）：目标模型（或思考等级）、时间与变更指令源；
//! - 展开态：使用思考块同款的压暗 Markdown（[`super::tool::muted_markdown_lines`]）渲染
//!   模型切换对比与注入的系统提示词。
//!
//! 行数计算 [`rows`] 与绘制 [`draw`] 严格共用同一套换行与 Markdown 渲染逻辑。

use ratatui::layout::Rect;

use crate::app::state::ModelSwitchNoticeView;
use crate::ui::main_content::lx::Grid;
use crate::ui::{style, text};

use super::visible_row;

/// 构造展开态的 Markdown 源文本（首段为前后模型对比，其后为注入的提示词）。
fn expanded_markdown(notice: &ModelSwitchNoticeView, from_override: Option<&str>) -> String {
    let transition = notice.transition_line(from_override);
    if notice.injected_prompt.trim().is_empty() {
        transition
    } else {
        format!("{transition}\n\n{}", notice.injected_prompt.trim())
    }
}

/// 提示占用的物理行数（与 [`draw`] 严格同口径）。
pub(super) fn rows(
    notice: &ModelSwitchNoticeView,
    from_override: Option<&str>,
    count: usize,
    width: u16,
    expanded: bool,
) -> usize {
    let mut total = super::wrap(&title(count), width).len();
    total += super::wrap(&description(notice, count), width).len();
    if expanded {
        let md = expanded_markdown(notice, from_override);
        total += super::tool::muted_markdown_lines(&md, width).len();
    }
    total
}

/// 标题（连续多条合并为 `-- Model switched xN --`）。
pub(super) fn title(count: usize) -> String {
    text::model_switch_notice_title(count)
}

/// 头部描述：模型/思考等级、时间与更新的指令源。
pub(super) fn description(notice: &ModelSwitchNoticeView, count: usize) -> String {
    let variant_only = count == 1 && notice.is_variant_only();
    text::model_switch_notice_desc(
        variant_only,
        &notice.to_label,
        notice.to_variant.as_deref(),
        &crate::platform::local_time_label(notice.created_at_ms),
        &notice.updated_sources,
    )
}

/// 绘制模型切换提示并推进内容行号。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    notice: &ModelSwitchNoticeView,
    from_override: Option<&str>,
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
    for line in super::wrap(&description(notice, count), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::muted(), content.width);
        }
        *row_index += 1;
    }
    if !expanded {
        return;
    }
    let md = expanded_markdown(notice, from_override);
    for segments in super::tool::muted_markdown_lines(&md, content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            super::tool::put_segments(grid, content.x, y, content.width, &segments);
        }
        *row_index += 1;
    }
}
