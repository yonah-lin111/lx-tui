//! `exec_command` / `write_stdin` 卡片：树形摘要 + 按需原位展开。
//!
//! 版式（对应 design §10.2 / §10.3）：
//!
//! ```text
//! ■ Run command · Running
//!   │  $ cargo test chat::tools
//!   │  test read::empty_file ... ok
//!   │  test result: ok. 24 passed
//!   └─ … +21 more lines          ← 点这一行展开
//! ```
//!
//! - **标题行**沿用文件工具同一套 `■ 动词` 语法；方块颜色/呼吸表达状态，
//!   状态文字用 ` · ` 分隔紧跟在动词右侧（不是行尾右对齐，右上角没有 `[details]`）；
//!   窄屏整项丢弃状态、再收敛动词，方块永远保留；
//! - **树形体**从第 2 行起：命令、cwd、输出共用一根竖线 `│`，**最后一行**用
//!   `└─` 收口（与 prompt list、文件工具的 `  └─` 同一套直角语法；两段前缀等宽
//!   5 列，内容逐行左对齐）；
//! - 折叠时只留最近 [`PREVIEW_LINES`] 行输出，被省略的行数用
//!   `… +N more lines` **显式**标在树体**末尾**，这一行就是展开入口；
//! - 展开时补上 cwd 与全部保留输出（上限 [`EXPANDED_LINES`]，滚动交给外层历史区），
//!   末尾一行变成 `[collapse]`，点它折回去；
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
            title_rows() + interaction_body(view, expanded).len()
        }
        _ => title_rows(),
    }
}

/// 命令卡片树体行（命令 + cwd + 输出 + 末尾可点击行）；空卡片也至少一行占位。
///
/// **与 [`draw_command`] 共用同一份构造**：行数口径只有一个来源。
fn command_body(view: &CommandView, expanded: bool) -> Vec<BodyRow> {
    let lines = output_lines(&view.output_tail);
    let mut body = vec![BodyRow::command(
        &view.command,
        view.process_session_id,
        view.tty,
    )];
    // 只在展开时出现的内容（cwd、输出被淘汰的告警）。
    let mut extras: Vec<BodyRow> = Vec::new();
    let cwd = text::agent::cwd_label(&view.cwd);
    if !cwd.trim().is_empty() {
        extras.push(BodyRow::detail(cwd));
    }
    if view.dropped_bytes > 0 {
        extras.push(BodyRow::warn(text::agent::EARLIER_OUTPUT_DISCARDED));
    }
    if expanded {
        body.extend(extras.iter().cloned());
    }
    body.extend(output_rows(
        &lines,
        expanded,
        text::agent::WAITING_FOR_OUTPUT,
    ));
    if let Some(row) = toggle_row(&lines, expanded, !extras.is_empty()) {
        body.push(row);
    }
    body
}

/// 交互卡片树体行（命令 + 输入预览 + 输出 + 末尾可点击行）。
fn interaction_body(view: &ProcessInteractionView, expanded: bool) -> Vec<BodyRow> {
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
    if expanded && view.dropped_bytes > 0 {
        body.push(BodyRow::warn(text::agent::EARLIER_OUTPUT_DISCARDED));
    }
    let lines = output_lines(&view.output);
    body.extend(output_rows(&lines, expanded, text::agent::NO_NEW_OUTPUT));
    if let Some(row) = toggle_row(&lines, expanded, view.dropped_bytes > 0) {
        body.push(row);
    }
    body
}

/// 输出行（折叠时只取尾部 [`PREVIEW_LINES`] 行，展开时取尾部 [`EXPANDED_LINES`] 行）；
/// 没有输出时给一行显式占位（`placeholder`）。
fn output_rows(lines: &[String], expanded: bool, placeholder: &str) -> Vec<BodyRow> {
    if lines.is_empty() {
        return vec![BodyRow::detail(placeholder)];
    }
    let shown = if expanded {
        lines.len().min(EXPANDED_LINES)
    } else {
        lines.len().min(PREVIEW_LINES)
    };
    let start = lines.len().saturating_sub(shown);
    lines[start..]
        .iter()
        .map(|line| BodyRow::output(line.clone()))
        .collect()
}

/// 树体末尾的展开/收起行（可点击，见 `history::ToolHit` 的命中区）。
///
/// - 折叠且**有被省略的输出行**：`… +21 more lines` —— 点它展开；
/// - 展开：`[collapse]` —— 点它折回。展开态只要真的多露出过东西（省略行或 cwd 这类
///   注脚）就一定有这一行，否则用户点开后再也找不到往里收的入口；
/// - 折叠且输出全在：不画——短卡片没有省略行，`… +0 more lines` 只会是噪声
///   （标题行仍可点，展开后 cwd 与 `[collapse]` 才出现）。
fn toggle_row(lines: &[String], expanded: bool, has_extras: bool) -> Option<BodyRow> {
    let hidden = lines.len().saturating_sub(lines.len().min(PREVIEW_LINES));
    if expanded {
        return (hidden > 0 || has_extras)
            .then(|| BodyRow::toggle(text::agent::DETAILS_CLOSE, None));
    }
    (hidden > 0).then(|| {
        BodyRow::toggle(
            format!(
                "{}{} {}",
                text::agent::EARLIER_OUTPUT_HINT,
                hidden,
                text::agent::EARLIER_OUTPUT_SUFFIX,
            ),
            Some(format!("{}{}", text::agent::EARLIER_OUTPUT_HINT, hidden)),
        )
    })
}

