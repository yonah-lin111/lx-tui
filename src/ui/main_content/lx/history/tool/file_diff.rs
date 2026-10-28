//! 文件工具的差异卡片：标题 + **逐文件**的路径/正文块。
//!
//! ```text
//! ■ Write
//!   ├─ docs/a.md (+3 -0)
//!   └─ 1 + 状态…
//!      2 + 下一行
//! ```
//!
//! - 标题只写裸动词（`Edit` / `Write`），不再有 `2 files` 合并标题；
//! - 每个文件：路径+计数是首节点，差异正文是一个多行节点（首行 `└─`，其余行
//!   左对齐）；多个文件依次用 `├─` 连接，最后一个文件收口；
//! - **没有动作按钮**：整块卡片本身就是点击面——点折叠内容展开、点展开内容收起；
//!   折叠态在正文下一行写剩余行数（`… N more lines`），提示行是续行、不再单独收口；
//! - 预览只取 Added/Removed 行、按真实列宽换行，**最多 3 个物理行**；上下文不占额度；
//! - 展开显示全部已保存 hunks、1 行上下文、hunk 间 `⋮` 分隔与 EOF 元信息；
//! - 新增绿、删除红，同时保留 `+`/`-`；行号与上下文用次要信息色；
//! - 增删行铺**整行底色**，上下文行不铺；正文按扩展名做轻量语法高亮；
//! - 纯渲染：只读 `FileDiffView`，不读文件、不解析 JSON。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{DiffHunkView, DiffLineView, FileChangeView, FileDiffView, ToolCallView};
use crate::chat::tools::{DiffLineKind, FileChangeKind};
use crate::ui::markdown_render::highlight_line;
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::{
    Segment, TREE_COLS, header_with_verb, put_segments, single_line, title_rows, wrap_spans,
};

/// 折叠态每文件最多预览的正文物理行数。
const PREVIEW_LINES: usize = 3;
/// Tab 制表位宽度（列）。
const TAB_COLS: usize = 4;
/// 路径/正文取样上限（限住超长串的每帧成本）。
const TEXT_SAMPLE_CHARS: usize = 4096;
/// 文件头里路径可用的最小列数；低于它先丢计数、再丢路径。
const MIN_PATH_COLS: usize = 8;

/// 单次调用是否有可展示的差异正文。
///
/// 旧记录没有 `diff` 字段：即使文件列表非空也只走两行摘要，不制造空展开入口。
pub(super) fn is_card(call: &ToolCallView) -> bool {
    call.file_view()
        .is_some_and(|view| !view.is_empty() && view.files.iter().any(|file| file.diff.is_some()))
}

/// 卡片占用的渲染行数（标题行 + 文件块）。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    title_rows() + layout(call, width, expanded).len()
}

/// 绘制文件差异卡片；推进行数必须与 [`rows`] 严格相等。
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
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header_with_verb(
            call,
            rect.width,
            loading_frame,
            animating,
            verb_of(call),
            None,
        );
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    for row in layout(call, rect.width, expanded) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            // 先铺**整行**底色（100% 宽，含左侧缩进与行尾空白）再写字：
            // `put_segments` 只写字符，不负责行背景，行尾留白会掉色。
            match row.fill {
                Some(fill) => {
                    grid.background_row(rect.x, y, rect.width, fill);
                    let tinted: Vec<Segment> = row
                        .segments
                        .iter()
                        .map(|segment| {
                            Segment::new(segment.text.clone(), segment.style.patch(fill))
                        })
                        .collect();
                    put_segments(grid, rect.x, y, rect.width, &tinted);
                }
                None => put_segments(grid, rect.x, y, rect.width, &row.segments),
            }
        }
        *row_index += 1;
    }
}

/// 一个物理行：分段 + 可选整行底色。
///
/// 底色必须与分段分开携带：`put_segments` 逐字符写入网格，不会替空格铺背景。
struct Row {
    segments: Vec<Segment>,
    fill: Option<Style>,
}

impl Row {
    /// 不铺底色的普通行。
    fn plain(segments: Vec<Segment>) -> Self {
        Self {
            segments,
            fill: None,
        }
    }
}

