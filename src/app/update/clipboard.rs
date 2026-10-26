//! 异步剪贴板事务：粘贴目标快照、读取回投、可确认写入与剪切删除。
//!
//! 事件循环只在后台线程读写剪贴板；结果回投后重新校验目标实例与修订号。

use crate::app::selection::SelectionSource;
use crate::app::state::{AppState, ClipboardRequest, PaneView};
use crate::layout::PaneId;

use super::selection::{clear_selection, prompt_selection_bounds, prompt_selection_text};

/// 当前粘贴目标快照；按键回退读取剪贴板前捕获，回投时再校验。
pub fn paste_target(state: &AppState) -> crate::app::state::PasteTarget {
    let page_rename = state.overlay.is_none() && super::page_rename_active(state);
    let overlay = state
        .overlay
        .as_ref()
        .map(crate::app::overlay::Overlay::kind);
    let pane = state
        .active_pane()
        .map(|_| state.active_tab().layout.focus());
    let revision = if page_rename {
        super::dialog::active_text_input(state).map_or(0, |input| input.revision())
    } else if overlay.is_some() {
        super::dialog::active_text_input(state).map_or(0, |input| input.revision())
    } else {
        match state.focus() {
            crate::app::state::FocusedContainer::Prompt => state.prompt.revision(),
            crate::app::state::FocusedContainer::Todo => {
                state.todo.editing().map_or(0, |edit| edit.input.revision())
            }
            crate::app::state::FocusedContainer::Panes => {
                let Some(pane) = state.active_pane() else {
                    return crate::app::state::PasteTarget {
                        focus: state.focus(),
                        pane: None,
                        overlay,
                        page_rename,
                        revision: 0,
                    };
                };
                if pane.view == PaneView::Lx {
                    if let Some(ask) = pane.lx.ask.as_ref()
                        && ask.editing
                        && let Some(editor) = ask.custom.get(ask.current)
                    {
                        editor.revision()
                    } else {
                        pane.lx.revision()
                    }
                } else {
                    0
                }
            }
            crate::app::state::FocusedContainer::Sidebar => 0,
        }
    };
    crate::app::state::PasteTarget {
        focus: state.focus(),
        pane,
        overlay,
        page_rename,
        revision,
    }
}

/// 回投时目标是否仍是同一个编辑器实例。
pub fn paste_target_matches(state: &AppState, target: &crate::app::state::PasteTarget) -> bool {
    &paste_target(state) == target
}

/// 申请一个剪贴板请求序号。
pub fn next_clipboard_id(state: &mut AppState) -> u64 {
    let id = state.next_clipboard_id;
    state.next_clipboard_id = state.next_clipboard_id.saturating_add(1);
    id
}

/// 发起一次异步粘贴读取；同一次按键的回投由目标快照校验。
pub fn request_paste(state: &mut AppState) -> bool {
    // 浮层内只有真正可编辑的输入框接收粘贴；菜单/确认框等吞掉，不后台读剪贴板。
    if state.overlay.is_some() && super::dialog::active_text_input(state).is_none() {
        return false;
    }
    let target = paste_target(state);
    let id = next_clipboard_id(state);
    let generation = state.clipboard_generation;
    state.clipboard_requests.push(ClipboardRequest::Read {
        id,
        target,
        generation,
    });
    true
}

/// 发起剪切：先记录事务并写剪贴板，写入确认前不删除任何文字。
pub fn request_cut(state: &mut AppState) -> bool {
    let Some(candidate) = cut_candidate(state) else {
        return false;
    };
    let id = next_clipboard_id(state);
    state.pending_cut = Some(crate::app::state::PendingCut {
        id,
        text: candidate.text.clone(),
        source: candidate.source,
        revision: candidate.revision,
        anchor: candidate.anchor,
    });
    state.clipboard_requests.push(ClipboardRequest::Write {
        id,
        text: candidate.text,
    });
    true
}

/// 一次可剪切选区：文本、来源、修订号与反馈锚点。
struct CutCandidate {
    text: String,
    source: SelectionSource,
    revision: u64,
    anchor: Option<PaneId>,
}

