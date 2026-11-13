//! lx 聊天历史的视口状态：滚动偏移、逐帧几何记忆与卡片悬停 / 折叠入口。
//!
//! 从 `chat/mod.rs` 拆出（单文件行数上限）；这些函数都只改窗格状态，
//! 几何由事件循环或 UI 纯函数传入，不做 IO。

use crate::app::state::{AppState, CardSection, CardTarget};
use crate::tiling::PaneId;

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
        let inner = crate::tiling::pane_inner_rect(*rect);
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
        let inner = crate::tiling::pane_inner_rect(*rect);
        let Some(history_rect) = state
            .pane_anywhere(*pane)
            .filter(|target| {
                // 面板打开时历史区被任务输出替换，没人消费 viewport_top / 置顶记忆；
                // 这里若继续排版，会与 Agents 详情抢同一个 PaneCache 单快照槽。
                target.view == crate::app::state::PaneView::Lx && target.lx.task.is_none()
            })
            .and_then(|target| crate::ui::main_content::lx::history_rect(inner, &target.lx))
        else {
            continue;
        };
        // 变化前锚点：必须在 `history_view` 重建快照之前读上一帧的几何。
        let base_scroll = state
            .pane_anywhere(*pane)
            .map_or(0, |target| target.lx.chat.history_scroll);
        let anchor = state
            .pane_anywhere(*pane)
            .filter(|target| !target.lx.chat.follow_bottom)
            .and_then(|target| {
                crate::ui::main_content::lx::history_anchor_before(
                    history_rect,
                    &target.lx,
                    cache.pane(*pane),
                    base_scroll,
                )
            });
        let Some(mut view) = state.pane_anywhere(*pane).and_then(|target| {
            crate::ui::main_content::lx::history_view(inner, &target.lx, cache.pane(*pane))
        }) else {
            continue;
        };
        // 内容变化后补偿：把锚点块的新旧行差加回顶部行，阅读位置不动。
        // 补偿永不改 `follow_bottom`；只有用户自己滚到底才恢复跟随。
        if let Some(anchor) = &anchor {
            let new_row = state.pane_anywhere(*pane).and_then(|target| {
                crate::ui::main_content::lx::history_anchor_row(
                    history_rect,
                    &target.lx,
                    cache.pane(*pane),
                    anchor,
                )
            });
            if let Some(new_row) = new_row {
                let delta = new_row as isize - anchor.row as isize;
                let max_top = view
                    .total_rows
                    .saturating_sub(usize::from(view.body.height));
                // 基准必须是捕获锚点用的 `history_scroll`，不是可能已被新几何
                // 钳制过的 `view.scroll`；否则收缩时会把同一个钳制算两次。
                let next = (base_scroll as isize + delta).clamp(0, max_top as isize) as usize;
                if next != base_scroll {
                    if let Some(target) = state.pane_mut_anywhere(*pane) {
                        target.lx.chat.history_scroll = next;
                        changed = true;
                    }
                    // 置顶行让位可能随新顶部行变化：用同一快照重算一次。
                    if let Some(updated) = state.pane_anywhere(*pane).and_then(|target| {
                        crate::ui::main_content::lx::history_view(
                            inner,
                            &target.lx,
                            cache.pane(*pane),
                        )
                    }) {
                        view = updated;
                    }
                }
            }
        }
        let Some(target) = state.pane_mut_anywhere(*pane) else {
            continue;
        };
        // 几何收缩（resize/锚点条目被删除/置顶行让位）时，视图顶部会被钳到
        // `total - body.height`；状态必须同步回写，否则后续锚点和滚轮都会以
        // 越界旧行号为基准，表现为静默贴底或延迟一帧停住。
        if !target.lx.chat.follow_bottom {
            let max_top = view
                .total_rows
                .saturating_sub(usize::from(view.body.height));
            let clamped = target.lx.chat.history_scroll.min(max_top);
            if target.lx.chat.history_scroll != clamped {
                target.lx.chat.history_scroll = clamped;
                changed = true;
            }
        }
        if target.lx.chat.viewport_top != Some(view.scroll) {
            target.lx.chat.viewport_top = Some(view.scroll);
            changed = true;
        }
        if target.lx.chat.suppressed_prompt_header != view.suppressed {
            target.lx.chat.suppressed_prompt_header = view.suppressed;
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
        let inner = crate::tiling::pane_inner_rect(*rect);
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

/// 每轮把 BTW 对话可见行数与 composer 尺寸落状态（绘制前写）。
pub(crate) fn resolve_task_btw_visible(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut crate::ui::RenderCache,
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::tiling::pane_inner_rect(*rect);
        let Some((needs_measure, visible, anchor)) = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .filter(|target| target.lx.is_btw_tab())
            .and_then(|target| {
                let thread = target.lx.btw.active_thread()?;
                let needs_measure = thread.rows_dirty || thread.total_rows == 0;
                let output = crate::ui::main_content::lx::task_output_inner(inner, &target.lx)?;
                let anchor = if !thread.follow && needs_measure {
                    crate::ui::main_content::lx::task_btw_anchor_before(
                        inner,
                        &target.lx,
                        cache.pane(*pane),
                        thread.scroll,
                    )
                } else {
                    None
                };
                Some((needs_measure, usize::from(output.height), anchor))
            })
        else {
            continue;
        };
        let measured = if needs_measure {
            state
                .pane_anywhere(*pane)
                .and_then(|target| {
                    crate::ui::main_content::lx::task_btw_rows(inner, &target.lx, cache.pane(*pane))
                })
                .map(|(_, total, fresh)| (total, fresh))
        } else {
            None
        };
        let anchor_row = anchor.as_ref().and_then(|anchor| {
            state.pane_anywhere(*pane).and_then(|target| {
                crate::ui::main_content::lx::task_btw_anchor_row(
                    inner,
                    &target.lx,
                    cache.pane(*pane),
                    anchor,
                )
            })
        });
        // 只有编辑态才需要同步 composer 的软换行宽度/可见行。
        let composer = state.pane_anywhere(*pane).and_then(|target| {
            crate::ui::main_content::lx::task_btw_composer_rect(inner, &target.lx)
        });
        let Some(target) = state.pane_mut_anywhere(*pane) else {
            continue;
        };
        if let Some(composer) = composer {
            let width = composer.width.saturating_sub(3).max(1);
            let rows = composer.height.saturating_sub(2).max(1);
            if let Some(thread) = target.lx.btw.active_thread_mut() {
                thread.composer.editor_mut().resize(width, rows);
            }
        }
        let Some(thread) = target.lx.btw.active_thread_mut() else {
            continue;
        };
        let (measured_total, fresh) = measured.unwrap_or((thread.total_rows, true));
        let total = if fresh {
            measured_total
        } else {
            thread.total_rows
        };
        let max = total.saturating_sub(visible);
        let should_follow = thread.follow || total <= visible.saturating_add(1);
        let mut next_scroll = if should_follow { max } else { thread.scroll };
        if !should_follow {
            if let Some(anchor) = anchor.as_ref()
                && let Some(new_row) = anchor_row
            {
                let delta = new_row as isize - anchor.row as isize;
                next_scroll = (next_scroll as isize + delta).clamp(0, max as isize) as usize;
            } else {
                next_scroll = next_scroll.min(max);
            }
        }
        if thread.visible != visible
            || thread.total_rows != total
            || thread.rows_dirty
            || thread.follow != should_follow
            || thread.scroll != next_scroll
        {
            thread.visible = visible;
            if fresh {
                thread.total_rows = total;
                thread.rows_dirty = false;
            }
            thread.follow = should_follow;
            thread.scroll = next_scroll;
            changed = true;
        }
    }
    changed
}

