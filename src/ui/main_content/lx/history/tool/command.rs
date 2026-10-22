//! `exec_command` / `write_stdin` 卡片：标题 + 折行的命令。
//!
//! ```text
//! ■ Run command · Exited 0         ← 橙 = 进程还活着，绿 = 已停止
//!   └─ $ { echo "==== 项目概览 ===="
//!          echo "Rust 版本: ... }    ← 长命令按列宽折行（折叠最多 2 行）
//! ```
//!
//! - **标题行**沿用文件工具同一套 `■ 动词` 语法；方块颜色/呼吸表达状态，
//!   状态文字用 ` · ` 分隔紧跟在动词右侧（不是行尾右对齐）；
//!   窄屏整项丢弃状态、再收敛动词，方块永远保留；
//! - **进程存活时整行橙色**（[`StepMark::Alive`]）：判据是进程状态而不是工具结算
//!   状态——命令交回后台后工具已经 `Completed`，但进程还在跑；停止后回翡翠绿；
//! - **命令折行**：`$ 命令` 按可用列宽软换行，折叠态最多 2 个物理行、展开态铺开
//!   全部（上限与普通工具详情一致）。折叠/展开靠**点卡片内容**，没有按钮、
//!   没有 `open` 行（后台任务面板改用 `^o` / 空输入框按 `↓` 打开）；
//! - 卡片只读快照：不解析 JSON、不读进程、不读数据库。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{
    CommandView, ProcessInteractionView, ProcessStateView, ToolCallView, ToolStatus,
};
use crate::ui::{style, text};

use super::super::{Grid, StepMark, visible_row};
use super::{
    TREE_COLS, header_with_verb, mark_of, put_segments, single_line, title_rows,
    truncate_with_ellipsis,
};

/// 单行取样上限（限住每帧成本）。
const SAMPLE_CHARS: usize = 512;
/// 树体行首连接符两侧的缩进（与 [`TREE_COLS`] 同口径：`  │  ` = 2 + 1 + 2）。
const TREE_INDENT: usize = 2;

/// 命令 / 交互卡片占用的渲染行数：标题 + 命令的物理行。
///
/// `expanded` 只影响折叠额度：折叠最多 [`super::file_summary::DETAIL_PREVIEW_ROWS`]
/// 行，展开最多 [`super::file_summary::DETAIL_MAX_ROWS`] 行——与普通工具详情同一口径。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => {
            title_rows() + command_lines(&view.command, width, expanded)
        }
        Some(crate::app::state::ToolDetailView::Interaction(view)) => {
            title_rows() + command_lines(&view.command, width, expanded)
        }
        _ => title_rows(),
    }
}

/// 命令正文的物理行数（与 [`draw_body`] 同一份折行口径）。
fn command_lines(command: &str, width: u16, expanded: bool) -> usize {
    command_body(command, width, expanded, false).len()
}

/// 折叠态可提亮的命令行数（折叠额度内的内容行，不含剩余行数提示）。
pub(super) fn foldable_rows(call: &ToolCallView, width: u16) -> usize {
    let command = match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => &view.command,
        Some(crate::app::state::ToolDetailView::Interaction(view)) => &view.command,
        _ => return 0,
    };
    command_body(command, width, false, false)
        .iter()
        .filter(|row| !row.text.starts_with("… "))
        .count()
}

/// 命令**内容**行数（不含末尾的「剩余行数」提示行）；供「能否展开」判定用。
pub(super) fn content_rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    let command = match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => &view.command,
        Some(crate::app::state::ToolDetailView::Interaction(view)) => &view.command,
        _ => return 0,
    };
    command_body(command, width, expanded, false)
        .iter()
        .filter(|row| !row.text.starts_with("… "))
        .count()
}

