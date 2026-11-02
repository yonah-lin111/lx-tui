//! 上下文压缩提示（compact checkpoint）的物理行布局与绘制。
//!
//! 系统事件步骤卡片（与 [`super::model_switch`] / [`super::rewind`] 共用
//! [`super::system_event::SystemCard`]）：
//! - 标题行：菱形标记 `◆`（`StepKind::System`）+ `Context compacted`（accent）；
//! - 元信息默认可见：`  ├─ model: gpt-5 · 10:42` + `  ├─ covered sequence 0–7`；
//! - 正文只有压缩摘要是折叠项：折叠态保留前
//!   [`super::system_event::CONTENT_PREVIEW_ROWS`] 行 + `… N more lines`，
//!   展开后铺开全部（压暗 Markdown，与工具输出同一套交互）；
//! - 运行中仍是单行状态：`◇ Compacting context... · 3s`。
//!
//! 快照行数、点击面与绘制数据都从 [`SystemCard`] 取，绝不各算一份。

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
pub(super) fn build(
    notice: &CompactNoticeView,
    count: usize,
    width: u16,
    expanded: bool,
) -> SystemCard {
    let mark = StepMark::Done.segment(StepKind::System, 0, false);
    let header = vec![Segment::new(
        format!("{mark}{}", title(count)),
        style::accent(),
    )];
    let fixed = vec![
        Node::text(description(notice), style::muted()),
        Node::text(range_line(notice), style::muted()),
    ];
    let summary = super::tool::muted_markdown_lines(&notice.summary, body_width(width));
    SystemCard::new(width, header, fixed, summary, expanded)
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

/// 覆盖范围行。
fn range_line(notice: &CompactNoticeView) -> String {
    let range = if notice.auto {
        text::LX_COMPACT_AUTO_RANGE.to_string()
    } else {
        notice.range_label()
    };
    format!("{} {range}", text::LX_COMPACT_RANGE_LABEL)
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
