//! 系统事件卡片（`/compact`、`/model`、`/rewind`）共用的「标题 + 树体」排版。
//!
//! ```text
//! ◆ Model switched
//!   ├─ model: gpt-5 · 16:59
//!   └─ instructions: core/model, core/agents_md
//! ```
//!
//! 标题是唯一不带连接符的行（菱形标记 + 事件名，accent）；详情节点走工具卡同一套
//! [`tree`]（`├─` / `└─` / `│`），续行按节点收口规则补竖线或等宽空白。行数、绘制
//! 与快照都从同一份 [`SystemCard`] 取，避免滚动条与视口锚点漂移。

use ratatui::layout::Rect;

use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::tool::tree::{self, Layout, Node, Section};
use crate::ui::main_content::lx::history::tool::{Segment, put_segments, wrap_spans};

use super::visible_row;

/// 一张系统事件卡片：标题行（已折行）+ 树体（已排版）。
pub(super) struct SystemCard {
    header: Vec<Vec<Segment>>,
    body: Layout,
}

impl SystemCard {
    /// 按宽度排版标题与详情节点；`nodes` 至少一个，空列表等价于没有详情。
    pub(super) fn new(width: u16, header: Vec<Segment>, nodes: Vec<Node>) -> Self {
        Self {
            header: wrap_spans(&header, usize::from(width)),
            body: tree::layout(&[Section::fixed(nodes)], width),
        }
    }

    /// 卡片占用的物理行数（标题 + 树体）。
    pub(super) fn rows(&self) -> usize {
        self.header.len() + self.body.rows.len()
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
