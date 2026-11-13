//! `lx` 视图窗格内的鼠标按下分派：Ask 面板、后台任务面板、命令候选浮层、聊天控件、超链接/生图 `Ctrl + 点击` 与选区。

use std::time::Instant;

use crossterm::event::{KeyModifiers, MouseEvent};
use ratatui::layout::Rect;

use crate::app::state::{AppState, CardTarget, PaneKind, PaneView, TaskTab};
use crate::app::update;
use crate::config::Config;
use crate::tiling::PaneId;
use crate::ui;

use super::super::hit::{
    apply_lx_chat_hit, handle_lx_history_scrollbar_press, handle_lx_input_scrollbar_press,
    handle_task_agents_scrollbar_press, handle_task_btw_scrollbar_press,
    handle_task_list_scrollbar_press, lx_toast_at, pane_at, pane_content_inner,
};
use crate::event::session::Geometry;

/// 处理 `PaneView::Lx` 窗格内部的鼠标左键按下事件。
pub(super) fn handle_lx_pane_mouse_down_left(
    mouse: &MouseEvent,
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    // lx 页：提及面板候选 > 聊天控件 > 后台任务面板控件 > 入口按钮 > …
    update::focus_pane(state, pane);
    // `ask` 面板优先于其它一切：面板内点击一律吞掉，绝不穿透到历史/输入框。
    //
    // 后台任务页打开时 ask 面板**不可见**（task 与 ask 共用输入框矩形，
    // 绘制由 task 接管）：此时绝不能命中隐藏面板，否则点任务列表会
    // 静默改一个看不见的答案，而列表本身永远点不动。
    if let Some(target) = state
        .pane_anywhere(pane)
        .filter(|target| target.lx.task.is_none())
        && target.lx.ask.as_ref().is_some_and(|ask| ask.editing)
        && let Some((visual_row, col)) =
            ui::main_content::lx::ask_editor_cell_at(inner, &target.lx, mouse.column, mouse.row)
    {
        let clicks = crate::app::selection::next_click_count(
            &mut state.selection_click,
            mouse.column,
            mouse.row,
            Instant::now(),
        );
        update::clear_selection(state);
        if let Some(target_mut) = state.pane_mut_anywhere(pane)
            && let Some(ask_mut) = target_mut.lx.ask.as_mut()
            && let Some(editor) = ask_mut.editor_mut()
        {
            editor.begin_text_selection_with_clicks(visual_row, col, clicks);
            state.ask_input_drag = Some(pane);
        }
        *dirty = true;
        return;
    }
    let ask_hit = state
        .pane_anywhere(pane)
        .filter(|target| target.lx.task.is_none())
        .and_then(|target| {
            let rect = ui::main_content::lx::ask_rect(inner, &target.lx)?;
            if !rect.contains((mouse.column, mouse.row).into()) {
                return None;
            }
            Some(ui::main_content::lx::ask_hit(
                inner,
                &target.lx,
                mouse.column,
                mouse.row,
            ))
        });
    if let Some(hit) = ask_hit {
        if let Some(hit) = hit {
            update::apply_ask_click(state, pane, hit);
        }
        *dirty = true;
        return;
    }
    // 输出容器的滚动条：thumb 开始拖拽，轨道点击跳转。
    if let Some(rects) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::task_rects(inner, &target.lx))
        && let Some(page) = state.pane_anywhere(pane).and_then(|t| t.lx.task.as_ref())
        && page.tab == TaskTab::Processes
        && let Some(bar) = ui::main_content::lx::task::output_scrollbar(rects.output, page)
        && bar.track.contains((mouse.column, mouse.row).into())
    {
        match ui::widgets::scrollbar::thumb_grab_offset(&bar, mouse.row) {
            Some(grab) => state.task_scroll_drag = Some(grab),
            None => {
                let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, mouse.row);
                update::set_task_output_scroll(state, pane, offset);
            }
        }
        update::focus_pane(state, pane);
        *dirty = true;
        return;
    }
    // Agents 详情和 BTW 对话滚动条：与主历史同一套拖拽/轨道点击。
    if handle_task_agents_scrollbar_press(state, pane, inner, mouse.column, mouse.row, cache)
        || handle_task_btw_scrollbar_press(state, pane, inner, mouse.column, mouse.row, cache)
    {
        update::focus_pane(state, pane);
        *dirty = true;
        return;
    }
    if handle_task_list_scrollbar_press(state, pane, inner, mouse.column, mouse.row) {
        update::focus_pane(state, pane);
        *dirty = true;
        return;
    }
    // 后台任务面板：左上角 tab、右上角 `[esc]`、筛选胶囊与 Agents 列表行点击（置于滚动条之后）。
    let task_hit = state.pane_anywhere(pane).and_then(|target| {
        target.lx.task.as_ref()?;
        let rects = ui::main_content::lx::task_rects(inner, &target.lx)?;
        ui::main_content::lx::task::hit(rects.list, &target.lx, mouse.column, mouse.row)
    });
    if let Some(hit) = task_hit {
        match hit {
            ui::main_content::lx::task::TaskHit::Tab(index) => {
                update::select_task_tab(state, pane, index);
            }
            ui::main_content::lx::task::TaskHit::Close => {
                update::close_task_page(state, pane);
            }
            ui::main_content::lx::task::TaskHit::Agent(index) => {
                handle_task_row_click(state, pane, index, false);
            }
            ui::main_content::lx::task::TaskHit::Qa(index) => {
                handle_task_row_click(state, pane, index, true);
            }
            ui::main_content::lx::task::TaskHit::AgentFilter(filter) => {
                update::select_task_agent_filter(state, pane, filter);
            }
            ui::main_content::lx::task::TaskHit::Search => {
                handle_task_search_click(state, pane, inner, mouse.column, mouse.row);
            }
        }
        *dirty = true;
        return;
    }
    let paste_hit = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::paste_item_at(inner, &target.lx, mouse.column, mouse.row)
    });
    if let Some(index) = paste_hit {
        update::select_lx_paste(state, pane, index);
        *dirty = true;
        return;
    }
    let mention_tab_hit = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::mention_tab_at(inner, &target.lx, mouse.column, mouse.row)
    });
    if let Some(tab) = mention_tab_hit {
        update::switch_lx_mention_tab(state, pane, tab);
        *dirty = true;
        return;
    }
    let mention_hit = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::mention_item_at(inner, &target.lx, mouse.column, mouse.row)
    });
    if let Some(index) = mention_hit {
        update::select_lx_mention(state, pane, index);
        *dirty = true;
        return;
    }
    let refer_hit = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::refer_item_at(inner, &target.lx, mouse.column, mouse.row)
    });
    if let Some(index) = refer_hit {
        update::select_lx_refer(state, pane, index);
        *dirty = true;
        return;
    }
    // `/` 命令面板候选：优先于聊天控件、点选输入框与整页选区（与 `@` 面板一致）。
    let slash_hit = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::slash_item_at(inner, &target.lx, mouse.column, mouse.row)
    });
    if let Some(index) = slash_hit {
        update::select_lx_slash(state, pane, index);
        *dirty = true;
        return;
    }
    // 浮层边界拦截：点击落在任一已打开命令面板矩形内（如顶栏蓝色横线、
    // 空态占位行或滚动条列）时直接吞掉，绝不穿透到底层入口、历史或文本选区。
    if state.pane_anywhere(pane).is_some_and(|target| {
        ui::main_content::lx::command_panel_contains(inner, &target.lx, mouse.column, mouse.row)
    }) {
        *dirty = true;
        return;
    }
    if state.pane_mut_anywhere(pane).is_some_and(|target| {
        let canceled_text = target.lx.text_paste_cancel();
        let canceled_path = target.lx.paste_cancel();
        canceled_text || canceled_path
    }) {
        *dirty = true;
        return;
    }
    // 聊天控件：thinking 折叠头部 / 停止按钮 / 历史入口。
    let chat_hit = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::chat_hit(inner, &target.lx, mouse.column, mouse.row, cache.pane(pane))
    });
    if let Some(hit) = chat_hit {
        apply_lx_chat_hit(state, pane, inner, hit, cache);
        *dirty = true;
        return;
    }
    if handle_lx_input_scrollbar_press(state, pane, inner, mouse.column, mouse.row)
        || handle_lx_history_scrollbar_press(state, pane, inner, mouse.column, mouse.row, cache)
    {
        // 滚动条按下：只移动视口，不清除已有主内容区选区。
        *dirty = true;
        return;
    }
    let site = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::ai_web_site_hit(inner, &target.lx, mouse.column, mouse.row)
    });
    let entry = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::entry_hit(inner, &target.lx, mouse.column, mouse.row)
    });
    update::clear_selection(state);
    if let Some(site) = site {
        update::set_ai_web_site(state, pane, site);
        update::clear_lx_selection(state);
    } else if let Some(entry) = entry {
        update::set_lx_entry(state, pane, entry);
        update::clear_lx_selection(state);
    } else if let Some((visual_row, col)) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::input_geom(inner, &target.lx))
        .filter(|geom| geom.contains(mouse.column, mouse.row))
        .map(|geom| geom.cell(mouse.column, mouse.row))
    {
        let clicks = crate::app::selection::next_click_count(
            &mut state.selection_click,
            mouse.column,
            mouse.row,
            Instant::now(),
        );
        update::begin_lx_input_selection_with_clicks(state, pane, visual_row, col, clicks);
    } else if let Some((content_row, content_col, target)) =
        state.pane_anywhere(pane).and_then(|target| {
            ui::main_content::lx::press_content_hit(
                inner,
                &target.lx,
                mouse.column,
                mouse.row,
                mouse.modifiers,
                cache.pane(pane),
            )
        })
    {
        // 主消息列表与后台管理主内容区：可折叠卡片保持单击折叠/长按拖选语义；
        // 普通正文行以及可折叠卡片内已渲染的 Markdown 表格行支持双击选词、三击选整格。
        //
        // **顺序不能反**：`begin_lx_selection*` 内部先 `clear_selection`（会清空
        // `lx_card_press`），必须先开始选区再写 `lx_card_press`。
        if let Some(card) = target {
            state.selection_click = None;
            if mouse.modifiers.contains(KeyModifiers::CONTROL) && is_immediate_open_card(&card) {
                apply_immediate_lx_open_card(state, pane, card);
            } else {
                update::begin_lx_selection(state, pane, content_row, content_col);
                state.lx_card_press = Some((pane, card));
            }
        } else {
            let clicks = crate::app::selection::next_click_count(
                &mut state.selection_click,
                mouse.column,
                mouse.row,
                Instant::now(),
            );
            let unit = crate::app::selection::SelectionUnit::from_clicks(clicks);
            let range = state.pane_anywhere(pane).and_then(|pane_target| {
                ui::main_content::lx::unit_cell_range_opt(
                    inner,
                    &pane_target.lx,
                    (content_row, content_col),
                    unit,
                    cache.pane(pane),
                )
            });
            if let Some((start, end)) = range {
                update::begin_lx_selection_range(state, pane, start, end, unit);
            } else {
                // 装饰行/左侧 `▎` 栏等不可选位置：不要留下折叠的 Word/Line
                // 选区，否则 Ctrl/Cmd+C 会被空选区吞掉。
                update::clear_selection(state);
            }
        }
    }
    *dirty = true;
}