/// 末尾展开/收起行的**卡片内行偏移**（0 = 标题行）；没有这一行时返回 None。
///
/// `history::layout` 用它定位可点击的命中矩形：渲染与命中必须共用同一份树体构造，
/// 否则点击区会与画出来的那一行错位。
pub(super) fn toggle_offset(call: &ToolCallView, expanded: bool) -> Option<usize> {
    let body = match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => command_body(view, expanded),
        Some(crate::app::state::ToolDetailView::Interaction(view)) => {
            interaction_body(view, expanded)
        }
        _ => return None,
    };
    body.last()
        .is_some_and(|row| row.toggle)
        .then(|| body.len() - 1)
}

/// 树体的一行：**未加连接符**的内容 + 着色档位。
///
/// 连接符由 [`draw_body`] 统一补：除最后一行外都是 `  │  `，最后一行 `  └─ `。
/// 这样「哪一行收口」只有一处判断，行数永远等于 `body.len()`。
#[derive(Clone)]
struct BodyRow {
    text: String,
    style: ratatui::style::Style,
    /// 单行化后的取样上限（命令可更长，输出同样限长）。
    sample: usize,
    /// 是否是末尾的展开/收起行（命中区单独收，见 `history::layout`）。
    toggle: bool,
    /// 完整文案放不下时的紧凑形态；连它都放不下就只留连接符（行仍可点）。
    ///
    /// 只给折叠提示用：`… +6 more lines` 在窄窗格里会被截成 `… +6 more …`，
    /// 退成 `… +6` 既可读又点得动。展开态的 `[collapse]` 没有更短的等价说法，
    /// 放不下就整段不画（与「整项丢弃」一致）。
    compact: Option<String>,
}

impl BodyRow {
    fn command(command: &str, _session: Option<u64>, _tty: bool) -> Self {
        Self {
            text: format!("{}{command}", text::agent::COMMAND_PROMPT_PREFIX),
            style: style::chat_tool_param(),
            sample: SAMPLE_CHARS,
            toggle: false,
            compact: None,
        }
    }

    fn detail(content: impl Into<String>) -> Self {
        Self {
            text: content.into(),
            style: style::agent_step_detail(),
            sample: SAMPLE_CHARS,
            toggle: false,
            compact: None,
        }
    }

    fn warn(content: impl Into<String>) -> Self {
        Self {
            text: content.into(),
            style: style::agent_warn(),
            sample: SAMPLE_CHARS,
            toggle: false,
            compact: None,
        }
    }

    fn output(content: impl Into<String>) -> Self {
        Self {
            text: content.into(),
            style: style::text(),
            sample: SAMPLE_CHARS,
            toggle: false,
            compact: None,
        }
    }

    /// 末尾的展开/收起行（沿用旧 `[details]` 的次要信息色：同一处提示，只是挪到了底部）。
    fn toggle(content: impl Into<String>, compact: Option<String>) -> Self {
        Self {
            text: content.into(),
            style: style::agent_step_detail(),
            sample: SAMPLE_CHARS,
            toggle: true,
            compact,
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
            draw_interaction(
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
        _ => {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                let status = plain_status_label(call.status);
                let segments = header_with_verb(
                    call,
                    rect.width,
                    loading_frame,
                    text::agent::VERB_EXEC_COMMAND,
                    status,
                );
                put_segments(grid, rect.x, y, rect.width, &segments);
            }
            *row_index += 1;
        }
    }
}

/// 命令卡片：标题 + 树体（命令 / cwd / 输出 / 收口）。
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
    let status = process_status(view.state);
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
    expanded: bool,
) {
    let status = process_status(view.state);
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
        mark,
    );
    draw_body(
        grid,
        rect,
        scroll,
        row_index,
        &interaction_body(view, expanded),
    );
}

/// 标题行（`■ 动词 · 状态`）；推进 1 行。
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
    mark: StepMark,
) {
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = super::header_with_mark(call, rect.width, loading_frame, verb, status, mark);
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
            // 展开/收起行按「整项丢弃」让位：完整 → 紧凑（只留 `… +N`）→ 整段不画。
            // 绝不截半段——`[colla…` 读不出含义，还会让人以为点不了。
            // （丢掉后标题行与 `^o` 仍是可用入口。）
            let content = if row.toggle {
                let full = single_line(&row.text, row.sample);
                if full.width() <= content_cols {
                    full
                } else {
                    row.compact
                        .as_deref()
                        .map(|compact| single_line(compact, row.sample))
                        .filter(|compact| compact.width() <= content_cols)
                        .unwrap_or_default()
                }
            } else {
                let single = single_line(&row.text, row.sample);
                if single.width() <= content_cols {
                    single
                } else {
                    truncate_with_ellipsis(&single, content_cols)
                }
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

/// 进程状态文案：`Running` / `Exited 0` / `exit 2`。
///
/// **不显示运行时长**。时长只在后台任务面板里给（那里会定时刷新），因为：
///
/// 1. 消息列表是历史记录，一张卡片的时长每帧都在变会让整段历史"活起来"，
///    滚动与复制时都不稳定；
/// 2. 时长属于"现在还有几个服务在跑、各自跑了多久"这件事，面板才是回答它的地方。
fn process_status(state: ProcessStateView) -> Option<String> {
    Some(text::agent::proc_exit_label(state))
}

/// 没有类型化详情时的状态词（例如只有失败错误的卡片）。
fn plain_status_label(status: ToolStatus) -> Option<String> {
    match status {
        ToolStatus::Completed | ToolStatus::Failed => None,
        other => Some(text::tool_status_label(other).to_string()),
    }
}
