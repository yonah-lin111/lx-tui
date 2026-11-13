//! `Agents` 页签：详情区（原历史矩形）+ 列表（原输入框矩形）。
//!
//! 详情区复用主历史那套 `HistorySource` / `snapshot` / `render` / 滚动条；
//! 列表每行 `❯ ● explorer  running  14s  <task 首行>`。窄窗格整项丢弃，
//! 绝不把关键段截成半截。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{AgentsState, SubagentRun, TaskPage};
use crate::ui::main_content::lx::history::{self, HistorySource, HistoryViewport};
use crate::ui::{style, text};

use super::super::Grid;
use super::detail::{self, DetailView, EmptyState};
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
    let view = DetailView {
        source: HistorySource::from_run(run).with_viewport(HistoryViewport {
            scroll,
            follow_bottom: run.follow,
        }),
        scroll,
        follow: run.follow,
    };
    let empty = if run.loaded {
        EmptyState::Placeholder(text::LX_TASK_AGENTS_EMPTY)
    } else {
        EmptyState::Loading {
            label: text::LX_TASK_AGENTS_LOADING,
            frame: loading_frame,
        }
    };
    detail::draw(grid, rect, view, empty, cache)
}

/// 空消息流（已加载为空或尚在懒加载）时的单行提示绘制（绘制与取词共用）。
pub(crate) fn draw_empty_detail(
    grid: &mut Grid,
    rect: Rect,
    run: &SubagentRun,
    loading_frame: usize,
) {
    let empty = if run.loaded {
        EmptyState::Placeholder(text::LX_TASK_AGENTS_EMPTY)
    } else {
        EmptyState::Loading {
            label: text::LX_TASK_AGENTS_LOADING,
            frame: loading_frame,
        }
    };
    detail::draw_empty(grid, rect, empty);
}

/// 详情区行数（`resolve_task_agents_visible` 用）。
///
/// 复用事件循环的跨帧 `PaneCache`：量行与绘制命中同一份快照，一轮 delta 只排版一次。
pub(crate) fn detail_rows(
    rect: Rect,
    run: &SubagentRun,
    cache: &mut crate::ui::PaneCache,
) -> (usize, bool) {
    let view = DetailView {
        source: HistorySource::from_run(run).with_viewport(HistoryViewport {
            scroll: run.scroll,
            follow_bottom: run.follow,
        }),
        scroll: run.scroll,
        follow: run.follow,
    };
    detail::measure(rect, view, cache)
}

/// 详情区变化前的阅读锚点：只读上一帧同宽度快照，不触发重排。
pub(crate) fn anchor_before(
    area: Rect,
    lx: &crate::app::state::Lx,
    cache: &mut crate::ui::PaneCache,
    scroll: usize,
) -> Option<history::HistoryAnchor> {
    let rect = super::super::layout(area, lx).task_output?;
    let page = lx.task.as_ref()?;
    let run = page
        .detail_agent_index(&lx.agents)
        .and_then(|index| lx.agents.runs.get(index))?;
    let view = DetailView {
        source: HistorySource::from_run(run).with_viewport(HistoryViewport {
            scroll: run.scroll,
            follow_bottom: run.follow,
        }),
        scroll: run.scroll,
        follow: run.follow,
    };
    detail::anchor_before(rect, view, scroll, cache)
}

/// 详情区变化后的锚点行；快照刚由 `detail_rows` 重建。
pub(crate) fn anchor_row(
    area: Rect,
    lx: &crate::app::state::Lx,
    cache: &mut crate::ui::PaneCache,
    anchor: &history::HistoryAnchor,
) -> Option<usize> {
    let rect = super::super::layout(area, lx).task_output?;
    let page = lx.task.as_ref()?;
    let run = page
        .detail_agent_index(&lx.agents)
        .and_then(|index| lx.agents.runs.get(index))?;
    let view = DetailView {
        source: HistorySource::from_run(run).with_viewport(HistoryViewport {
            scroll: run.scroll,
            follow_bottom: run.follow,
        }),
        scroll: run.scroll,
        follow: run.follow,
    };
    detail::anchor_row(rect, view, anchor, cache)
}

/// 步骤标记文本（`■ `）占用的列数。
const STEP_MARK_COLS: u16 = 2;

/// 固定列几何；所有 x 都是相对 `list.x` 的偏移。
#[derive(Debug, Clone, Copy)]
struct ListColumns {
    /// 树形层级槽宽度（第一层也留空占位）。
    gutter: u16,
    /// 身份列最大宽度。
    identity_w: u16,
    /// 状态列最大宽度。
    status_w: u16,
    /// 时长列最大宽度。
    age_w: u16,
    x_identity: u16,
    x_status: u16,
    x_age: u16,
    x_title: u16,
}

