//! 上下文压缩提示（compact checkpoint）的物理行布局与绘制。
//!
//! 系统事件步骤卡片：
//! - 折叠态为单行紧凑条目：行首菱形标记 `◆`（`StepKind::System`）+ 标题（accent）+
//!   ` · ` 分隔的描述（muted），如 `◆ Context compacted · model: gpt-5 · 10:42`；
//! - 运行中为琥珀色动态呼吸条目：`◇ Compacting context... · 3s`；
//! - 展开态下挂思考块同款树形连接符（`  └─ `）+ 5 列正文缩进 + 压暗 Markdown，
//!   追加覆盖范围与摘要正文。
//!
//! 行数计算 [`rows`] 与绘制 [`draw`] 严格共用同一套 `wrap_spans`。

use ratatui::layout::Rect;

use crate::app::state::{CompactNoticeView, LxCompact};
use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::tool::Segment;
use crate::ui::main_content::lx::history::{StepKind, StepMark};
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

/// 已完成检查点的头部单行分段。
fn header_segments(notice: &CompactNoticeView, count: usize) -> Vec<Segment> {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let title = title(count);
    let desc = description(notice);
    vec![
        Segment::new(format!("{mark}{title}"), style::accent()),
        Segment::new(format!(" · {desc}"), style::muted()),
    ]
}

/// 运行中压缩请求的头部单行分段。
fn running_header_segments(
    compact: &LxCompact,
    loading_frame: usize,
    animating: bool,
) -> Vec<Segment> {
    let mark = StepMark::Running.segment(StepKind::System, loading_frame, animating);
    let title = text::chat::COMPACTING_LABEL;
    let mut segments = vec![Segment::new(
        format!("{mark}{title}"),
        style::chat_step_mark_alive(),
    )];
    if let Some(elapsed) = compact.elapsed_ms.filter(|&ms| ms > 0) {
        let dur = text::agent::duration_label(elapsed as u64);
        segments.push(Segment::new(format!(" · {dur}"), style::muted()));
    }
    segments
}

/// 提示占用的行数（与 [`draw`] 同口径）。
pub(super) fn rows(notice: &CompactNoticeView, count: usize, width: u16, expanded: bool) -> usize {
    let mut total =
        super::tool::wrap_spans(&header_segments(notice, count), usize::from(width)).len();
    if expanded {
        let (_, body_width) = tree_body_geometry(width);
        total += super::wrap(&range_line(notice), body_width).len();
        total += super::tool::muted_markdown_lines(&notice.summary, body_width).len();
    }
    total
}

/// 运行中压缩占用的行数（与 [`draw_running`] 同口径）。
pub(super) fn running_rows(compact: &LxCompact, width: u16) -> usize {
    let segments = running_header_segments(compact, 0, false);
    super::tool::wrap_spans(&segments, usize::from(width)).len()
}

/// 标题（连续多条合并为 `Context compacted xN`）。
pub(super) fn title(count: usize) -> String {
    text::compact_notice_title(count)
}

/// 头部描述：模型与时间；已撤销的条目明确标注。
pub(super) fn description(notice: &CompactNoticeView) -> String {
    let base = text::compact_notice(
        &notice.model_label,
        &crate::platform::local_time_label(notice.created_at_ms),
    );
    if notice.active {
        base
    } else {
        format!("{base} · reverted")
    }
}

/// 展开后的覆盖范围行。
fn range_line(notice: &CompactNoticeView) -> String {
    let range = if notice.auto {
        text::LX_COMPACT_AUTO_RANGE.to_string()
    } else {
        notice.range_label()
    };
    format!("{} {range}", text::LX_COMPACT_RANGE_LABEL)
}

/// 绘制提示并推进内容行号；`count` 为合并的连续提示数。
pub(super) fn draw(
    grid: &mut Grid,
    notice: &CompactNoticeView,
    count: usize,
    expanded: bool,
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
    if !expanded {
        return;
    }
    let (indent, body_width) = tree_body_geometry(content.width);
    let tree = super::thinking_tree(content.width);
    // 只有展开态的第一个物理行画连接符；首行被滚出视口时不再补画。
    let mut first_expanded_row = true;
    for line in super::wrap(&range_line(notice), body_width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            if first_expanded_row && tree {
                put_tree_leaf(grid, content, y);
            }
            grid.put_clipped(
                content.x.saturating_add(indent),
                y,
                &line,
                style::muted(),
                body_width,
            );
        }
        first_expanded_row = false;
        *row_index += 1;
    }
    for segments in super::tool::muted_markdown_lines(&notice.summary, body_width) {
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

/// 绘制正在运行的压缩提示并推进行号。
pub(super) fn draw_running(
    grid: &mut Grid,
    compact: &LxCompact,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
    loading_frame: usize,
    animating: bool,
) {
    let segments = running_header_segments(compact, loading_frame, animating);
    for line in super::tool::wrap_spans(&segments, usize::from(content.width)) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            super::tool::put_segments(grid, content.x, y, content.width, &line);
        }
        *row_index += 1;
    }
}