/// 处理 macOS 终端将 `Ctrl + 左键点击` 映射为 `MouseButton::Right + CONTROL` 时的 `lx` 超链接/生图打开。
///
/// 命中并消费时返回 `true`（不弹出右键菜单）。
pub(super) fn handle_lx_ctrl_right_click(
    mouse: &MouseEvent,
    state: &mut AppState,
    geometry: &Geometry,
    config: &Config,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) -> bool {
    let Geometry {
        screen,
        rects,
        view,
    } = geometry;
    if state.overlay.is_some() || !mouse.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    // 排除全局 Toast 与窗格内 lx Toast 覆盖区域，防止浮层穿透。
    if ui::toast::rect(state, view, rects, *screen, config)
        .is_some_and(|toast| toast.contains((mouse.column, mouse.row).into()))
        || lx_toast_at(rects, state, mouse.column, mouse.row).is_some()
    {
        return false;
    }
    let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) else {
        return false;
    };
    let inner = pane_content_inner(state, view, pane, inner);
    if inner.width == 0 || inner.height == 0 {
        return false;
    }
    // 排除已打开的命令面板与 Ask 面板区域，防止浮层穿透。
    let Some(card) = state
        .pane_anywhere(pane)
        .filter(|target| target.kind == PaneKind::Terminal && target.view == PaneView::Lx)
        .filter(|target| {
            let in_cmd = ui::main_content::lx::command_panel_contains(
                inner,
                &target.lx,
                mouse.column,
                mouse.row,
            );
            let in_ask = target.lx.task.is_none()
                && ui::main_content::lx::ask_rect(inner, &target.lx)
                    .is_some_and(|rect| rect.contains((mouse.column, mouse.row).into()));
            !in_cmd && !in_ask
        })
        .and_then(|target| {
            ui::main_content::lx::press_content_hit(
                inner,
                &target.lx,
                mouse.column,
                mouse.row,
                mouse.modifiers,
                cache.pane(pane),
            )
        })
        .and_then(|(_, _, card)| card)
        .filter(is_immediate_open_card)
    else {
        return false;
    };
    update::focus_pane(state, pane);
    apply_immediate_lx_open_card(state, pane, card);
    *dirty = true;
    true
}

