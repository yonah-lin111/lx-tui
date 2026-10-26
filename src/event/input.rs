//! 键盘与粘贴终端事件分派：单行重命名拦截、路由分派与剪贴板/终端透传。

use std::collections::HashMap;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneView};
use crate::app::update;
use crate::input::{self, GlobalEdit, Routed};
use crate::layout::PaneId;
use crate::pty::PtySession;

use super::mouse::hit::copy_with_toast;
use super::paste;
use super::session::{Geometry, ensure_workspace_visible, reveal_active_tab, write_to_pane};

/// 处理终端按键事件。
pub(crate) fn handle_key_event(
    key: KeyEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    cache: &mut crate::ui::RenderCache,
    dirty: &mut bool,
) {
    if key.kind == KeyEventKind::Release {
        return;
    }
    let view = &geometry.view;

    // 全局编辑快捷键先于焦点分流：有选区的复制、可编辑目标的剪切/粘贴/全选。
    // 返回 false 表示应继续按焦点路由（终端 PTY 的 Ctrl+C / Ctrl+V / Ctrl+A）。
    if let Some(edit) = input::global_edit_key(key)
        && handle_global_edit(edit, key, state, geometry, cache, dirty)
    {
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
        Routed::Pane(bytes) => {
            let id = state.active_tab().layout.focus();
            write_to_pane(sessions, id, &bytes);
            update::reset_pane_scroll(state, id);
            *dirty = true;
        }
        Routed::Lx(command) => {
            update::apply_lx_input(state, command);
            *dirty = true;
        }
        Routed::Todo(code) => {
            if update::apply_todo_key(state, code) {
                *dirty = true;
            }
        }
    }
}

/// 全局编辑快捷键分派；返回 true 表示已消费，false 交给焦点路由（终端 PTY）。
fn handle_global_edit(
    edit: GlobalEdit,
    key: KeyEvent,
    state: &mut AppState,
    geometry: &Geometry,
    cache: &mut crate::ui::RenderCache,
    dirty: &mut bool,
) -> bool {
    let terminal_pane = state.overlay.is_none()
        && !update::page_rename_active(state)
        && state.panes_focused()
        && state
            .active_pane()
            .is_some_and(|pane| pane.view != PaneView::Lx);
    let control_only = key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::SUPER);
    match edit {
        GlobalEdit::SelectAll => {
            if terminal_pane {
                return false;
            }
            if update::select_all_focused(state) {
                *dirty = true;
            }
            true
        }
        GlobalEdit::Copy => {
            if let Some(selection) = update::copyable_selection(state, &geometry.rects, cache) {
                copy_with_toast(state, &selection.text);
                *dirty = true;
                return true;
            }
            if terminal_pane {
                // 无本地选区：Ctrl+C 原样进 PTY；Cmd+C 没有可复制内容，吞掉。
                return !control_only;
            }
            // 引用文件夹目录弹窗的 Ctrl+C 是显式“复制引用”，交给浮层路由。
            if matches!(state.overlay, Some(Overlay::ReferencedFolderMenu(_))) {
                return false;
            }
            true
        }
        GlobalEdit::Cut => {
            if terminal_pane {
                return false;
            }
            if update::request_cut(state) {
                *dirty = true;
            }
            true
        }
        GlobalEdit::Paste => {
            if terminal_pane && control_only {
                // Ctrl+V 是 shell/readline 的 literal-next，不抢。
                return false;
            }
            // 只登记请求；事件循环在阻塞线程读取剪贴板并校验目标快照后回投。
            if update::request_paste(state) {
                *dirty = true;
            }
            true
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
    // 页重命名输入：粘贴进入行内单行输入，不穿透到底层。
    if state.overlay.is_none() && state.prompt_focused() && update::page_rename_active(state) {
        if let Some(input) = update::active_text_input_mut(state)
            && input.insert_str(&text)
        {
            *dirty = true;
        }
        return;
    }
    // 浮层内的单行输入统一走同一个粘贴入口；浮层打开时绝不穿透到底层。
    if state.overlay.is_some() {
        if matches!(
            state.overlay,
            Some(Overlay::ReferencedFolderSearch(_) | Overlay::WorktreeOpen(_))
        ) {
            // 搜索框插入后需要重新过滤，worktree 查询框需要重新钳制高亮。
            if let Some(input) = update::active_text_input_mut(state)
                && input.insert_str(&text)
            {
                if matches!(state.overlay, Some(Overlay::ReferencedFolderSearch(_))) {
                    update::refresh_referenced_search(state);
                } else {
                    update::normalize_worktree_selection(state);
                }
                *dirty = true;
            }
            return;
        }
        if let Some(input) = update::active_text_input_mut(state)
            && input.insert_str(&text)
        {
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
