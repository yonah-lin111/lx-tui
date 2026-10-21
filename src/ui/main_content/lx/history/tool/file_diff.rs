//! 文件工具的差异卡片：合并卡 + 每文件 3 行预览 / 完整展开。
//!
//! ```text
//! ■ Edit 2 files (+34 -1)
//!   ├─ src/main.rs (+1 -1)
//!   │     8 - hello
//!   │     8 + world
//!   └─ src/lib.rs (+0 -0)
//!        No content changes
//!        Show full diff
//! ```
//!
//! - **行数只有一个来源**：[`rows`] 与 [`draw`] 共用 [`layout`] 产出的物理行；
//! - 连续的**同种**文件调用（`edit` 挨着 `edit`、`write` 挨着 `write`，中间没有正文
//!   步骤打断）合并成**一张卡**：标题给出文件数与总计，文件行按调用顺序扁平铺开，
//!   共用一份展开状态。单次调用是长度为 1 的组，标题只写裸动词、不写 `1 file`；
//! - 预览只取 Added/Removed 行、按真实列宽换行，**最多 3 个物理行**；上下文不占额度；
//!   省略提示单独占一行（不占 3 行额度）；
//! - 展开显示全部已保存 hunks、1 行上下文、hunk 间 `⋮` 分隔与 EOF 元信息；
//! - 新增绿、删除红，同时保留 `+`/`-`；行号与上下文用次要信息色；
//! - 增删行铺**整行底色**（`agent_added_bg` / `agent_removed_bg`），上下文行不铺；
//!   正文按文件扩展名做轻量语法高亮（`.md` 走 markdown 词法，未知扩展名不假高亮）；
//! - 纯渲染：只读 `FileDiffView`，不读文件、不解析 JSON。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{DiffHunkView, DiffLineView, FileChangeView, FileDiffView, ToolCallView};
use crate::chat::tools::{DiffLineKind, FileChangeKind};
use crate::ui::markdown_render::highlight_line;
use crate::ui::{style, text};

use super::super::{Grid, StepKind, StepMark, visible_row};
use super::{
    Segment, TREE_COLS, header_with_verb, put_segments, segments_cols, single_line, title_rows,
    truncate_with_ellipsis,
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

/// 该组是否含可展示的差异正文（与 [`is_card`] 同口径）。
pub(super) fn has_diff(calls: &[&ToolCallView]) -> bool {
    calls.iter().any(|call| is_card(call))
}

/// 卡片占用的渲染行数（标题行 + 文件详情 + 末尾动作行）。
pub(super) fn rows(calls: &[&ToolCallView], width: u16, expanded: bool) -> usize {
    if width == 0 || calls.is_empty() {
        return 0;
    }
    title_rows() + layout(calls, width, expanded, false).len()
}

/// 末尾动作行（展开 / 收起入口）在卡片内的行偏移；没有可展开正文时返回 None。
///
/// 与进程卡片的 `open` 是两种不同的命中动作：这一行切换整张文件卡片的展开状态，
/// 绝不打开后台任务面板（见 `history::ToolHitKind`）。动作行**常驻**——即使预览
/// 没有省略也画 `Show full diff`，用户不必先判断「有没有被截断」才知道能不能点。
pub(super) fn action_offset(calls: &[&ToolCallView], width: u16, expanded: bool) -> Option<usize> {
    if !has_diff(calls) {
        return None;
    }
    // 动作行恒为最后一行：几何直接取同一份 `layout`，绝不另算一遍。
    let rows = layout(calls, width, expanded, false).len();
    Some(title_rows() + rows.saturating_sub(1))
}

/// 绘制文件差异卡片；推进行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    calls: &[&ToolCallView],
    loading_frame: usize,
    expanded: bool,
    animating: bool,
    hovered: bool,
) {
    if rect.width == 0 || calls.is_empty() {
        return;
    }
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = if calls.len() > 1 {
            group_header(calls, rect.width, loading_frame, animating)
        } else {
            header_with_verb(
                calls[0],
                rect.width,
                loading_frame,
                animating,
                verb_of(calls[0]),
                None,
            )
        };
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    for row in layout(calls, rect.width, expanded, hovered) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            // 先铺整行底色再写字：`put_segments` 只写字符，不负责行背景。
            if let Some(fill) = row.fill {
                grid.background_row(rect.x, y, rect.width, fill);
            }
            put_segments(grid, rect.x, y, rect.width, &row.segments);
        }
        *row_index += 1;
    }
}

