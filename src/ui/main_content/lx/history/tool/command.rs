//! `exec_command` / `write_stdin` 卡片：树形摘要 + 按需原位展开。
//!
//! 版式（对应 design §10.2 / §10.3）：
//!
//! ```text
//! ■ Run command                              Running · 12s  [details]
//!   │  $ cargo test chat::tools
//!   │  … +21 earlier lines
//!   │  test read::empty_file ... ok
//!   └─ test result: ok. 24 passed
//! ```
//!
//! - **标题行**沿用文件工具同一套 `■ 动词` 语法；方块颜色/呼吸表达状态，
//!   状态文字与 `[details]` 靠右（窄屏先丢文字、再丢标记，方块永远保留）；
//! - **树形体**从第 2 行起：命令、cwd、输出共用一根竖线 `│`，**最后一行**用
//!   `└─` 收口（与 prompt list、文件工具的 `  └─` 同一套直角语法；两段前缀等宽
//!   5 列，内容逐行左对齐）；
//! - 折叠时只留最近 [`PREVIEW_LINES`] 行输出，被省略的行数用
//!   `… +N earlier lines` **显式**标在树体里（绝不静默丢）；
//! - 展开时补上 cwd 与全部保留输出（上限 [`EXPANDED_LINES`]，滚动交给外层历史区）；
//! - 没有输出时显式占位（`Waiting for output` / `No new output`），绝不画空行；
//! - 卡片只读快照：不解析 JSON、不读进程、不读数据库。
//!
//! 版式刻意**只用 `│` 与 `└─` 两种符号**：命令、stdin、每行输出都靠同一根竖线串成
//! 一个整体，只有最末一行收口。早先试过给每个「节点」开头再加 `├─`，结果一段输出
//! 里混着 `├─` / `│` / `└─` 三种符号，比原来更花；统一成一根树干后反而干净
//! （用户反馈：保留原来那套）。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{
    CommandView, ProcessInteractionView, ProcessStateView, ToolCallView, ToolStatus,
};
use crate::ui::{style, text};

use super::super::{Grid, StepMark, visible_row};
use super::{
    TREE_COLS, header_with_verb, mark_of, put_segments, single_line, title_rows,
    truncate_with_ellipsis,
};

/// 命令卡片折叠时的输出预览行数。
///
/// 与视图常量 `COMMAND_PREVIEW_LINES` 同值（同一口径来源）；这里保留本地名是因为
/// 渲染层不该为了一个数字跨模块引用 app 状态类型。
pub(super) const PREVIEW_LINES: usize =
    crate::app::state::lx::chat::tool_view::COMMAND_PREVIEW_LINES;
/// 展开时单卡片最多显示的输出行数（滚动交给外层历史区）。
const EXPANDED_LINES: usize = 200;
/// 单行取样上限（限住每帧成本）。
const SAMPLE_CHARS: usize = 512;
/// 树体行首连接符两侧的缩进（与 [`TREE_COLS`] 同口径：`  │  ` = 2 + 1 + 2）。
const TREE_INDENT: usize = 2;

/// 命令 / 交互卡片占用的渲染行数；`expanded` 由窗格内展开状态决定。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => {
            title_rows() + command_body(view, expanded).len()
        }
        Some(crate::app::state::ToolDetailView::Interaction(view)) => {
            title_rows() + interaction_body(view).len()
        }
        _ => title_rows(),
    }
}

/// 命令卡片树体行（命令 + cwd + 输出 + 收口提示）；空卡片也至少一行占位。
///
/// **与 [`draw_command`] 共用同一份构造**：行数口径只有一个来源。
fn command_body(view: &CommandView, expanded: bool) -> Vec<BodyRow> {
    let lines = output_lines(&view.output_tail);
    let mut body = vec![BodyRow::command(
        &view.command,
        view.process_session_id,
        view.tty,
    )];
    if expanded {
        let cwd = text::agent::cwd_label(&view.cwd);
        if !cwd.trim().is_empty() {
            body.push(BodyRow::detail(cwd));
        }
        if view.dropped_bytes > 0 {
            body.push(BodyRow::warn(text::agent::EARLIER_OUTPUT_DISCARDED));
        }
    }
    if lines.is_empty() {
        body.push(BodyRow::detail(text::agent::WAITING_FOR_OUTPUT));
        return body;
    }
    let total = lines.len().min(EXPANDED_LINES);
    let shown = if expanded {
        lines.len().min(EXPANDED_LINES)
    } else {
        lines.len().min(PREVIEW_LINES)
    };
    // 折叠时把「还有多少行没显示」显式标出来（展开后不再需要这条）。
    if !expanded && total > shown {
        body.push(BodyRow::detail(format!(
            "{}{} {}",
            text::agent::EARLIER_OUTPUT_HINT,
            total - shown,
            text::agent::EARLIER_OUTPUT_SUFFIX,
        )));
    }
    let start = lines.len().saturating_sub(shown);
    body.extend(
        lines[start..]
            .iter()
            .map(|line| BodyRow::output(line.clone())),
    );
    body
}

