//! BTW 页签的键盘路由：列表搜索/导航与编辑态 composer。

use crate::app::state::AppState;
use crate::tiling::PaneId;

use super::{
    enter_btw_edit, leave_btw_edit, scroll_btw_detail, stop_current, submit_current, switch_qa,
};

/// BTW 页签内的键盘路由。
pub fn apply_input(
    state: &mut AppState,
    pane: PaneId,
    command: crate::app::actions::LxInput,
) -> bool {
    use crate::app::actions::LxInput;
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if !target.lx.is_btw_tab() {
        return false;
    }
    if !target.lx.btw.editing {
        return apply_list_input(state, pane, command);
    }
    // 浮层优先：提及 / 粘贴 / 快捷引用 / md 块。
    let panel_open = target.lx.btw.active_composer().is_some_and(|editor| {
        editor.mention_open()
            || editor.mention_visible()
            || editor.paste_panel().is_some()
            || editor.text_paste_panel().is_some()
            || editor.refer_active()
            || editor.panel.is_some()
    });
    if panel_open {
        return apply_composer_panel_input(state, pane, command);
    }
    match command {
        LxInput::Escape => leave_btw_edit(state, pane),
        LxInput::Enter => submit_current(state, pane),
        LxInput::StopProcess => stop_current(state, pane),
        // PgUp/PgDn 翻上方对话，不进入 composer 视口。
        LxInput::PageUp | LxInput::PageDown => {
            let delta = if command == LxInput::PageUp { -10 } else { 10 };
            scroll_btw_detail(state, pane, delta)
        }
        // 编辑态没有 tab 切换与焦点轮转。
        LxInput::Tab | LxInput::PrevStep => true,
        _ => edit_composer(state, pane, command),
    }
}

fn apply_list_input(
    state: &mut AppState,
    pane: PaneId,
    command: crate::app::actions::LxInput,
) -> bool {
    use crate::app::actions::LxInput;
    match command {
        LxInput::Char(_)
        | LxInput::Backspace
        | LxInput::Delete
        | LxInput::Home
        | LxInput::End
        | LxInput::LineStart
        | LxInput::LineEnd
        | LxInput::WordLeft
        | LxInput::WordRight
        | LxInput::DeleteToLineStart
        | LxInput::DeleteToLineEnd
        | LxInput::DeleteWordBackward
        | LxInput::DeleteWordForward
        | LxInput::SelectAll
        | LxInput::Undo
        | LxInput::Redo => edit_search(state, pane, command),
        LxInput::Left | LxInput::Right => {
            let empty = state
                .pane_anywhere(pane)
                .is_some_and(|target| target.lx.btw.search.text().is_empty());
            if empty {
                let delta = if command == LxInput::Left { -1 } else { 1 };
                crate::app::update::pane::cycle_task_tab(state, pane, delta)
            } else {
                edit_search(state, pane, command)
            }
        }
        LxInput::Up | LxInput::Down => {
            let delta = if command == LxInput::Up { -1 } else { 1 };
            let qa_id = {
                let Some(target) = state.pane_mut_anywhere(pane) else {
                    return false;
                };
                if !target.lx.btw.move_selection(delta) {
                    return true;
                }
                target.lx.btw.active.clone()
            };
            if let Some(qa_id) = qa_id {
                super::switch_qa(state, pane, &qa_id);
            }
            true
        }
        // 只有 Shift+Enter（EnterFolder）进入编辑；Ctrl+Enter（Newline）不进入。
        LxInput::EnterFolder => enter_btw_edit(state, pane),
        LxInput::Enter => true,
        LxInput::Tab | LxInput::PrevStep => true,
        LxInput::PageUp | LxInput::PageDown => {
            let delta = if command == LxInput::PageUp { -10 } else { 10 };
            scroll_btw_detail(state, pane, delta)
        }
        LxInput::StopProcess => stop_current(state, pane),
        LxInput::Escape => {
            let cleared = state
                .pane_mut_anywhere(pane)
                .is_some_and(|target| target.lx.btw.clear_search());
            if cleared {
                true
            } else {
                crate::app::update::chat::close_task_page(state, pane);
                true
            }
        }
        _ => true,
    }
}