/// 一个物理行：分段 + 可选整行底色。
///
/// 底色必须与分段分开携带：`put_segments` 逐字符写入网格，不会替空格铺背景；
/// 新增/删除行要铺满**整行宽度**（含前缀与行尾空白），否则看不出这一行的范围。
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

/// 全部物理行（标题行除外）；[`rows`] 与 [`draw`] 共用。
///
/// 末尾**恒有一行动作行**（`… more changes` / `Show full diff` / `collapse`）：
/// 它是展开与收起的唯一入口，标题行不再可点。`hovered` 只影响动作行的颜色，
/// 不影响行数——因此 [`rows`] 传什么都一样。
fn layout(calls: &[&ToolCallView], width: u16, expanded: bool, hovered: bool) -> Vec<Row> {
    let files = files_of(calls);
    let mut rows = Vec::new();
    let last_index = files.len().saturating_sub(1);
    for (index, file) in files.iter().enumerate() {
        let last = index == last_index;
        rows.push(Row::plain(file_header(file, last, width)));
        rows.extend(file_body(file, width, expanded, last));
    }
    let omitted: usize = calls
        .iter()
        .filter_map(|call| call.file_view())
        .map(|view| view.omitted_files)
        .sum();
    if omitted > 0 {
        rows.push(Row::plain(prefixed(
            &continuation_prefix(),
            text::agent::file_diff_omitted_files(omitted),
            style::agent_warn(),
        )));
    }
    // 动作行只在真的存在可展开正文时出现：旧记录没有 `diff` 字段，画一个
    // 点了没反应的入口只会骗人（与 [`action_offset`] 同一判据）。
    if expandable(calls) {
        let action = if expanded {
            text::agent::FILE_DIFF_COLLAPSE
        } else if preview_is_omitted(calls, width) {
            text::agent::FILE_DIFF_OMITTED
        } else {
            text::agent::FILE_DIFF_SHOW_ALL
        };
        rows.push(Row::plain(vec![Segment::new(
            format!("{}{action}", continuation_prefix()),
            style::chat_action_label(hovered),
        )]));
    }
    rows
}

/// 组内按调用顺序、文件提交顺序扁平展开的文件列表。
fn files_of<'a>(calls: &[&'a ToolCallView]) -> Vec<&'a FileChangeView> {
    calls
        .iter()
        .filter_map(|call| call.file_view())
        .flat_map(|view| view.files.iter())
        .collect()
}

/// 组内是否有任何可展开的差异正文（与 [`has_diff`] 同口径）。
fn expandable(calls: &[&ToolCallView]) -> bool {
    calls
        .iter()
        .filter_map(|call| call.file_view())
        .any(|view| view.files.iter().any(|file| file.diff.is_some()))
}

/// 折叠预览是否被截断（决定动作行用哪一种文案）。
///
/// 与 [`preview_rows`] 同一口径：按每文件 3 个物理行的额度重算一遍，只有真的还有
/// 内容没画出来才算省略。计算量是 O(hunks)，只在布局时做，不进热路径。
fn preview_is_omitted(calls: &[&ToolCallView], width: u16) -> bool {
    let files = files_of(calls);
    let last_index = files.len().saturating_sub(1);
    files.iter().enumerate().any(|(index, file)| {
        let Some(diff) = file.diff.as_ref() else {
            return false;
        };
        let prefix = body_prefix(width, index == last_index);
        let available = usize::from(width).saturating_sub(prefix.width());
        let gutter = gutter_width(diff.hunks());
        preview_omitted(diff.hunks(), gutter, available.max(1))
    })
}

