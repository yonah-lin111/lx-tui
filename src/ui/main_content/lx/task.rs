//! lx 页后台任务面板：**替换输入框容器的组件**。
//!
//! 它不是一个独立页面——`pane.lx.task` 有值时，`lx.rs::layout` 把输入框矩形的高度
//! 换成「tab 行 + 列表行」，`build_chat` 在这个矩形里画面板而不是画输入框。
//! 因此：
//!
//! - 上方**消息列表照旧**：置顶行、滚动条、聊天栏全部走原本的 `history` 路径；
//! - 面板只占输入框那一块，不覆盖整个 pane；
//! - 键位提示内嵌到**窗格底边框**（`status.rs` 画模型名的同一条边线），
//!   面板打开期间不显示模型名 / 用量 / 进程计数。
//!
//! ```text
//! ╭─ lx-[Native Agent] ─────────────────────── ws:…command-plan-tools ─╮
//! │ 我是 Claude，一个由 Anthropic 开发的 AI 助手…                        │
//! │ Build · deepseek-v4.1 · 2.1s · 50.3 tok/s                          │
//! │                                                                     │
//! │ Processes · Agents                                            esc   │
//! │ ❯ while read -r line              /workspace/web        running 32s  │
//! ╰─ ↑↓ select  ^d stop  tabs ←→  esc ─────────────────────────────────╯
//! ```
//!
//! 列表封顶 [`LIST_MAX`] 行，与 opencode composer 的 `maxHeight={5}` 同值；
//! 不额外加 padding / gap 空行——那是 Web 布局的呼吸感，在字符终端里会白吃对话空间。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{TaskPage, TaskTab};
use crate::ui::{style, text};

use super::grid::CONTINUATION;

/// 面板顶部边框占用的行数。
///
/// 一条横线把面板与上方消息列表分开——这条线**属于面板内部**（画在 `lay.input`
/// 矩形之内），不是第二个窗格边框，所以不会重演早期「双重边框」那个 bug。
pub(super) const TOP_BORDER: u16 = 1;
/// tab 行占用的行数（顶边框之下）。
pub(super) const TAB_ROW: u16 = 1;
/// 相邻键位提示之间的空档（列）。
const HINT_GAP: usize = 2;
/// 列表最多显示的行数。
///
/// 比 opencode 的 `maxHeight={5}` 再高一档：5 行在 24 行窗格里信息量偏少，
/// 而面板高度固定，多一点列表区不会让布局跳动。
pub(super) const LIST_MAX: usize = 6;
/// 行首高亮标记占位列数（`❯ ` 或两个空格）。
const MARKER_COLS: u16 = 2;
/// 行内各段之间的最小空档（列）。
const GAP: u16 = 2;
/// 命令段至少保留的列数：比这更窄就只剩几个字母，没有识别价值。
const COMMAND_MIN: u16 = 10;
/// 显示 cwd 所需的最小文字区宽度：再窄就整段丢弃 cwd。
const CWD_MIN_ROOM: u16 = 30;

/// 面板高度（顶边框 + tab 行 + 固定 [`LIST_MAX`] 行）；由 `lx.rs::layout` 定输入框矩形。
///
/// **高度固定**，不随条目数变化：列表 0 条和 6 条时面板一样高。理由有二：
///
/// 1. 打开面板时上方消息列表不会因为「当前有几个后台进程」而被挤动一下再弹回来；
/// 2. 空态能占满整个列表区，`no running processes` 不会孤零零吊在面板顶上一行。
///
/// 条目超过 [`LIST_MAX`] 时列表内部滚动（`window_start` 跟高亮走）。
pub(super) fn height(_page: &TaskPage) -> u16 {
    u16::try_from(LIST_MAX)
        .unwrap_or(LIST_MAX as u16)
        .saturating_add(TAB_ROW)
        .saturating_add(TOP_BORDER)
}