/// 交互卡片树体行（命令 + 输入预览 + 输出 + 空占位）。
fn interaction_body(view: &ProcessInteractionView) -> Vec<BodyRow> {
    let mut body = vec![BodyRow::command(
        &view.command,
        Some(view.process_session_id),
        false,
    )];
    if let Some(input) = &view.input_preview {
        body.push(BodyRow::detail(format!(
            "{} {input}",
            text::agent::INPUT_PREFIX
        )));
    }
    if view.dropped_bytes > 0 {
        body.push(BodyRow::warn(text::agent::EARLIER_OUTPUT_DISCARDED));
    }
    let lines = output_lines(&view.output);
    if lines.is_empty() {
        body.push(BodyRow::detail(text::agent::NO_NEW_OUTPUT));
        return body;
    }
    let shown = lines.len().min(PREVIEW_LINES);
    if lines.len() > shown {
        body.push(BodyRow::detail(format!(
            "{}{} {}",
            text::agent::EARLIER_OUTPUT_HINT,
            lines.len() - shown,
            text::agent::EARLIER_OUTPUT_SUFFIX,
        )));
    }
    let start = lines.len().saturating_sub(shown);
    body.extend(
        lines[start..]
            .iter()
            .map(|line| BodyRow::output(line.clone())),
    );
    body
}

/// 树体的一行：**未加连接符**的内容 + 着色档位。
///
/// 连接符由 [`draw_body`] 统一补：除最后一行外都是 `  │  `，最后一行 `  └─ `。
/// 这样「哪一行收口」只有一处判断，行数永远等于 `body.len()`。
struct BodyRow {
    text: String,
    style: ratatui::style::Style,
    /// 单行化后的取样上限（命令可更长，输出同样限长）。
    sample: usize,
}

impl BodyRow {
    fn command(command: &str, _session: Option<u64>, _tty: bool) -> Self {
        Self {
            text: format!("{}{command}", text::agent::COMMAND_PROMPT_PREFIX),
            style: style::chat_tool_param(),
            sample: SAMPLE_CHARS,
        }
    }

    fn detail(content: impl Into<String>) -> Self {
        Self {
            text: content.into(),
            style: style::agent_step_detail(),
            sample: SAMPLE_CHARS,
        }
    }

    fn warn(content: impl Into<String>) -> Self {
        Self {
            text: content.into(),
            style: style::agent_warn(),
            sample: SAMPLE_CHARS,
        }
    }

    fn output(content: impl Into<String>) -> Self {
        Self {
            text: content.into(),
            style: style::text(),
            sample: SAMPLE_CHARS,
        }
    }
}

/// 输出文本 → 显示行（保留中间空行；末尾换行不额外占行）。
fn output_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// 绘制命令或交互卡片；推进的行数必须与 [`rows`] 严格相等。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    expanded: bool,
) {
    if rect.width == 0 {
        // 宽度 0：与 [`rows`] 一致地不占行（外层布局已跳过该块）。
        return;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => {
            draw_command(
                grid,
                rect,
                scroll,
                row_index,
                call,
                view,
                loading_frame,
                expanded,
            );
        }
        Some(crate::app::state::ToolDetailView::Interaction(view)) => {
            draw_interaction(grid, rect, scroll, row_index, call, view, loading_frame);
        }
        _ => {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                let status = plain_status_label(call.status);
                let segments = header_with_verb(
                    call,
                    rect.width,
                    loading_frame,
                    text::agent::VERB_EXEC_COMMAND,
                    status,
                    expanded,
                );
                put_segments(grid, rect.x, y, rect.width, &segments);
            }
            *row_index += 1;
        }
    }
}

/// 命令卡片：标题 + 树体（命令 / cwd / 输出 / 收口）。
#[allow(clippy::too_many_arguments)]
fn draw_command(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &CommandView,
    loading_frame: usize,
    expanded: bool,
) {
    let status = process_status(view.state, view.started_at_ms);
    // 进程仍在跑时方块继续呼吸——即便本次工具调用已经结算（命令被留在了后台）。
    let mark = if view.state.is_running() && call.status != ToolStatus::Failed {
        StepMark::Running
    } else {
        mark_of(call.status, loading_frame)
    };
    draw_title(
        grid,
        rect,
        scroll,
        row_index,
        call,
        loading_frame,
        text::agent::VERB_EXEC_COMMAND,
        status,
        expanded,
        mark,
    );
    draw_body(grid, rect, scroll, row_index, &command_body(view, expanded));
}