fn is_immediate_open_card(card: &CardTarget) -> bool {
    matches!(
        card,
        CardTarget::OpenUrl { .. }
            | CardTarget::Attachment { .. }
            | CardTarget::DownloadOriginalImage { .. }
    )
}

fn apply_immediate_lx_open_card(state: &mut AppState, pane: PaneId, card: CardTarget) {
    let agents_tab = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .is_some_and(|page| page.tab == TaskTab::Agents);
    if agents_tab {
        update::toggle_agent_card(state, pane, card);
    } else {
        update::toggle_lx_card(state, pane, card);
    }
}

/// 后台任务列表行单击：选中并预览；同一行 400ms 内再次单击进入编辑态。
fn handle_task_row_click(state: &mut AppState, pane: PaneId, index: usize, btw: bool) {
    let now = Instant::now();
    let double = state
        .task_list_click
        .is_some_and(|(last_pane, last_index, at)| {
            last_pane == pane
                && last_index == index
                && now.duration_since(at) <= std::time::Duration::from_millis(400)
        });
    state.task_list_click = Some((pane, index, now));
    if btw {
        if double {
            // 双击窗口内可能用 ↑/↓ 改过高亮；编辑必须落在点击的同一行。
            update::select_btw_qa(state, pane, index);
            update::enter_btw_edit(state, pane);
        } else {
            update::select_btw_qa(state, pane, index);
        }
    } else if double {
        update::select_task_agent(state, pane, index);
        update::enter_task_agent_edit(state, pane);
    } else {
        update::select_task_agent(state, pane, index);
    }
}

