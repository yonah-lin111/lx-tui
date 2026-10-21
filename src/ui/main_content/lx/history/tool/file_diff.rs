//! 文件工具的差异卡片：文件头 + 每文件 3 行预览 / 完整展开。
//!
//! ```text
//! ■ Edit
//!   ├─ src/main.rs (+1 -1)
//!   │     8 - hello
//!   │     8 + world
//!   └─ src/lib.rs → src/core.rs (+0 -0)
//!        No content changes
//! ```
//!
//! - **行数只有一个来源**：[`rows`] 与 [`draw`] 共用 [`layout`] 产出的物理行；
//! - 预览只取 Added/Removed 行、按真实列宽换行，**最多 3 个物理行**；上下文不占额度；
//!   省略提示单独占一行（不占 3 行额度）；
//! - 展开显示全部已保存 hunks、1 行上下文、hunk 间 `⋮` 分隔与 EOF 元信息；
//! - 新增绿、删除红，同时保留 `+`/`-`；行号与上下文用次要信息色；
//! - 纯渲染：只读 `FileDiffView`，不读文件、不解析 JSON。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{
    DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView, FileDiffView, ToolCallView,
};
use crate::chat::tools::{DiffLineKind, FileChangeKind};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::{Segment, TREE_COLS, header_with_verb, put_segments, single_line, title_rows};

/// 折叠态每文件最多预览的正文物理行数。
const PREVIEW_LINES: usize = 3;
/// Tab 制表位宽度（列）。
const TAB_COLS: usize = 4;
/// 路径/正文取样上限（限住超长串的每帧成本）。
const TEXT_SAMPLE_CHARS: usize = 4096;
/// 文件头里路径可用的最小列数；低于它先丢计数、再丢路径。
const MIN_PATH_COLS: usize = 8;

/// 该调用是否有可展示的差异正文（否则退回原来的两行摘要）。
///
/// 旧记录没有 `diff` 字段：即使文件列表非空也只走两行摘要，不制造空展开入口。
pub(super) fn has_diff(call: &ToolCallView) -> bool {
    call.file_view()
        .is_some_and(|view| !view.is_empty() && view.files.iter().any(|file| file.diff.is_some()))
}

/// 卡片占用的渲染行数（标题行 + 文件详情）。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    match call.file_view().filter(|view| !view.is_empty()) {
        Some(view) => title_rows() + layout(view, width, expanded).len(),
        None => title_rows() + 1,
    }
}

/// 标题行在卡片内的偏移（0 = 标题行）：点击它切换整张卡片的展开状态。
///
/// 只有正文保留聊天选区行为；这里只登记标题一行。
pub(super) fn toggle_offset(call: &ToolCallView) -> Option<usize> {
    has_diff(call).then_some(0)
}

/// 绘制文件差异卡片；推进行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    expanded: bool,
    animating: bool,
) {
    if rect.width == 0 {
        return;
    }
    let Some(view) = call.file_view().filter(|view| !view.is_empty()) else {
        return;
    };
    let verb = match call.name.as_str() {
        "write" => text::agent::VERB_TOOL_WRITE,
        _ => text::agent::VERB_TOOL_EDIT,
    };
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header_with_verb(call, rect.width, loading_frame, animating, verb, None);
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    for row in layout(view, rect.width, expanded) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            put_segments(grid, rect.x, y, rect.width, &row);
        }
        *row_index += 1;
    }
}

/// 全部物理行（标题行除外）；[`rows`] 与 [`draw`] 共用。
fn layout(view: &FileDiffDetailView, width: u16, expanded: bool) -> Vec<Vec<Segment>> {
    let mut rows = Vec::new();
    let last_index = view.files.len().saturating_sub(1);
    for (index, file) in view.files.iter().enumerate() {
        let last = index == last_index;
        rows.push(file_header(file, last, width));
        rows.extend(file_body(file, width, expanded, last));
    }
    if view.omitted_files > 0 {
        rows.push(prefixed(
            &continuation_prefix(),
            text::agent::file_diff_omitted_files(view.omitted_files),
            style::agent_warn(),
        ));
    }
    rows
}

