//! lx 页的第二个页面：后台任务（进程 / 子代理）。
//!
//! 与浮层不同，这一页**画在窗格自己的内容区里**：不覆盖整屏、不拦截其他窗格，
//! 也不用保存/恢复 lx 页的任何状态——`pane.lx` 只是多了一个 `task` 字段，页面
//! 关掉就回到原样。
//!
//! 版式对齐 opencode 的 composer 多 tab：顶边框是页面标题 + tab 指示，主体是当前
//! tab 的列表，底边框是键位提示。列表宽度自适应窗格，行内为 `命令 · cwd · 时长`。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{TaskPage, TaskTab};
use crate::ui::{style, text};

use super::grid::CONTINUATION;

/// 行首高亮标记占位列数（`❯ ` 或两个空格）。
const MARKER_COLS: u16 = 2;
/// 行内各段之间的最小空档（列）。
const GAP: u16 = 2;
/// 命令段至少保留的列数：比这更窄就只剩几个字母，没有识别价值。
const COMMAND_MIN: u16 = 10;
/// 显示 cwd 所需的最小文字区宽度：再窄就整段丢弃 cwd。
const CWD_MIN_ROOM: u16 = 30;

/// 渲染后台任务页（整个窗格内容区）。
pub(crate) fn render(area: Rect, buf: &mut Buffer, page: &TaskPage, focused: bool) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    // 先清底：这一页整屏接管，不残留 lx 页的画痕。
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_char(' ');
                cell.set_style(Style::default());
            }
        }
    }
    draw_frame(buf, area, page, focused);
    let body = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if body.width == 0 || body.height == 0 {
        return;
    }
    match page.tab {
        TaskTab::Processes => draw_processes(buf, body, page),
        // 子代理 tab：本阶段是预留位，只给占位提示。
        TaskTab::Agents => draw_placeholder(buf, body, text::LX_TASK_AGENTS_EMPTY),
    }
}

/// 页面框：圆角边框 + 顶边标题/右侧 tab 指示 + 底边键位提示。
///
/// 边线用失焦/聚焦同款灰阶（与窗格边框一致），标题用强调色——页面的视觉重心
/// 在列表内容上，边框只负责划分区域。
fn draw_frame(buf: &mut Buffer, area: Rect, page: &TaskPage, focused: bool) {
    let border = style::border(focused);
    let right = area.right().saturating_sub(1);
    let bottom = area.bottom().saturating_sub(1);
    // 四条边。
    for x in area.x + 1..right {
        put_char(buf, x, area.y, '─', border);
        put_char(buf, x, bottom, '─', border);
    }
    for y in area.y + 1..bottom {
        put_char(buf, area.x, y, '│', border);
        put_char(buf, right, y, '│', border);
    }
    put_char(buf, area.x, area.y, '╭', border);
    put_char(buf, right, area.y, '╮', border);
    put_char(buf, area.x, bottom, '╰', border);
    put_char(buf, right, bottom, '╯', border);

    // 顶边：左侧标题，右侧 tab 指示（当前 tab 高亮）；放不下时先让位给标题。
    let title = format!(" {} ", text::LX_TASK_PAGE_TITLE);
    let title_end = put_str(
        buf,
        area.x.saturating_add(2),
        area.y,
        &title,
        style::border_title(),
        right,
    );
    let tabs_w = tab_indicator_width();
    if right.saturating_sub(tabs_w) > title_end {
        let mut tabs_x = right.saturating_sub(tabs_w);
        for (index, tab) in TaskTab::ALL.iter().enumerate() {
            if index > 0 {
                tabs_x = put_str(
                    buf,
                    tabs_x,
                    area.y,
                    text::LX_TASK_TAB_SEP,
                    style::muted(),
                    right,
                );
            }
            let tab_style = if *tab == page.tab {
                style::accent()
            } else {
                style::muted()
            };
            tabs_x = put_str(buf, tabs_x, area.y, tab.label(), tab_style, right);
        }
    }

    // 底边：键位提示贴左端。
    put_str(
        buf,
        area.x.saturating_add(2),
        bottom,
        &format!(" {} ", text::LX_TASK_PAGE_FOOTER),
        style::muted(),
        right,
    );
}

/// tab 指示整段显示宽度（含分隔符）。
fn tab_indicator_width() -> u16 {
    let labels: u16 = TaskTab::ALL
        .iter()
        .map(|tab| {
            let width = tab.label().width();
            u16::try_from(width).unwrap_or(u16::MAX)
        })
        .sum();
    let seps = u16::try_from(text::LX_TASK_TAB_SEP.width())
        .unwrap_or(0)
        .saturating_mul((TaskTab::ALL.len().saturating_sub(1)) as u16);
    labels.saturating_add(seps).saturating_add(2)
}

/// 进程列表：每行 `❯ 命令 · cwd · running 12s`，行尾右对齐时长。
///
/// 宽度分配按优先级：命令先占、cwd 次之、时长最后——窄窗格里先牺牲 cwd，
/// 再硬切命令，时长始终可见（它回答"这个进程是不是刚起的"）。
fn draw_processes(buf: &mut Buffer, body: Rect, page: &TaskPage) {
    if page.processes.is_empty() {
        draw_placeholder(buf, body, text::LX_TASK_PROCESSES_EMPTY);
        return;
    }
    let visible = usize::from(body.height);
    let start = window_start(page, visible);
    for (offset, process) in page.processes.iter().skip(start).take(visible).enumerate() {
        let index = start + offset;
        let y = body.y + u16::try_from(offset).unwrap_or(0);
        let selected = index == page.window.active;
        if selected {
            put_str(
                buf,
                body.x,
                y,
                text::AGENT_SELECTED_MARKER,
                style::accent(),
                body.right(),
            );
        }
        let row_x = body.x.saturating_add(MARKER_COLS);
        let row_w = body.width.saturating_sub(MARKER_COLS);
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
        put_str(buf, row_x, y, &command, row_style, row_x + command_w);
        if cwd_w > 0 {
            let cwd = clip(&process.cwd, cwd_w);
            put_str(
                buf,
                row_x.saturating_add(command_w).saturating_add(GAP),
                y,
                &cwd,
                style::muted(),
                row_x + text_w,
            );
        }
        if show_age {
            put_str(
                buf,
                body.right().saturating_sub(age_w),
                y,
                &age,
                style::muted(),
                body.right(),
            );
        }
    }
}

/// 空态/占位提示：水平居中、垂直取首行（与列表同一起点，视线不跳）。
fn draw_placeholder(buf: &mut Buffer, body: Rect, label: &str) {
    let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
    let x = body.x + body.width.saturating_sub(width) / 2;
    put_str(buf, x, body.y, label, style::muted(), body.right());
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

/// 写单个字符（越界忽略）。
fn put_char(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.reset();
        cell.set_char(ch);
        cell.set_style(style);
    }
}

/// 逐字符写入并裁剪到 `limit` 列；返回写入结束后的下一列。
fn put_str(buf: &mut Buffer, x: u16, y: u16, content: &str, style: Style, limit: u16) -> u16 {
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
