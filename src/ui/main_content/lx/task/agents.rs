//! `Agents` 页签：详情区（原历史矩形）+ 列表（原输入框矩形）。
//!
//! 详情区复用主历史那套 `HistorySource` / `snapshot` / `render` / 滚动条；
//! 列表每行 `❯ ● explorer  running  14s  <task 首行>`。窄窗格整项丢弃，
//! 绝不把关键段截成半截。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AgentsState, SubagentRun, TaskPage};
use crate::ui::main_content::lx::history::{self, HistorySource};
use crate::ui::{style, text};

use super::super::Grid;
use crate::ui::main_content::lx::history::StepKind;

/// 列表行里角色段与状态段之间的分隔。
const CELL_GAP: u16 = 2;
/// 状态段至少保留的列数：再窄就只剩下「run…」，没有识别价值。
const STATUS_MIN: u16 = 6;
/// 角色段最小列数。
const ROLE_MIN: u16 = 4;

/// 详情区绘制：子 agent 的完整消息流（复用主历史渲染与滚动条）。
pub(crate) fn draw_detail(
    grid: &mut Grid,
    rect: Rect,
    run: &SubagentRun,
    scroll: usize,
    loading_frame: usize,
    cache: &mut crate::ui::PaneCache,
) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    let source = HistorySource::from_chat(&run.chat, &run.compact);
    if run.chat.messages.is_empty() {
        draw_empty_detail(grid, rect, run, loading_frame);
        return 1;
    }
    let view = history::view_source_plain(rect, source, cache);
    let visible = usize::from(view.body.height);
    let max = view.total_rows.saturating_sub(visible);
    // 跟随底部时贴尾；用户上翻过（`follow == false`）就停在他看的位置。
    let top = if run.follow { max } else { scroll.min(max) };
    let total = history::render_source(grid, view.body, source, top, cache);
    if let Some(bar) = history::scrollbar(view.body, view.total_rows, visible, top) {
        for y in bar.thumb.y..bar.thumb.bottom() {
            grid.set(bar.thumb.x, y, '▐', style::text());
        }
    }
    total
}

/// 空消息流（已加载为空或尚在懒加载）时的单行提示绘制（绘制与取词共用）。
pub(crate) fn draw_empty_detail(
    grid: &mut Grid,
    rect: Rect,
    run: &SubagentRun,
    loading_frame: usize,
) {
    if run.loaded {
        grid.put_clipped(
            rect.x,
            rect.y,
            text::LX_TASK_AGENTS_EMPTY,
            style::muted(),
            rect.width,
        );
    } else {
        // 历史 run 的消息流还在读库：用与主历史相同的呼吸圆点表达 loading。
        let mark = crate::ui::main_content::lx::history::StepMark::Running;
        let mark_text = format!("{} ", mark.glyph(StepKind::Body, loading_frame, true));
        grid.put_clipped(
            rect.x,
            rect.y,
            &mark_text,
            style::chat_step_mark_running_body(),
            rect.width,
        );
        grid.put_clipped(
            rect.x.saturating_add(2),
            rect.y,
            text::LX_TASK_AGENTS_LOADING,
            style::muted(),
            rect.width,
        );
    }
}

/// 详情区行数（`resolve_task_agents_visible` 用）。
///
/// 这里用一次性缓存量一次行数：Agents 页打开时只有一个高亮项，成本低于
/// 把渲染缓存搬进 `app/`；真正的绘制仍走事件循环的跨帧缓存。
pub(crate) fn detail_rows(rect: Rect, run: &SubagentRun) -> usize {
    let content_width = history::content_rect(rect).width;
    if content_width == 0 {
        return usize::from(rect.height);
    }
    let source = HistorySource::from_chat(&run.chat, &run.compact);
    history::measure(content_width, source)
}

