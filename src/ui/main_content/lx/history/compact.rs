//! 上下文压缩提示（compact checkpoint）的物理行布局与绘制。
//!
//! 系统事件步骤卡片（与 [`super::model_switch`] / [`super::rewind`] 共用
//! [`super::system_event::SystemCard`]）：
//! - 标题行：菱形标记 `◆`（`StepKind::System`）+ `Context compacted`（accent）；
//! - 折叠态：标题下挂一行详情 `  └─ model: gpt-5 · 10:42`（muted）；
//! - 展开态：追加覆盖范围与压暗 Markdown 摘要，多个节点走 `├─` / `└─` 树体；
//! - 运行中仍是单行状态：`◇ Compacting context... · 3s`。
//!
//! 行数计算 [`rows`] 与绘制 [`draw`] 都从 [`SystemCard`] 取，绝不各算一份。

use ratatui::layout::Rect;

use crate::app::state::{CompactNoticeView, LxCompact};
use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::tool::Segment;
use crate::ui::main_content::lx::history::tool::tree::Node;
use crate::ui::main_content::lx::history::{StepKind, StepMark};
use crate::ui::{style, text};

use super::system_event::{SystemCard, body_width};
use super::visible_row;

/// 已完成检查点的卡片（折叠 / 展开同一构造器）。
fn card(notice: &CompactNoticeView, count: usize, width: u16, expanded: bool) -> SystemCard {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let header = vec![Segment::new(
        format!("{mark}{}", title(count)),
        style::accent(),
    )];
    let mut nodes = vec![Node::text(description(notice), style::muted())];
    if expanded {
        nodes.push(Node::text(range_line(notice), style::muted()));
        nodes.push(Node::block(super::tool::muted_markdown_lines(
            &notice.summary,
            body_width(width),
        )));
    }
    SystemCard::new(width, header, nodes)
}

/// 运行中压缩请求的单行分段。
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
    card(notice, count, width, expanded).rows()
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
    card(notice, count, content.width, expanded).draw(grid, content, scroll, row_index);
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
