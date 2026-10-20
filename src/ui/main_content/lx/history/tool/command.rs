//! `exec_command` / `write_stdin` 卡片：**三行**（标题 / 命令 / `[open]`）。
//!
//! ```text
//! ■ Run command · Running          ← 橙 = 进程还活着，绿 = 已停止
//!   │  $ cargo test chat::tools
//!   └─ [open]                      ← 点这一行打开后台任务面板
//! ```
//!
//! - **标题行**沿用文件工具同一套 `■ 动词` 语法；方块颜色/呼吸表达状态，
//!   状态文字用 ` · ` 分隔紧跟在动词右侧（不是行尾右对齐）；
//!   窄屏整项丢弃状态、再收敛动词，方块永远保留；
//! - **进程存活时整行橙色**（[`StepMark::Alive`]）：判据是进程状态而不是工具结算
//!   状态——命令交回后台后工具已经 `Completed`，但进程还在跑；停止后回翡翠绿；
//! - **树体只剩命令一行 + `[open]` 一行**：命令、cwd、输出与 `… +N more lines`
//!   全部搬进后台任务面板，消息列表里只回答"哪条命令还在跑"；
//! - `[open]` 独占树体末行、用次要信息灰，是**唯一**可点击的命中区；标题与命令
//!   都是纯展示（卡片没有可展开的内容）；
//! - 卡片只读快照：不解析 JSON、不读进程、不读数据库。

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

/// 单行取样上限（限住每帧成本）。
const SAMPLE_CHARS: usize = 512;
/// 树体行首连接符两侧的缩进（与 [`TREE_COLS`] 同口径：`  │  ` = 2 + 1 + 2）。
const TREE_INDENT: usize = 2;

/// 命令 / 交互卡片占用的渲染行数：**恒为 3 行**（标题 / 命令 / `[open]`）。
///
/// 输出整体搬到后台任务面板后，卡片不再有折叠与展开两种形态，行数因此恒定；
/// `expanded` 参数保留是为了与 [`crate::ui::main_content::lx::history::tool::rows`]
/// 的统一签名对齐（传什么都一样）。
pub(super) fn rows(call: &ToolCallView, width: u16, _expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(_))
        | Some(crate::app::state::ToolDetailView::Interaction(_)) => title_rows() + BODY_ROWS,
        _ => title_rows(),
    }
}

/// 树体行数：命令 + `[open]`。
const BODY_ROWS: usize = 2;

/// 命令卡片树体行：命令 + `[open]`。
///
/// **与 [`draw_command`] 共用同一份构造**：行数口径只有一个来源。输出、cwd 与
/// `… +N more lines` 都已搬进后台任务面板，消息列表里只看得到"哪条命令在跑"。
fn command_body(view: &CommandView) -> Vec<BodyRow> {
    vec![
        BodyRow::command(&view.command),
        BodyRow::open(text::agent::TASK_OPEN_LABEL),
    ]
}

/// 交互卡片树体行：命令 + `[open]`（stdin 与输出同样只在面板里看）。
fn interaction_body(view: &ProcessInteractionView) -> Vec<BodyRow> {
    vec![
        BodyRow::command(&view.command),
        BodyRow::open(text::agent::TASK_OPEN_LABEL),
    ]
}

/// `[open]` 行的**卡片内行偏移**（0 = 标题行）；进程卡片恒有这一行。
///
/// `history::layout` 用它定位可点击的命中矩形：渲染与命中必须共用同一份树体构造，
/// 否则点击区会与画出来的那一行错位。
pub(super) fn open_offset(call: &ToolCallView) -> Option<usize> {
    let body = match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(_))
        | Some(crate::app::state::ToolDetailView::Interaction(_)) => BODY_ROWS,
        _ => return None,
    };
    // 标题占 1 行，`[open]` 是树体末行。
    Some(title_rows() + body - 1)
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
    /// 是否是末行的 `[open]`（按「整项丢弃」让位，不截半段）。
    ///
    /// 只有它可以被整段丢弃：`[ope…` 读不出含义，还会让人以为点不了；
    /// 丢文案不改行数，命中区与几何都不会漂移。
    open: bool,
}

impl BodyRow {
    /// 命令原文行（`$ ` 前缀 + 命令，金黄参数色）。
    fn command(command: &str) -> Self {
        Self {
            text: format!("{}{command}", text::agent::COMMAND_PROMPT_PREFIX),
            style: style::chat_tool_param(),
            sample: SAMPLE_CHARS,
            open: false,
        }
    }

    /// 末行的 `[open]` 入口（次要信息灰；点击打开后台任务面板）。
    fn open(content: impl Into<String>) -> Self {
        Self {
            text: content.into(),
            style: style::agent_step_detail(),
            sample: SAMPLE_CHARS,
            open: true,
        }
    }
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
    _expanded: bool,
    animating: bool,
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

/// 命令卡片：标题（橙 = 存活）+ 树体（命令 / `[open]`）。
fn draw_command(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &CommandView,
    loading_frame: usize,
    animating: bool,
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
    draw_body(grid, rect, scroll, row_index, &command_body(view));
}

/// 交互卡片：标题 + 树体（命令 / `[open]`）。
fn draw_interaction(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &ProcessInteractionView,
    loading_frame: usize,
    animating: bool,
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
    draw_body(grid, rect, scroll, row_index, &interaction_body(view));
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
            // `[open]` 按「整项丢弃」让位：放不下就整段不画，绝不截半段
            // （`[ope…` 读不出含义，还会让人以为点不了）。丢掉不改行数。
            let content = if row.open {
                let full = single_line(&row.text, row.sample);
                if full.width() <= content_cols {
                    full
                } else {
                    String::new()
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