/// 工具名 → 卡片动词（非文件工具回退 `Edit`）。
fn verb_of(call: &ToolCallView) -> &'static str {
    match call.name.as_str() {
        "write" => text::agent::VERB_TOOL_WRITE,
        _ => text::agent::VERB_TOOL_EDIT,
    }
}

/// 合并卡的**状态汇总**：一档颜色回答「这张卡整体怎么样」。
///
/// 严重度：失败 > 中断 > 仍在跑 > 取消 > 完成。合并后标题只剩一个方块，
/// 不能把「哪一个调用失败了」显式画出来，因此失败必须压过其它一切。
fn group_mark(calls: &[&ToolCallView]) -> StepMark {
    let has = |matched: &dyn Fn(&ToolCallView) -> bool| calls.iter().any(|call| matched(call));
    if has(&|call| call.status == crate::app::state::ToolStatus::Failed) {
        StepMark::Failed
    } else if has(&|call| call.status == crate::app::state::ToolStatus::Interrupted) {
        StepMark::Interrupted
    } else if has(&|call| {
        matches!(
            call.status,
            crate::app::state::ToolStatus::InputStreaming
                | crate::app::state::ToolStatus::Pending
                | crate::app::state::ToolStatus::Running
        )
    }) {
        StepMark::Running
    } else if has(&|call| call.status == crate::app::state::ToolStatus::Cancelled) {
        StepMark::Cancelled
    } else {
        StepMark::Done
    }
}

/// 合并卡标题：`■ Edit 2 files (+34 -1)`。
///
/// 单次调用不走这里（标题保持裸动词），因为 `1 file` 只是把最常见的情况说成汇总。
fn group_header(
    calls: &[&ToolCallView],
    width: u16,
    loading_frame: usize,
    animating: bool,
) -> Vec<Segment> {
    let limit = usize::from(width);
    let mark = group_mark(calls);
    let tint = match mark {
        StepMark::Failed => Some(style::agent_fail()),
        StepMark::Interrupted => Some(style::agent_warn()),
        _ => None,
    };
    let paint = |segment: Segment| match tint {
        Some(tint) => Segment::new(segment.text, tint),
        None => segment,
    };
    let lead = paint(Segment::new(
        mark.segment(loading_frame, animating),
        mark.style(StepKind::Tool, false),
    ));
    if segments_cols(std::slice::from_ref(&lead)) > limit {
        return vec![lead];
    }
    let running = matches!(mark, StepMark::Running);
    let label = format!(
        "{} {} ",
        verb_of(calls[0]),
        text::agent::tool_files_label(calls.len())
    );
    let verb = paint(Segment::new(
        label,
        if running {
            style::chat_tool_verb_running()
        } else {
            style::chat_tool_verb()
        },
    ));
    let mut segments = vec![lead, verb];
    // 动词放不下就尾部省略（不挤掉行首方块）。
    while segments_cols(&segments) > limit && segments.len() > 1 {
        let remaining = limit.saturating_sub(segments[0].cols());
        let trimmed = truncate_with_ellipsis(&segments[1].text, remaining);
        if trimmed.is_empty() {
            segments.truncate(1);
        } else {
            segments[1] = Segment::new(trimmed, segments[1].style);
        }
    }
    // 合计计数：整项放得下才画（与逐文件行同一个「先丢计数」的让位顺序）。
    let mut counts = group_count_segments(calls);
    if segments_cols(&segments) + segments_cols(&counts) > limit {
        counts.clear();
    }
    segments.extend(counts);
    segments
}