/// 当前焦点下的可编辑选区；只读屏幕/终端/历史选区返回 None。
fn cut_candidate(state: &AppState) -> Option<CutCandidate> {
    let (text, source, revision, anchor) = if state.overlay.is_some()
        || (state.overlay.is_none() && super::page_rename_active(state))
    {
        let input = super::dialog::active_text_input(state)?;
        (
            input.selected_text()?,
            if super::page_rename_active(state) {
                SelectionSource::PageRename
            } else {
                SelectionSource::OverlayText
            },
            input.revision(),
            None,
        )
    } else {
        match state.focus() {
            crate::app::state::FocusedContainer::Prompt => (
                prompt_selection_text(state)?,
                SelectionSource::Prompt,
                state.prompt.revision(),
                Some(state.prompt.id()),
            ),
            crate::app::state::FocusedContainer::Todo => {
                let input = &state.todo.editing()?.input;
                (
                    input.selected_text()?,
                    SelectionSource::Todo,
                    input.revision(),
                    None,
                )
            }
            crate::app::state::FocusedContainer::Panes => {
                let id = state.active_tab().layout.focus();
                let pane = state.pane_anywhere(id)?;
                if pane.view != PaneView::Lx {
                    return None;
                }
                if let Some(ask) = pane.lx.ask.as_ref()
                    && ask.editing
                    && let Some(editor) = ask.custom.get(ask.current)
                {
                    (
                        editor.selected_text()?,
                        SelectionSource::Ask(id),
                        editor.revision(),
                        Some(id),
                    )
                } else {
                    (
                        pane.lx.selected_text()?,
                        SelectionSource::LxInput(id),
                        pane.lx.revision(),
                        Some(id),
                    )
                }
            }
            crate::app::state::FocusedContainer::Sidebar => return None,
        }
    };
    Some(CutCandidate {
        text,
        source,
        revision,
        anchor,
    })
}

/// 剪贴板写入成功后执行删除；返回反馈锚点（None 表示目标已变化、未删除）。
pub fn finish_cut(state: &mut AppState, id: u64) -> Option<Option<PaneId>> {
    let pending = state.pending_cut.clone()?;
    if pending.id != id {
        return None;
    }
    state.pending_cut = None;
    let (current_text, current_revision) = current_selection(state, pending.source)?;
    if current_revision != pending.revision || current_text != pending.text {
        return None;
    }
    delete_selection_source(state, pending.source);
    Some(pending.anchor)
}

/// 读取指定来源当前选中文本与修订号。
fn current_selection(state: &AppState, source: SelectionSource) -> Option<(String, u64)> {
    match source {
        SelectionSource::Screen | SelectionSource::Terminal(_) | SelectionSource::Lx(_) => None,
        SelectionSource::Prompt => Some((prompt_selection_text(state)?, state.prompt.revision())),
        SelectionSource::LxInput(pane) => {
            let lx = &state.pane_anywhere(pane)?.lx;
            Some((lx.selected_text()?, lx.revision()))
        }
        SelectionSource::Ask(pane) => {
            let ask = state.pane_anywhere(pane)?.lx.ask.as_ref()?;
            let editor = ask.custom.get(ask.current)?;
            Some((editor.selected_text()?, editor.revision()))
        }
        SelectionSource::Todo => {
            let input = &state.todo.editing()?.input;
            Some((input.selected_text()?, input.revision()))
        }
        SelectionSource::OverlayText => {
            state.overlay.as_ref()?;
            let input = super::dialog::active_text_input(state)?;
            Some((input.selected_text()?, input.revision()))
        }
        SelectionSource::PageRename => {
            if state.overlay.is_some() || !super::page_rename_active(state) {
                return None;
            }
            let input = super::dialog::active_text_input(state)?;
            Some((input.selected_text()?, input.revision()))
        }
    }
}

/// 删除指定来源的当前选区（一次撤销记录）。
fn delete_selection_source(state: &mut AppState, source: SelectionSource) {
    match source {
        SelectionSource::Prompt => {
            if let Some((start, end)) = prompt_selection_bounds(state) {
                state.prompt.replace_range(start, end, "");
                clear_selection(state);
            }
        }
        SelectionSource::LxInput(pane) => {
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.delete_selection();
            }
        }
        SelectionSource::Ask(pane) => {
            if let Some(target) = state.pane_mut_anywhere(pane)
                && let Some(ask) = target.lx.ask.as_mut()
                && let Some(editor) = ask.custom.get_mut(ask.current)
            {
                editor.delete_selection();
            }
        }
        SelectionSource::Todo => {
            if let Some(input) = state.todo.edit_input_mut() {
                input.delete_selection();
            }
        }
        SelectionSource::OverlayText | SelectionSource::PageRename => {
            if let Some(input) = super::dialog::active_text_input_mut(state) {
                input.delete_selection();
            }
        }
        SelectionSource::Screen | SelectionSource::Terminal(_) | SelectionSource::Lx(_) => {}
    }
}

/// 写入失败时清除事务并返回反馈锚点。
pub fn fail_cut(state: &mut AppState, id: u64) -> Option<Option<PaneId>> {
    if state
        .pending_cut
        .as_ref()
        .is_some_and(|pending| pending.id == id)
    {
        let pending = state.pending_cut.take()?;
        return Some(pending.anchor);
    }
    None
}