/// 在输入框矩形里画面板：顶边框 → tab 行（右端 `esc`）→ 当前 tab 的列表。
pub(super) fn draw(grid: &mut super::grid::Grid, rect: Rect, page: &TaskPage) {
    if rect.width == 0 || rect.height < TOP_BORDER + TAB_ROW {
        return;
    }
    // 顶边框：一条横线把面板与上方消息列表分开。
    //
    // 用 `border_title()`（淡青 + dim，与窗格边框标题同色系），不抢焦点；
    // 它画在面板矩形**内部**，不与窗格边框重叠，所以不是双重边框。
    grid.put_clipped(
        rect.x,
        rect.y,
        &"─".repeat(usize::from(rect.width)),
        style::border_title(),
        rect.width,
    );

    // tab 行：当前 tab 用与顶部 tab 栏**同一套**激活样式（青底黑字），其余次要色。
    // 几何与 [`hit`] 共用 [`tab_row_rects`]，保证"画在这、点在那"。
    let tab_y = rect.y.saturating_add(TOP_BORDER);
    let (tabs, close) = tab_row_rects(rect);
    for (index, tab) in TaskTab::ALL.iter().enumerate() {
        if index > 0 {
            let sep_w = u16::try_from(text::LX_TASK_TAB_SEP.width()).unwrap_or(0);
            let sep_x = tabs
                .get(index)
                .map_or(rect.x, |(_, x, _)| x.saturating_sub(sep_w));
            put_grid(
                grid,
                sep_x,
                tab_y,
                text::LX_TASK_TAB_SEP,
                style::muted(),
                rect.right(),
            );
        }
        let Some((_, x, _)) = tabs.get(index) else {
            continue;
        };
        let tab_style = if *tab == page.tab {
            style::selected_item()
        } else {
            style::muted()
        };
        put_grid(grid, *x, tab_y, tab.label(), tab_style, rect.right());
    }
    if let Some(close) = close {
        put_grid(
            grid,
            close.x,
            close.y,
            &text::LX_TASK_CLOSE.label(),
            style::muted(),
            rect.right(),
        );
    }

    let chrome = TOP_BORDER.saturating_add(TAB_ROW);
    let list = Rect::new(
        rect.x,
        rect.y.saturating_add(chrome),
        rect.width,
        rect.height.saturating_sub(chrome),
    );
    if list.height == 0 {
        return;
    }
    match page.tab {
        TaskTab::Processes => draw_processes(grid, list, page),
        // 子代理 tab：本阶段是预留位，只给占位提示。
        TaskTab::Agents => draw_placeholder(grid, list, text::LX_TASK_AGENTS_EMPTY),
    }
}

/// 窗格底边框上的键位提示（`╰─ ↑↓ select  ^d stop  tabs ←→  esc ─╯`）。
///
/// 与 `status.rs` 同一套做法：**只改写文字实际落到的单元格**，其余底边线保留 `─`，
/// 两端角字符不动，因此边框始终连续。放不下时从尾部截断，不显示半截键位。
pub(super) fn draw_footer(buf: &mut Buffer, area: Rect, page: &TaskPage) {
    // 底边框行：内容区下沿之外那一行（`status.rs` 用的是同一个 y）。
    let y = area.bottom();
    let left = area.x.saturating_add(2);
    let right = area.right().saturating_sub(1);
    if right <= left {
        return;
    }
    // 按项拼接：窄窗格里**整项丢弃**（从右往左），绝不把 `[back esc]` 截成半截。
    let hints: &[text::Hint] = match page.tab {
        TaskTab::Processes => &text::LX_TASK_HINTS_PROCESSES,
        TaskTab::Agents => &text::LX_TASK_HINTS_AGENTS,
    };
    let room = usize::from(right.saturating_sub(left));
    let mut rendered = String::new();
    for hint in hints {
        let label = hint.label();
        let sep = if rendered.is_empty() { 0 } else { HINT_GAP };
        if rendered.width() + sep + label.width() > room {
            break;
        }
        if sep > 0 {
            rendered.push_str(&" ".repeat(sep));
        }
        rendered.push_str(&label);
    }
    if rendered.is_empty() {
        return;
    }
    let width = u16::try_from(rendered.width()).unwrap_or(0);
    // 先清成空格再写文字；未覆盖的部分保持底边线。
    for x in left..left.saturating_add(width) {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(Style::default());
        }
    }
    put_buf(
        buf,
        left,
        y,
        &rendered,
        style::muted(),
        left.saturating_add(width),
    );
}