/// 文件头：`  ├─ path → to (+N -N)`；最后一个文件收口为 `  └─ `。
///
/// 路径按剩余列宽做中间省略（保留首尾层级）；放不下时先丢计数、再丢路径，
/// 连接符始终保留。
fn file_header(file: &FileChangeView, last: bool, width: u16) -> Vec<Segment> {
    let connector = if last {
        text::agent::TREE_LEAF
    } else {
        text::agent::TREE_BRANCH
    };
    let limit = usize::from(width);
    let used = connector.width();
    let mut segments = vec![Segment::new(connector, style::chat_notice_connector())];
    let raw_path = match file.kind {
        FileChangeKind::Moved { .. } => {
            let to = file.to.as_deref().unwrap_or("");
            format!("{}{}{to}", file.path, text::agent::FILE_DIFF_MOVE_SEP)
        }
        _ => file.path.clone(),
    };
    let path = single_line(&raw_path, TEXT_SAMPLE_CHARS);
    // 计数段：未知时显式标出，绝不用 0 冒充。
    let counts: Vec<Segment> = if file.counts_known {
        match (file.added, file.removed) {
            (Some(added), Some(removed)) => vec![
                Segment::new(
                    format!("({}", text::agent::added_label(added)),
                    style::agent_added(),
                ),
                Segment::new(" ", style::text()),
                Segment::new(
                    format!("{})", text::agent::removed_label(removed)),
                    style::agent_removed(),
                ),
            ],
            _ => Vec::new(),
        }
    } else {
        vec![Segment::new(
            text::agent::FILE_DIFF_COUNTS_UNKNOWN,
            style::agent_step_detail(),
        )]
    };
    let counts_cols: usize = counts.iter().map(Segment::cols).sum();
    let room = limit.saturating_sub(used + 1);
    if room == 0 {
        return segments;
    }
    // 优先保住路径：计数只有在路径还剩 [`MIN_PATH_COLS`] 列时才画。
    let with_counts = room.saturating_sub(1 + counts_cols);
    let keep_counts = counts_cols > 0 && room > 1 + counts_cols && with_counts >= MIN_PATH_COLS;
    let path_room = if keep_counts { with_counts } else { room };
    let path = text::ellipsize_middle(&path, path_room);
    if !path.is_empty() {
        segments.push(Segment::new(" ", style::text()));
        segments.push(Segment::new(path, style::chat_tool_param()));
    }
    if keep_counts {
        segments.push(Segment::new(" ", style::text()));
        segments.extend(counts);
    }
    segments
}

/// 文件正文行（预览或完整展开）。
fn file_body(file: &FileChangeView, width: u16, expanded: bool, last: bool) -> Vec<Vec<Segment>> {
    // 极窄时整个树形前缀让位，保证变更符号还看得见。
    let prefix = if usize::from(width) > TREE_COLS + 3 {
        if last {
            continuation_prefix()
        } else {
            format!("  {}  ", text::agent::MARKER_TREE_BAR)
        }
    } else {
        String::new()
    };
    let available = usize::from(width).saturating_sub(prefix.width());
    let Some(diff) = file.diff.as_ref() else {
        return Vec::new();
    };
    match diff {
        FileDiffView::Unavailable { reason } => vec![prefixed(
            &prefix,
            text::agent::file_diff_unavailable(reason),
            style::agent_step_detail(),
        )],
        FileDiffView::Complete { hunks, .. } | FileDiffView::Truncated { hunks, .. } => {
            if hunks.is_empty() {
                return vec![prefixed(
                    &prefix,
                    text::agent::FILE_DIFF_NO_CHANGES,
                    style::agent_step_detail(),
                )];
            }
            let gutter = gutter_width(hunks);
            let mut rows = if expanded {
                expanded_body(hunks, gutter, available, &prefix, diff)
            } else {
                preview_body(hunks, gutter, available, &prefix)
            };
            if expanded && diff.is_truncated() {
                rows.push(prefixed(
                    &prefix,
                    text::agent::FILE_DIFF_TRUNCATED,
                    style::agent_warn(),
                ));
            }
            if expanded && diff.simplified() {
                rows.push(prefixed(
                    &prefix,
                    text::agent::FILE_DIFF_SIMPLIFIED,
                    style::agent_step_detail(),
                ));
            }
            rows
        }
    }
}

/// 折叠预览：只取 Added/Removed 行，最多 [`PREVIEW_LINES`] 个物理行。
fn preview_body(
    hunks: &[DiffHunkView],
    gutter: usize,
    available: usize,
    prefix: &str,
) -> Vec<Vec<Segment>> {
    let mut rows = Vec::new();
    let mut budget = PREVIEW_LINES;
    let mut omitted = false;
    // 只看增删行：上下文不占预览额度。
    let mut changes = hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind != DiffLineKind::Context);
    while let Some(line) = changes.next() {
        let physical = render_line(line, gutter, available, prefix);
        if physical.len() > budget {
            // 这一行自己就写不完：剩下的内容同样属于「省略」。
            rows.extend(physical.into_iter().take(budget));
            omitted = true;
            break;
        }
        budget -= physical.len();
        rows.extend(physical);
        if budget == 0 {
            // 正好写满：还有没有后续变更决定是否出省略提示。
            omitted = changes.next().is_some();
            break;
        }
    }
    if omitted {
        // 提示行单独计行，不占 3 行正文额度。
        rows.push(prefixed(
            prefix,
            text::agent::FILE_DIFF_OMITTED,
            style::agent_step_detail(),
        ));
    }
    rows
}