/// 当前筛选集的树形前缀：列宽计算与绘制共用，避免每行重复回溯兄弟节点。
fn list_prefixes(runs: &[&SubagentRun]) -> Vec<Option<String>> {
    runs.iter()
        .enumerate()
        .map(|(index, run)| tree_prefix(run, runs, index))
        .collect()
}

/// 逐字符宽度，与 [`Grid::put`] / [`Grid::put_clipped`] 的写入口径一致。
///
/// 不能用 `UnicodeWidthStr::width`：它会把 ZWJ emoji 序列合并成 2 列，
/// 而 Grid 是按字符逐格写入的，混用会让列宽和实际占用脱节。
fn text_width(value: &str) -> u16 {
    let width: usize = value.chars().map(|ch| ch.width().unwrap_or(0)).sum();
    u16::try_from(width).unwrap_or(u16::MAX)
}

/// 按当前筛选结果计算固定列；滚动时列不跳。
fn list_columns(runs: &[&SubagentRun], prefixes: &[Option<String>], now_ms: i64) -> ListColumns {
    let gutter = prefixes
        .iter()
        .flatten()
        .map(|prefix| text_width(prefix))
        .max()
        .unwrap_or(0);
    let identity_w = runs
        .iter()
        .map(|run| text_width(&run.display_identity()))
        .max()
        .unwrap_or(0)
        .max(ROLE_MIN);
    let status_w = runs
        .iter()
        .map(|run| {
            text_width(&text::agent_status_with_queue(
                &run.status,
                run.queued_count,
            ))
        })
        .max()
        .unwrap_or(0)
        .max(STATUS_MIN);
    let age_w = runs
        .iter()
        .map(|run| {
            text_width(&crate::app::state::lx::task::age_label(
                run.elapsed_ms(now_ms),
            ))
        })
        .max()
        .unwrap_or(0);
    // 状态图标本身是 `■ ` 两列；身份紧跟其后，不再额外加 gap。
    let x_identity = super::MARKER_COLS
        .saturating_add(gutter)
        .saturating_add(STEP_MARK_COLS);
    let x_status = x_identity
        .saturating_add(identity_w)
        .saturating_add(CELL_GAP);
    let x_age = x_status.saturating_add(status_w).saturating_add(CELL_GAP);
    let x_title = x_age.saturating_add(age_w).saturating_add(CELL_GAP);
    ListColumns {
        gutter,
        identity_w,
        status_w,
        age_w,
        x_identity,
        x_status,
        x_age,
        x_title,
    }
}

/// 编辑态：只画追问 composer（高度由 `task_panel_height` 决定）。
pub(super) fn draw_edit(
    grid: &mut Grid,
    rect: Rect,
    lx: &crate::app::state::Lx,
    focused: bool,
) -> Option<(u16, u16)> {
    if rect.width == 0 || rect.height < 3 {
        return None;
    }
    let page = lx.task.as_ref()?;
    let agent_id = page.agent_edit.as_deref()?;
    let composer = lx.agents.composer(agent_id)?;
    let run = lx.agents.by_agent(agent_id)?;
    let editor = composer.editor();
    let visible = usize::from(rect.height.saturating_sub(2)).max(1);
    let total = editor.visual_rows().len().max(1);
    let scroll = editor.scroll().min(total.saturating_sub(visible));
    let dims = super::super::input::draw(grid, rect, editor, focused, scroll, visible);
    let marker = format!(" {} ", text::LX_TASK_AGENT_INPUT_MARK);
    let marker_w = u16::try_from(marker.width()).unwrap_or(u16::MAX);
    // 标记要么完整显示，要么整块不画；绝不把 "Subagent" 截成半截。
    if rect.width >= marker_w.saturating_add(2) {
        grid.put_clipped(
            rect.x.saturating_add(1),
            rect.y,
            &marker,
            style::accent(),
            rect.width.saturating_sub(2),
        );
    }
    let mode = lx.agents.reply_mode(agent_id);
    draw_agent_badge(grid, rect, run, marker_w, mode);
    dims
}

/// composer 矩形：仅 Agents 编辑态且目标 composer 已创建时返回。
pub(crate) fn composer_rect(rect: Rect, lx: &crate::app::state::Lx) -> Option<Rect> {
    if !lx.agents_editing() || rect.height < 3 {
        return None;
    }
    lx.agent_edit_id()
        .and_then(|agent_id| lx.agents.composer(agent_id))?;
    Some(rect)
}

