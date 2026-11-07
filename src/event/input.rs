//! 键盘与粘贴终端事件分派：单行重命名拦截、路由分派与剪贴板/终端透传。

use std::collections::HashMap;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::actions::OverlayKey;
use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneView};
use crate::app::update;
use crate::input::{self, Routed, TodoKey};
use crate::layout::{self, PaneId};
use crate::pty::PtySession;
use crate::ui;

use super::mouse::hit::copy_with_toast;
use super::paste;
use super::session::{Geometry, ensure_workspace_visible, reveal_active_tab, write_to_pane};

/// 处理终端按键事件。
pub(crate) fn handle_key_event(
    key: KeyEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    let view = &geometry.view;
    if key.kind == KeyEventKind::Release {
        return;
    }

    // 对齐 OpenCode (priority 101)：裸 Esc 最优先取消当前可见文字选区并消费按键，
    // 绝不穿透到终端 PTY、模态浮层关闭、自动补全面板关闭、退出编辑态或双 Esc 中断。
    if key.code == KeyCode::Esc && key.modifiers.is_empty() && update::escape_clear_selection(state)
    {
        *dirty = true;
        return;
    }

    let mode = state
        .active_pane()
        .map(|pane| pane.terminal.mode())
        .unwrap_or_else(TermMode::empty);
    let overlay = state.overlay.as_ref().map(Overlay::kind);
    let pane_lx = state
        .active_pane()
        .is_some_and(|pane| pane.view == PaneView::Lx);
    // 终端窗格有保留选区时，Ctrl/Cmd+C 优先复制选区并保留高亮，不向 PTY 发送 ^C。
    if overlay.is_none()
        && state.focus() == crate::app::state::FocusedContainer::Panes
        && !pane_lx
        && is_copy_chord(key)
        && copy_terminal_selection(state)
    {
        *dirty = true;
        return;
    }
    // 侧栏工作区历史列表处于打开态且侧栏持焦点时，处理搜索框输入与列表导航。
    if overlay.is_none()
        && state.sidebar_focused()
        && !state.sidebar_collapsed
        && update::workspace_history_mode(state)
        && !(key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL))
    {
        if let Some(history_key) = input::sidebar_history_key(key) {
            let visible = ui::workspace_history_visible_rows(view, state).unwrap_or(1);
            match history_key {
                OverlayKey::Clear | OverlayKey::Copy => {
                    if let Some(text) = state.workspace_history.search.selected_text() {
                        if !text.trim().is_empty() {
                            copy_with_toast(state, &text, None);
                        }
                        *dirty = true;
                    } else if update::clear_workspace_history_search(state, visible) {
                        *dirty = true;
                    }
                }
                OverlayKey::Cut => {
                    if let Some(text) = update::cut_workspace_history_search(state, visible) {
                        if !text.is_empty() {
                            copy_with_toast(state, &text, None);
                        }
                        *dirty = true;
                    }
                }
                OverlayKey::Enter => {
                    if update::apply_workspace_history_key(state, history_key, visible) {
                        ensure_workspace_visible(state, view);
                        reveal_active_tab(state, view);
                        *dirty = true;
                    }
                }
                _ => {
                    if update::apply_workspace_history_key(state, history_key, visible) {
                        *dirty = true;
                    }
                }
            }
        }
        return;
    }
    let Some(routed) = input::route(key, mode, state.focus(), overlay, pane_lx) else {
        return;
    };
    if update::prompt_recent_open(state)
        && view.prompt.height > 0
        && !ui::prompt_frame::prompt_recent_visible(view.prompt, state)
    {
        // 面板过矮无法展示 recent 抽屉时自动收起，避免不可见抽屉吞掉键盘输入。
        update::close_prompt_recent(state);
    }
    // recent 抽屉展开时，prompt 的保存/复制/剪切快捷键同样由抽屉吞掉，
    // 避免“键盘已归抽屉”却仍改到后方编辑器或写系统剪贴板。
    if ui::prompt_frame::prompt_recent_visible(view.prompt, state)
        && matches!(
            &routed,
            Routed::SavePrompt | Routed::Copy | Routed::Cut | Routed::CopyTemplateBlock
        )
    {
        return;
    }

    match routed {
        Routed::Action(action) => {
            update::apply(action, state);
            *dirty = true;
        }
        Routed::Editor(command) => {
            if ui::prompt_frame::prompt_recent_visible(view.prompt, state) {
                // 抽屉展开时键盘归 recent：↑/↓ 选择、Enter 打开、Esc/Ctrl+R 收起，
                // 其余键由 update 层的兜底吞掉，不改上方视图。
                if update::apply_prompt_recent_key(state, &command, view.prompt) {
                    *dirty = true;
                }
                return;
            }
            update::apply_editor(state, command);
            if update::prompt_recent_open(state)
                && view.prompt.height > 0
                && !ui::prompt_frame::prompt_recent_visible(view.prompt, state)
            {
                update::close_prompt_recent(state);
            }
            *dirty = true;
        }
        Routed::Overlay(key) => {
            if matches!(key, OverlayKey::Clear | OverlayKey::Copy)
                && let Some(text) = overlay_selected_text(state)
            {
                if !text.trim().is_empty() {
                    copy_with_toast(state, &text, None);
                }
                *dirty = true;
                return;
            }
            if key == OverlayKey::Cut {
                if let Some(text) = update::cut_overlay_selected_text(state)
                    && !text.is_empty()
                {
                    copy_with_toast(state, &text, None);
                }
                *dirty = true;
                return;
            }
            // 确认删除浮层：Up/Down 按真实视口高度滚动待删列表，小屏下也能触达尾部。
            if matches!(key, OverlayKey::Up | OverlayKey::Down)
                && let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref()
            {
                let visible_rows =
                    ui::overlay::confirm_close_layout(state, geometry.screen, confirm).map_or(
                        crate::app::overlay::CONFIRM_DELETE_LIST_MAX_ROWS,
                        |layout| layout.visible_rows,
                    );
                update::scroll_confirm_close_with_rows(
                    state,
                    if key == OverlayKey::Up { -1 } else { 1 },
                    visible_rows,
                );
                *dirty = true;
                return;
            }
            let was_menu = matches!(state.overlay, Some(Overlay::Menu(_)));
            let was_worktree = matches!(state.overlay, Some(Overlay::WorktreeOpen(_)));
            let was_new_ws = matches!(
                state.overlay,
                Some(Overlay::NewWorkspace(_) | Overlay::TrustWorkspace(_))
            );
            if let Some((pane, bytes)) = update::apply_overlay_key(state, key) {
                write_to_pane(sessions, pane, &bytes);
                update::reset_pane_scroll(state, pane);
            }
            let menu_stays_open = matches!(state.overlay, Some(Overlay::Menu(_)));
            if key == OverlayKey::Enter
                && ((was_menu && !menu_stays_open) || was_worktree || was_new_ws)
            {
                ensure_workspace_visible(state, view);
                reveal_active_tab(state, view);
            }
            *dirty = true;
        }
        Routed::SavePrompt => {
            update::save_prompt_shortcut(state);
            *dirty = true;
        }
        Routed::Copy => {
            let anchor = Some(state.prompt.id());
            if update::prompt_tree_mode(state) {
                // 树视图 Ctrl+C：有搜索框选区时复制选区，否则清空搜索过滤。
                if let Some(text) = update::prompt_search_selection(state) {
                    if !text.is_empty() {
                        copy_with_toast(state, &text, anchor);
                    }
                } else {
                    update::clear_prompt_search(state);
                }
                *dirty = true;
            } else if update::prompt_pages_mode(state) {
                if let Some(text) = update::page_search_selection(state) {
                    if !text.is_empty() {
                        copy_with_toast(state, &text, anchor);
                    }
                } else {
                    update::clear_page_search(state);
                }
                *dirty = true;
            } else if let Some(text) = update::prompt_copy_text(state) {
                copy_with_toast(state, &text, anchor);
                *dirty = true;
            }
        }
        Routed::Cut => {
            let anchor = Some(state.prompt.id());
            if update::prompt_tree_mode(state) {
                if let Some(text) = update::cut_prompt_search(state)
                    && !text.is_empty()
                {
                    copy_with_toast(state, &text, anchor);
                }
            } else if update::prompt_pages_mode(state) {
                if let Some(text) = update::cut_page_search(state)
                    && !text.is_empty()
                {
                    copy_with_toast(state, &text, anchor);
                }
            } else if let Some((start, end)) = update::prompt_selection_bounds(state)
                && let Some(text) = state.prompt.text().get(start..end).map(str::to_string)
                && state.prompt.replace_range(start, end, "")
            {
                update::clear_selection(state);
                if !text.is_empty() {
                    copy_with_toast(state, &text, anchor);
                }
            }
            *dirty = true;
        }
        Routed::CopyTemplateBlock => {
            if let Some(text) = update::copy_template_block_at_cursor(state) {
                let anchor = Some(state.prompt.id());
                copy_with_toast(state, &text, anchor);
                *dirty = true;
            }
        }
        Routed::Pane(bytes) => {
            let id = state.active_tab().layout.focus();
            update::clear_terminal_selection(state);
            write_to_pane(sessions, id, &bytes);
            update::reset_pane_scroll(state, id);
            *dirty = true;
        }
        Routed::Lx(command) => {
            if command == crate::app::actions::LxInput::Clear
                && copy_lx_selection(state, &geometry.rects, cache)
            {
                *dirty = true;
            } else if command == crate::app::actions::LxInput::Cut {
                if cut_lx_selection(state) {
                    *dirty = true;
                }
            } else {
                update::apply_lx_input(state, command);
                *dirty = true;
            }
        }
        Routed::Todo(code) => {
            if code == TodoKey::Clear
                && let Some(text) = state
                    .todo
                    .edit_input_mut()
                    .and_then(|inp| inp.selected_text())
            {
                if !text.trim().is_empty() {
                    let anchor = Some(state.prompt.id());
                    copy_with_toast(state, &text, anchor);
                }
                *dirty = true;
                return;
            }
            if update::apply_todo_key(state, code) {
                *dirty = true;
            }
        }
    }
}

