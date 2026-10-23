//! `exec_command` / `write_stdin` 卡片：标题 + 折行的命令（+ 终态输出）。
//!
//! ```text
//! ■ Run command · Exited 0
//!   │  $ cargo test                 ← 参数区：折叠 2 行，展开更多（可独立折叠）
//!   │  test result: ok. 24 passed   ← 输出区：首 3 + 尾 2 行预览（可独立折叠）
//!   └─ … 18 more lines              ← 输出区自己的省略提示
//! ```
//!
//! - **两个可独立折叠的区域**：命令参数与命令输出各自有展开状态和 hover 面，
//!   点其中一块不会连带另一块；折叠态一律 dim，**悬停或展开升为终端默认前景**，
//!   两块同一口径（没有整行灰底）；
//! - **长任务（进程仍存活）只给一行引导**：命令 + `  └─ open task panel · live output`，
//!   消息列表不铺任何输出——完整实时输出在后台任务面板里。整卡点击打开面板并定位
//!   该进程；
//! - **标题行**沿用文件工具同一套 `■ 动词` 语法；方块颜色/呼吸表达状态，
//!   状态文字用 ` · ` 分隔紧跟在动词右侧；窄屏整项丢弃状态、再收敛动词；
//! - **命令折行**：`$ 命令` 按可用列宽软换行，折叠态最多 2 个物理行、展开态铺开
//!   全部（上限与普通工具详情一致）；输出同样按列宽折行并受同一份行预算约束；
//! - `write_stdin` 保留「标题 + 命令」快照，不铺本次输出；
//! - 卡片只读快照：不解析 JSON、不读进程、不读数据库。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{
    CardSection, CommandView, ProcessStateView, ToolCallView, ToolDetailView, ToolStatus,
};
use crate::ui::{style, text};

use super::super::{Grid, StepMark, visible_row, wrap};
use super::file_summary::DETAIL_MAX_ROWS;
use super::{
    Folds, TREE_COLS, header_with_verb, mark_of, output_style, param_lines, put_segments,
    single_line, title_rows, truncate_with_ellipsis,
};

/// 单行取样上限（限住每帧成本）。
const SAMPLE_CHARS: usize = 512;
/// 终态折叠预览的物理行：首 [`OUTPUT_PREVIEW_HEAD`] + 尾 [`OUTPUT_PREVIEW_TAIL`]，
/// 与本地命令卡片同一口径（中间的省略由末行 `… N more lines` 说明）。
const OUTPUT_PREVIEW_HEAD: usize = 3;
const OUTPUT_PREVIEW_TAIL: usize = 2;
/// 树体行首连接符两侧的缩进（与 [`TREE_COLS`] 同口径：`  │  ` = 2 + 1 + 2）。
const TREE_INDENT: usize = 2;

/// 树体两块的行数（标题之外）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Blocks {
    /// 命令参数。
    pub(super) params: usize,
    /// 输出（运行中只有一行引导）。
    pub(super) output: usize,
}

/// 命令卡片占用的渲染行数：标题 + 树体物理行。
pub(super) fn rows(call: &ToolCallView, width: u16, folds: Folds) -> usize {
    if width == 0 {
        return 0;
    }
    let blocks = blocks(call, width, folds);
    title_rows() + blocks.params + blocks.output
}

/// 树体两块各自的高度（与 [`draw`] 共用同一份构造）。
pub(super) fn blocks(call: &ToolCallView, width: u16, folds: Folds) -> Blocks {
    if width == 0 {
        return Blocks::default();
    }
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(view)) => Blocks {
            params: command_body(&view.command, width, folds.body).len(),
            output: if view.state.is_running() {
                // 长任务：只有一行「open task panel · live output」。
                1
            } else {
                finished_output(view, width, folds.output, false).len()
            },
        },
        Some(ToolDetailView::Interaction(view)) => Blocks {
            params: command_body(&view.command, width, folds.body).len(),
            output: 0,
        },
        _ => Blocks::default(),
    }
}