/// 一次调用的全部物理行（标题行除外）；[`rows`]、[`draw`] 与命中共用。
fn layout(call: &ToolCallView, width: u16, expanded: bool) -> Vec<Row> {
    let mut rows = content_rows(call, width, expanded);
    if !expanded {
        let remaining = remaining_lines(call, width);
        if remaining > 0 {
            rows.push(Row::plain(prefixed(
                &continuation_prefix(),
                text::agent::file_diff_more_lines(remaining),
                style::agent_step_detail(),
            )));
        }
    }
    if expanded {
        let header_rows: Vec<usize> = file_header_row_indexes(call, width);
        for (index, row) in rows.iter_mut().enumerate() {
            if header_rows.contains(&index) || row.fill.is_some() {
                continue;
            }
            row.fill = Some(style::chat_expanded_bg());
        }
    }
    rows
}

/// 展开态里「文件头」所在的行号（每文件一行，不铺展开底色）。
fn file_header_row_indexes(call: &ToolCallView, width: u16) -> Vec<usize> {
    let mut indexes = Vec::new();
    let mut row = 0usize;
    for file in files_of(call) {
        indexes.push(row);
        row += 1 + file_body(file, width, true).len();
    }
    indexes
}

/// 差异内容行（文件头 + 正文 + 省略文件提示），**不含**剩余行数提示。
fn content_rows(call: &ToolCallView, width: u16, expanded: bool) -> Vec<Row> {
    let files = files_of(call);
    let last_index = files.len().saturating_sub(1);
    let tree_body = draws_body_tree(width);
    let mut rows = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let last = index == last_index;
        let mut body = file_body(file, width, expanded);
        let has_body = !body.is_empty();
        let connector = if !last || (has_body && tree_body) {
            text::agent::TREE_BRANCH
        } else {
            text::agent::TREE_LEAF
        };
        rows.push(Row::plain(file_header(file, connector, width)));
        apply_body_prefixes(&mut body, last, width);
        rows.extend(body);
    }
    let omitted: usize = call
        .file_view()
        .map(|view| view.omitted_files)
        .unwrap_or_default();
    if omitted > 0 {
        rows.push(Row::plain(prefixed(
            &continuation_prefix(),
            text::agent::file_diff_omitted_files(omitted),
            style::agent_warn(),
        )));
    }
    // `partial`：本次补丁里有 hunk 应用后内容没变。它只存在于摘要里，
    // 差异正文本身看不出来，必须单独成行，否则模型看到的 `partial: true`
    // 在 UI 上完全消失。
    if call
        .summary
        .as_deref()
        .is_some_and(|summary| summary.partial)
    {
        rows.push(Row::plain(prefixed(
            &continuation_prefix(),
            text::agent::TOOL_PARTIAL,
            style::agent_step_detail(),
        )));
    }
    rows
}

/// 展开是否真的能看到更多内容（判据只看内容行，不看「剩余行数」提示行）。
pub(super) fn has_more_content(call: &ToolCallView, width: u16) -> bool {
    content_rows(call, width, true).len() > content_rows(call, width, false).len()
}

/// 折叠态被省略的物理行数：展开内容行数减预览内容行数。
fn remaining_lines(call: &ToolCallView, width: u16) -> usize {
    content_rows(call, width, true)
        .len()
        .saturating_sub(content_rows(call, width, false).len())
}

/// 按调用顺序、文件提交顺序扁平展开的文件列表。
fn files_of(call: &ToolCallView) -> Vec<&FileChangeView> {
    call.file_view()
        .map(|view| view.files.iter().collect())
        .unwrap_or_default()
}

/// 工具名 → 卡片动词（非文件工具回退 `Edit`）。
fn verb_of(call: &ToolCallView) -> &'static str {
    match call.name.as_str() {
        "write" => text::agent::VERB_TOOL_WRITE,
        _ => text::agent::VERB_TOOL_EDIT,
    }
}

