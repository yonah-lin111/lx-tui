//! lx 聊天历史的视口状态：滚动偏移、逐帧几何记忆与卡片悬停 / 折叠入口。
//!
//! 从 `chat/mod.rs` 拆出（单文件行数上限）；这些函数都只改窗格状态，
//! 几何由事件循环或 UI 纯函数传入，不做 IO。

use crate::app::state::{AppState, CardTarget};
use crate::layout::PaneId;

/// 滚动历史视图（正数向下）；`total`/`visible` 为渲染行数与视口行数；返回是否移动。
///
/// 状态里的 `history_scroll` 是**顶部行索引**（0 = 最早）；`follow_bottom` 表示
/// 视口钉在最新一屏（顶部行 = `total - visible`）。
pub fn scroll_lx_history(
    state: &mut AppState,
    pane: PaneId,
    delta: isize,
    total: usize,
    visible: usize,
) -> bool {
    // 鼠标滚动历史即退出撤销高亮/动作面板；即使内容不可滚也吞掉这次滚轮。
    let cancelled = super::rewind::cancel_on_scroll(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return cancelled;
    }
    let max_top = total.saturating_sub(visible);
    let current = if target.lx.chat.follow_bottom {
        max_top
    } else {
        target.lx.chat.history_scroll.min(max_top)
    };
    let next = (current as isize + delta).clamp(0, max_top as isize) as usize;
    let moved = next != current || target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = next;
    target.lx.chat.follow_bottom = next >= max_top;
    moved || cancelled
}

/// 直接设置历史视口顶部行偏移（滚动条点击/拖拽）；`total`/`visible` 为渲染行数与视口行数。
/// 顶部行 == `total - visible` 时视为钉底跟随；返回视口是否移动。
pub fn set_lx_history_scroll(
    state: &mut AppState,
    pane: PaneId,
    offset: usize,
    total: usize,
    visible: usize,
) -> bool {
    // 拖拽/点击历史滚动条同样退出撤销高亮。
    let cancelled = super::rewind::cancel_on_scroll(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return cancelled;
    }
    let max_top = total.saturating_sub(visible);
    let clamped = offset.min(max_top);
    let before = target.lx.chat.history_scroll;
    let was_following = target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = clamped;
    target.lx.chat.follow_bottom = clamped >= max_top;
    before != clamped || was_following != target.lx.chat.follow_bottom || cancelled
}

/// 双击 Esc 选中最新轮次后的**一次性居中**：把目标用户消息滚到视口中央。
///
/// `app/` 拿不到几何，故与 [`resolve_lx_history`] 同一做法：事件循环在绘制前
/// 调用。内容不足一屏（滚动条不可滚动）时不改动视口，只清掉请求。
pub(crate) fn resolve_lx_rewind_center(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut crate::ui::RenderCache,
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::layout::pane_inner_rect(*rect);
        let Some(message_id) = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| target.lx.rewind.center_target.clone())
        else {
            continue;
        };
        let resolved = state.pane_anywhere(*pane).and_then(|target| {
            crate::ui::main_content::lx::history_center_target(
                inner,
                &target.lx,
                &message_id,
                cache.pane(*pane),
            )
        });
        let Some(target) = state.pane_mut_anywhere(*pane) else {
            continue;
        };
        target.lx.rewind.center_target = None;
        let Some((top, max_top)) = resolved else {
            continue;
        };
        changed |= target.lx.chat.history_scroll != top || target.lx.chat.follow_bottom;
        target.lx.chat.history_scroll = top;
        target.lx.chat.follow_bottom = top >= max_top;
    }
    changed
}