/// 右侧目标胶囊：` <nickname> [role] · private|report|steer `；左侧必须给 `Subagent` 标记留位。
fn draw_agent_badge(
    grid: &mut Grid,
    rect: Rect,
    run: &SubagentRun,
    marker_w: u16,
    mode: crate::app::state::lx::agent::AgentReplyMode,
) {
    let name = run
        .nickname
        .as_deref()
        .filter(|nickname| !nickname.is_empty())
        .unwrap_or_else(|| run.role.as_str());
    let mode = if run.status == "running" {
        match run.chat.pending_steers.len() {
            0 => "steer".to_string(),
            count => format!("steer {count}"),
        }
    } else {
        mode.label().to_string()
    };
    let with_mode = format!(" {name} [{}] · {mode} ", run.role.as_str());
    let base = format!(" {name} [{}] ", run.role.as_str());
    for label in [with_mode.as_str(), base.as_str()] {
        let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
        if rect.width <= width.saturating_add(marker_w).saturating_add(2) {
            continue;
        }
        let x = rect.right().saturating_sub(width).saturating_sub(1);
        if x <= rect.x.saturating_add(marker_w) {
            continue;
        }
        grid.put(x, rect.y, label, style::btw_qa_badge());
        return;
    }
}

/// 列表绘制：`❯ ■ Turing [explorer]  running  2m 59s  <task 首行>`。
///
/// 固定列：状态图标、身份、状态、时长、标题在所有行同一起点；窄窗格从右往左
/// 整列丢弃（标题 → 时长 → 状态），身份与状态图标永远保留。
pub(super) fn draw_list(grid: &mut Grid, list: Rect, page: &TaskPage, agents: &AgentsState) {
    let indices = agents.visible_indices(page.agent_filter, page.agents_search.text());
    let visible_runs: Vec<&SubagentRun> = indices
        .iter()
        .filter_map(|index| agents.runs.get(*index))
        .collect();
    if visible_runs.is_empty() {
        let label = if page.agents_loading {
            text::LX_TASK_AGENTS_LIST_LOADING
        } else if !page.agents_search.text().is_empty() {
            text::LX_TASK_AGENTS_SEARCH_EMPTY
        } else if page.agent_filter != crate::app::state::AgentFilter::All {
            text::LX_TASK_AGENTS_FILTER_EMPTY
        } else {
            text::LX_TASK_AGENTS_EMPTY
        };
        super::draw_placeholder(grid, list, label);
        return;
    }
    let visible = usize::from(list.height);
    let start = window_start(page, agents, visible);
    let now = page.now_ms;
    let prefixes = list_prefixes(&visible_runs);
    let columns = list_columns(&visible_runs, &prefixes, now);
    let right = list.right();
    for (offset, run) in visible_runs.iter().skip(start).take(visible).enumerate() {
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
        let row_style = if selected {
            style::text()
        } else {
            style::muted()
        };
        // 层级槽：前缀只写在自己的槽内；第一层不画连接符，但同样留空占位。
        let gutter_x = list.x.saturating_add(super::MARKER_COLS);
        let mark_x = gutter_x.saturating_add(columns.gutter);
        // 连层级槽 + 状态图标都放不下时只保留选择标记，禁止把连接符写进滚动条列，
        // 也禁止留下没有图标收口的裸 `└─`。
        if gutter_x >= right || mark_x.saturating_add(STEP_MARK_COLS) > right {
            continue;
        }
        if let Some(prefix) = prefixes.get(index).and_then(Option::as_deref) {
            grid.put_clipped(
                gutter_x,
                y,
                prefix,
                style::chat_notice_connector(),
                columns.gutter,
            );
        }
        // 行首：步骤标记（与历史同一套字形：运行中 `●`、失败/取消/完成各自语义）。
        let mark = if run.is_running() {
            crate::ui::main_content::lx::history::StepMark::Running
        } else {
            match run.status.as_str() {
                "failed" => crate::ui::main_content::lx::history::StepMark::Failed,
                "blocked" | "interrupted" => {
                    crate::ui::main_content::lx::history::StepMark::Interrupted
                }
                "cancelled" => crate::ui::main_content::lx::history::StepMark::Cancelled,
                _ => crate::ui::main_content::lx::history::StepMark::Done,
            }
        };
        // 列表标记保持静态：运行状态由文字与状态色表达，闪烁只留给详情里的步骤。
        let mark_text = format!("{} ", mark.glyph(StepKind::Tool, 0, false));
        grid.put_clipped(
            mark_x,
            y,
            &mark_text,
            mark.style(StepKind::Tool, false),
            STEP_MARK_COLS,
        );
        let identity_x = list.x.saturating_add(columns.x_identity);
        let identity_room = right.saturating_sub(identity_x);
        if identity_room == 0 {
            continue;
        }
        let identity = run.display_identity();
        if columns.identity_w <= identity_room {
            grid.put_clipped(identity_x, y, &identity, row_style, columns.identity_w);
        } else {
            // 极端窄窗格：身份也必须保留，按显示宽度用单字符省略号收尾。
            let clipped = clip_ellipsis(&identity, identity_room);
            grid.put_clipped(identity_x, y, &clipped, row_style, identity_room);
            continue;
        }
        let status_x = list.x.saturating_add(columns.x_status);
        if status_x.saturating_add(columns.status_w) > right {
            continue;
        }
        let status_label = text::agent_status_with_queue(&run.status, run.queued_count);
        let status_style = match run.status.as_str() {
            "running" | "blocked" | "interrupted" => style::agent_warn(),
            "failed" => style::agent_fail(),
            _ => style::agent_step_detail(),
        };
        grid.put_clipped(status_x, y, &status_label, status_style, columns.status_w);
        let age_x = list.x.saturating_add(columns.x_age);
        if age_x.saturating_add(columns.age_w) <= right {
            let age = crate::app::state::lx::task::age_label(run.elapsed_ms(now));
            let age_w = text_width(&age);
            // 时长是数字列，右对齐在固定宽度内，不把标题起点带跑。
            let age_start = age_x.saturating_add(columns.age_w).saturating_sub(age_w);
            grid.put_clipped(age_start, y, &age, style::agent_step_detail(), age_w);
        }
        // 严格从右往左降级：标题先于时长消失，绝不回填到空出来的时长槽。
        let title_x = list.x.saturating_add(columns.x_title);
        let title_w = right.saturating_sub(title_x);
        let title = run.display_title();
        if title_w > 0 && !title.is_empty() {
            grid.put_clipped(
                title_x,
                y,
                &clip_ellipsis(title, title_w),
                row_style,
                title_w,
            );
        }
    }
}