/// 文件头：`  ├─ path (+N -N)`；最后一个文件收口为 `  └─ `。
///
/// 路径按剩余列宽做中间省略（保留首尾层级）；放不下时先丢计数、再丢路径，
/// 连接符始终保留。**路径恒为 `chat_tool_param()`（浅黄），不随 hover/展开变色**。
fn file_header(file: &FileChangeView, connector: &str, width: u16) -> Vec<Segment> {
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

/// 正文是否仍画树体连接符（极窄时让位，保证变更符号还看得见）。
fn draws_body_tree(width: u16) -> bool {
    usize::from(width) > TREE_COLS + 3
}

/// 把正文行包上树体前缀：首行 `├─`/`└─`，续行 `│`/空白。
fn apply_body_prefixes(rows: &mut [Row], last: bool, width: u16) {
    if !draws_body_tree(width) {
        return;
    }
    for (index, row) in rows.iter_mut().enumerate() {
        let prefix = if index == 0 {
            if last {
                text::agent::TREE_LEAF
            } else {
                text::agent::TREE_BRANCH
            }
        } else if last {
            text::agent::TREE_PAD
        } else {
            text::agent::TREE_BAR
        };
        let style = tint(style::chat_notice_connector(), row.fill);
        row.segments.insert(0, Segment::new(prefix, style));
    }
}

/// 文件正文行（预览或完整展开），不含树体前缀。
fn file_body(file: &FileChangeView, width: u16, expanded: bool) -> Vec<Row> {
    let available =
        usize::from(width).saturating_sub(if draws_body_tree(width) { TREE_COLS } else { 0 });
    let Some(diff) = file.diff.as_ref() else {
        return Vec::new();
    };
    let lang = language_of(&file.path);
    match diff {
        FileDiffView::Unavailable { reason } => vec![Row::plain(vec![Segment::new(
            text::agent::file_diff_unavailable(reason),
            style::agent_step_detail(),
        )])],
        FileDiffView::Complete { hunks, .. } | FileDiffView::Truncated { hunks, .. } => {
            if hunks.is_empty() {
                return vec![Row::plain(vec![Segment::new(
                    text::agent::FILE_DIFF_NO_CHANGES,
                    style::agent_step_detail(),
                )])];
            }
            let gutter = gutter_width(hunks);
            let mut rows = if expanded {
                expanded_body(hunks, gutter, available, lang)
            } else {
                preview_rows(hunks, gutter, available, lang)
            };
            if expanded && diff.is_truncated() {
                rows.push(Row::plain(vec![Segment::new(
                    text::agent::FILE_DIFF_TRUNCATED,
                    style::agent_warn(),
                )]));
            }
            if expanded && diff.simplified() {
                rows.push(Row::plain(vec![Segment::new(
                    text::agent::FILE_DIFF_SIMPLIFIED,
                    style::agent_step_detail(),
                )]));
            }
            rows
        }
    }
}

/// 折叠预览：只取 Added/Removed 行，最多 [`PREVIEW_LINES`] 个物理行。
fn preview_rows(
    hunks: &[DiffHunkView],
    gutter: usize,
    available: usize,
    lang: Option<&'static str>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut budget = PREVIEW_LINES;
    let mut changes = hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind != DiffLineKind::Context);
    for line in changes.by_ref() {
        let physical = render_line(line, gutter, available, lang);
        if physical.len() > budget {
            rows.extend(physical.into_iter().take(budget));
            break;
        }
        budget -= physical.len();
        rows.extend(physical);
        if budget == 0 {
            break;
        }
    }
    rows
}

