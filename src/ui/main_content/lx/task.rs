//! lx 页的第二个页面：后台任务（进程 / 子代理）面板。
//!
//! 与浮层不同，这一页**画在窗格自己的内容区里**：不覆盖整屏、不拦截其他窗格，
//! 也不用保存/恢复 lx 页的任何状态——`pane.lx` 只是多了一个 `task` 字段，面板
//! 关掉就回到原样。
//!
//! 形态对齐 opencode 的 composer：**对话在上、控制面板在下**，键位提示落在窗格
//! **最底边框**上：
//!
//! ```text
//! ╭─ lx-[Native Agent] ─────────────────────── ws:…command-plan-tools ─╮
//! │ 我是 Claude，一个由 Anthropic 开发的 AI 助手…                        │
//! │ Build · deepseek-v4.1 · 2.1s · 50.3 tok/s                          │
//! │                                                                     │
//! │ Subagents   Processes                                        esc    │
//! │ ❯ while read -r line              /workspace/web        running 32s │
//! ╰─ show inactive  ^d stop  tabs ←/→ ─────────────────────────────────╯
//! ```
//!
//! - **上半区**是普通聊天历史（只读快照，滚轮/键盘可滚），输入框被面板替换；
//! - **下半区**是控制面板：第一行 tab（`Processes` / `Agents`）+ 右侧 `esc`，
//!   之后是当前 tab 的列表；
//! - **键位提示写在窗格底边框上**（与 `status.rs` 把状态写在底边框上同一套做法），
//!   不再占用正文行。
//!
//! 早先这一页整页接管、列表贴在最顶部、还自带一个圆角内框——与窗格边框叠成双重
//! 边框，也让人看不到上下文。现在保留历史、面板沉底，边框只有窗格那一层。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{Lx, TaskPage, TaskTab};
use crate::ui::{style, text};

use super::grid::{CONTINUATION, Grid};
use super::history;

/// 行首高亮标记占位列数（`❯ ` 或两个空格）。
const MARKER_COLS: u16 = 2;
/// 行内各段之间的最小空档（列）。
const GAP: u16 = 2;
/// 命令段至少保留的列数：比这更窄就只剩几个字母，没有识别价值。
const COMMAND_MIN: u16 = 10;
/// 显示 cwd 所需的最小文字区宽度：再窄就整段丢弃 cwd。
const CWD_MIN_ROOM: u16 = 30;
/// 面板与上方历史之间至少保留的历史行数：再少就不如直接让面板占满（信息更完整）。
const MIN_HISTORY: u16 = 3;
/// 面板最多占内容区高度的百分比。
const PANEL_MAX_PERCENT: u16 = 60;

/// 后台任务页的版面划分（渲染与滚轮命中共用同一份几何）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TaskLayout {
    /// 上方历史区（只读快照；不含底部面板与底边框）。
    pub history: Rect,
    /// 控制面板：第一行 tab，其后为列表。
    pub panel: Rect,
}

/// 计算后台任务页版面。
///
/// `area` 为窗格内容区（已被窗格边框包围）。底边框那一行留给键位提示，不参与划分。
pub(crate) fn layout(area: Rect, page: &TaskPage) -> TaskLayout {
    // 底边框行归键位提示；正文区到它上一行为止。
    let body_h = area.height.saturating_sub(1);
    if body_h == 0 {
        return TaskLayout {
            history: Rect::new(area.x, area.y, area.width, 0),
            panel: Rect::new(area.x, area.y, area.width, 0),
        };
    }
    // 面板高度：tab 行 + 列表行；列表行数随条目变化，但不超过上限百分比。
    let entries = u16::try_from(page.items_shown()).unwrap_or(u16::MAX);
    // 面板 = tab 行 + 列表行。空态也占一行（显示占位提示）。
    let wanted = entries.saturating_add(TAB_ROW);
    let cap = body_h
        .saturating_mul(PANEL_MAX_PERCENT)
        .saturating_div(100)
        .max(TAB_ROW);
    let panel_h = wanted.min(cap).min(body_h);
    // 面板沉底；上方余下的给历史。历史太薄时把面板收小，保证至少 MIN_HISTORY 行。
    let panel_h = panel_h.min(body_h.saturating_sub(MIN_HISTORY).max(TAB_ROW));
    let panel_y = area.y.saturating_add(body_h.saturating_sub(panel_h));
    let history_h = panel_y.saturating_sub(area.y);
    TaskLayout {
        history: Rect::new(area.x, area.y, area.width, history_h),
        panel: Rect::new(area.x, panel_y, area.width, panel_h),
    }
}

/// 面板首行（tab 行）占用的行数。
const TAB_ROW: u16 = 1;
/// 面板内容左右各留的空白列（与输入框外边距对齐，避免文字贴窗格边框）。
const PANEL_PAD: u16 = 1;

/// 渲染后台任务页：上方历史 + 下方控制面板 + 底边框键位。
///
/// 不画自己的框：窗格边框就是唯一的一层框。
pub(crate) fn render(area: Rect, buf: &mut Buffer, lx: &Lx, page: &TaskPage, focused: bool) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let lay = layout(area, page);
    // 先清底：面板会替换输入框，不能让旧输入框的画痕留在下面。
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_char(' ');
                cell.set_style(Style::default());
            }
        }
    }
    // 上半区：普通聊天历史（只读快照，按当前滚动位置绘制）。
    if lay.history.height > 0 && !lx.chat.messages.is_empty() {
        let mut grid = Grid::new(lay.history);
        let view = history::view(lay.history, lx);
        history::render(&mut grid, view.body, lx, view.scroll);
        if let Some(bar) = history::scrollbar(
            view.body,
            view.total_rows,
            usize::from(view.body.height),
            view.scroll,
        ) {
            for y in bar.thumb.y..bar.thumb.bottom() {
                grid.set(bar.thumb.x, y, '▐', style::text());
            }
        }
        blit(&grid, buf, lay.history);
    }
    // 下半区：控制面板。
    draw_panel(buf, lay.panel, page);
    // 最底边框：键位提示（覆盖在窗格底边框上，两端角保留）。
    draw_footer(buf, area, page, focused);
}