/// 命令参数区展开后是否**真的多出内容**（不能多时点击不登记）。
pub(super) fn params_expandable(call: &ToolCallView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    let command = match call.detail.as_deref() {
        Some(ToolDetailView::Command(view)) => &view.command,
        Some(ToolDetailView::Interaction(view)) => &view.command,
        _ => return false,
    };
    command_body(command, width, true).len() > command_body(command, width, false).len()
}

/// 输出区展开后是否**真的多出内容**。
pub(super) fn output_expandable(call: &ToolCallView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref() else {
        return false;
    };
    if view.state.is_running() {
        return false;
    }
    finished_output(view, width, true, false).len()
        > finished_output(view, width, false, false).len()
}

/// 整张卡片的树体（未加连接符）：命令参数、输出（或运行中的引导行）。
///
/// 与 [`draw`]、[`rows`]、[`blocks`] 共用同一份构造：行数口径只有一个来源。
fn body_rows(
    call: &ToolCallView,
    width: u16,
    folds: Folds,
    hovered: Option<CardSection>,
) -> Vec<BodyRow> {
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(view)) => {
            let mut out = command_body(&view.command, width, folds.body);
            if view.state.is_running() {
                out.push(guide_row(hovered == Some(CardSection::Output)));
            } else {
                out.extend(finished_output(
                    view,
                    width,
                    folds.output,
                    hovered == Some(CardSection::Output),
                ));
            }
            out
        }
        Some(ToolDetailView::Interaction(view)) => command_body(&view.command, width, folds.body),
        _ => Vec::new(),
    }
}

/// 命令参数区（`$ 命令`）：恒为浅黄，不随 hover / 展开变色。
fn command_body(command: &str, width: u16, expanded: bool) -> Vec<BodyRow> {
    // 已带提示符就不重复（调用方可能传入完整命令行）。
    let text = if command.starts_with(text::agent::COMMAND_PROMPT_PREFIX) {
        command.to_string()
    } else {
        format!("{}{command}", text::agent::COMMAND_PROMPT_PREFIX)
    };
    param_lines(&text, width, expanded)
        .into_iter()
        .map(|line| BodyRow {
            text: line.text,
            style: line.style,
            sample: SAMPLE_CHARS,
            section: CardSection::Body,
        })
        .collect()
}

/// 终态输出：折叠态首 [`OUTPUT_PREVIEW_HEAD`] + 尾 [`OUTPUT_PREVIEW_TAIL`] 行 + 省略行；
/// 展开态铺开已保存输出（上限 [`DETAIL_MAX_ROWS`]）。
fn finished_output(view: &CommandView, width: u16, expanded: bool, hovered: bool) -> Vec<BodyRow> {
    let rows = output_lines(&view.output_tail, output_width(width));
    if rows.is_empty() {
        return Vec::new();
    }
    let style = output_style(expanded, hovered);
    let visible = if expanded {
        rows.len().min(DETAIL_MAX_ROWS)
    } else if rows.len() <= OUTPUT_PREVIEW_HEAD + OUTPUT_PREVIEW_TAIL {
        rows.len()
    } else {
        OUTPUT_PREVIEW_HEAD + OUTPUT_PREVIEW_TAIL
    };
    let mut out: Vec<BodyRow> = Vec::with_capacity(visible + 1);
    for index in 0..visible {
        let text = if !expanded && index >= OUTPUT_PREVIEW_HEAD {
            // 折叠预览的尾部：接在首部之后，跳过中间被省略的行。
            rows[rows.len() - (visible - index)].clone()
        } else {
            rows[index].clone()
        };
        out.push(BodyRow {
            text,
            style,
            sample: SAMPLE_CHARS,
            section: CardSection::Output,
        });
    }
    // 折叠态末尾写省略行数：它是「还能展开」的唯一文案，跟随输出区的 hover 档。
    let shown = if expanded { rows.len() } else { visible };
    if !expanded && rows.len() > shown {
        out.push(BodyRow {
            text: text::agent::file_diff_more_lines(rows.len() - shown),
            style: output_style(false, hovered),
            sample: SAMPLE_CHARS,
            section: CardSection::Output,
        });
    }
    out
}