/// 组内增删合计：`(+34 -1)`；有未知计数时追加 ` (?)`。
fn group_count_segments(calls: &[&ToolCallView]) -> Vec<Segment> {
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut unknown = false;
    for call in calls {
        let Some(view) = call.file_view() else {
            continue;
        };
        unknown |= view.omitted_files > 0;
        for file in &view.files {
            match (file.counts_known, file.added, file.removed) {
                (true, Some(a), Some(r)) => {
                    added += a;
                    removed += r;
                }
                _ => unknown = true,
            }
        }
    }
    let mut segments = vec![
        Segment::new(
            format!("({}", text::agent::added_label(added)),
            style::agent_added(),
        ),
        Segment::new(" ", style::text()),
        Segment::new(
            format!("{})", text::agent::removed_label(removed)),
            style::agent_removed(),
        ),
    ];
    if unknown {
        segments.push(Segment::new(
            format!(" {}", text::agent::FILE_DIFF_COUNTS_UNKNOWN),
            style::agent_step_detail(),
        ));
    }
    segments
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

/// 文件正文行的树形前缀：极窄时整个前缀让位，保证变更符号还看得见。
///
/// [`file_body`] 与 [`preview_is_omitted`] 共用同一份口径，否则「动作行文案」与
/// 「预览实际画了几行」会各自算各自的，省略提示时有时无。
fn body_prefix(width: u16, last: bool) -> String {
    if usize::from(width) > TREE_COLS + 3 {
        if last {
            continuation_prefix()
        } else {
            format!("  {}  ", text::agent::MARKER_TREE_BAR)
        }
    } else {
        String::new()
    }
}

/// 文件正文行（预览或完整展开）。
fn file_body(file: &FileChangeView, width: u16, expanded: bool, last: bool) -> Vec<Row> {
    let prefix = body_prefix(width, last);
    let available = usize::from(width).saturating_sub(prefix.width());
    let Some(diff) = file.diff.as_ref() else {
        return Vec::new();
    };
    let lang = language_of(&file.path);
    match diff {
        FileDiffView::Unavailable { reason } => vec![Row::plain(prefixed(
            &prefix,
            text::agent::file_diff_unavailable(reason),
            style::agent_step_detail(),
        ))],
        FileDiffView::Complete { hunks, .. } | FileDiffView::Truncated { hunks, .. } => {
            if hunks.is_empty() {
                return vec![Row::plain(prefixed(
                    &prefix,
                    text::agent::FILE_DIFF_NO_CHANGES,
                    style::agent_step_detail(),
                ))];
            }
            let gutter = gutter_width(hunks);
            let mut rows = if expanded {
                expanded_body(hunks, gutter, available, &prefix, lang)
            } else {
                preview_rows(hunks, gutter, available, &prefix, lang)
            };
            if expanded && diff.is_truncated() {
                rows.push(Row::plain(prefixed(
                    &prefix,
                    text::agent::FILE_DIFF_TRUNCATED,
                    style::agent_warn(),
                )));
            }
            if expanded && diff.simplified() {
                rows.push(Row::plain(prefixed(
                    &prefix,
                    text::agent::FILE_DIFF_SIMPLIFIED,
                    style::agent_step_detail(),
                )));
            }
            rows
        }
    }
}

/// 折叠预览：只取 Added/Removed 行，最多 [`PREVIEW_LINES`] 个物理行。
///
/// 省略提示**不在这里**：它已经是卡片末尾的常驻动作行（见 [`layout`]），这里只
/// 负责画预览，由 [`preview_omitted`] 决定动作行用哪一种文案。
fn preview_rows(
    hunks: &[DiffHunkView],
    gutter: usize,
    available: usize,
    prefix: &str,
    lang: Option<&'static str>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut budget = PREVIEW_LINES;
    // 只看增删行：上下文不占预览额度。
    let mut changes = hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind != DiffLineKind::Context);
    for line in changes.by_ref() {
        let physical = render_line(line, gutter, available, prefix, lang);
        if physical.len() > budget {
            // 这一行自己就写不完：剩下的内容同样属于「省略」。
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

/// 折叠预览是否截断了内容（决定动作行文案：`… more changes` 还是 `Show full diff`）。
///
/// 统计的是**物理行**总数而不是变更条数：一行超长被折成 4 行时，前 3 行就吃满了
/// 额度，第 4 行同样是省略。
fn preview_omitted(hunks: &[DiffHunkView], gutter: usize, available: usize) -> bool {
    let mut budget = PREVIEW_LINES;
    let mut changes = hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind != DiffLineKind::Context)
        .peekable();
    while let Some(line) = changes.next() {
        let physical = render_line(line, gutter, available, "", None).len();
        if physical > budget {
            return true;
        }
        budget -= physical;
        if budget == 0 {
            return changes.peek().is_some();
        }
    }
    false
}

/// 完整展开：全部已保存 hunks、hunk 间 `⋮` 与 EOF 元信息。
fn expanded_body(
    hunks: &[DiffHunkView],
    gutter: usize,
    available: usize,
    prefix: &str,
    lang: Option<&'static str>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for (index, hunk) in hunks.iter().enumerate() {
        if index > 0 {
            rows.push(Row::plain(prefixed(
                prefix,
                text::agent::FILE_DIFF_HUNK_SEP,
                style::agent_step_detail(),
            )));
        }
        for line in &hunk.lines {
            rows.extend(render_line(line, gutter, available, prefix, lang));
            if line.no_final_newline {
                rows.push(Row::plain(prefixed(
                    prefix,
                    text::agent::FILE_DIFF_NO_FINAL_NEWLINE,
                    style::agent_step_detail(),
                )));
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
///
/// 增删行铺整行底色：底色同时补到每个分段上（分段样式本身没有背景），
/// 再由 [`draw`] 先把整行刷成底色——两处缺一，行尾空白或字符格就会掉色。
fn render_line(
    line: &DiffLineView,
    gutter: usize,
    available: usize,
    prefix: &str,
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
    // 高亮分段本身不带背景：先把整行底色补到每段上，再换行——否则网格先铺的底色
    // 会被 `put_segments` 写回的无背景字符覆盖（字符格样式优先于行底色）。
    let spans: Vec<(String, Style)> = highlight_spans(lang, &safe, content_style)
        .into_iter()
        .map(|(text, span_style)| (text, tint(span_style, fill)))
        .collect();
    let wrapped = wrap_spans(&spans, content_width.max(1));
    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut segments = Vec::with_capacity(chunk.len() + 3);
            let head_style = tint(style::agent_step_detail(), fill);
            if index == 0 {
                segments.push(Segment::new(format!("{prefix}{number_text}"), head_style));
                segments.push(Segment::new(sign, tint(content_style, fill)));
                if separate {
                    segments.push(Segment::new(" ", tint(style::text(), fill)));
                }
            } else {
                // 续行保留空 gutter，不重复行号或符号。
                segments.push(Segment::new(
                    format!("{prefix}{}", " ".repeat(head_cols)),
                    head_style,
                ));
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

/// 按显示列换行，同时保留语法高亮分段：跨行处切开当前分段，不丢样式。
fn wrap_spans(spans: &[(String, Style)], width: usize) -> Vec<Vec<Segment>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Segment>> = Vec::new();
    let mut current: Vec<Segment> = Vec::new();
    let mut col = 0usize;
    for (text, style) in spans {
        let mut buf = String::new();
        for ch in text.chars() {
            let cell = ch.width().unwrap_or(1).max(1);
            if col + cell > width && col > 0 {
                if !buf.is_empty() {
                    current.push(Segment::new(std::mem::take(&mut buf), *style));
                }
                rows.push(std::mem::take(&mut current));
                col = 0;
            }
            buf.push(ch);
            col += cell;
        }
        if !buf.is_empty() {
            current.push(Segment::new(buf, *style));
        }
    }
    rows.push(current);
    rows
}

/// 按扩展名取轻量语法高亮的语言标记；`None` = 纯文本（不做假高亮）。
///
/// 只在文件级调用一次（不在每行重复解析路径）。
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
///
/// `plain` 是「没有识别出任何语法的那部分」使用的样式：差异卡片传增删语义色，
/// 让未高亮的字符仍然表达「新增 / 删除」，而不是变成与上下文同色的中性文本。
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
///
/// 逐行扫描（不跨行保留围栏状态）：差异卡片里每一行都是独立的一段，围栏配对
/// 依赖上下文本来就不成立，逐行高亮行内标记（标题/粗体/行内代码/列表）足够。
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
