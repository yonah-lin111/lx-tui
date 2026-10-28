//! 卡片树体共用排版：**逻辑节点 → 物理行 → 连接符**，行数、绘制与点击几何共用一份结果。
//!
//! ```text
//! ■ Ask · 2 questions
//!   ├─ Q1: 布局
//!   │  A1: 双栏
//!   └─ Q2: 主题
//!      A2: 深色, 用青色
//! ```
//!
//! - 一个 [`Node`] 是一段可含多行/多段样式的文本块：**首物理行**画 `├─`（后面还有
//!   节点）或 `└─`（最后一个节点），同节点内部的续行（第二行起、折行、折叠提示）
//!   画 `│`（后面还有节点）或等宽空白（最后一个节点的续行收口）；
//! - 多行节点用于「命令输出 / diff 正文 / 计划步骤 / 问答对」这类**一块内容**：
//!   它们只占一个连接符，内部行左对齐，不再每行重复 `├─`；
//! - [`Section`] 允许一张卡片拥有多个**独立折叠**的块（命令参数与输出、搜索摘要与
//!   命中），每块按自己的 [`Preview`] 折叠，省略提示只在该块确有隐藏内容时出现；
//! - 极窄宽度（放不下 [`TREE_COLS`]）整段让位：不画连接符，内容按整宽折行，标题与
//!   状态词仍然优先可见；
//! - 行数只有 [`Layout::rows`] 一个来源，`rows` / `draw` / 点击面都从这里取。

use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::ui::style;

use super::super::{Grid, visible_row};
use super::{Segment, TREE_COLS, put_segments, wrap_spans};
use crate::ui::text;

/// 折叠预览：保留前 `head` 行与后 `tail` 行（`tail == 0` 表示只保留前 `head` 行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Preview {
    pub(crate) head: usize,
    pub(crate) tail: usize,
}

impl Preview {
    /// 只保留头部若干行（文件摘要、搜索命中）。
    pub(crate) const fn head(head: usize) -> Self {
        Self { head, tail: 0 }
    }

    /// 首尾都保留（命令输出：首 3 + 尾 2）。
    pub(crate) const fn head_tail(head: usize, tail: usize) -> Self {
        Self { head, tail }
    }

    /// 折叠态展示的物理行数。
    fn shown(self, total: usize) -> usize {
        total.min(self.head.saturating_add(self.tail))
    }
}

/// 一个逻辑节点：一个或多个逻辑行，每行可含多段样式，按可用列宽折行。
#[derive(Debug, Clone)]
pub(crate) struct Node {
    lines: Vec<Vec<Segment>>,
}

impl Node {
    /// 单逻辑行节点（现有字段）。
    pub(crate) fn new(spans: Vec<Segment>) -> Self {
        Self { lines: vec![spans] }
    }

    /// 多逻辑行节点（命令输出 / diff / 计划步骤 / 问答对）：整块只占一个连接符。
    pub(crate) fn block(lines: Vec<Vec<Segment>>) -> Self {
        Self { lines }
    }

    /// 单段纯文本节点。
    pub(crate) fn text(value: impl Into<String>, style: Style) -> Self {
        Self::new(vec![Segment::new(value, style)])
    }
}

/// 卡片里一个可独立折叠的块。
#[derive(Debug, Clone)]
pub(crate) struct Section {
    pub(crate) nodes: Vec<Node>,
    /// 折叠预算；`None` 表示该块始终展开。
    pub(crate) preview: Option<Preview>,
}

impl Section {
    pub(crate) fn new(nodes: Vec<Node>, preview: Option<Preview>) -> Self {
        Self { nodes, preview }
    }

    /// 常显块（状态、参数外的元信息）。
    pub(crate) fn fixed(nodes: Vec<Node>) -> Self {
        Self {
            nodes,
            preview: None,
        }
    }
}

/// 一个物理行：连接符（可能为空）+ 内容。
pub(crate) struct Row {
    pub(crate) prefix: &'static str,
    pub(crate) segments: Vec<Segment>,
    /// 内容所属节点；省略提示行归属到它继续的那个节点。
    node: Option<usize>,
    /// 是否该节点的**首个**物理行（决定 `├─`/`└─`；续行只能 `│`/空白）。
    first: bool,
}

/// 一次排版的结果。
pub(crate) struct Layout {
    pub(crate) rows: Vec<Row>,
    /// 每个**可见**节点首行在 `rows` 中的下标（点击/链接几何用）。
    pub(crate) node_rows: Vec<usize>,
    /// 展开后的内容物理行数（不含省略提示行）。
    pub(crate) total: usize,
    /// 折叠态被隐藏的内容物理行数。
    pub(crate) hidden: usize,
}