/// 处理终端粘贴事件。
pub(crate) fn handle_paste_event(
    text: String,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    dirty: &mut bool,
) {
    // 浮层打开时粘贴只进入当前浮层的单行输入框，绝不穿透到底层终端。
    if state.overlay.is_some() {
        if update::apply_overlay_paste(state, &text) {
            *dirty = true;
        }
        return;
    }
    // 侧栏：仅在工作区历史列表模式下将粘贴写入搜索框，活跃列表模式下吞掉粘贴。
    if state.sidebar_focused() {
        if update::workspace_history_mode(state)
            && update::apply_workspace_history_paste(state, &text, 10)
        {
            *dirty = true;
        }
        return;
    }
    if state.todo_focused() {
        // 编辑态写入输入框；列表态吞掉，绝不写隐藏终端。
        if update::apply_todo_paste(state, &text) {
            *dirty = true;
        }
        return;
    }
    if state.prompt_focused() {
        if update::prompt_recent_open(state) {
            // 键盘/粘贴都归 recent：抽屉展开时不改动后方编辑器。
            return;
        }
        paste::handle_terminal_paste(state, text);
        *dirty = true;
        return;
    }
    // lx 视图：粘贴进入 lx 输入框（路径识别 + 参照面板，或纯文本），不写入隐藏终端。
    if state
        .active_pane()
        .is_some_and(|pane| pane.view == PaneView::Lx)
    {
        // `ask` 面板激活：编辑态写入手写缓冲，列表态整段吞掉——绝不写隐藏草稿，
        // 也不触发路径粘贴面板。
        let id = state.active_tab().layout.focus();
        if state
            .pane_anywhere(id)
            .is_some_and(|pane| pane.lx.task.is_some())
        {
            if update::apply_task_paste(state, id, &text) {
                *dirty = true;
            }
            return;
        }
        if state
            .pane_anywhere(id)
            .is_some_and(|pane| pane.lx.ask.is_some())
        {
            if update::apply_ask_paste(state, id, &text) {
                *dirty = true;
            }
            return;
        }
        if paste::handle_lx_paste(state, text) {
            *dirty = true;
        }
        return;
    }
    let id = state.active_tab().layout.focus();
    update::clear_terminal_selection(state);
    let bracketed = state
        .active_pane()
        .is_some_and(|pane| pane.terminal.mode().contains(TermMode::BRACKETED_PASTE));
    let payload = if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    };
    write_to_pane(sessions, id, &payload);
    update::reset_pane_scroll(state, id);
    *dirty = true;
}