/// 编辑搜索框；文本变化后同步过滤结果。
fn edit_search(state: &mut AppState, pane: PaneId, command: crate::app::actions::LxInput) -> bool {
    use crate::app::actions::LxInput;
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let input = &mut target.lx.btw.search;
    let before = (input.text().to_string(), input.cursor(), input.selection());
    match command {
        LxInput::Char(ch) => input.insert_char(ch),
        LxInput::Backspace => input.backspace(),
        LxInput::Delete => input.delete(),
        LxInput::Left => input.move_left(),
        LxInput::Right => input.move_right(),
        LxInput::Home | LxInput::LineStart => input.move_home(),
        LxInput::End | LxInput::LineEnd => input.move_end(),
        LxInput::WordLeft => input.move_word_left(),
        LxInput::WordRight => input.move_word_right(),
        LxInput::DeleteToLineStart => input.delete_to_line_start(),
        LxInput::DeleteToLineEnd => input.delete_to_line_end(),
        LxInput::DeleteWordBackward => input.delete_word_backward(),
        LxInput::DeleteWordForward => input.delete_word_forward(),
        LxInput::SelectAll => input.select_all(),
        LxInput::Undo => {
            input.undo();
        }
        LxInput::Redo => {
            input.redo();
        }
        _ => {}
    }
    let changed = before != (input.text().to_string(), input.cursor(), input.selection());
    let switched = if changed {
        let switched = target.lx.btw.sync_selection_after_filter();
        if switched {
            target.lx.btw.active.clone()
        } else {
            None
        }
    } else {
        None
    };
    if changed {
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    if let Some(qa_id) = switched {
        switch_qa(state, pane, &qa_id);
    }
    changed
}

fn edit_composer(
    state: &mut AppState,
    pane: PaneId,
    command: crate::app::actions::LxInput,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(composer) = target.lx.btw.active_composer_mut() else {
        return false;
    };
    let before = composer.revision();
    super::super::super::pane::apply_editor_edit(composer, command);
    composer.revision() != before
}

/// 浮层优先的按键处理（与主输入同一套面板语义）。
fn apply_composer_panel_input(
    state: &mut AppState,
    pane: PaneId,
    command: crate::app::actions::LxInput,
) -> bool {
    use crate::app::actions::LxInput;
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(composer) = target.lx.btw.active_composer_mut() else {
        return false;
    };
    if composer.text_paste_panel().is_some() {
        match command {
            LxInput::Up => {
                composer.text_paste_move(-1);
            }
            LxInput::Down => {
                composer.text_paste_move(1);
            }
            LxInput::Enter => {
                let action = composer
                    .text_paste_panel()
                    .and_then(|panel| panel.active_action());
                match action {
                    Some(action) => {
                        composer.text_paste_confirm(action);
                    }
                    None => {
                        composer.text_paste_cancel();
                    }
                }
            }
            LxInput::Escape => {
                composer.text_paste_cancel();
            }
            _ => {}
        }
        return true;
    }
    if composer.paste_panel().is_some() {
        match command {
            LxInput::Up => {
                composer.paste_move(-1);
            }
            LxInput::Down => {
                composer.paste_move(1);
            }
            LxInput::Enter => {
                match composer
                    .paste_panel()
                    .and_then(|panel| panel.active_action())
                {
                    Some(action) => {
                        composer.paste_confirm(action);
                    }
                    None => {
                        composer.paste_cancel();
                    }
                }
            }
            LxInput::Escape => {
                composer.paste_cancel();
            }
            _ => {}
        }
        return true;
    }
    if composer.mention_visible() {
        match command {
            LxInput::Left => {
                composer.mention_switch_tab(-1);
                return true;
            }
            LxInput::Right => {
                composer.mention_switch_tab(1);
                return true;
            }
            LxInput::Escape => {
                composer.mention_escape();
                return true;
            }
            LxInput::EnterFolder => {
                if !composer.mention_enter_folder() {
                    // 非目录 / 非 Files / 加载态：回落为换行，不能吞掉 Shift+Enter。
                    super::super::super::pane::apply_editor_edit(composer, command);
                }
                return true;
            }
            _ => {}
        }
    }
    if composer.mention_open() {
        match command {
            LxInput::Up => {
                composer.mention_move(-1);
                return true;
            }
            LxInput::Down => {
                composer.mention_move(1);
                return true;
            }
            LxInput::Tab | LxInput::Enter => {
                composer.mention_confirm();
                return true;
            }
            _ => {}
        }
    }
    if composer.mention_visible() {
        return super::super::super::pane::apply_editor_edit(composer, command);
    }
    if composer.refer_active() {
        match command {
            LxInput::Up => {
                composer.refer_move(-1);
                return true;
            }
            LxInput::Down => {
                composer.refer_move(1);
                return true;
            }
            LxInput::Tab | LxInput::Enter => {
                composer.refer_confirm();
                return true;
            }
            LxInput::Escape => {
                composer.refer_escape();
                return true;
            }
            _ => {}
        }
    }
    if composer.panel.is_some() {
        match command {
            LxInput::Up => {
                composer.panel_move(-1);
            }
            LxInput::Down => {
                composer.panel_move(1);
            }
            LxInput::Tab | LxInput::Enter => {
                composer.panel_accept();
            }
            LxInput::Escape => {
                composer.panel_close();
            }
            _ => {}
        }
        return true;
    }
    false
}