/// 窗格几何变化后让所有 Agents 详情重新量行。
///
/// `total_rows` 只在 `rows_dirty` 时重算；resize 不产生 delta，必须显式失效，
/// 否则滚动条总量、翻页与拖拽钳制会一直沿用旧宽度下的行数。
pub(crate) fn invalidate_task_agent_rows(state: &mut AppState) {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        if target.view != crate::app::state::PaneView::Lx {
            continue;
        }
        for run in &mut target.lx.agents.runs {
            run.rows_dirty = true;
        }
        target.lx.btw.mark_rows_dirty();
    }
}

/// 每轮把 Agents 详情区的可见行数与总行数落状态（翻页/滚动条几何用）。
///
/// 只在消息流变化后重量一次：历史正文的 markdown 排版是这里最贵的一步，
/// 每帧重算会白白吃掉整帧预算。
pub(crate) fn resolve_task_agents_visible(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut crate::ui::RenderCache,
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::tiling::pane_inner_rect(*rect);
        let Some((needs_measure, visible, anchor)) = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| {
                let page = target.lx.task.as_ref()?;
                if page.tab != crate::app::state::TaskTab::Agents {
                    return None;
                }
                let run = page
                    .detail_agent_index(&target.lx.agents)
                    .and_then(|index| target.lx.agents.runs.get(index))?;
                let needs_measure = run.rows_dirty || run.total_rows == 0;
                let output = crate::ui::main_content::lx::task_output_inner(inner, &target.lx)?;
                // 变化前锚点：必须在 `task_agents_rows` 重建快照之前读旧几何。
                let anchor = if !run.follow && needs_measure {
                    crate::ui::main_content::lx::task_agents_anchor_before(
                        inner,
                        &target.lx,
                        cache.pane(*pane),
                        run.scroll,
                    )
                } else {
                    None
                };
                Some((needs_measure, usize::from(output.height), anchor))
            })
        else {
            continue;
        };
        let measured = if needs_measure {
            state
                .pane_anywhere(*pane)
                .and_then(|target| {
                    crate::ui::main_content::lx::task_agents_rows(
                        inner,
                        &target.lx,
                        cache.pane(*pane),
                    )
                })
                .map(|(_, total, fresh)| (total, fresh))
        } else {
            None
        };
        // 新快照里的锚点行：与旧锚点行相减即为阅读位置需要补偿的位移。
        let anchor_row = anchor.as_ref().and_then(|anchor| {
            state.pane_anywhere(*pane).and_then(|target| {
                crate::ui::main_content::lx::task_agents_anchor_row(
                    inner,
                    &target.lx,
                    cache.pane(*pane),
                    anchor,
                )
            })
        });
        // Agents composer 与 BTW 同源：编辑态必须用实际 composer 宽度/正文行数
        // 驱动软换行与光标可见性，否则 `Lx` 会一直停在构造时的 `u16::MAX`。
        let composer = state.pane_anywhere(*pane).and_then(|target| {
            crate::ui::main_content::lx::task_agent_composer_rect(inner, &target.lx)
        });
        let Some(target) = state.pane_mut_anywhere(*pane) else {
            continue;
        };
        if let Some(composer) = composer {
            let width = composer.width.saturating_sub(3).max(1);
            let rows = composer.height.saturating_sub(2).max(1);
            if let Some(agent_id) = target.lx.agent_edit_id().map(str::to_string)
                && let Some(composer) = target.lx.agents.composer_mut(&agent_id)
            {
                composer.editor_mut().resize(width, rows);
            }
        }
        let Some(page) = target.lx.task.as_mut() else {
            continue;
        };
        let Some(active) = page.detail_agent_index(&target.lx.agents) else {
            continue;
        };
        let Some(run) = target.lx.agents.runs.get_mut(active) else {
            continue;
        };
        let (measured_total, fresh) = measured.unwrap_or((run.total_rows, true));
        // 旧快照的 total 不能覆盖已经落状态的值；只用它参与本帧的保守钳制。
        let total = if fresh {
            measured_total
        } else {
            run.total_rows
        };
        let max = total.saturating_sub(visible);
        // 只由显式状态决定是否跟随：用户滚到底/点击回到底部会写 `run.follow = true`。
        // 不能用「scroll 已到旧 max」短路——宽度变化或内容收缩后旧 max 变小，会把
        // 正在阅读的用户重新吸附到底部；补偿也不应主动恢复跟随。
        let should_follow = run.follow || total <= visible.saturating_add(1);
        let mut next_scroll = if should_follow { max } else { run.scroll };
        // 阅读态且内容在上方变高/变矮：按锚点行差补偿，不把用户顶走。
        if !should_follow {
            if let Some(anchor) = anchor.as_ref()
                && let Some(new_row) = anchor_row
            {
                let delta = new_row as isize - anchor.row as isize;
                next_scroll = (next_scroll as isize + delta).clamp(0, max as isize) as usize;
            } else {
                // 锚点缺失（首帧/宽度变化）也要把越界状态钳回，避免留一帧不一致。
                next_scroll = next_scroll.min(max);
            }
        }
        if run.visible != visible
            || run.total_rows != total
            || run.rows_dirty
            || run.follow != should_follow
            || run.scroll != next_scroll
        {
            run.visible = visible;
            if fresh {
                run.total_rows = total;
                run.rows_dirty = false;
            }
            run.follow = should_follow;
            run.scroll = next_scroll;
            changed = true;
        }
    }
    changed
}

