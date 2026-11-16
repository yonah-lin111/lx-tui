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
use unicode_width::UnicodeWidthStr;

use crate::ui::style;

use super::super::{Grid, visible_row};
use super::{Segment, TREE_COLS, put_segments, wrap_spans};
use crate::ui::main_content::lx::history::truncate_with_ellipsis;
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
    /// 视图保留窗口之外已省略的逻辑行数（如 `web_search` / `mcp` 的 `result_more`）。
    pub(crate) omitted_lines: usize,
    /// 该块内容在执行阶段是否已被截断（用于在 `… N more lines` 后追加 `[truncated]`）。
    pub(crate) truncated: bool,
}

impl Section {
    pub(crate) fn new(nodes: Vec<Node>, preview: Option<Preview>) -> Self {
        Self {
            nodes,
            preview,
            omitted_lines: 0,
            truncated: false,
        }
    }

    /// 常显块（状态、参数外的元信息）。
    pub(crate) fn fixed(nodes: Vec<Node>) -> Self {
        Self {
            nodes,
            preview: None,
            omitted_lines: 0,
            truncated: false,
        }
    }

    /// 标记视图窗口之外额外省略的行数（折叠态与 `section_hidden` 累加，展开态单独展示）。
    pub(crate) fn with_omitted(mut self, omitted_lines: usize) -> Self {
        self.omitted_lines = omitted_lines;
        self
    }

    /// 标记该块在执行阶段被截断。
    pub(crate) fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }
}

/// 一个物理行：连接符（可能为空）+ 内容。
#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) prefix: &'static str,
    pub(crate) prefix_style: Option<Style>,
    pub(crate) segments: Vec<Segment>,
    /// 内容所属节点；省略提示行归属到它继续的那个节点。
    node: Option<usize>,
    /// 是否该节点的**首个**物理行（决定 `├─`/`└─`；续行只能 `│`/空白）。
    first: bool,
}

impl Row {
    /// 该物理行的文字列区间（相对内容区左缘）；纯树线/空白行返回 `None`。
    pub(crate) fn text_cols(&self) -> Option<(u16, u16)> {
        super::super::text_cols::cols_from(
            0,
            std::iter::once(self.prefix)
                .chain(self.segments.iter().map(|segment| segment.text.as_str())),
        )
    }
}

