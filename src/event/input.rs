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
    let Some(routed) = input::route(key, mode, state.focus(), overlay, pane_lx) else {
        return;
    };

    match routed {
        Routed::Action(action) => {
            update::apply(action, state);
            *dirty = true;
        }
        Routed::Editor(command) => {
            update::apply_editor(state, command);
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
    // 侧栏没有键盘交互（`input::route` 把它的按键全吞掉），粘贴必须同样吞掉：
    // 否则焦点在侧栏时粘贴会落进别的容器的输入框。
    if state.sidebar_focused() {
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
    let task_open = request.is_some_and(|target| target.lx.task.is_some());
    let ask_blocked =
        request.is_some_and(|target| target.lx.ask.as_ref().is_some_and(|ask| !ask.editing));
    if ask_blocked {
        // Ask 列表态没有可见输入框：绝不复制隐藏的底层草稿。
        return false;
    }
    // 任务页占用输入框矩形时，底层草稿不可见：跳过输入框选区，仅保留主内容区复制。
    if !task_open
        && let Some(text) = request.and_then(|target| {
            if let Some(ask) = target.lx.ask.as_ref()
                && ask.editing
            {
                ask.editor().and_then(|ed| ed.selected_text())
            } else if let Some((start, end)) = target.lx.preview_selection() {
                Some(target.lx.projection().copy_display_range(start..end))
            } else {
                target.lx.selected_text()
            }
        })
    {
        if !text.trim().is_empty() {
            copy_with_toast(state, &text, Some(pane));
        }
        return true;
    }
    let Some(selection) = state.lx_selection.filter(|sel| sel.pane() == pane) else {
        return false;
    };
    let Some(range) = selection.range() else {
        return false;
    };
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
    let text = ui::main_content::lx::copy_text(area, &target.lx, range, cache.pane(pane));
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
        .is_some_and(|target| target.view == PaneView::Lx && target.lx.task.is_none())
    {
        return false;
    }
    let (text, from_ask) = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        if target.lx.ask.as_ref().is_some_and(|ask| ask.editing) {
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
    if !from_ask {
        update::refresh_slash_panel(state, pane);
    }
    if !text.is_empty() {
        copy_with_toast(state, &text, Some(pane));
    }
    true
}

/// 读取当前浮层单行输入框中被选中的文本。
fn overlay_selected_text(state: &AppState) -> Option<String> {
    match state.overlay.as_ref()? {
        Overlay::Rename(r) => r.input.selected_text(),
        Overlay::NewWorkspace(ws) => ws.input.selected_text(),
        Overlay::NewNode(n) => n.name.selected_text(),
        Overlay::SaveDraft(d) => d.name.selected_text(),
        Overlay::WorktreeOpen(w) => w.query.selected_text(),
        Overlay::ReferencedFolderInput(i) => i.input.selected_text(),
        Overlay::ReferencedFolderSearch(s) => s.query.selected_text(),
        Overlay::ResumeSessionEdit(r) => r.input.selected_text(),
        _ => None,
    }
}