/// 每轮把各 lx 窗格的「当前视口顶部行」与置顶行抑制记忆落状态（绘制前写）。
///
/// 渲染层只读、不写状态（AGENTS：渲染只读），而展开切换需要知道用户正看着哪一行，
/// 故放在事件循环里、`tui.draw` 之前写。一次 `history_view` 同时给出两处结论，
/// 且与绘制共用同一份几何快照。返回是否有窗格记忆变化。
pub(crate) fn resolve_lx_history(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut crate::ui::RenderCache,
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::layout::pane_inner_rect(*rect);
        let resolved = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| {
                crate::ui::main_content::lx::history_view(inner, &target.lx, cache.pane(*pane))
            })
            .map(|view| (view.scroll, view.suppressed));
        let Some((scroll, suppressed)) = resolved else {
            continue;
        };
        let Some(target) = state.pane_mut_anywhere(*pane) else {
            continue;
        };
        if target.lx.chat.viewport_top != Some(scroll) {
            target.lx.chat.viewport_top = Some(scroll);
            changed = true;
        }
        if target.lx.chat.suppressed_prompt_header != suppressed {
            target.lx.chat.suppressed_prompt_header = suppressed;
            changed = true;
        }
    }
    changed
}

/// 每轮把后台任务面板的**输出可见行数**落状态（绘制前写）。
///
/// 面板打开时输入框被控制面板替换，输出容器高度由窗格几何决定；键盘翻页与滚动条
/// 几何需要它，而 `app/` 拿不到窗格几何，故与 [`resolve_lx_history`] 同一套做法
/// 放在事件循环里。
///
/// 只算 `task_output_rows`（一次 `layout`），**不**顺带算历史行数：面板打开时历史区
/// 根本没画，历史行数无人消费，而 `history_rows` 内部会对整段会话正文跑一遍
/// markdown 排版，白烧每帧 CPU。
pub fn resolve_task_output_visible(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::layout::pane_inner_rect(*rect);
        let Some(output_visible) = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .filter(|target| target.lx.task.is_some())
            .and_then(|target| crate::ui::main_content::lx::task_output_rows(inner, &target.lx))
        else {
            continue;
        };
        let Some(page) = state
            .pane_mut_anywhere(*pane)
            .and_then(|target| target.lx.task.as_mut())
        else {
            continue;
        };
        if page.output_visible != output_visible {
            // 只做「保持阅读位置」的钳制；原本贴尾才继续跟随最新。
            page.resize_output_viewport(output_visible);
            changed = true;
        }
    }
    changed
}

/// 每轮把 Agents 详情区的可见行数与总行数落状态（翻页/滚动条几何用）。
///
/// 只在消息流变化后重量一次：历史正文的 markdown 排版是这里最贵的一步，
/// 每帧重算会白白吃掉整帧预算。
pub fn resolve_task_agents_visible(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::layout::pane_inner_rect(*rect);
        let Some((needs_measure, visible)) = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| {
                let page = target.lx.task.as_ref()?;
                if page.tab != crate::app::state::TaskTab::Agents {
                    return None;
                }
                let run = target.lx.agents.runs.get(page.agents_window.active)?;
                Some((
                    run.rows_dirty || run.total_rows == 0,
                    usize::from(
                        crate::ui::main_content::lx::task_output_inner(inner, &target.lx)?.height,
                    ),
                ))
            })
        else {
            continue;
        };
        let total = if needs_measure {
            state
                .pane_anywhere(*pane)
                .and_then(|target| crate::ui::main_content::lx::task_agents_rows(inner, &target.lx))
                .map(|(_, total)| total)
        } else {
            None
        };
        let Some(target) = state.pane_mut_anywhere(*pane) else {
            continue;
        };
        let Some(page) = target.lx.task.as_mut() else {
            continue;
        };
        let Some(run) = target.lx.agents.runs.get_mut(page.agents_window.active) else {
            continue;
        };
        let total = total.unwrap_or(run.total_rows);
        if run.visible != visible || run.total_rows != total || run.rows_dirty {
            run.visible = visible;
            run.total_rows = total;
            run.rows_dirty = false;
            let max = total.saturating_sub(visible);
            run.scroll = if run.follow { max } else { run.scroll.min(max) };
            changed = true;
        }
    }
    changed
}