/// 一次排版的结果。
#[derive(Debug, Clone)]
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
    /// 每个物理行的文字列区间；行数与 `rows` 严格一一对应。
    pub(crate) fn row_cols(&self) -> Vec<Option<(u16, u16)>> {
        self.rows.iter().map(Row::text_cols).collect()
    }

    /// 拼成「前缀 + 内容」的段行（沿用旧测试/调用方的 `Vec<Vec<Segment>>` 口径）。
    pub(crate) fn segments(&self) -> Vec<Vec<Segment>> {
        self.rows
            .iter()
            .map(|row| {
                let mut segments = Vec::with_capacity(row.segments.len() + 1);
                if !row.prefix.is_empty() {
                    segments.push(Segment::new(
                        row.prefix,
                        row.prefix_style
                            .unwrap_or_else(style::chat_notice_connector),
                    ));
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

/// 合并组二级子树（8 列前缀）在极窄宽度下是否仍画连接符。
pub(crate) fn keeps_nested_tree(width: u16) -> bool {
    usize::from(width) > super::TREE_NESTED_COLS
}

/// 内容可用列数（一级树 5 列前缀）。
pub(crate) fn content_width(width: u16) -> usize {
    if keeps_tree(width) {
        usize::from(width).saturating_sub(TREE_COLS).max(1)
    } else {
        usize::from(width).max(1)
    }
}

/// 合并组内二级子节点内容可用列数（二级树 8 列前缀）。
pub(crate) fn nested_content_width(width: u16) -> usize {
    if keeps_nested_tree(width) {
        usize::from(width)
            .saturating_sub(super::TREE_NESTED_COLS)
            .max(1)
    } else {
        usize::from(width).max(1)
    }
}

/// 树体排版（单卡或平铺快照）；`sections` 按显示顺序排列，各自独立折叠。
pub(crate) fn layout(sections: &[Section], width: u16) -> Layout {
    layout_with_roots(sections, width, None)
}

/// 合并组内树体排版：前 `root_nodes` 个节点按一级树（5 列前缀）折行，后续子节点按二级树（8 列前缀）折行。
pub(crate) fn layout_in_group(sections: &[Section], width: u16, root_nodes: usize) -> Layout {
    layout_with_roots(sections, width, Some(root_nodes))
}

fn layout_with_roots(sections: &[Section], width: u16, root_nodes: Option<usize>) -> Layout {
    let total_nodes: usize = sections.iter().map(|section| section.nodes.len()).sum();
    let has_nested_children = root_nodes.is_some_and(|roots| total_nodes > roots);
    let tree = if has_nested_children {
        keeps_nested_tree(width)
    } else {
        keeps_tree(width)
    };
    let root_body = if tree {
        usize::from(width).saturating_sub(TREE_COLS).max(1)
    } else {
        usize::from(width).max(1)
    };
    let nested_body = if tree {
        usize::from(width)
            .saturating_sub(super::TREE_NESTED_COLS)
            .max(1)
    } else {
        usize::from(width).max(1)
    };
    let node_width = |node_index: usize| match root_nodes {
        Some(roots) if node_index >= roots => nested_body,
        _ => root_body,
    };

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
        }
        let mut lines: Vec<Line> = Vec::new();
        for (index, node) in section.nodes.iter().enumerate() {
            let node_index = next_node + index;
            let body = node_width(node_index);
            for logical in &node.lines {
                for spans in wrap_spans(logical, body) {
                    lines.push(Line {
                        segments: spans,
                        node: node_index,
                    });
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
            let first = !seen.contains(&line.node);
            if first {
                seen.push(line.node);
                node_rows.push(rows.len());
            }
            last_visible = Some(line.node);
            rows.push(Row {
                prefix: "",
                prefix_style: None,
                segments: line.segments.clone(),
                node: Some(line.node),
                first,
            });
        }
        let total_omitted = section_hidden.saturating_add(section.omitted_lines);
        if total_omitted > 0 {
            let hint_body = last_visible.map_or(root_body, node_width);
            let base_hint = text::agent::file_diff_more_lines(total_omitted);
            let hint = if section.truncated {
                let with_trunc = format!("{base_hint} {}", text::agent::INLINE_TRUNCATED);
                if unicode_width::UnicodeWidthStr::width(with_trunc.as_str()) <= hint_body {
                    Some(with_trunc)
                } else if unicode_width::UnicodeWidthStr::width(base_hint.as_str()) <= hint_body {
                    Some(base_hint)
                } else {
                    None
                }
            } else if unicode_width::UnicodeWidthStr::width(base_hint.as_str()) <= hint_body {
                Some(base_hint)
            } else {
                None
            };
            if let Some(hint) = hint {
                // 提示行是本块最后可见节点的续行：仍用空白列，不再单独画一个
                // `└─`，否则长输出里每个折叠块都多一个收口符。
                rows.push(Row {
                    prefix: "",
                    prefix_style: None,
                    segments: vec![Segment::new(hint, style::fold_hint())],
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

/// 组内调用体：首节点作为一级分支（非末调用 `├─`、末调用 `└─`），后续节点缩进为二级子树。
pub(crate) fn apply_group_connector(rows: &mut [Row], last: bool) {
    apply_group_connector_with_roots(rows, last, 1);
}

/// 组内调用体（支持前 `root_nodes` 个节点同属一级分支，如多目标 `wait_agent`）。
pub(crate) fn apply_group_connector_with_roots(rows: &mut [Row], last: bool, root_nodes: usize) {
    let roots = root_nodes.max(1);
    let Some(last_node) = rows.iter().rev().find_map(|row| row.node) else {
        return;
    };
    let last_root = rows
        .iter()
        .rev()
        .filter_map(|row| row.node)
        .find(|&node| node < roots);
    for row in rows {
        if row.prefix.is_empty() {
            continue;
        }
        let Some(node) = row.node else {
            continue;
        };
        if node < roots {
            let is_last_root = last && Some(node) == last_root;
            row.prefix = match (is_last_root, row.first) {
                (false, true) => text::agent::TREE_BRANCH,
                (false, false) => text::agent::TREE_BAR,
                (true, true) => text::agent::TREE_LEAF,
                (true, false) => text::agent::TREE_PAD,
            };
        } else {
            let last_child = node == last_node;
            row.prefix = match (last, last_child, row.first) {
                (false, false, true) => text::agent::TREE_NESTED_BRANCH_BRANCH,
                (false, false, false) => text::agent::TREE_NESTED_BRANCH_BAR,
                (false, true, true) => text::agent::TREE_NESTED_BRANCH_LEAF,
                (false, true, false) => text::agent::TREE_NESTED_BRANCH_PAD,
                (true, false, true) => text::agent::TREE_NESTED_LEAF_BRANCH,
                (true, false, false) => text::agent::TREE_NESTED_LEAF_BAR,
                (true, true, true) => text::agent::TREE_NESTED_LEAF_LEAF,
                (true, true, false) => text::agent::TREE_NESTED_LEAF_PAD,
            };
        }
    }
}

/// 调用在合并组内的首节点连接符颜色（失败红、中断黄、其余默认灰）。
pub(crate) fn connector_style_of(call: &crate::app::state::ToolCallView) -> Style {
    match super::group::effective_status(call) {
        crate::app::state::ToolStatus::Failed => style::agent_fail(),
        crate::app::state::ToolStatus::Interrupted => style::agent_warn(),
        _ => style::chat_notice_connector(),
    }
}

/// 为合并组内调用装饰首节点连接符颜色与尾部状态文字。
pub(crate) fn decorate_group_call(
    rows: &mut [Row],
    call: &crate::app::state::ToolCallView,
    status: Option<String>,
    width: u16,
) {
    let effective = super::group::effective_status(call);
    let connector_override = match effective {
        crate::app::state::ToolStatus::Failed => Some(style::agent_fail()),
        crate::app::state::ToolStatus::Interrupted => Some(style::agent_warn()),
        _ => None,
    };
    if let Some(conn_style) = connector_override
        && let Some(first_node) = rows.iter().find_map(|row| row.node)
    {
        for row in rows.iter_mut() {
            if row.node == Some(first_node) && row.first {
                row.prefix_style = Some(conn_style);
            }
        }
    }
    let status_style = match effective {
        crate::app::state::ToolStatus::Failed => style::agent_fail(),
        crate::app::state::ToolStatus::Interrupted => style::agent_warn(),
        crate::app::state::ToolStatus::Running
        | crate::app::state::ToolStatus::Pending
        | crate::app::state::ToolStatus::InputStreaming => style::chat_step_label_alive(),
        _ => style::agent_step_detail(),
    };
    let status = status.or_else(|| {
        if matches!(
            call.status,
            crate::app::state::ToolStatus::Failed | crate::app::state::ToolStatus::Interrupted
        ) && call
            .error
            .as_deref()
            .is_none_or(|err| err.trim().is_empty())
        {
            Some(text::tool_status_label(call.status).to_string())
        } else {
            None
        }
    });
    append_status_styled(rows, status, status_style, width);
}

pub(crate) fn append_status_styled(
    rows: &mut [Row],
    status: Option<String>,
    status_style: Style,
    width: u16,
) {
    let Some(status) = status.filter(|s| !s.is_empty()) else {
        return;
    };
    let Some(first_node) = rows.iter().find_map(|row| row.node) else {
        return;
    };
    let Some(row) = rows
        .iter_mut()
        .rev()
        .find(|row| row.node == Some(first_node))
    else {
        return;
    };
    let used = UnicodeWidthStr::width(row.prefix)
        .saturating_add(row.segments.iter().map(Segment::cols).sum::<usize>());
    let available = usize::from(width).saturating_sub(used);
    let sep_width = UnicodeWidthStr::width(text::LX_CHAT_TOOL_STATUS_SEP);
    let min_status_cols = UnicodeWidthStr::width(status.as_str()).min(2);
    if available < sep_width.saturating_add(min_status_cols) {
        return;
    }
    let tail = format!("{}{status}", text::LX_CHAT_TOOL_STATUS_SEP);
    let tail = truncate_with_ellipsis(&tail, available);
    if !tail.is_empty() {
        row.segments.push(Segment::new(tail, status_style));
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
                segments.push(Segment::new(
                    row.prefix,
                    row.prefix_style
                        .unwrap_or_else(style::chat_notice_connector),
                ));
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

    #[test]
    fn tail_slice_marks_first_visible_line_of_wrapped_node() {
        let tail_block = Node::block(vec![
            vec![Segment::new("hidden head of tail node", style::text())],
            vec![Segment::new("visible tail line", style::text())],
        ]);
        let sections = vec![Section::new(
            vec![node("first"), tail_block],
            Some(Preview::head_tail(1, 1)),
        )];
        let layout = layout(&sections, 40);
        assert_eq!(layout.node_rows, vec![0, 1]);
        assert_eq!(layout.rows[1].prefix, text::agent::TREE_LEAF);
    }

    #[test]
    fn group_connector_nests_child_nodes_under_first_node() {
        let result_block = Node::block(vec![
            vec![Segment::new("Title: Ratatui", style::text())],
            vec![Segment::new("URL: https://ratatui.rs/", style::text())],
        ]);
        let sections = vec![
            Section::fixed(vec![node("\"rust tui\"")]),
            Section::new(vec![result_block], Some(Preview::head(1))),
        ];
        let mut non_last = layout(&sections, 60);
        apply_group_connector(&mut non_last.rows, false);
        let non_last_prefixes: Vec<&str> = non_last.rows.iter().map(|row| row.prefix).collect();
        assert_eq!(
            non_last_prefixes,
            vec![
                text::agent::TREE_BRANCH,
                text::agent::TREE_NESTED_BRANCH_LEAF,
                text::agent::TREE_NESTED_BRANCH_PAD,
            ]
        );

        let mut last = layout(&sections, 60);
        apply_group_connector(&mut last.rows, true);
        let last_prefixes: Vec<&str> = last.rows.iter().map(|row| row.prefix).collect();
        assert_eq!(
            last_prefixes,
            vec![
                text::agent::TREE_LEAF,
                text::agent::TREE_NESTED_LEAF_LEAF,
                text::agent::TREE_NESTED_LEAF_PAD,
            ]
        );
    }
}