/// 完整展开：全部已保存 hunks、hunk 间 `⋮` 与 EOF 元信息。
fn expanded_body(
    hunks: &[DiffHunkView],
    gutter: usize,
    available: usize,
    prefix: &str,
    diff: &FileDiffView,
) -> Vec<Vec<Segment>> {
    let _ = diff;
    let mut rows = Vec::new();
    for (index, hunk) in hunks.iter().enumerate() {
        if index > 0 {
            rows.push(prefixed(
                prefix,
                text::agent::FILE_DIFF_HUNK_SEP,
                style::agent_step_detail(),
            ));
        }
        for line in &hunk.lines {
            rows.extend(render_line(line, gutter, available, prefix));
            if line.no_final_newline {
                rows.push(prefixed(
                    prefix,
                    text::agent::FILE_DIFF_NO_FINAL_NEWLINE,
                    style::agent_step_detail(),
                ));
            }
        }
    }
    rows
}

/// 正文续行前缀（末节点无竖线）：与 [`TREE_COLS`] 等宽。
fn continuation_prefix() -> String {
    " ".repeat(TREE_COLS)
}

/// 行号槽宽度：按全部已保存行号的十进制位数；没有行号时 1 列。
fn gutter_width(hunks: &[DiffHunkView]) -> usize {
    hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .flat_map(|line| [line.old_line, line.new_line])
        .flatten()
        .max()
        .map_or(1, |max| max.to_string().len())
}

/// 一行差异 → 若干物理行（按可用列宽换行，续行保留空 gutter、不重复符号）。
fn render_line(
    line: &DiffLineView,
    gutter: usize,
    available: usize,
    prefix: &str,
) -> Vec<Vec<Segment>> {
    let (sign, number, content_style) = match line.kind {
        DiffLineKind::Added => ("+", line.new_line, style::agent_added()),
        DiffLineKind::Removed => ("-", line.old_line, style::agent_removed()),
        DiffLineKind::Context => (" ", line.new_line, style::text()),
    };
    let number = number.unwrap_or(0);
    // 行号槽与符号分开着色：行号是弱化元信息，`+`/`-` 必须与正文同色。
    let number_text = format!("{number:>gutter$} ");
    let full_cols = number_text.width() + 2;
    // 让位顺序：完整 gutter → 只留 `符号 ` → 只留符号（正文至少 1 列）。
    let (number_text, separate, head_cols, content_width) = if available > full_cols + 1 {
        (number_text, true, full_cols, available - full_cols)
    } else if available > 3 {
        (String::new(), true, 2, available - 2)
    } else {
        (String::new(), false, 1, available.saturating_sub(1))
    };
    let safe = safe_text(&line.text);
    let wrapped = wrap_text(&safe, content_width.max(1));
    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut segments = Vec::with_capacity(4);
            if index == 0 {
                segments.push(Segment::new(
                    format!("{prefix}{number_text}"),
                    style::agent_step_detail(),
                ));
                segments.push(Segment::new(sign, content_style));
                if separate {
                    segments.push(Segment::new(" ", style::text()));
                }
            } else {
                // 续行保留空 gutter，不重复行号或符号。
                segments.push(Segment::new(
                    format!("{prefix}{}", " ".repeat(head_cols)),
                    style::agent_step_detail(),
                ));
            }
            segments.push(Segment::new(chunk, content_style));
            segments
        })
        .collect()
}

/// 带树形前缀的单段提示行。
fn prefixed(prefix: &str, content: impl Into<String>, content_style: Style) -> Vec<Segment> {
    vec![
        Segment::new(prefix.to_string(), style::agent_step_detail()),
        Segment::new(content.into(), content_style),
    ]
}

/// 安全文本：Tab 按 4 列制表位展开，其余控制字符转空格（不执行 ESC/ANSI）。
fn safe_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut col = 0usize;
    for ch in text.chars() {
        match ch {
            '\t' => {
                let next = (col / TAB_COLS + 1) * TAB_COLS;
                for _ in col..next {
                    out.push(' ');
                }
                col = next;
            }
            ch if ch.is_control() => {
                out.push(' ');
                col += 1;
            }
            ch => {
                out.push(ch);
                col += ch.width().unwrap_or(1);
            }
        }
    }
    out
}

/// 按显示列换行（宽字符不半写，宽于整行时单独占一行）。
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        let cell = ch.width().unwrap_or(1).max(1);
        if col + cell > width && col > 0 {
            rows.push(std::mem::take(&mut current));
            col = 0;
        }
        current.push(ch);
        col += cell;
    }
    rows.push(current);
    rows
}

#[cfg(test)]
mod tests;
