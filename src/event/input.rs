//! 键盘与粘贴终端事件分派：单行重命名拦截、路由分派与剪贴板/终端透传。

use std::collections::HashMap;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneView};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::input::{self, Routed};
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

    // 页重命名输入优先于编辑器：除 Ctrl+Q 外的按键按单行输入语义消费。
    let quit = key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL);
    if !quit
        && state.overlay.is_none()
        && state.prompt_focused()
        && update::page_rename_active(state)
    {
        if let Some(overlay_key) = input::page_rename_key(key) {
            update::apply_page_rename_key(state, overlay_key);
        }
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
            let was_confirm = matches!(
                state.overlay,
                Some(Overlay::ConfirmClose(_) | Overlay::ConfirmSyncWorkspaceCwd(_))
            );
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
            if was_confirm || was_menu || was_worktree || was_new_ws {
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
            if update::prompt_tree_mode(state) {
                // 树视图 Ctrl+C 与模态输入框一致：清空搜索过滤。
                update::clear_prompt_search(state);
                *dirty = true;
            } else if let Some(text) = update::prompt_copy_text(state) {
                copy_with_toast(state, &text);
                *dirty = true;
            }
        }
        Routed::CopyTemplateBlock => {
            if let Some(text) = update::copy_template_block_at_cursor(state) {
                copy_with_toast(state, &text);
                *dirty = true;
            }
        }
        Routed::Pane(bytes) => {
            let id = state.active_tab().layout.focus();
            write_to_pane(sessions, id, &bytes);
            update::reset_pane_scroll(state, id);
            *dirty = true;
        }
        Routed::Lx(command) => {
            if command == crate::app::actions::LxInput::Clear
                && copy_lx_selection(state, &geometry.rects, cache)
            {
                *dirty = true;
            } else {
                update::apply_lx_input(state, command);
                *dirty = true;
            }
        }
        Routed::Todo(code) => {
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
    // 引用文件夹输入/搜索弹窗：粘贴进入对应单行输入框，不穿透到底层。
    if let Some(overlay) = state.overlay.as_mut() {
        match overlay {
            Overlay::ReferencedFolderInput(input) => {
                input.input.insert_str(&text);
                *dirty = true;
                return;
            }
            Overlay::ReferencedFolderSearch(search) => {
                if search.query.insert_str(&text) {
                    update::refresh_referenced_search(state);
                    *dirty = true;
                }
                return;
            }
            _ => {}
        }
    }
    // 浮层打开时粘贴吞掉，不穿透到底层终端（与键盘过滤一致）。
    if state.overlay.is_some() {
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

/// 复制焦点 lx 窗格的主内容区选区文本并弹出页内 Toast；选区保留，无选区返回 false。
fn copy_lx_selection(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut ui::RenderCache,
) -> bool {
    let pane = state.active_tab().layout.focus();
    // `/resume` 历史面板打开时 Ctrl/Cmd+C 归会话 ID 复制，不走主内容区选区复制。
    if state.pane_anywhere(pane).is_some_and(|target| {
        matches!(
            target.lx.slash,
            Some(crate::app::state::SlashPanel::History(_))
        )
    }) {
        return false;
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
    if !text.trim().is_empty() {
        let now = std::time::Instant::now();
        if crate::platform::write_clipboard(&text) {
            update::show_lx_toast(
                state,
                pane,
                Toast::new(ToastKind::Default, ui::text::TOAST_COPIED, Some(pane), now)
                    .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
            );
        } else {
            tracing::warn!("clipboard write failed");
            update::show_lx_toast(
                state,
                pane,
                Toast::new(
                    ToastKind::Danger,
                    ui::text::TOAST_COPY_FAILED,
                    Some(pane),
                    now,
                )
                .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
            );
        }
    }
    true
}