/// 按显示宽度裁剪并用单字符 `…` 收尾；宽字符不会写半格。
fn clip_ellipsis(text: &str, max: u16) -> String {
    if max == 0 {
        return String::new();
    }
    if text_width(text) <= max {
        return text.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let budget = usize::from(max) - 1;
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let cell = ch.width().unwrap_or(0);
        if width.saturating_add(cell) > budget {
            break;
        }
        out.push(ch);
        width = width.saturating_add(cell);
    }
    out.push('…');
    out
}

/// 树形缩进前缀：直接子 agent 不缩进；孙辈起用 `├─ `/`└─ ` 表示同级末项。
///
/// 父节点被当前筛选隐藏时不画悬空连接符（`Active`/`Done` 视图的树视觉仍完整）。
fn tree_prefix(run: &SubagentRun, runs: &[&SubagentRun], index: usize) -> Option<String> {
    let depth = run.depth();
    if depth < 2 {
        return None;
    }
    let parent = parent_path(&run.agent_path)?;
    if !runs
        .iter()
        .any(|other| other.agent_path.eq_ignore_ascii_case(parent))
    {
        return None;
    }
    // 只看同一父节点下的后续兄弟；遇到更浅层或另一个父分支即当前子树结束，
    // 更深的后代属于当前/后续兄弟的子树，继续向后找。
    let last = !runs.get(index.saturating_add(1)..).is_some_and(|rest| {
        rest.iter()
            .find(|other| other.depth() <= depth)
            .is_some_and(|other| {
                other.depth() == depth
                    && parent_path(&other.agent_path)
                        .is_some_and(|other_parent| other_parent.eq_ignore_ascii_case(parent))
            })
    });
    let connector = if last {
        text::agent::TREE_LEAF.trim_start()
    } else {
        text::agent::TREE_BRANCH.trim_start()
    };
    let indent = "  ".repeat(depth.saturating_sub(2));
    Some(format!("{indent}{connector}"))
}

/// 父路径（`/root/turing/curie` -> `/root/turing`）。
fn parent_path(agent_path: &str) -> Option<&str> {
    agent_path
        .trim_end_matches('/')
        .rsplit_once('/')
        .map(|(parent, _)| parent)
}

/// 列表窗口起点（与进程列表同口径）。
pub(super) fn window_start(page: &TaskPage, agents: &AgentsState, visible: usize) -> usize {
    let len = agents
        .visible_indices(page.agent_filter, page.agents_search.text())
        .len();
    if visible == 0 || len <= visible {
        return 0;
    }
    if let Some(start) = page.agents_window.viewport {
        return start.min(len.saturating_sub(visible));
    }
    let active = page.agents_window.active.min(len - 1);
    active.saturating_sub(visible - 1).min(len - visible)
}