impl Layout {
    /// 拼成「前缀 + 内容」的段行（沿用旧测试/调用方的 `Vec<Vec<Segment>>` 口径）。
    pub(crate) fn segments(&self) -> Vec<Vec<Segment>> {
        self.rows
            .iter()
            .map(|row| {
                let mut segments = Vec::with_capacity(row.segments.len() + 1);
                if !row.prefix.is_empty() {
                    segments.push(Segment::new(row.prefix, style::chat_notice_connector()));
                }
                segments.extend(row.segments.iter().cloned());
                segments
            })
            .collect()
    }
}

/// 极窄宽度下是否还画树体前缀。
fn keeps_tree(width: u16) -> bool {
    usize::from(width) > TREE_COLS
}

/// 内容可用列数。
pub(crate) fn content_width(width: u16) -> usize {
    if keeps_tree(width) {
        usize::from(width).saturating_sub(TREE_COLS).max(1)
    } else {
        usize::from(width).max(1)
    }
}

/// 树体排版；`sections` 按显示顺序排列，各自独立折叠。
pub(crate) fn layout(sections: &[Section], width: u16) -> Layout {
    let body = content_width(width);
    let tree = keeps_tree(width);
    let mut rows: Vec<Row> = Vec::new();
    let mut node_rows: Vec<usize> = Vec::new();
    let mut total: usize = 0;
    let mut hidden: usize = 0;
    if width == 0 {
        return Layout {
            rows,
            node_rows,
            total,
            hidden,
        };
    }
    let mut next_node = 0usize;

    for section in sections {
        // 本块的物理行（已折行），保持节点归属。
        struct Line {
            segments: Vec<Segment>,
            node: usize,
            first: bool,
        }
        let mut lines: Vec<Line> = Vec::new();
        for (index, node) in section.nodes.iter().enumerate() {
            let node_index = next_node + index;
            let mut first = true;
            for logical in &node.lines {
                for spans in wrap_spans(logical, body) {
                    lines.push(Line {
                        segments: spans,
                        node: node_index,
                        first,
                    });
                    first = false;
                }
            }
        }
        next_node += section.nodes.len();
        if lines.is_empty() {
            continue;
        }
        let section_total = lines.len();
        total += section_total;
        let shown = section
            .preview
            .map_or(section_total, |preview| preview.shown(section_total));
        let section_hidden = section_total.saturating_sub(shown);
        hidden += section_hidden;
        // 首尾预览：head 从前往后，tail 从后往前；不重复同一行。
        let head = section
            .preview
            .map_or(shown, |preview| preview.head.min(shown));
        let tail = shown.saturating_sub(head);
        let mut visible: Vec<usize> = (0..head).collect();
        visible.extend(section_total.saturating_sub(tail)..section_total);
        // 记录本块每个可见节点的首行位置（节点完全被隐藏时没有位置）。
        let mut seen: Vec<usize> = Vec::new();
        let mut last_visible: Option<usize> = None;
        for &index in &visible {
            let line = &lines[index];
            if line.first && !seen.contains(&line.node) {
                seen.push(line.node);
                node_rows.push(rows.len());
            }
            last_visible = Some(line.node);
            rows.push(Row {
                prefix: "",
                segments: line.segments.clone(),
                node: Some(line.node),
                first: line.first,
            });
        }
        if section_hidden > 0 {
            let hint = text::agent::file_diff_more_lines(section_hidden);
            if unicode_width::UnicodeWidthStr::width(hint.as_str()) <= body {
                // 提示行是本块最后可见节点的续行：仍用空白列，不再单独画一个
                // `└─`，否则长输出里每个折叠块都多一个收口符。
                rows.push(Row {
                    prefix: "",
                    segments: vec![Segment::new(hint, style::agent_step_detail())],
                    node: last_visible,
                    first: false,
                });
            }
        }
    }

    // 连接符按**节点**分配：首行 `├─`/`└─`，续行 `│`/空白；最后一个可见节点收口。
    let last_node = rows.iter().rev().find_map(|row| row.node);
    for row in &mut rows {
        if !tree {
            continue;
        }
        let Some(node) = row.node else {
            continue;
        };
        let last = Some(node) == last_node;
        row.prefix = match (last, row.first) {
            (true, true) => text::agent::TREE_LEAF,
            (true, false) => text::agent::TREE_PAD,
            (false, true) => text::agent::TREE_BRANCH,
            (false, false) => text::agent::TREE_BAR,
        };
    }
    Layout {
        rows,
        node_rows,
        total,
        hidden,
    }
}