/// 命令卡片树体行：`$ 命令` 按列宽折行后的物理行。
///
/// **与 [`draw_command`]、[`rows`] 共用同一份构造**：行数口径只有一个来源。输出、
/// cwd 与 `… +N more lines` 都已搬进后台任务面板，消息列表里只看得到"哪条命令在跑"。
fn command_body(command: &str, width: u16, expanded: bool, hovered: bool) -> Vec<BodyRow> {
    // 已带提示符就不重复（调用方可能传入完整命令行）。
    let text = if command.starts_with(text::agent::COMMAND_PROMPT_PREFIX) {
        command.to_string()
    } else {
        format!("{}{command}", text::agent::COMMAND_PROMPT_PREFIX)
    };
    // 折叠内容悬停提亮为默认前景（没有按钮，hover 是「这块可点」的唯一提示）。
    let line = BodyRow {
        text,
        style: if hovered {
            style::chat_tool_param_hover()
        } else {
            style::chat_tool_param()
        },
        sample: SAMPLE_CHARS,
    };
    wrap_body_row(line, width, expanded)
}

/// 一条树体行按可用列宽折成若干物理行（首行带 `$ `，续行不重复）。
///
/// 预算与普通工具详情一致：折叠 [DETAIL_PREVIEW_ROWS] 行、展开 [DETAIL_MAX_ROWS] 行；
/// 截断时末行补 `…`。极窄（放不下任何正文）时不折行，避免把内容撕成碎片。
fn wrap_body_row(row: BodyRow, width: u16, expanded: bool) -> Vec<BodyRow> {
    use super::file_summary::{DETAIL_MAX_ROWS, DETAIL_PREVIEW_ROWS};
    let available = usize::from(width.saturating_sub(super::TREE_COLS as u16));
    if available < super::MIN_PARAM_COLS {
        return vec![row];
    }
    let budget = if expanded {
        DETAIL_MAX_ROWS
    } else {
        DETAIL_PREVIEW_ROWS
    };
    // 组装按「可用列 × 行预算」放宽，让长命令真正折行而不是一路压成省略号。
    let safe = single_line(&row.text, row.sample.saturating_mul(budget).max(row.sample));
    let chars: Vec<char> = safe.chars().collect();
    let mut out: Vec<BodyRow> = Vec::new();
    let mut start = 0usize;
    while start < chars.len() && out.len() < budget {
        let mut col = 0usize;
        let mut end = start;
        while end < chars.len() {
            let cell = chars[end].width().unwrap_or(1).max(1);
            if col + cell > available {
                break;
            }
            col += cell;
            end += 1;
        }
        if end == start {
            end = start + 1;
        }
        let mut text: String = chars[start..end].iter().collect();
        // 末行仍有内容没画完：补 `…`（截断标记不占新行）。
        if end < chars.len() && out.len() + 1 == budget {
            while text.width() + 1 > available && !text.is_empty() {
                text.pop();
            }
            text.push('…');
        }
        out.push(BodyRow {
            text: text.trim_end().to_string(),
            style: row.style,
            sample: row.sample,
        });
        start = end;
    }
    if out.is_empty() {
        out.push(BodyRow {
            text: String::new(),
            style: row.style,
            sample: row.sample,
        });
    }
    // 折叠态末尾单独一行写剩余行数（展开后消失）；不占折叠额度。
    // 窄到放不下整句就不画，避免半截提示。
    if !expanded && start < chars.len() {
        let hint =
            text::agent::file_diff_more_lines(estimate_remaining_lines(&chars[start..], available));
        if hint.width() <= available {
            out.push(BodyRow {
                text: hint,
                style: style::agent_step_detail(),
                sample: SAMPLE_CHARS,
            });
        }
    }
    out
}

/// 剩余内容还能折成几行（只用于提示文案，按整列硬切估算即可）。
fn estimate_remaining_lines(rest: &[char], available: usize) -> usize {
    let width = available.max(1);
    let cols: usize = rest.iter().map(|ch| ch.width().unwrap_or(1).max(1)).sum();
    cols.div_ceil(width).max(1)
}