/// 面板上的可点击目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskHit {
    /// 左上角的某个 tab（索引 `TaskTab::ALL`）。
    Tab(usize),
    /// 右上角的 `[esc]`：关闭面板。
    Close,
}

/// tab 行各段几何：`(tab 索引/标签的起始列, 宽度)` + `[esc]` 矩形。
///
/// 渲染与命中共用同一份计算，避免"画在这、点在那"。
fn tab_row_rects(rect: Rect) -> (Vec<(usize, u16, u16)>, Option<Rect>) {
    let tab_y = rect.y.saturating_add(TOP_BORDER);
    let mut x = rect.x;
    let mut tabs = Vec::with_capacity(TaskTab::ALL.len());
    for (index, tab) in TaskTab::ALL.iter().enumerate() {
        if index > 0 {
            let sep_w = u16::try_from(text::LX_TASK_TAB_SEP.width()).unwrap_or(0);
            x = x.saturating_add(sep_w);
        }
        let label_w = u16::try_from(tab.label().width()).unwrap_or(0);
        tabs.push((index, x, label_w));
        x = x.saturating_add(label_w);
    }
    let close_label = text::LX_TASK_CLOSE.label();
    let esc_w = u16::try_from(close_label.width()).unwrap_or(0);
    let esc_x = rect.right().saturating_sub(esc_w);
    let close = (esc_x > x).then(|| Rect::new(esc_x, tab_y, esc_w, 1));
    (tabs, close)
}

/// 面板点击命中：tab 或 `[esc]`；点在别处返回 None。
pub(crate) fn hit(rect: Rect, _page: &TaskPage, column: u16, row: u16) -> Option<TaskHit> {
    if rect.width == 0 || rect.height < TOP_BORDER + TAB_ROW {
        return None;
    }
    let (tabs, close) = tab_row_rects(rect);
    if let Some(close) = close
        && close.contains((column, row).into())
    {
        return Some(TaskHit::Close);
    }
    tabs.iter()
        .find(|(_, x, width)| {
            let right = x.saturating_add(*width);
            row == rect.y.saturating_add(TOP_BORDER) && column >= *x && column < right
        })
        .map(|(index, _, _)| TaskHit::Tab(*index))
}

/// 进程列表：每行 `❯ 命令 · cwd · running 12s`，行尾右对齐时长。
///
/// 宽度分配按优先级：命令先占、cwd 次之、时长最后——窄窗格里先牺牲 cwd，
/// 再硬切命令，时长始终可见（它回答"这个进程是不是刚起的"）。
fn draw_processes(grid: &mut super::grid::Grid, list: Rect, page: &TaskPage) {
    if page.processes.is_empty() {
        draw_placeholder(grid, list, text::LX_TASK_PROCESSES_EMPTY);
        return;
    }
    let visible = usize::from(list.height);
    let start = window_start(page, visible);
    for (offset, process) in page.processes.iter().skip(start).take(visible).enumerate() {
        let index = start + offset;
        let y = list.y + u16::try_from(offset).unwrap_or(0);
        let selected = index == page.window.active;
        if selected {
            put_grid(
                grid,
                list.x,
                y,
                text::AGENT_SELECTED_MARKER,
                style::accent(),
                list.right(),
            );
        }
        let row_x = list.x.saturating_add(MARKER_COLS);
        let row_w = list.width.saturating_sub(MARKER_COLS);
        if row_w == 0 {
            continue;
        }
        let age = format!(
            "{}{}",
            text::LX_TASK_AGE_PREFIX,
            process.age_label(page.now_ms)
        );
        let age_w = u16::try_from(age.width()).unwrap_or(u16::MAX);
        let show_age = age_w <= row_w;
        let text_w = row_w.saturating_sub(if show_age {
            age_w.saturating_add(GAP)
        } else {
            0
        });
        let row_style = if selected {
            style::text()
        } else {
            style::muted()
        };
        // 命令段与 cwd 段共享 `text_w`：命令是识别进程的依据，必须留足；
        // 余量不够看清命令时整段丢弃 cwd（窄窗格里 cwd 最不重要）。
        let cwd_w = if text_w >= CWD_MIN_ROOM {
            (text_w.saturating_sub(GAP) / 2).min(text_w.saturating_sub(COMMAND_MIN))
        } else {
            0
        };
        let command_w = text_w.saturating_sub(if cwd_w > 0 {
            cwd_w.saturating_add(GAP)
        } else {
            0
        });
        // 命令列固定宽度、cwd 从命令列的固定偏移起——各行 cwd 左缘对齐成列，
        // 扫一眼就能对比不同进程的工作目录。
        let command = clip(&process.command, command_w);
        put_grid(grid, row_x, y, &command, row_style, row_x + command_w);
        if cwd_w > 0 {
            let cwd = clip(&process.cwd, cwd_w);
            put_grid(
                grid,
                row_x.saturating_add(command_w).saturating_add(GAP),
                y,
                &cwd,
                style::muted(),
                row_x + text_w,
            );
        }
        if show_age {
            put_grid(
                grid,
                list.right().saturating_sub(age_w),
                y,
                &age,
                style::muted(),
                list.right(),
            );
        }
    }
}

