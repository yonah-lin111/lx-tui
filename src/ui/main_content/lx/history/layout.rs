//! 历史布局：把快照的内容行区间映射成屏幕矩形。
//!
//! 从 `history.rs` 拆出（单文件行数上限）。行数与卡片几何全部来自
//! [`PaneCache`] 的快照，`layout` / `render` / 命中读同一份数据，不再各自排版。

use ratatui::layout::Rect;

use crate::app::state::Lx;

use super::cache::PaneCache;
use super::snapshot::HistorySnapshot;
use super::{CardHit, HistoryLayout, content_rect, rows_to_u16};

/// 计算历史布局（按当前展开状态）。`area` 为整个历史区（含滚动条列）。
pub(crate) fn layout(area: Rect, lx: &Lx, cache: &mut PaneCache) -> HistoryLayout {
    let content = content_rect(area);
    let snapshot = cache.snapshot(content.width, lx);
    to_layout(area, content.width, &snapshot)
}

/// 快照 → 现有 [`HistoryLayout`]（字段语义不变）。
pub(super) fn to_layout(area: Rect, width: u16, snapshot: &HistorySnapshot) -> HistoryLayout {
    if width == 0 {
        return HistoryLayout {
            area,
            thinking: Vec::new(),
            cards: Vec::new(),
            messages: Vec::new(),
            total_rows: 0,
        };
    }
    let rect_at = |start: usize, rows: usize| -> Rect {
        Rect::new(
            area.x,
            area.y.saturating_add(rows_to_u16(start)),
            width,
            rows_to_u16(rows),
        )
    };
    HistoryLayout {
        area,
        thinking: snapshot
            .thinking
            .iter()
            .map(|(message, step, (start, rows))| (*message, *step, rect_at(*start, *rows)))
            .collect(),
        cards: snapshot
            .cards
            .iter()
            .map(|region| CardHit {
                kind: region.kind.clone(),
                card: rect_at(region.card_start, region.card_rows),
                clickable: region.clickable,
            })
            .collect(),
        messages: snapshot.messages.clone(),
        total_rows: snapshot.total_rows,
    }
}