/// 是否为 Ctrl+C / Cmd+C 复制快捷键（不含 Alt / Shift）。
fn is_copy_chord(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c' | 'C'))
        && (key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::SUPER))
        && !key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
}

/// 复制焦点终端窗格的选区文本并在该窗格顶部居中弹出 Toast；选区保留，无选区返回 false。
fn copy_terminal_selection(state: &mut AppState) -> bool {
    let pane = state.active_tab().layout.focus();
    let Some(text) = update::terminal_selection_text(state, pane) else {
        return false;
    };
    copy_with_toast(state, &text, Some(pane));
    true
}

/// 复制焦点 lx 窗格的底部输入框选区或主内容区选区文本并在该 lx 窗格内弹出 Toast；
/// 选区保留，无选区返回 false。
fn copy_lx_selection(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut ui::RenderCache,
) -> bool {
    let pane = state.active_tab().layout.focus();
    // `/resume` 历史面板打开时 Ctrl/Cmd+C 归会话 ID 复制，不走选区复制。
    if state.pane_anywhere(pane).is_some_and(|target| {
        matches!(
            target.lx.slash,
            Some(crate::app::state::SlashPanel::History(_))
        )
    }) {
        return false;
    }
    let request = state
        .pane_anywhere(pane)
        .filter(|target| target.view == PaneView::Lx);
    let ask_blocked =
        request.is_some_and(|target| target.lx.ask.as_ref().is_some_and(|ask| !ask.editing));
    if ask_blocked {
        // Ask 列表态没有可见输入框：绝不复制隐藏的底层草稿。
        return false;
    }
    // 任务页打开时优先复制内联追问框选区；未打开时按 Ask 手写框 / 展开芯片预览 / 底部输入框顺序复制。
    if let Some(text) = request.and_then(|target| {
        if let Some(task) = target.lx.task.as_ref() {
            task.agent_reply
                .as_ref()
                .and_then(|reply| reply.selected_text())
        } else if let Some(ask) = target.lx.ask.as_ref()
            && ask.editing
        {
            ask.editor().and_then(|ed| ed.selected_text())
        } else if let Some((start, end)) = target.lx.preview_selection() {
            Some(target.lx.projection().copy_display_range(start..end))
        } else {
            target.lx.selected_text()
        }
    }) {
        if !text.trim().is_empty() {
            copy_with_toast(state, &text, Some(pane));
        }
        return true;
    }
    let Some(selection) = state.lx_selection.filter(|sel| sel.pane() == pane) else {
        return false;
    };
    if selection.range().is_none() {
        return false;
    }
    let area = rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| layout::pane_inner_rect(*rect));
    let Some((area, target)) = area.zip(state.pane_anywhere(pane)) else {
        return false;
    };
    if target.view != PaneView::Lx {
        return false;
    }
    let text = ui::main_content::lx::copy_text(
        area,
        &target.lx,
        (selection.anchor(), selection.cursor()),
        cache.pane(pane),
    );
    if !text.is_empty() {
        copy_with_toast(state, &text, Some(pane));
    }
    true
}