/// 把整块网格 blit 进 Buffer（宽字符占位列跳过，禁止渲染半个字符）。
fn blit(grid: &Grid, buf: &mut Buffer, area: Rect) {
    use ratatui::buffer::CellDiffOption;
    for dy in 0..area.height {
        for dx in 0..area.width {
            let glyph = grid.cell(area.x + dx, area.y + dy);
            if let Some(cell) = buf.cell_mut((area.x + dx, area.y + dy)) {
                if glyph.ch == CONTINUATION {
                    cell.set_diff_option(CellDiffOption::Skip);
                    continue;
                }
                cell.set_char(glyph.ch);
                cell.set_style(glyph.style);
                cell.set_diff_option(CellDiffOption::None);
            }
        }
    }
}

/// 控制面板：首行 tab 指示（右侧 `esc`），其后是当前 tab 的列表。
fn draw_panel(buf: &mut Buffer, panel: Rect, page: &TaskPage) {
    if panel.width == 0 || panel.height == 0 {
        return;
    }
    // tab 行：当前 tab 亮色，其余次要色；右端 `esc` 提示关闭。
    let mut x = panel.x.saturating_add(PANEL_PAD);
    for (index, tab) in TaskTab::ALL.iter().enumerate() {
        if index > 0 {
            x = put_str(
                buf,
                x,
                panel.y,
                text::LX_TASK_TAB_SEP,
                style::muted(),
                panel.right(),
            );
        }
        let tab_style = if *tab == page.tab {
            style::accent()
        } else {
            style::muted()
        };
        x = put_str(buf, x, panel.y, tab.label(), tab_style, panel.right());
    }
    let esc = text::LX_TASK_ESC;
    let esc_w = u16::try_from(esc.width()).unwrap_or(0);
    let esc_x = panel
        .right()
        .saturating_sub(esc_w)
        .saturating_sub(PANEL_PAD);
    if esc_x > x.saturating_add(GAP) {
        put_str(buf, esc_x, panel.y, esc, style::muted(), panel.right());
    }
    let list = Rect::new(
        panel.x.saturating_add(PANEL_PAD),
        panel.y.saturating_add(TAB_ROW),
        panel.width.saturating_sub(PANEL_PAD.saturating_mul(2)),
        panel.height.saturating_sub(TAB_ROW),
    );
    if list.height == 0 {
        return;
    }
    match page.tab {
        TaskTab::Processes => draw_processes(buf, list, page),
        // 子代理 tab：本阶段是预留位，只给占位提示。
        TaskTab::Agents => draw_placeholder(buf, list, text::LX_TASK_AGENTS_EMPTY),
    }
}

/// 窗格底边框上的键位提示：`╰─ show inactive  ^d stop  tabs ←/→ ─────╯`。
///
/// 与 `status.rs` 同一套做法：**只改写文字实际落到的单元格**，其余底边线保留 `─`，
/// 两端角字符不动，因此边框始终连续。放不下时从尾部截断，不显示半截键位。
fn draw_footer(buf: &mut Buffer, area: Rect, page: &TaskPage, focused: bool) {
    let _ = focused;
    // 内容区最后一行就是窗格底边框（`Rect` 的 `bottom()` 是开区间，故减一）。
    let y = area.bottom().saturating_sub(1);
    let left = area.x.saturating_add(2);
    let right = area.right().saturating_sub(1);
    if right <= left {
        return;
    }
    let hints = footer_hints(page);
    let clipped = clip(&hints, right.saturating_sub(left));
    // 先清成空格，再写文字；未覆盖的部分保持底边线 `─`。
    let width = u16::try_from(clipped.width()).unwrap_or(0);
    for x in left..left.saturating_add(width) {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(Style::default());
        }
    }
    put_str(
        buf,
        left,
        y,
        &clipped,
        style::muted(),
        left.saturating_add(width),
    );
}

/// 底边框键位提示（按当前 tab 给最相关的动作）。
fn footer_hints(page: &TaskPage) -> String {
    match page.tab {
        TaskTab::Processes => text::LX_TASK_FOOTER_PROCESSES.to_string(),
        TaskTab::Agents => text::LX_TASK_FOOTER_AGENTS.to_string(),
    }
}

/// 进程列表：每行 `❯ 命令 · cwd · running 12s`，行尾右对齐时长。
///
/// 宽度分配按优先级：命令先占、cwd 次之、时长最后——窄窗格里先牺牲 cwd，
/// 再硬切命令，时长始终可见（它回答"这个进程是不是刚起的"）。
fn draw_processes(buf: &mut Buffer, list: Rect, page: &TaskPage) {
    if page.processes.is_empty() {
        draw_placeholder(buf, list, text::LX_TASK_PROCESSES_EMPTY);
        return;
    }
    let visible = usize::from(list.height);
    let start = window_start(page, visible);
    for (offset, process) in page.processes.iter().skip(start).take(visible).enumerate() {
        let index = start + offset;
        let y = list.y + u16::try_from(offset).unwrap_or(0);
        let selected = index == page.window.active;
        if selected {
            put_str(
                buf,
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
fn draw_placeholder(buf: &mut Buffer, list: Rect, label: &str) {
    let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
    let x = list.x + list.width.saturating_sub(width) / 2;
    put_str(buf, x, list.y, label, style::muted(), list.right());
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
