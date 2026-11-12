//! BTW 页签的键盘路由、composer 浮层与 `/` 命令面板。

use crate::app::state::lx::btw::BtwFocus;
use crate::app::state::{AppState, InputHintLevel, Lx};
use crate::app::update::show_lx_hint;
use crate::tiling::PaneId;
use crate::ui::text;

use super::{is_btw_list_command, open_btw_list, stop_current, submit_current, switch_qa};

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
    // QA 选择器优先：打开期间独占键盘（↑↓/Enter/Esc），不许落进草稿。
    if target.lx.btw.picking.is_some() {
        return apply_picker_input(state, pane, command);
    }
    let focus = target.lx.btw.focus;
    // BTW 输入框的 `/` 命令面板：导航/采纳/关闭由这里独占，其它按键继续编辑并重算。
    let slash_open = target
        .lx
        .btw
        .active_composer()
        .is_some_and(|composer| composer.slash.is_some());
    if slash_open {
        use crate::app::actions::LxInput;
        match command {
            LxInput::Up | LxInput::Down => {
                let delta = if command == LxInput::Up { -1 } else { 1 };
                if let Some(panel) = state
                    .pane_mut_anywhere(pane)
                    .and_then(|target| target.lx.btw.active_composer_mut())
                    .and_then(|composer| composer.slash.as_mut())
                {
                    panel.move_active(delta);
                }
                return true;
            }
            LxInput::Enter | LxInput::Tab => return accept_btw_composer_command(state, pane),
            LxInput::Escape => {
                if let Some(composer) = state
                    .pane_mut_anywhere(pane)
                    .and_then(|target| target.lx.btw.active_composer_mut())
                {
                    composer.slash = None;
                }
                return true;
            }
            _ => return edit_composer(state, pane, command),
        }
    }
    // 空输入框：←/→ 轮转底部 tab（输入框里有内容时仍按光标移动处理）。
    if focus == BtwFocus::Composer
        && matches!(
            command,
            crate::app::actions::LxInput::Left | crate::app::actions::LxInput::Right
        )
        && target
            .lx
            .btw
            .active_composer()
            .is_some_and(|composer| composer.input.is_empty())
    {
        let delta = if command == crate::app::actions::LxInput::Left {
            -1
        } else {
            1
        };
        crate::app::update::pane::cycle_task_tab(state, pane, delta);
        return true;
    }
    // 浮层优先：提及 / 粘贴 / 快捷引用 / md 块。
    let composer = target.lx.btw.active_composer();
    let panel_open = composer.is_some_and(|editor| {
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
        LxInput::Escape => {
            if focus != BtwFocus::Composer {
                if let Some(target) = state.pane_mut_anywhere(pane) {
                    target.lx.btw.focus = BtwFocus::Composer;
                }
                return true;
            }
            super::super::task_page::close_task_page(state, pane);
            true
        }
        LxInput::Tab => {
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.btw.focus = target.lx.btw.focus.next();
            }
            true
        }
        LxInput::Enter => match focus {
            BtwFocus::Composer => {
                let is_list = state
                    .pane_anywhere(pane)
                    .and_then(|target| target.lx.btw.active_composer())
                    .is_some_and(|composer| is_btw_list_command(&composer.input));
                if is_list {
                    if let Some(composer) = state
                        .pane_mut_anywhere(pane)
                        .and_then(|target| target.lx.btw.active_composer_mut())
                    {
                        composer.clear();
                    }
                    open_btw_list(state, pane)
                } else {
                    submit_current(state, pane)
                }
            }
            BtwFocus::Conversation => false,
        },
        LxInput::Up | LxInput::Down => match focus {
            BtwFocus::Composer => edit_composer(state, pane, command),
            BtwFocus::Conversation => {
                let delta = if command == LxInput::Up { -1 } else { 1 };
                scroll_conversation(state, pane, delta)
            }
        },
        LxInput::StopProcess => stop_current(state, pane),
        // 空草稿 + Ctrl+A：回退默认回答者，避免无法退出旧目标集合。
        LxInput::LineStart => {
            let empty = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.btw.active_thread())
                .is_some_and(|thread| thread.composer.is_empty());
            if empty {
                if let Some(thread) = state
                    .pane_mut_anywhere(pane)
                    .and_then(|target| target.lx.btw.active_thread_mut())
                    && !thread.targets.is_empty()
                {
                    thread.targets.clear();
                    show_lx_hint(
                        state,
                        pane,
                        InputHintLevel::Warning,
                        text::LX_BTW_TARGETS_RESET,
                    );
                    return true;
                }
                return false;
            }
            edit_composer(state, pane, command)
        }
        _ => edit_composer(state, pane, command),
    }
}