/// 每轮把 `ask` 统一列表的 `(滚动起点, 总行数, 可视行数)` 落状态。
///
/// 与 [`resolve_task_output_visible`] 同款：渲染层只读，几何由事件循环在绘制前写入。
/// `PageUp`/`PageDown` 在 `app/` 层拿不到窗格矩形，只能消费这里落下的上一帧窗口。
pub fn resolve_ask_list_window(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::tiling::pane_inner_rect(*rect);
        let Some((scroll, total, visible)) = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| crate::ui::main_content::lx::ask_list_window(inner, &target.lx))
        else {
            continue;
        };
        if let Some(ask) = state
            .pane_mut_anywhere(*pane)
            .and_then(|target| target.lx.ask.as_mut())
        {
            changed |= ask.sync_list_window(scroll, total, visible);
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

/// 回到底部：把当前视图钉到最新一屏并清除悬停态；返回是否发生变化。
///
/// 后台任务面板打开时按**当前页签的上方内容区**执行：`Processes` 贴输出尾部，
/// `Agents` 恢复选中子 agent 的跟随（并同步滚动偏移）。
pub fn jump_lx_history_bottom(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx {
        return false;
    }
    if let Some(page) = target.lx.task.as_mut() {
        target.lx.chat.jump_bottom_hover = false;
        return match page.tab {
            crate::app::state::TaskTab::Processes => {
                let before = page.output_scroll;
                page.pin_output_to_tail();
                page.output_scroll != before
            }
            crate::app::state::TaskTab::Agents => page
                .detail_agent_index(&target.lx.agents)
                .and_then(|index| target.lx.agents.runs.get_mut(index))
                .is_some_and(|run| {
                    let changed = !run.follow;
                    run.follow = true;
                    if run.visible > 0 {
                        run.scroll = run.total_rows.saturating_sub(run.visible);
                    }
                    changed
                }),
            crate::app::state::TaskTab::Btw => {
                target.lx.btw.active_thread_mut().is_some_and(|thread| {
                    let changed = !thread.follow;
                    thread.follow = true;
                    if thread.visible > 0 {
                        thread.scroll = thread.total_rows.saturating_sub(thread.visible);
                    }
                    changed
                })
            }
        };
    }
    if target.lx.chat.messages.is_empty() {
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
    let exists = state.pane_anywhere(pane).is_some_and(|target| {
        target.lx.chat.messages.iter().any(|message| {
            message.id == message_id
                && message
                    .reports
                    .iter()
                    .any(|report| report.agent_id == agent_id)
        })
    });
    if !exists {
        return false;
    }
    // 展开/追加批次会改变行数：跟随底部时先钉住用户正在看的顶行。
    super::pin_lx_viewport(state, pane);
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
/// 用户要求「点击折叠内容后展开，点击展开内容后重新折叠」：工具卡按块登记点击面，
/// 系统事件只登记可折叠正文（标题与元信息不响应）；没有单独的动作按钮，也没有 hover
/// 高亮。与键盘入口（`^o` / `^t`）共用同一份展开状态与视口锚定实现。
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
        CardTarget::ModelSwitch { switch_id } => {
            super::model_switch::toggle_model_switch(state, pane, &switch_id)
        }
        CardTarget::Attachment { path } => {
            if !path.is_empty() {
                state
                    .effects
                    .push(crate::app::effects::Effect::RevealInFileManager {
                        path: std::path::PathBuf::from(path),
                    });
            }
            true
        }
        CardTarget::DownloadOriginalImage { attachment_id } => {
            super::pin_lx_viewport(state, pane);
            super::web::trigger_ai_web_original_image_download(state, pane, &attachment_id)
        }
        CardTarget::Recap { message_id } => {
            super::pin_lx_viewport(state, pane);
            let Some(target) = state.pane_mut_anywhere(pane) else {
                return false;
            };
            target
                .lx
                .chat
                .toggle_section(&message_id, "recap", CardSection::Body)
        }
        CardTarget::Report {
            message_id,
            agent_id,
            more,
        } => toggle_report_card(state, pane, &message_id, &agent_id, more),
        CardTarget::OpenAgent { agent_id } => super::open_agent_page(state, pane, &agent_id),
        CardTarget::UserMessage { message_id } => {
            let exists = state.pane_anywhere(pane).is_some_and(|target| {
                target
                    .lx
                    .chat
                    .messages
                    .iter()
                    .any(|message| message.id == message_id)
            });
            if !exists {
                return false;
            }
            // 长用户消息折叠/展开会改变行数，和工具卡一样先锚定。
            super::pin_lx_viewport(state, pane);
            let Some(target) = state.pane_mut_anywhere(pane) else {
                return false;
            };
            target.lx.chat.toggle_user_message_expanded(&message_id)
        }
        CardTarget::UserPaste {
            message_id,
            paste_id,
            occurrence,
        } => {
            super::pin_lx_viewport(state, pane);
            let Some(target) = state.pane_mut_anywhere(pane) else {
                return false;
            };
            target
                .lx
                .chat
                .toggle_user_paste(&message_id, paste_id, occurrence);
            true
        }
        CardTarget::OpenUrl { url } => {
            if !url.is_empty() {
                state
                    .effects
                    .push(crate::app::effects::Effect::OpenBrowser { url });
            }
            true
        }
    };
    if changed {
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    changed
}

/// 切换指定 thinking 块展开状态。
///
/// 先 [`super::pin_lx_viewport`] 再切换：跟随底部时 `history_scroll` 是过期值，不先锚定就会
/// 让展开后的界面跳回历史最顶部（表现为「点了没反应 / 展不开」）。
pub fn toggle_thinking_at(
    state: &mut AppState,
    pane: PaneId,
    message_index: usize,
    step_index: usize,
) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(message) = target.lx.chat.messages.get(message_index) else {
        return false;
    };
    if !matches!(
        message.step_at(step_index),
        Some(crate::app::state::AssistantStepView::Part { part, .. })
            if part.kind == crate::chat::events::PartKind::Reasoning
    ) {
        return false;
    }
    super::pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.toggle_reasoning(message_index, step_index);
    crate::app::update::clear_lx_selection_for(state, pane);
    true
}

/// 切换最近一条 assistant 消息里的最后一个 thinking 块（Ctrl+T）。
pub fn toggle_latest_thinking(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let mut found = None;
    'outer: for (message_index, message) in target.lx.chat.messages.iter().enumerate().rev() {
        for (step_index, step) in message.steps.iter().enumerate().rev() {
            if step
                .part()
                .is_some_and(|part| part.kind == crate::chat::events::PartKind::Reasoning)
            {
                found = Some((message_index, step_index));
                break 'outer;
            }
        }
    }
    let Some((message_index, step_index)) = found else {
        return false;
    };
    super::pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.toggle_reasoning(message_index, step_index);
    crate::app::update::clear_lx_selection_for(state, pane);
    true
}