/// 列表绘制：`❯ ● explorer  running  14s  <task 首行>`。
pub(super) fn draw_list(grid: &mut Grid, list: Rect, page: &TaskPage, agents: &AgentsState) {
    if agents.runs.is_empty() {
        grid.put_clipped(
            list.x,
            list.y,
            text::LX_TASK_AGENTS_EMPTY,
            style::muted(),
            list.width,
        );
        return;
    }
    let visible = usize::from(list.height);
    let start = window_start(page, agents, visible);
    let now = crate::db::now_ms();
    for (offset, run) in agents.runs.iter().skip(start).take(visible).enumerate() {
        let index = start + offset;
        let y = list.y + u16::try_from(offset).unwrap_or(0);
        let selected = index == page.agents_window.active;
        if selected {
            grid.put_clipped(
                list.x,
                y,
                text::AGENT_SELECTED_MARKER,
                style::accent(),
                list.width,
            );
        }
        let row_x = list.x.saturating_add(super::MARKER_COLS);
        let row_w = list.width.saturating_sub(super::MARKER_COLS);
        if row_w == 0 {
            continue;
        }
        let row_style = if selected {
            style::text()
        } else {
            style::muted()
        };
        // 行首：步骤标记（与历史同一套字形：运行中 `●`、失败/取消/完成各自语义）。
        let mark = if run.is_running() {
            crate::ui::main_content::lx::history::StepMark::Running
        } else {
            match run.status.as_str() {
                "failed" => crate::ui::main_content::lx::history::StepMark::Failed,
                "cancelled" => crate::ui::main_content::lx::history::StepMark::Cancelled,
                _ => crate::ui::main_content::lx::history::StepMark::Done,
            }
        };
        // 列表标记保持静态：运行状态由文字与状态色表达，闪烁只留给详情里的步骤。
        let mark_text = format!("{} ", mark.glyph(StepKind::Tool, 0, false));
        let mark_style = mark.style(StepKind::Tool, false);
        let mark_w = u16::try_from(mark_text.width()).unwrap_or(2);
        grid.put_clipped(row_x, y, &mark_text, mark_style, row_w);
        let mut x = row_x.saturating_add(mark_w);
        // 角色 / 状态 / 时长三段固定宽度优先；task 吃掉剩余。
        let age = crate::app::state::lx::task::age_label(run.elapsed_ms(now));
        let age_w = u16::try_from(age.width()).unwrap_or(0);
        let role_w = u16::try_from(run.role.as_str().width())
            .unwrap_or(0)
            .max(ROLE_MIN);
        let status_w = u16::try_from(run.status.width())
            .unwrap_or(0)
            .max(STATUS_MIN);
        let fixed = role_w
            .saturating_add(CELL_GAP)
            .saturating_add(status_w)
            .saturating_add(CELL_GAP)
            .saturating_add(age_w)
            .saturating_add(CELL_GAP);
        // 窄窗格：放不下角色 + 状态就整项丢弃（只留标记，与进程行同一规则）。
        if fixed.saturating_add(ROLE_MIN) > row_w {
            continue;
        }
        let role_right = x.saturating_add(role_w);
        grid.put_clipped(x, y, run.role.as_str(), row_style, role_right);
        x = role_right.saturating_add(CELL_GAP);
        let status_right = x.saturating_add(status_w);
        let status_style = match run.status.as_str() {
            "running" => style::agent_warn(),
            "failed" => style::agent_fail(),
            _ => style::agent_step_detail(),
        };
        grid.put_clipped(x, y, &run.status, status_style, status_right);
        x = status_right.saturating_add(CELL_GAP);
        let age_right = x.saturating_add(age_w);
        grid.put_clipped(x, y, &age, style::agent_step_detail(), age_right);
        x = age_right.saturating_add(CELL_GAP);
        // 只显示短标题；绝不把 task 提示词塞进列表（旧记录没有标题则留空）。
        let title = run.display_title();
        if x < list.right() && !title.is_empty() {
            grid.put_clipped(x, y, title, row_style, list.right());
        }
    }
}

/// 列表窗口起点（与进程列表同口径）。
pub(super) fn window_start(page: &TaskPage, agents: &AgentsState, visible: usize) -> usize {
    let len = agents.runs.len();
    if visible == 0 || len <= visible {
        return 0;
    }
    if let Some(start) = page.agents_window.viewport {
        return start.min(len.saturating_sub(visible));
    }
    let active = page.agents_window.active.min(len - 1);
    active.saturating_sub(visible - 1).min(len - visible)
}