/// 树体的一行：**未加连接符**的内容 + 着色档位。
///
/// 连接符由 [`draw_body`] 统一补：除最后一行外都是 `  │  `，最后一行 `  └─ `。
/// 这样「哪一行收口」只有一处判断，行数永远等于 `body.len()`。
#[derive(Clone)]
struct BodyRow {
    text: String,
    style: ratatui::style::Style,
    /// 单行化后的取样上限（命令可能很长）。
    sample: usize,
}

/// 绘制命令或交互卡片；推进的行数必须与 [`rows`] 严格相等。
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
    hovered: bool,
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
                animating,
                expanded,
                hovered,
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
                animating,
                expanded,
                hovered,
            );
        }
        _ => {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                let status = plain_status_label(call.status);
                let segments = header_with_verb(
                    call,
                    rect.width,
                    loading_frame,
                    animating,
                    text::agent::VERB_EXEC_COMMAND,
                    status,
                );
                put_segments(grid, rect.x, y, rect.width, &segments);
            }
            *row_index += 1;
        }
    }
}

/// 进程存活时标题的橙色档位（决策 1：橙 = 还在跑，绿 = 停了）。
///
/// 判据是**进程状态**而不是工具的结算状态：命令交回后台后工具已 `Completed`，
/// 但进程仍在跑，此时必须继续显示橙色。
fn process_mark(call: &ToolCallView, state: ProcessStateView, loading_frame: usize) -> StepMark {
    if state.is_running() && call.status != ToolStatus::Failed {
        StepMark::Alive
    } else {
        mark_of(call.status, loading_frame)
    }
}

/// 命令卡片：标题（橙 = 存活）+ 折行的命令。
#[allow(clippy::too_many_arguments)]
fn draw_command(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &CommandView,
    loading_frame: usize,
    animating: bool,
    expanded: bool,
    hovered: bool,
) {
    let status = process_status(view.state);
    let mark = process_mark(call, view.state, loading_frame);
    draw_title(
        grid,
        rect,
        scroll,
        row_index,
        call,
        loading_frame,
        animating,
        text::agent::VERB_EXEC_COMMAND,
        status,
        mark,
    );
    draw_body(
        grid,
        rect,
        scroll,
        row_index,
        &command_body(&view.command, rect.width, expanded, hovered),
        expanded.then(style::chat_expanded_bg),
    );
}

/// 交互卡片：标题 + 折行的命令。
#[allow(clippy::too_many_arguments)]
fn draw_interaction(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &ProcessInteractionView,
    loading_frame: usize,
    animating: bool,
    expanded: bool,
    hovered: bool,
) {
    let status = process_status(view.state);
    let mark = process_mark(call, view.state, loading_frame);
    draw_title(
        grid,
        rect,
        scroll,
        row_index,
        call,
        loading_frame,
        animating,
        text::agent::interaction_verb(view.action),
        status,
        mark,
    );
    draw_body(
        grid,
        rect,
        scroll,
        row_index,
        &command_body(&view.command, rect.width, expanded, hovered),
        expanded.then(style::chat_expanded_bg),
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
    animating: bool,
    verb: &str,
    status: Option<String>,
    mark: StepMark,
) {
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = super::header_with_mark(
            call,
            rect.width,
            loading_frame,
            animating,
            verb,
            status,
            mark,
        );
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
}

/// 绘制树体：除最后一行外 `  │  `，最后一行 `  └─ `；推进的行数 = `body.len()`。
fn draw_body(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    body: &[BodyRow],
    fill: Option<ratatui::style::Style>,
) {
    // 末行收口：剩余行数提示行也要收口（它本身是卡片最后一行）。
    let last = body.len().saturating_sub(1);
    let limit = usize::from(rect.width);
    let content_x = rect
        .x
        .saturating_add(u16::try_from(TREE_COLS).unwrap_or(u16::MAX));
    for (index, row) in body.iter().enumerate() {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            if let Some(fill) = fill {
                grid.background_row(rect.x, y, rect.width, fill);
            }
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
            let style = match fill {
                Some(fill) => row.style.patch(fill),
                None => row.style,
            };
            grid.put_clipped(
                content_x,
                y,
                &content,
                style,
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
