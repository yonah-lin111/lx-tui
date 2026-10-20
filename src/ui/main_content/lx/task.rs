//! lx 页的第二个页面：后台任务（进程 / 子代理）。
//!
//! 与浮层不同，这一页**画在窗格自己的内容区里**：不覆盖整屏、不拦截其他窗格，
//! 也不用保存/恢复 lx 页的任何状态——`pane.lx` 只是多了一个 `task` 字段，页面
//! 关掉就回到原样。
//!
//! 版式对齐 opencode 的 composer：列表占满内容区，底部一条**横栏**给出 tab 与键位，
//! 栏的 `├` `┤` 直接接在窗格左右边框上：
//!
//! ```text
//! ╭─ lx-[Native Agent] ──────────────────╮  ← 窗格边框（只有这一层框）
//! │ ❯ while read -r line    /w/web   12s │
//! │   npm run dev           /w/web   1m   │
//! ├─ Processes · Agents ── [↑/↓] [^d] ───┤  ← 页内唯一的横栏
//! ╰──────────────────────────────────────╯
//! ```
//!
//! 早先这一页画了**完整的圆角内框**，于是和窗格边框叠成双重边框（用户报的 bug）。
//! 现在这条横栏画在内容区最后一行，横线本身替代该行的内容，`├`/`┤` 落在窗格
//! 竖直边框所在列——视觉上是一条从边框长出来的分隔线，不再是第二个框。

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
///
/// 内容区已被窗格边框包围，这里**不再画第二个框**：只画列表与底部横栏。
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
    // 最后一行留给底栏；列表占其余高度。
    let bar_y = area.bottom().saturating_sub(1);
    let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    draw_bar(buf, area, bar_y, page, focused);
    if body.width == 0 || body.height == 0 {
        return;
    }
    match page.tab {
        TaskTab::Processes => draw_processes(buf, body, page),
        // 子代理 tab：本阶段是预留位，只给占位提示。
        TaskTab::Agents => draw_placeholder(buf, body, text::LX_TASK_AGENTS_EMPTY),
    }
}

/// 底部横栏：`├─ Processes · Agents ──────── [↑/↓] select  [^d] stop ─┤`。
///
/// `├` 与 `┤` 写在**窗格左右边框所在的列**（内容区左右各外扩一列），横线贯穿整行。
/// 这样这条栏读作「从窗格边框长出来的一条分隔线」，而不是第二个方框的顶边——
/// 与 `status.rs` 把状态写在窗格底边框上同一套做法（都在内容区之外的那一列/行上落笔）。
///
/// 让位顺序：先砍键位提示的尾部（`[esc] back` → `[←/→] tab` → …），
/// 再丢整段提示，最后只剩 tab 与横线。tab 是页面的导航，永远保留。
fn draw_bar(buf: &mut Buffer, area: Rect, y: u16, page: &TaskPage, focused: bool) {
    let border = style::border(focused);
    // 左右各外扩一列，压在窗格的竖直边框上（`╭─…─╮` 的中段）。
    let left = area.x.saturating_sub(1);
    let right = area.right();
    if right <= left {
        return;
    }
    // 整行先铺横线，再在两端换成接缝。
    for x in left..=right {
        put_char(buf, x, y, '─', border);
    }
    put_char(buf, left, y, '├', border);
    put_char(buf, right, y, '┤', border);

    // 文字区：`├─` 之后到 `─┤` 之前（接缝两侧各留一列空白）。
    let text_left = left.saturating_add(3);
    let text_right = right.saturating_sub(1);
    if text_right <= text_left {
        return;
    }

    // 左段：tab 指示（当前 tab 高亮，其余次要色）。
    let mut x = text_left;
    for (index, tab) in TaskTab::ALL.iter().enumerate() {
        if index > 0 {
            x = put_str(buf, x, y, text::LX_TASK_TAB_SEP, style::muted(), text_right);
        }
        let tab_style = if *tab == page.tab {
            style::accent()
        } else {
            style::muted()
        };
        x = put_str(buf, x, y, tab.label(), tab_style, text_right);
    }

    // 右段：键位提示贴右端；放不下就整段丢掉（宁可不提示，也不显示半截键位）。
    let hints = text::LX_TASK_PAGE_FOOTER;
    let hints_w = u16::try_from(hints.width()).unwrap_or(u16::MAX);
    let hints_x = text_right.saturating_sub(hints_w);
    if hints_x > x.saturating_add(GAP) {
        put_str(buf, hints_x, y, hints, style::muted(), text_right);
    }
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