/// 点击搜索行：按列定位文本光标。
fn handle_task_search_click(state: &mut AppState, pane: PaneId, area: Rect, column: u16, row: u16) {
    let Some((rects, btw)) = state.pane_anywhere(pane).and_then(|target| {
        let rects = ui::main_content::lx::task_rects(area, &target.lx)?;
        Some((rects, target.lx.is_btw_tab()))
    }) else {
        return;
    };
    let search = ui::main_content::lx::task::search_rect(rects.list);
    if !search.contains((column, row).into()) {
        return;
    }
    let text_x = search.x.saturating_add(2);
    if column < text_x {
        return;
    }
    let width = usize::from(search.right().saturating_sub(text_x));
    let (text, cursor) = state
        .pane_anywhere(pane)
        .and_then(|target| {
            if btw {
                let input = &target.lx.btw.search;
                Some((input.text().to_string(), input.cursor()))
            } else {
                let input = &target.lx.task.as_ref()?.agents_search;
                Some((input.text().to_string(), input.cursor()))
            }
        })
        .unwrap_or_default();
    let index = crate::ui::widgets::input::char_index_at_col(
        &text,
        cursor,
        width,
        column.saturating_sub(text_x),
    );
    if let Some(target) = state.pane_mut_anywhere(pane) {
        if btw {
            target.lx.btw.search.set_cursor(index);
        } else if let Some(page) = target.lx.task.as_mut() {
            page.agents_search.set_cursor(index);
        }
    }
    update::clear_selection(state);
    state.task_list_click = None;
}
