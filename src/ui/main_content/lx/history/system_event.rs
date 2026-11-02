//! 系统事件卡片（`/compact`、`/model`、`/rewind`）共用的「标题 + 树体」排版。
//!
//! ```text
//! ◆ Model switched
//!   ├─ model: gpt-5 · 16:59
//!   ├─ instructions: core/model, core/agents_md
//!   └─ switched Claude Sonnet 4 -> GPT-5
//!      … 6 more lines
//! ```
//!
//! 标题是唯一不带连接符的行（菱形标记 + 事件名，accent）；元信息节点始终可见，
//! 只有正文内容（压缩摘要 / 注入提示词）按工具卡同一套预算折叠：折叠态保留前
//! [`super::tool::tree::Preview`] 行 + `… N more lines`，展开后铺开全部。
//! 树体走 [`tree`]（`├─` / `└─` / `│`），行数、绘制与快照都从同一份 [`SystemCard`]
//! 取，避免滚动条与视口锚点漂移。

use ratatui::layout::Rect;

use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::tool::tree::{self, Layout, Section};
use crate::ui::main_content::lx::history::tool::{Segment, put_segments, wrap_spans};

use super::visible_row;

/// 内容折叠预算：折叠态保留前 N 个物理行（与普通工具详情同一口径）。
pub(super) const CONTENT_PREVIEW_ROWS: usize = 2;

/// 一张系统事件卡片：标题行（已折行）+ 树体（已排版）。
pub(super) struct SystemCard {
    header: Vec<Vec<Segment>>,
    body: Layout,
    /// 正文是否超出折叠预算：决定整卡是否登记点击面。
    foldable: bool,
}

impl SystemCard {
    /// 按宽度排版标题与树体分块；`foldable` 表示正文存在折叠预算之外的
    /// 物理行（展开态点一下仍应收起）。
    pub(super) fn new(
        width: u16,
        header: Vec<Segment>,
        sections: Vec<Section>,
        foldable: bool,
    ) -> Self {
        Self {
            header: wrap_spans(&header, usize::from(width)),
            body: tree::layout(&sections, width),
            foldable,
        }
    }

    /// 卡片占用的物理行数（标题 + 树体）。
    pub(super) fn rows(&self) -> usize {
        self.header.len() + self.body.rows.len()
    }

    /// 正文是否有可折叠内容（没有时不登记点击面）。
    pub(super) fn foldable(&self) -> bool {
        self.foldable
    }

    /// 绘制并推进内容行号；行数恒等于 [`Self::rows`]。
    pub(super) fn draw(
        &self,
        grid: &mut Grid,
        content: Rect,
        scroll: usize,
        row_index: &mut usize,
    ) {
        for line in &self.header {
            if let Some(y) = visible_row(content, scroll, *row_index) {
                put_segments(grid, content.x, y, content.width, line);
            }
            *row_index += 1;
        }
        tree::draw(grid, content, scroll, row_index, &self.body.rows);
    }
}

/// 详情正文的可用列数（与 [`tree::layout`] 的折行口径一致）。
pub(super) fn body_width(width: u16) -> u16 {
    u16::try_from(tree::content_width(width)).unwrap_or(u16::MAX)
}