/// 运行中的引导行：长任务只留这一行，点卡片打开后台任务面板看完整实时输出。
fn guide_row(hovered: bool) -> BodyRow {
    BodyRow {
        text: text::agent::TASK_OPEN_GUIDE.to_string(),
        style: output_style(false, hovered),
        sample: SAMPLE_CHARS,
        section: CardSection::Output,
    }
}

/// 输出正文可用列数（与命令行共用树体前缀让位）。
fn output_width(width: u16) -> u16 {
    width
        .saturating_sub(u16::try_from(TREE_COLS).unwrap_or(u16::MAX))
        .max(1)
}

/// 输出按列宽折行（末尾空行不占位）。
fn output_lines(output: &str, width: u16) -> Vec<String> {
    if output.is_empty() {
        return Vec::new();
    }
    let mut rows: Vec<String> = Vec::new();
    for line in output.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            rows.push(String::new());
            continue;
        }
        rows.extend(wrap(line, width));
    }
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// 树体的一行：**未加连接符**的内容 + 着色档位 + 所属区块。
///
/// 连接符由 [`draw_body`] 统一补：参数区内部用 `  │  `、参数区最后一行收口
/// `  └─ `；**输出区不画连接符**（只用等宽空白对齐），即 `└─` 只包裹参数部分。
#[derive(Clone)]
struct BodyRow {
    text: String,
    style: ratatui::style::Style,
    /// 单行化后的取样上限（命令可能很长）。
    sample: usize,
    /// 该行属于哪一块：参数区收口，输出区不画树。
    section: CardSection,
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
    folds: Folds,
    animating: bool,
    hovered: Option<CardSection>,
) {
    if rect.width == 0 {
        // 宽度 0：与 [`rows`] 一致地不占行（外层布局已跳过该块）。
        return;
    }
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(view)) => {
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
        }
        Some(ToolDetailView::Interaction(view)) => {
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
            return;
        }
    }
    draw_body(
        grid,
        rect,
        scroll,
        row_index,
        &body_rows(call, rect.width, folds, hovered),
    );
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

/// 绘制树体：参数区内部 `  │  `、参数区最后一行 `  └─ `，输出区只留等宽空白。
///
/// `└─` 只包裹**参数部分**（与文件工具的 `└─ 详情` 同口径）；输出内容是正文，
/// 继续缩进在参数之下的内容列，但不画连接符。推进的行数 = `body.len()`。
fn draw_body(grid: &mut Grid, rect: Rect, scroll: usize, row_index: &mut usize, body: &[BodyRow]) {
    let limit = usize::from(rect.width);
    // 参数区的最后一行收口；没有参数行时整段都不画树。
    let params_last = body
        .iter()
        .rposition(|row| row.section == CardSection::Body);
    let content_x = rect
        .x
        .saturating_add(u16::try_from(TREE_COLS).unwrap_or(u16::MAX));
    for (index, row) in body.iter().enumerate() {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            // 连接符：`  │  ` / `  └─ `。`│` 与 `└` 落在同一列，树干连成一条线。
            let bar_style = style::chat_notice_connector();
            let indent = u16::try_from(TREE_INDENT).unwrap_or(0);
            let bar = match params_last {
                Some(last) if index < last => Some(text::agent::MARKER_TREE_BAR),
                Some(last) if index == last => Some(text::agent::MARKER_TREE_LEAF),
                _ => None,
            };
            if let Some(marker) = bar {
                grid.put_clipped(rect.x, y, &" ".repeat(TREE_INDENT), bar_style, rect.width);
                grid.put_clipped(
                    rect.x.saturating_add(indent),
                    y,
                    marker,
                    bar_style,
                    rect.width.saturating_sub(indent),
                );
            }
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