/// 空态/占位提示：水平居中、垂直取首行（与列表同一起点，视线不跳）。
fn draw_placeholder(grid: &mut super::grid::Grid, list: Rect, label: &str) {
    let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
    let x = list.x + list.width.saturating_sub(width) / 2;
    put_grid(grid, x, list.y, label, style::muted(), list.right());
}

/// 列表窗口起点：高亮在视口内时不动，否则贴边滚动（与 `PanelWindow` 同口径）。
///
/// 显式视口（滚轮）优先；否则以高亮为准做最小滚动。
fn window_start(page: &TaskPage, visible: usize) -> usize {
    let len = page.processes.len();
    if visible == 0 || len <= visible {
        return 0;
    }
    if let Some(start) = page.window.viewport {
        return start.min(len.saturating_sub(visible));
    }
    let active = page.window.active.min(len - 1);
    active.saturating_sub(visible - 1).min(len - visible)
}

/// 逐字符写入网格并裁剪到 `limit` 列；返回写入结束后的下一列。
fn put_grid(
    grid: &mut super::grid::Grid,
    x: u16,
    y: u16,
    content: &str,
    style: Style,
    limit: u16,
) -> u16 {
    let width = limit.saturating_sub(x);
    grid.put_clipped(x, y, content, style, width);
    x.saturating_add(u16::try_from(content.width()).unwrap_or(0))
        .min(limit)
}

/// 逐字符写入缓冲区并裁剪到 `limit` 列；返回写入结束后的下一列。
fn put_buf(buf: &mut Buffer, x: u16, y: u16, content: &str, style: Style, limit: u16) -> u16 {
    let mut cx = x;
    for ch in content.chars() {
        let width = ch.width().unwrap_or(0) as u16;
        if width == 0 {
            continue;
        }
        if cx.saturating_add(width) > limit {
            break;
        }
        if let Some(cell) = buf.cell_mut((cx, y)) {
            cell.reset();
            cell.set_char(ch);
            cell.set_style(style);
        }
        for offset in 1..width {
            if let Some(cell) = buf.cell_mut((cx.saturating_add(offset), y)) {
                cell.reset();
                cell.set_char(CONTINUATION);
                cell.set_style(style);
            }
        }
        cx = cx.saturating_add(width);
    }
    cx
}

/// 按显示宽度硬切（无省略号；宽字符不半写）。
fn clip(value: &str, max: u16) -> String {
    if max == 0 {
        return String::new();
    }
    if u16::try_from(value.width()).unwrap_or(u16::MAX) <= max {
        return value.to_string();
    }
    let mut out = String::new();
    let mut width = 0u16;
    for ch in value.chars() {
        let cell = ch.width().unwrap_or(0) as u16;
        if width.saturating_add(cell) > max {
            break;
        }
        out.push(ch);
        width = width.saturating_add(cell);
    }
    out
}

#[cfg(test)]
mod tests;