/// 编辑 composer 的纯文本/选区命令。
/// BTW 输入框的 `/` 面板候选：只列 BTW 自己的命令。
fn btw_command_entries(query: &str) -> Vec<crate::app::state::SlashEntry> {
    let query = query.trim_start_matches('/').to_ascii_lowercase();
    let matched = ["btwlist", "btw-list"]
        .iter()
        .any(|keyword| keyword.starts_with(&query) || keyword.contains(&query));
    if !matched {
        return Vec::new();
    }
    vec![crate::app::state::SlashEntry::Command(
        crate::app::state::lx::slash::SlashCommand::BtwList,
    )]
}

/// 重算 BTW 输入框的 `/` 命令面板；只接管 `Commands`，不覆盖二级面板。
fn refresh_composer_slash(composer: &mut Lx) {
    if composer
        .slash
        .as_ref()
        .is_some_and(|panel| !matches!(panel, crate::app::state::SlashPanel::Commands { .. }))
    {
        return;
    }
    let closed = |composer: &mut Lx| {
        if matches!(
            composer.slash,
            Some(crate::app::state::SlashPanel::Commands { .. })
        ) {
            composer.slash = None;
        }
    };
    let masked = crate::app::paste::mask_pasted_text(&composer.input);
    let Some(query) = crate::app::state::lx::slash::slash_trigger(&masked, composer.cursor) else {
        closed(composer);
        return;
    };
    let items = btw_command_entries(&query);
    if items.is_empty() {
        closed(composer);
        return;
    }
    composer.slash = Some(crate::app::state::SlashPanel::Commands {
        items,
        window: crate::app::state::lx::slash::PanelWindow::new(0),
        query,
    });
}

/// 采纳 BTW 输入框里高亮的 `/` 命令。
fn accept_btw_composer_command(state: &mut AppState, pane: PaneId) -> bool {
    let Some(command) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.btw.active_composer())
        .and_then(|composer| composer.slash.as_ref())
        .and_then(|panel| panel.active_command_entry())
        .map(|entry| match entry {
            crate::app::state::SlashEntry::Command(command) => *command,
        })
    else {
        return false;
    };
    if let Some(composer) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_composer_mut())
    {
        composer.clear();
        composer.slash = None;
    }
    match command {
        crate::app::state::lx::slash::SlashCommand::BtwList => open_btw_list(state, pane),
        _ => false,
    }
}

/// 鼠标点选 BTW 输入框的 `/` 面板条目：按该条目的 Enter 语义采纳。
pub fn select_btw_slash(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let valid = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_composer_mut())
        .and_then(|composer| composer.slash.as_mut())
        .is_some_and(|panel| {
            if index >= panel.item_count() {
                return false;
            }
            let _ = panel.set_active(index);
            true
        });
    if !valid {
        return false;
    }
    accept_btw_composer_command(state, pane)
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
    let changed = composer.revision() != before;
    refresh_composer_slash(composer);
    changed
}

fn apply_picker_input(
    state: &mut AppState,
    pane: PaneId,
    command: crate::app::actions::LxInput,
) -> bool {
    use crate::app::actions::LxInput;
    match command {
        LxInput::Up | LxInput::Down => {
            let delta = if command == LxInput::Up { -1 } else { 1 };
            state
                .pane_mut_anywhere(pane)
                .and_then(|target| {
                    let btw = &mut target.lx.btw;
                    btw.picking.map(|_| btw.move_picker(delta))
                })
                .unwrap_or(false)
        }
        LxInput::Enter | LxInput::Tab => {
            let Some(qa_id) = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.btw.picked_qa())
                .map(|summary| summary.qa_id.clone())
            else {
                return false;
            };
            switch_qa(state, pane, &qa_id)
        }
        LxInput::Escape => state
            .pane_mut_anywhere(pane)
            .is_some_and(|target| target.lx.btw.close_picker()),
        // 选择器打开时吞掉其它按键，避免误改草稿。
        _ => true,
    }
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
                composer.mention_enter_folder();
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
    // `@` 面板只接管导航/采纳；其余按键继续落回草稿。
    //
    // 之前这里直接返回 false，导致面板打开后退格删不掉 `@`、继续输入也进不了过滤器
    // （按键被面板吞掉）。与主输入一致：编辑后退格/字符会重新触发面板。
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
    // md 块面板：与主输入一致（↑↓ 移动、Tab/Enter 采纳、Esc 关闭）。
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

fn scroll_conversation(state: &mut AppState, pane: PaneId, delta: isize) -> bool {
    let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    else {
        return false;
    };
    let max = thread.total_rows.saturating_sub(thread.visible);
    let next = if delta < 0 {
        thread.scroll.saturating_sub(1)
    } else {
        thread.scroll.saturating_add(1).min(max)
    };
    if next == thread.scroll {
        return false;
    }
    thread.scroll = next;
    thread.follow = next >= max;
    true
}