/// 交互卡片：标题 + 树体（命令 / stdin / 输出 / 收口）。
fn draw_interaction(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &ProcessInteractionView,
    loading_frame: usize,
) {
    let status = process_status(view.state, None);
    let mark = if view.state.is_running() && call.status != ToolStatus::Failed {
        StepMark::Running
    } else {
        mark_of(call.status, loading_frame)
    };
    draw_title(
        grid,
        rect,
        scroll,
        row_index,
        call,
        loading_frame,
        text::agent::interaction_verb(view.action),
        status,
        false,
        mark,
    );
    draw_body(grid, rect, scroll, row_index, &interaction_body(view));
}

/// 标题行（`■ 动词` + 右对齐状态/标记）；推进 1 行。
#[allow(clippy::too_many_arguments)]
fn draw_title(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    verb: &str,
    status: Option<String>,
    expanded: bool,
    mark: StepMark,
) {
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = super::header_with_mark(
            call,
            rect.width,
            loading_frame,
            verb,
            status,
            expanded,
            mark,
        );
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
}

/// 绘制树体：除最后一行外 `  │  `，最后一行 `  └─ `；推进的行数 = `body.len()`。
fn draw_body(grid: &mut Grid, rect: Rect, scroll: usize, row_index: &mut usize, body: &[BodyRow]) {
    let last = body.len().saturating_sub(1);
    let limit = usize::from(rect.width);
    let content_x = rect
        .x
        .saturating_add(u16::try_from(TREE_COLS).unwrap_or(u16::MAX));
    for (index, row) in body.iter().enumerate() {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            // 连接符：`  │  ` / `  └─ `。`│` 与 `└` 落在同一列，树干连成一条线。
            let bar_style = style::chat_notice_connector();
            let indent = u16::try_from(TREE_INDENT).unwrap_or(0);
            let leaf = index == last;
            grid.put_clipped(rect.x, y, &" ".repeat(TREE_INDENT), bar_style, rect.width);
            grid.put_clipped(
                rect.x.saturating_add(indent),
                y,
                if leaf {
                    text::agent::MARKER_TREE_LEAF
                } else {
                    text::agent::MARKER_TREE_BAR
                },
                bar_style,
                rect.width.saturating_sub(indent),
            );
            let content_cols = limit.saturating_sub(TREE_COLS);
            let single = single_line(&row.text, row.sample);
            let content = if single.width() <= content_cols {
                single
            } else {
                truncate_with_ellipsis(&single, content_cols)
            };
            grid.put_clipped(
                content_x,
                y,
                &content,
                row.style,
                rect.width
                    .saturating_sub(u16::try_from(TREE_COLS).unwrap_or(u16::MAX)),
            );
        }
        *row_index += 1;
    }
}

/// 进程状态文案 + 运行时长：`Running · 12s` / `Exited 0` / `exit 2`。
///
/// 时长只在**跑够一秒**后才追加上去：刚 spawn 的命令显示 `Running · 1ms` 既是噪声，
/// 又会在输出预览刷新时反复抖动（每帧变一个数）。不足一秒统一显示纯状态词。
fn process_status(state: ProcessStateView, started_at_ms: Option<i64>) -> Option<String> {
    let label = text::agent::proc_exit_label(state);
    // 时长只在进程**仍在跑**时才有意义：终态的时间戳冻结在 `ended_at_ms`，
    // 拿 `now - started` 算会一路涨下去（"关掉了还在计时"那个 bug 的直接原因）。
    if state.is_running()
        && let Some(started) = started_at_ms
    {
        let elapsed = crate::db::now_ms().saturating_sub(started).max(0);
        if elapsed >= 1_000 {
            return Some(format!(
                "{label}{}{}",
                text::LX_CHAT_TOOL_STATUS_SEP,
                format_elapsed(elapsed)
            ));
        }
    }
    Some(label)
}

/// 时长短格式：`820ms` / `4s` / `1m 12s`。
fn format_elapsed(ms: i64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1_000.0;
    if seconds < 60.0 {
        return format!("{seconds:.0}s");
    }
    let minutes = (seconds / 60.0).floor() as i64;
    let rest = (seconds % 60.0).round() as i64;
    format!("{minutes}m {rest}s")
}

/// 没有类型化详情时的状态词（例如只有失败错误的卡片）。
fn plain_status_label(status: ToolStatus) -> Option<String> {
    match status {
        ToolStatus::Completed | ToolStatus::Failed => None,
        other => Some(text::tool_status_label(other).to_string()),
    }
}
