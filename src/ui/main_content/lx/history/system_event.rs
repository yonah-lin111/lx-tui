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
//! 树体走 [`tree`]（`├─` / `└─` / `│`），行数、点击面与绘制都从同一份 [`SystemCard`]
//! 取：点击面只覆盖可折叠正文（与命令卡片参数/输出的点击区同一口径），标题与元信息
//! 不响应点击。

use ratatui::layout::Rect;

use crate::ui::main_content::lx::Grid;
use crate::ui::main_content::lx::history::tool::tree::{self, Layout, Node, Preview, Section};
use crate::ui::main_content::lx::history::tool::{Segment, put_segments, wrap_spans};

use super::visible_row;

/// 内容折叠预算：折叠态保留前 N 个物理行（与普通工具详情同一口径）。
pub(super) const CONTENT_PREVIEW_ROWS: usize = 2;

/// 一张系统事件卡片：标题行（已折行）+ 树体（已排版）。
#[derive(Debug)]
pub(crate) struct SystemCard {
    header: Vec<Vec<Segment>>,
    body: Layout,
    /// 可折叠正文在卡片内的行区间（相对卡首，含 `… N more lines` 提示行）。
    click: Option<(usize, usize)>,
}

impl SystemCard {
    /// 按宽度排版标题与树体分块；`fixed` 是常显元信息，`content` 是可折叠正文
    /// （已按 [`tree::content_width`] 折好的物理行，空表示没有正文）。
    pub(super) fn new(
        width: u16,
        header: Vec<Segment>,
        fixed: Vec<Node>,
        content: Vec<Vec<Segment>>,
        expanded: bool,
    ) -> Self {
        let foldable = content.len() > CONTENT_PREVIEW_ROWS;
        let fixed_count = fixed.len();
        let mut sections = vec![Section::fixed(fixed)];
        if !content.is_empty() {
            sections.push(Section::new(
                vec![Node::block(content)],
                (!expanded).then(|| Preview::head(CONTENT_PREVIEW_ROWS)),
            ));
        }
        let header = wrap_spans(&header, usize::from(width));
        let body = tree::layout(&sections, width);
        // 只有正文确实超出折叠预算时才登记点击面：点击区从正文首个物理行到卡片末尾
        // （含折叠提示行），标题与元信息保持不可点击。
        let click = foldable
            .then(|| {
                let first = body.node_rows.get(fixed_count).copied().unwrap_or(0);
                (header.len() + first, body.rows.len().saturating_sub(first))
            })
            .filter(|(_, rows)| *rows > 0);
        Self {
            header,
            body,
            click,
        }
    }

    /// 卡片占用的物理行数（标题 + 树体）。
    pub(super) fn rows(&self) -> usize {
        self.header.len() + self.body.rows.len()
    }

    /// 可折叠正文的点击区（相对卡首的起始行与行数）；没有可折叠内容时为 None。
    pub(super) fn click_span(&self) -> Option<(usize, usize)> {
        self.click
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