/// 绘制树体并推进行号；推进行数恒等于 `rows.len()`。
pub(crate) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    rows: &[Row],
) {
    for row in rows {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            let mut segments: Vec<Segment> = Vec::with_capacity(row.segments.len() + 1);
            if !row.prefix.is_empty() {
                segments.push(Segment::new(row.prefix, style::chat_notice_connector()));
            }
            segments.extend(row.segments.iter().cloned());
            put_segments(grid, rect.x, y, rect.width, &segments);
        }
        *row_index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(text: &str) -> Node {
        Node::text(text, style::text())
    }

    #[test]
    fn prefixes_follow_node_boundaries_and_wrap() {
        let long = "x".repeat(20);
        let sections = vec![Section::fixed(vec![node(&long), node("tail")])];
        let layout = layout(&sections, 12);
        // 内容宽 7 列：第一节点折 3 行，第二节点 1 行。
        assert_eq!(layout.rows.len(), 4);
        assert_eq!(layout.rows[0].prefix, text::agent::TREE_BRANCH);
        assert_eq!(layout.rows[1].prefix, text::agent::TREE_BAR);
        assert_eq!(layout.rows[2].prefix, text::agent::TREE_BAR);
        assert_eq!(layout.rows[3].prefix, text::agent::TREE_LEAF);
    }

    #[test]
    fn last_node_continuation_is_padded() {
        // 最后一个节点收起后，续行用等宽空白，不再画竖线或重复收口符。
        let sections = vec![Section::fixed(vec![node("head"), node(&"y".repeat(20))])];
        let layout = layout(&sections, 12);
        let prefixes: Vec<&str> = layout.rows.iter().map(|row| row.prefix).collect();
        assert_eq!(prefixes[0], text::agent::TREE_BRANCH);
        assert_eq!(prefixes[1], text::agent::TREE_LEAF);
        assert_eq!(prefixes[2], text::agent::TREE_PAD);
        assert_eq!(prefixes[3], text::agent::TREE_PAD);
    }

    #[test]
    fn multi_line_node_uses_one_connector() {
        let block = Node::block(vec![
            vec![Segment::new("first", style::text())],
            vec![Segment::new("second", style::text())],
        ]);
        let sections = vec![Section::fixed(vec![node("head"), block])];
        let layout = layout(&sections, 40);
        assert_eq!(layout.rows.len(), 3);
        assert_eq!(layout.rows[0].prefix, text::agent::TREE_BRANCH);
        assert_eq!(layout.rows[1].prefix, text::agent::TREE_LEAF);
        assert_eq!(layout.rows[2].prefix, text::agent::TREE_PAD);
    }

    #[test]
    fn folded_section_reports_exact_hidden_lines_and_keeps_head_tail() {
        let nodes = vec![node("a"), node("b"), node("c"), node("d"), node("e")];
        let sections = vec![Section::new(nodes, Some(Preview::head_tail(2, 1)))];
        let layout = layout(&sections, 40);
        assert_eq!(layout.total, 5);
        assert_eq!(layout.hidden, 2);
        let flat = layout.segments();
        let text: Vec<String> = flat
            .iter()
            .map(|row| row.iter().map(|s| s.text.as_str()).collect())
            .collect();
        assert!(text[0].contains('a') && text[1].contains('b'));
        assert!(text[2].contains('e'), "尾部保留最后一行：{text:?}");
        assert!(text[3].contains("… 2 more lines"), "{text:?}");
        assert!(
            text[3].starts_with(text::agent::TREE_PAD),
            "折叠提示是续行，不单独收口：{text:?}"
        );
    }

    #[test]
    fn narrow_width_drops_the_tree_but_keeps_content() {
        let sections = vec![Section::fixed(vec![node("abc")])];
        let layout = layout(&sections, 4);
        assert_eq!(layout.rows.len(), 1);
        assert!(layout.rows[0].prefix.is_empty());
        assert_eq!(layout.rows[0].segments[0].text, "abc");
    }

    #[test]
    fn empty_section_still_produces_one_leaf_row() {
        let sections = vec![Section::fixed(vec![Node::new(Vec::new())])];
        let layout = layout(&sections, 40);
        assert_eq!(layout.rows.len(), 1);
        assert_eq!(layout.rows[0].prefix, text::agent::TREE_LEAF);
    }
}