/// 完整展开：全部已保存 hunks、hunk 间 `⋮` 与 EOF 元信息。
fn expanded_body(
    hunks: &[DiffHunkView],
    gutter: usize,
    available: usize,
    lang: Option<&'static str>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for (index, hunk) in hunks.iter().enumerate() {
        if index > 0 {
            rows.push(Row::plain(vec![Segment::new(
                text::agent::FILE_DIFF_HUNK_SEP,
                style::agent_step_detail(),
            )]));
        }
        for line in &hunk.lines {
            rows.extend(render_line(line, gutter, available, lang));
            if line.no_final_newline {
                rows.push(Row::plain(vec![Segment::new(
                    text::agent::FILE_DIFF_NO_FINAL_NEWLINE,
                    style::agent_step_detail(),
                )]));
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
    lang: Option<&'static str>,
) -> Vec<Row> {
    let (sign, number, content_style, fill) = match line.kind {
        DiffLineKind::Added => (
            "+",
            line.new_line,
            style::agent_added(),
            Some(style::agent_added_bg()),
        ),
        DiffLineKind::Removed => (
            "-",
            line.old_line,
            style::agent_removed(),
            Some(style::agent_removed_bg()),
        ),
        DiffLineKind::Context => (" ", line.new_line, style::text(), None),
    };
    let number = number.unwrap_or(0);
    let number_text = format!("{number:>gutter$} ");
    let full_cols = number_text.width() + 2;
    let (number_text, separate, head_cols, content_width) = if available > full_cols + 1 {
        (number_text, true, full_cols, available - full_cols)
    } else if available > 3 {
        (String::new(), true, 2, available - 2)
    } else {
        (String::new(), false, 1, available.saturating_sub(1))
    };
    let safe = safe_text(&line.text);
    let spans: Vec<Segment> = highlight_spans(lang, &safe, content_style)
        .into_iter()
        .map(|(text, span_style)| Segment::new(text, tint(span_style, fill)))
        .collect();
    let wrapped = wrap_spans(&spans, content_width.max(1));
    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut segments = Vec::with_capacity(chunk.len() + 2);
            let head_style = tint(style::agent_step_detail(), fill);
            if index == 0 {
                segments.push(Segment::new(number_text.clone(), head_style));
                segments.push(Segment::new(sign, tint(content_style, fill)));
                if separate {
                    segments.push(Segment::new(" ", tint(style::text(), fill)));
                }
            } else {
                segments.push(Segment::new(" ".repeat(head_cols), head_style));
            }
            segments.extend(chunk);
            Row { segments, fill }
        })
        .collect()
}

/// 分段样式叠加整行底色（分段自身不设背景，`patch` 只补 bg、不覆盖 fg）。
fn tint(segment: Style, fill: Option<Style>) -> Style {
    match fill {
        Some(fill) => segment.patch(fill),
        None => segment,
    }
}

/// 按扩展名取轻量语法高亮的语言标记；`None` = 纯文本（不做假高亮）。
fn language_of(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "md" | "markdown" | "mdx" => "md",
        "rs" => "rs",
        "js" | "jsx" | "mjs" | "cjs" => "js",
        "ts" | "tsx" => "ts",
        "java" | "kt" | "kts" | "scala" => "java",
        "py" => "py",
        "sh" | "bash" | "zsh" | "fish" => "sh",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "c" | "h" | "cc" | "cpp" | "hpp" | "go" | "cs" | "swift" => "c",
        _ => return None,
    })
}

/// 一行源码的高亮分段：`.md` 走 markdown 词法，其余交给共享的代码高亮器。
fn highlight_spans(lang: Option<&'static str>, text: &str, plain: Style) -> Vec<(String, Style)> {
    if lang == Some("md") {
        return markdown_spans(text, plain);
    }
    highlight_line(lang, text)
        .into_iter()
        .map(|(text, style)| {
            let style = if style == style::md_read_code_block() {
                plain
            } else {
                style
            };
            (text, style)
        })
        .collect()
}

/// markdown 行内高亮：把词法 token 的区间映射回原文分段。
fn markdown_spans(text: &str, plain: Style) -> Vec<(String, Style)> {
    if text.is_empty() {
        return Vec::new();
    }
    let tokens = crate::ui::markdown::scan(text)
        .into_iter()
        .next()
        .unwrap_or_default();
    let mut spans: Vec<(String, Style)> = Vec::new();
    let mut cursor = 0usize;
    for token in tokens {
        let start = token.range.start.min(text.len());
        let end = token.range.end.min(text.len());
        if start >= end {
            continue;
        }
        if let Some(gap) = text.get(cursor..start).filter(|gap| !gap.is_empty()) {
            spans.push((gap.to_string(), plain));
        }
        if let Some(part) = text.get(start..end) {
            spans.push((part.to_string(), crate::ui::prompt::token_style(token.kind)));
        }
        cursor = cursor.max(end);
    }
    if let Some(tail) = text.get(cursor..).filter(|tail| !tail.is_empty()) {
        spans.push((tail.to_string(), plain));
    }
    if spans.is_empty() {
        spans.push((text.to_string(), plain));
    }
    spans
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

#[cfg(test)]
mod tests;