/// 剪切焦点 lx 窗格底部输入框（或 ask 手写框）的选区；无选区返回 false。
fn cut_lx_selection(state: &mut AppState) -> bool {
    let pane = state.active_tab().layout.focus();
    // `/resume` 历史面板打开时 Ctrl/Cmd+X 无文本可剪。
    if state.pane_anywhere(pane).is_some_and(|target| {
        matches!(
            target.lx.slash,
            Some(crate::app::state::SlashPanel::History(_))
        )
    }) {
        return false;
    }
    if !state
        .pane_anywhere(pane)
        .is_some_and(|target| target.view == PaneView::Lx)
    {
        return false;
    }
    let (text, from_sub_editor) = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        if let Some(task) = target.lx.task.as_mut() {
            (
                task.agent_reply
                    .as_mut()
                    .and_then(|reply| reply.cut_selected()),
                true,
            )
        } else if target.lx.ask.as_ref().is_some_and(|ask| ask.editing) {
            (
                target
                    .lx
                    .ask
                    .as_mut()
                    .and_then(|ask| ask.editor_mut())
                    .and_then(|editor| editor.cut_selected()),
                true,
            )
        } else if target.lx.ask.is_some() {
            // ask 面板非编辑态：隐藏的草稿不是用户当前操作对象，禁止剪切。
            return false;
        } else {
            let text = target.lx.cut_selected();
            if text.is_some() {
                target.lx.end_history_browse();
            }
            (text, false)
        }
    };
    let Some(text) = text else {
        return false;
    };
    if !from_sub_editor {
        update::refresh_slash_panel(state, pane);
    }
    if !text.is_empty() {
        copy_with_toast(state, &text, Some(pane));
    }
    true
}

/// 读取当前浮层单行输入框中被选中的文本。
fn overlay_selected_text(state: &AppState) -> Option<String> {
    state.overlay.as_ref()?.input()?.selected_text()
}