/// 每轮把 `ask` 预览区的可见行数落状态（滚动上限钳制用）。
///
/// 与 [`resolve_task_output_visible`] 同款：渲染层只读，几何由事件循环在绘制前写入，
/// 状态里的偏移因此永远落在「真的能滚到」的范围内。
pub fn resolve_ask_preview_visible(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::layout::pane_inner_rect(*rect);
        let Some(visible) = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| crate::ui::main_content::lx::ask_preview_rows(inner, &target.lx))
        else {
            continue;
        };
        if let Some(ask) = state
            .pane_mut_anywhere(*pane)
            .and_then(|target| target.lx.ask.as_mut())
        {
            changed |= ask.resize_preview(visible);
        }
    }
    changed
}

/// 点击置顶行：跳到该 prompt（顶部对齐该消息起始行），并退出跟随底部。
/// `target_row` 由 `ui::main_content::lx::sticky_prompt_jump` 按当前几何给出。
pub fn jump_lx_history_to_prompt(state: &mut AppState, pane: PaneId, target_row: usize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let before = target.lx.chat.history_scroll;
    let was_following = target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = target_row;
    // 明确进入阅读态：置顶行随之按规则让位（顶行成了那条用户消息自己）。
    target.lx.chat.follow_bottom = false;
    before != target_row || was_following
}

/// 回到底部：把历史视口钉到最新一屏（跟随底部）并清除悬停态；返回是否发生变化。
pub fn jump_lx_history_bottom(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let changed = !target.lx.chat.follow_bottom || target.lx.chat.jump_bottom_hover;
    target.lx.chat.follow_bottom = true;
    target.lx.chat.jump_bottom_hover = false;
    changed
}

/// 设置「回到底部」按钮悬停态；返回是否变化。
pub fn hover_lx_jump_bottom(state: &mut AppState, pane: PaneId, hovered: bool) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.jump_bottom_hover == hovered {
        return false;
    }
    target.lx.chat.jump_bottom_hover = hovered;
    true
}

/// 设置聊天历史卡片悬停态；返回是否变化。
///
/// 只对 lx 窗格生效：悬停状态归窗格所有，指针换到别的窗格时必须清掉旧值，
/// 否则上一张卡片会一直保持高亮。
pub fn hover_lx_chat(state: &mut AppState, pane: PaneId, hover: Option<CardTarget>) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.hover == hover {
        return false;
    }
    target.lx.chat.hover = hover;
    true
}

/// 汇报块展开/折叠（槽位 `report:<agent_id>`，与工具卡片同一份展开状态）。
fn toggle_report_card(
    state: &mut AppState,
    pane: PaneId,
    message_id: &str,
    agent_id: &str,
    more: bool,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let slot = format!("report:{agent_id}");
    if more {
        target.lx.chat.expand_report_more(message_id, &slot);
        return true;
    }
    let expanded = target.lx.chat.toggle_detail(message_id, &slot);
    if !expanded {
        target.lx.chat.clear_report_expand(message_id, &slot);
    }
    true
}

/// 点击卡片（未拖动）：切换该块的折叠状态。
///
/// 用户要求「点击折叠内容后展开，点击展开内容后重新折叠」：整张卡片都是点击面，
/// 不再有单独的动作按钮，也没有 hover 高亮。与键盘入口（`^o` / `^t`）共用同一份
/// 展开状态与视口锚定实现。
pub fn toggle_lx_card(state: &mut AppState, pane: PaneId, target: CardTarget) -> bool {
    let changed = match target {
        CardTarget::Tool {
            message_id,
            call_id,
            section,
        } => super::activate_tool_card(state, pane, &message_id, &call_id, section),
        CardTarget::Thinking {
            message_index,
            step_index,
        } => super::toggle_thinking_at(state, pane, message_index, step_index),
        CardTarget::Shell {
            message_id,
            section,
        } => super::user_shell::activate_shell_card(state, pane, &message_id, section),
        CardTarget::Compact { checkpoint_id } => {
            super::compact::toggle_checkpoint(state, pane, &checkpoint_id)
        }
        CardTarget::Report {
            message_id,
            agent_id,
            more,
        } => toggle_report_card(state, pane, &message_id, &agent_id, more),
        CardTarget::OpenAgent { agent_id } => super::open_agent_page(state, pane, &agent_id),
    };
    if changed {
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    changed
}
