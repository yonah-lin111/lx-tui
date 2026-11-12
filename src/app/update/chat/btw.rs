//! 旁路问答（`/btw`）的应用层更新：打开页签、提交、事件回投与输入路由。
//!
//! 只改内存状态并登记 [`Effect::Btw`]；数据库与网络由事件层执行。

use crate::app::effects::{BtwEffect, BtwSubmission, BtwTargetChoice, Effect};
use crate::app::state::lx::btw::{BtwFocus, BtwQaSummary, BtwTargetView};
use crate::app::state::{AppState, ChatMessageView, InputHintLevel, Lx, TaskTab};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update::{show_lx_hint, show_lx_toast};
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::tiling::PaneId;
use crate::ui::text;

/// 打开 BTW 页签并请求 QA 列表。
pub fn open_btw_tab(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx {
        return false;
    }
    if target.lx.entry != crate::app::state::LxEntry::Native {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_BTW_ENTRY_UNAVAILABLE,
        );
        return false;
    }
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_BTW_NO_SESSION,
        );
        return false;
    };
    let page = target.lx.task.get_or_insert_with(Default::default);
    page.tab = TaskTab::Btw;
    page.agent_reply = None;
    page.agent_reply_target = None;
    state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
        pane,
        parent_session_id: session_id.clone(),
    }));
    // 同时刷新 subagent 实例列表：BTW 的 `@` 需要运行中与已结束实例都能解析。
    state.effects.push(Effect::ListSubagents {
        pane,
        parent_session: session_id,
    });
    true
}

fn is_btw_list_command(text: &str) -> bool {
    let line = text.trim();
    line.eq_ignore_ascii_case("/btwlist") || line.eq_ignore_ascii_case("/btw-list")
}

/// 打开 QA 选择器：页签已开时直接就地打开（必要时补拉列表），否则先打开页签。
pub fn open_btw_list(state: &mut AppState, pane: PaneId) -> bool {
    let already = state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.is_btw_tab());
    if already {
        let Some(parent_session_id) = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.chat.session_id.clone())
        else {
            return false;
        };
        state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
            pane,
            parent_session_id,
        }));
    } else if !open_btw_tab(state, pane) {
        return false;
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.btw.qa.is_empty() {
        target.lx.btw.pending_pick = true;
    } else {
        target.lx.btw.open_picker();
    }
    true
}

/// 选择器确认/点击：切到目标 QA 并加载它的线程历史。
pub fn select_btw_qa(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let Some(qa_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.btw.qa.get(index))
        .map(|summary| summary.qa_id.clone())
    else {
        return false;
    };
    switch_qa(state, pane, &qa_id)
}

fn switch_qa(state: &mut AppState, pane: PaneId, qa_id: &str) -> bool {
    let Some(parent_session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return false;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.btw.select_qa(qa_id);
    target.lx.btw.close_picker();
    target.lx.btw.pending_pick = false;
    target.lx.btw.focus = BtwFocus::Composer;
    state.effects.push(Effect::Btw(BtwEffect::LoadThread {
        pane,
        parent_session_id,
        qa_id: qa_id.to_string(),
    }));
    crate::app::update::sync_lx_mention_root(state, pane);
    true
}

/// `/btw [prompt]` 命令入口：打开页签，把 prompt 放入当前 QA 的 composer 并提交。
///
/// 裸 `/btw` 只回显 `"/btw "` 与灰色参数提示，不发送。
pub fn try_submit_btw_command(state: &mut AppState, pane: PaneId) -> bool {
    let Some((input, session_id)) = state.pane_anywhere(pane).and_then(|target| {
        if target.view != crate::app::state::PaneView::Lx || target.lx.shell_mode() {
            return None;
        }
        Some((target.lx.input.clone(), target.lx.chat.session_id.clone()))
    }) else {
        return false;
    };
    let Some((prompt, bare)) = crate::app::state::lx::slash::parse_btw_command(&input) else {
        return false;
    };
    if bare {
        if let Some(target) = state.pane_mut_anywhere(pane)
            && let Some(prefix) = crate::app::state::SlashCommand::Btw.echo_prefix()
        {
            if target.lx.input != prefix {
                target.lx.replace_input(prefix);
            } else {
                target.lx.cursor = prefix.len();
                target.lx.clear_text_selection();
            }
            target.lx.slash = None;
        }
        open_btw_tab(state, pane);
        return true;
    }
    let Some(session_id) = session_id else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_BTW_NO_SESSION,
        );
        return true;
    };
    if !open_btw_tab(state, pane) {
        return true;
    }
    let prompt = prompt.to_string();
    // 命令本身就是一次提交：主输入框内容不保留。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.clear();
    }
    if let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    {
        thread.composer.editor_mut().replace_input(&prompt);
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.btw.pending_prompt = true;
    }
    // QA 列表由事件回投后才有；先登记一次延迟提交，列表到达时立即发出。
    state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
        pane,
        parent_session_id: session_id,
    }));
    true
}

/// 进入 BTW 页签后按需加载 QA 列表（重复调用幂等）。
pub fn ensure_loaded(state: &mut AppState, pane: PaneId) -> bool {
    let Some((is_btw, session_id)) = state
        .pane_anywhere(pane)
        .map(|target| (target.lx.is_btw_tab(), target.lx.chat.session_id.clone()))
    else {
        return false;
    };
    if !is_btw {
        return false;
    }
    let Some(parent_session_id) = session_id else {
        return false;
    };
    state.effects.push(Effect::Btw(BtwEffect::LoadQaList {
        pane,
        parent_session_id,
    }));
    true
}

/// 提交当前 QA 的草稿；返回是否登记了提交。
pub fn submit_current(state: &mut AppState, pane: PaneId) -> bool {
    let Some(session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return false;
    };
    let Some((workspace, cwd)) = state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.workspace_cwd_for_pane(pane))
        .map(|workspace| {
            let cwd = state
                .workspace_cwd_for_pane(pane)
                .unwrap_or_else(|| workspace.clone());
            (workspace, cwd)
        })
    else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_BTW_NO_SESSION);
        return false;
    };
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(qa_id) = target.lx.btw.active.clone() else {
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_BTW_NO_QA);
        return false;
    };
    let Some(thread) = target.lx.btw.active_thread() else {
        return false;
    };
    let text_value = thread.composer.text().trim().to_string();
    if text_value.is_empty() {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_BTW_EMPTY_PROMPT,
        );
        return false;
    }
    let Some(selection) = target.lx.chat.effective_selection(&state.models) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return false;
    };
    let Some(frozen) = crate::app::state::FrozenModel::resolve(&state.models, &selection) else {
        show_lx_hint(state, pane, InputHintLevel::Error, text::LX_CHAT_NO_MODEL);
        return false;
    };
    let targets = match resolve_targets(&target.lx, &text_value) {
        Ok(targets) => targets,
        Err(message) => {
            show_lx_hint(state, pane, InputHintLevel::Warning, message);
            return false;
        }
    };
    let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    else {
        return false;
    };
    if !targets.is_empty() {
        thread.targets = targets
            .iter()
            .map(|choice| BtwTargetView {
                key: choice.agent_id.clone(),
                label: format!("{} [{}]", choice.display_nickname, choice.role_name),
                agent_id: Some(choice.agent_id.clone()),
            })
            .collect();
    }
    thread.submit_seq = thread.submit_seq.saturating_add(1);
    let submit_id = format!(
        "btw-{}-{}-{}",
        crate::db::now_ms(),
        pane.raw(),
        thread.submit_seq
    );
    let revision = thread.composer.revision();
    thread.pending_submit = Some((submit_id.clone(), revision));
    thread.pending_recipients = targets.len().max(1);
    let thread_id = thread.thread_id.clone();
    state
        .effects
        .push(Effect::Btw(BtwEffect::Submit(Box::new(BtwSubmission {
            pane,
            parent_session_id: session_id,
            qa_id,
            workspace,
            cwd,
            thread_id,
            submit_id,
            draft_revision: revision,
            text: text_value,
            targets,
            default_model: crate::db::chat::ModelSnapshot {
                provider: frozen.provider.clone(),
                model: frozen.model.clone(),
                variant: frozen.variant.clone(),
            },
            default_variant_params: frozen.variant_params.clone(),
            idle_timeout_ms: frozen.idle_timeout_ms,
        }))));
    true
}

/// 停止当前线程的活动与排队回答。
pub fn stop_current(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(qa_id) = target.lx.btw.active.clone() else {
        return false;
    };
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        return false;
    };
    let thread_id = target
        .lx
        .btw
        .active_thread()
        .and_then(|thread| thread.thread_id.clone());
    if let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    {
        thread.pending_submit = None;
        thread.pending_recipients = 0;
    }
    state.effects.push(Effect::Btw(BtwEffect::Stop {
        pane,
        parent_session_id: session_id,
        qa_id,
        thread_id,
    }));
    show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Default,
            text::LX_BTW_STOPPED,
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

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
            super::task_page::close_task_page(state, pane);
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
    super::super::pane::apply_editor_edit(composer, command);
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
        return super::super::pane::apply_editor_edit(composer, command);
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

/// 从草稿里的 `@[refer-subagent](...)` 解析本轮回答者；零实例时报错，不自动创建。
fn resolve_targets(
    target: &Lx,
    draft: &str,
) -> std::result::Result<Vec<BtwTargetChoice>, &'static str> {
    let masked = crate::app::paste::mask_pasted_text(draft);
    let mut chosen: Vec<BtwTargetChoice> = Vec::new();
    for reference in crate::app::markdown::parse_references(&masked) {
        if reference.kind != crate::app::markdown::ReferKind::Subagent {
            continue;
        }
        let raw = reference.path.trim();
        if raw.is_empty() {
            continue;
        }
        let (head, role) = match raw.rsplit_once(':') {
            Some((head, role)) => (head.trim(), Some(role.trim())),
            None => (raw, None),
        };
        let run = find_instance(target, head, role)?;
        if chosen.iter().any(|item| item.agent_id == run.agent_id) {
            continue;
        }
        chosen.push(BtwTargetChoice {
            agent_id: run.agent_id.clone(),
            display_nickname: run.nickname.clone().unwrap_or_else(|| run.agent_id.clone()),
            role_name: run.role.as_str().to_string(),
            source_child_session_id: run.child_session_id.clone(),
        });
    }
    Ok(chosen)
}

fn find_instance<'a>(
    target: &'a Lx,
    head: &str,
    role: Option<&str>,
) -> std::result::Result<&'a crate::app::state::SubagentRun, &'static str> {
    let matches = |run: &&crate::app::state::SubagentRun| {
        run.agent_id == head
            || run.nickname.as_deref() == Some(head)
            || (role.is_none_or(|role| role == run.role.as_str()) && run.role.as_str() == head)
    };
    let candidates: Vec<&crate::app::state::SubagentRun> = target
        .agents
        .runs
        .iter()
        .filter(|run| matches(run))
        .collect();
    match candidates.as_slice() {
        [run] => Ok(run),
        [] => Err(text::LX_BTW_NO_MATCHING_AGENT),
        _ => Err(text::LX_BTW_AMBIGUOUS_AGENT),
    }
}

/// 事件层回投：QA 列表已加载。
pub fn on_qa_list_loaded(
    state: &mut AppState,
    pane: PaneId,
    parent_session_id: &str,
    qa: Vec<BtwQaSummary>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() != Some(parent_session_id) {
        return false;
    }
    target.lx.btw.qa = qa;
    if target.lx.btw.active.is_none()
        && let Some(last) = target.lx.btw.qa.last()
    {
        let qa_id = last.qa_id.clone();
        target.lx.btw.select_qa(&qa_id);
    }
    if target.lx.btw.qa.is_empty() {
        target.lx.btw.pending_prompt = false;
        target.lx.btw.pending_pick = false;
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_BTW_NO_QA);
        return true;
    }
    if target.lx.btw.pending_pick {
        target.lx.btw.pending_pick = false;
        target.lx.btw.open_picker();
    }
    let qa_id = target
        .lx
        .btw
        .active
        .clone()
        .filter(|_| !target.lx.btw.qa.is_empty())
        .unwrap_or_default();
    let has_qa = !qa_id.is_empty();
    let pending_prompt = target.lx.btw.pending_prompt;
    // composer 是懒创建的：拿到 QA、建立线程之后才存在，此时再同步 `@` 根与实例。
    crate::app::update::sync_lx_mention_root(state, pane);
    if has_qa {
        state.effects.push(Effect::Btw(BtwEffect::LoadThread {
            pane,
            parent_session_id: parent_session_id.to_string(),
            qa_id,
        }));
        // `/btw prompt` 在 QA 列表到达后才真正提交（只认命令暂存，不误发普通草稿）。
        if pending_prompt {
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.btw.pending_prompt = false;
            }
            submit_current(state, pane);
        }
    }
    true
}

/// 事件层回投：线程历史已加载。
#[allow(clippy::too_many_arguments)]
pub fn on_thread_loaded(
    state: &mut AppState,
    pane: PaneId,
    qa_id: &str,
    thread_id: Option<String>,
    child_session_id: Option<String>,
    title: String,
    messages: Vec<crate::db::chat::ChatMessage>,
    targets: Vec<crate::app::state::lx::BtwTargetView>,
    loaded: bool,
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let thread = target.lx.btw.ensure_thread(qa_id);
    if thread.loaded {
        return false;
    }
    thread.chat.session_id = child_session_id.clone();
    thread.chat.messages = messages
        .iter()
        .map(|message| crate::app::update::view_message_of(message, &models))
        .collect();
    thread.thread_id = thread_id;
    thread.child_session_id = child_session_id;
    thread.title = title;
    if !targets.is_empty() {
        thread.targets = targets;
    }
    thread.loaded = loaded;
    thread.rows_dirty = true;
    thread.follow = true;
    // 线程建立后 composer 才存在：补一次 `@` 扫描根与实例同步。
    crate::app::update::sync_lx_mention_root(state, pane);
    true
}

/// 提交被接受后的回投字段（事件层 → 应用层）。
#[derive(Debug, Clone)]
pub struct AcceptedBtw {
    pub submit_id: String,
    pub thread_id: String,
    pub child_session_id: String,
    pub user_message_id: String,
    pub text: String,
}

/// 事件层回投：提交已被接受。
pub fn on_accepted(state: &mut AppState, pane: PaneId, event: AcceptedBtw) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(thread) = target.lx.btw.active_thread_mut() else {
        return false;
    };
    if thread.pending_submit.as_ref().map(|(id, _)| id.as_str()) != Some(event.submit_id.as_str()) {
        return false;
    }
    let revision = thread.pending_submit.as_ref().map(|(_, rev)| *rev);
    thread.pending_submit = None;
    thread.thread_id = Some(event.thread_id.clone());
    thread.child_session_id = Some(event.child_session_id.clone());
    if revision == Some(thread.composer.revision()) {
        thread.composer.clear();
    }
    thread.chat.messages.push(ChatMessageView {
        id: event.user_message_id.clone(),
        sequence: 0,
        kind: "chat".to_string(),
        role: "user".to_string(),
        status: "complete".to_string(),
        turn_index: 0,
        shell: None,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: PartKind::Text,
                content: event.text.clone(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    thread.rows_dirty = true;
    thread.follow = true;
    true
}

/// 事件层回投：某个回答者开始运行。
pub fn on_recipient_started(
    state: &mut AppState,
    child_session_id: &str,
    assistant_message_id: &str,
    label: &str,
) -> bool {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session_mut(child_session_id) else {
            continue;
        };
        if thread
            .chat
            .messages
            .iter()
            .any(|message| message.id == assistant_message_id)
        {
            return false;
        }
        thread.pending_recipients = thread.pending_recipients.saturating_sub(1);
        thread.chat.messages.push(ChatMessageView {
            id: assistant_message_id.to_string(),
            sequence: 0,
            kind: "chat".to_string(),
            role: "assistant".to_string(),
            status: "streaming".to_string(),
            turn_index: 0,
            shell: None,
            steps: Vec::new(),
            model_label: Some(label.to_string()),
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
            reports: Vec::new(),
            attachments: Vec::new(),
        });
        thread.rows_dirty = true;
        return true;
    }
    false
}

/// 事件层回投：旁路流增量。
pub fn on_delta(
    state: &mut AppState,
    child_session_id: &str,
    assistant_message_id: &str,
    deltas: &[ChatDelta],
) -> bool {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session_mut(child_session_id) else {
            continue;
        };
        thread.chat.stream_assistant = Some(assistant_message_id.to_string());
        let changed = super::subagent::apply_deltas(&mut thread.chat, deltas);
        thread.chat.stream_assistant = None;
        if changed {
            thread.rows_dirty = true;
            if thread.follow {
                thread.scroll = thread.total_rows;
            }
        }
        return changed;
    }
    false
}

/// 事件层回投：旁路工具状态。
pub fn on_tool_state(
    state: &mut AppState,
    child_session_id: &str,
    assistant_message_id: &str,
    update: BtwToolUpdate<'_>,
) -> bool {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session_mut(child_session_id) else {
            continue;
        };
        thread.chat.stream_assistant = Some(assistant_message_id.to_string());
        let changed = match update {
            BtwToolUpdate::Pending(call_id) => thread.chat.mark_tool_pending(call_id),
            BtwToolUpdate::Running(call_id) => thread.chat.mark_tool_running(call_id),
            BtwToolUpdate::Settled(settlement) => thread.chat.settle_tool_result(
                settlement.call_id,
                settlement.status,
                settlement.summary,
                settlement.result_json,
                settlement.error,
            ),
        };
        thread.chat.stream_assistant = None;
        if changed {
            thread.rows_dirty = true;
        }
        return changed;
    }
    false
}

/// 工具状态回投的三种形态。
pub enum BtwToolUpdate<'a> {
    Pending(&'a str),
    Running(&'a str),
    Settled(Box<BtwToolSettlement<'a>>),
}

/// 工具结算是最大变体：装箱避免整个事件枚举被撑大。
pub struct BtwToolSettlement<'a> {
    pub call_id: &'a str,
    pub status: crate::app::state::ToolStatus,
    pub summary: Option<crate::app::state::ToolSummary>,
    pub result_json: Option<String>,
    pub error: Option<String>,
}

/// 事件层回投：某个回答者结束。
pub fn on_finished(
    state: &mut AppState,
    child_session_id: &str,
    assistant_message_id: &str,
    reason: &StopReason,
    usage: crate::db::chat::Usage,
    duration_ms: i64,
) -> bool {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session_mut(child_session_id) else {
            continue;
        };
        thread.chat.stream_assistant = Some(assistant_message_id.to_string());
        thread.chat.close_stream(reason);
        if let Some(message) = thread
            .chat
            .messages
            .iter_mut()
            .find(|message| message.id == assistant_message_id)
        {
            message.status = match reason {
                StopReason::Complete => "complete".to_string(),
                StopReason::Cancelled => "interrupted".to_string(),
                StopReason::Failed { .. } => "failed".to_string(),
            };
            message.error_code = match reason {
                StopReason::Failed { code } => Some(code.clone()),
                _ => None,
            };
            message.input_tokens = usage.input;
            message.output_tokens = usage.output;
            message.cache_read_tokens = usage.cache_read;
            message.duration_ms = Some(duration_ms);
        }
        thread.chat.stream_assistant = None;
        thread.rows_dirty = true;
        return true;
    }
    false
}

/// 事件层回投：提交失败（保留草稿）。
pub fn on_failed(state: &mut AppState, pane: PaneId, message: &str) -> bool {
    // 提交失败不会再有回答者开始：清掉等待标记，loading 必须立即消失、草稿保留可重试。
    if let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    {
        thread.pending_submit = None;
        thread.pending_recipients = 0;
    }
    show_lx_hint(state, pane, InputHintLevel::Error, message);
    true
}

/// 读取某回答者 assistant 消息的完整 parts（结算前快照）。
pub fn btw_message_parts(
    state: &AppState,
    child_session_id: &str,
    assistant_message_id: &str,
) -> Vec<crate::db::chat::ChatPart> {
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_anywhere(pane) else {
            continue;
        };
        let Some(thread) = target.lx.btw.thread_by_child_session(child_session_id) else {
            continue;
        };
        if let Some(message) = thread
            .chat
            .messages
            .iter()
            .find(|message| message.id == assistant_message_id)
        {
            return crate::db::chat::chat_message_part_views(message);
        }
    }
    Vec::new()
}

/// 切换旁路对话里的折叠目标（思考块 / 工具卡片 / 用户消息 / 粘贴芯片）。
///
/// 与主历史一致：展开前先把贴底视口锚定到当前位置，避免展开后整段跳回顶部。
pub fn toggle_btw_card(
    state: &mut AppState,
    pane: PaneId,
    target: crate::app::state::CardTarget,
) -> bool {
    let Some(target_pane) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(thread) = target_pane.lx.btw.active_thread() else {
        return false;
    };
    let tool_present = match &target {
        crate::app::state::CardTarget::Tool {
            message_id,
            call_id,
            ..
        } => thread
            .chat
            .messages
            .iter()
            .any(|message| message.id == *message_id && message.tool(call_id).is_some()),
        _ => true,
    };
    let Some(thread) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.btw.active_thread_mut())
    else {
        return false;
    };
    if thread.follow {
        thread.scroll = thread.total_rows.saturating_sub(thread.visible);
        thread.follow = false;
    }
    let changed = match target {
        crate::app::state::CardTarget::Tool {
            message_id,
            call_id,
            section,
        } => tool_present && thread.chat.toggle_section(&message_id, &call_id, section),
        crate::app::state::CardTarget::Thinking {
            message_index,
            step_index,
        } => {
            let is_reasoning = thread
                .chat
                .messages
                .get(message_index)
                .and_then(|message| message.step_at(step_index))
                .is_some_and(|step| {
                    matches!(
                        step,
                        crate::app::state::AssistantStepView::Part { part, .. }
                            if part.kind == PartKind::Reasoning
                    )
                });
            if is_reasoning {
                thread.chat.toggle_reasoning(message_index, step_index);
            }
            is_reasoning
        }
        crate::app::state::CardTarget::UserMessage { message_id } => {
            thread.chat.toggle_user_message_expanded(&message_id)
        }
        crate::app::state::CardTarget::UserPaste {
            message_id,
            paste_id,
            occurrence,
        } => {
            thread
                .chat
                .toggle_user_paste(&message_id, paste_id, occurrence);
            true
        }
        _ => false,
    };
    if changed {
        thread.rows_dirty = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::actions::LxInput;
    use crate::app::state::{AppState, PaneView, TaskPage, TaskTab};

    fn btw_pane() -> (AppState, PaneId) {
        let mut state = AppState::demo();
        let pane = state.active_tab().layout.focus();
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.entry = crate::app::state::LxEntry::Native;
        target.lx.chat.session_id = Some("sess-test".into());
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Btw,
            ..Default::default()
        });
        target.lx.btw.ensure_thread("qa-1");
        target
            .lx
            .sync_btw_composer_context(Some(std::env::temp_dir()), None);
        (state, pane)
    }

    fn qa(id: &str, has_thread: bool) -> BtwQaSummary {
        BtwQaSummary {
            qa_id: id.to_string(),
            title: format!("question {id}"),
            has_thread,
            status: "idle".to_string(),
        }
    }

    #[test]
    fn btw_composer_slash_panel_lists_btwlist_and_opens_the_picker() {
        let (mut state, pane) = btw_pane();
        // 在 BTW 输入框输入 `/`：打开 BTW 专属命令面板，而不是主会话的命令面板。
        assert!(apply_input(&mut state, pane, LxInput::Char('/')));
        let composer = state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_composer()
            .expect("composer");
        let Some(crate::app::state::SlashPanel::Commands { items, .. }) = &composer.slash else {
            panic!("/ 面板应打开");
        };
        assert_eq!(items.len(), 1, "BTW 面板只列 BTW 自己的命令");
        assert!(matches!(
            items[0],
            crate::app::state::SlashEntry::Command(
                crate::app::state::lx::slash::SlashCommand::BtwList
            )
        ));
        assert!(
            state.pane_anywhere(pane).expect("pane").lx.slash.is_none(),
            "主输入框的 / 面板不能被 BTW 命令污染"
        );

        // 继续输入过滤：完整命令仍保留唯一候选。
        for ch in "btwlist".chars() {
            assert!(apply_input(&mut state, pane, LxInput::Char(ch)));
        }
        assert!(matches!(
            state
                .pane_anywhere(pane)
                .expect("pane")
                .lx
                .btw
                .active_composer()
                .expect("composer")
                .slash,
            Some(crate::app::state::SlashPanel::Commands { .. })
        ));

        // Enter 采纳：草稿被消费，QA 列表未到达时先挂起选择器。
        assert!(apply_input(&mut state, pane, LxInput::Enter));
        let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
        assert!(btw.pending_pick);
        let composer = btw.active_composer().expect("composer");
        assert_eq!(composer.input, "");
        assert!(composer.slash.is_none());

        // QA 列表回投：默认停当前 QA，并保留 DB 的 side-thread 标记。
        assert!(on_qa_list_loaded(
            &mut state,
            pane,
            "sess-test",
            vec![qa("qa-1", true), qa("qa-2", false)],
        ));
        let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
        assert_eq!(btw.picking, Some(0));
        assert!(btw.has_side_thread(&btw.qa[0]));
        assert!(!btw.has_side_thread(&btw.qa[1]));

        // ↓ 选到另一条 QA，Enter 切换并触发历史加载。
        assert!(apply_input(&mut state, pane, LxInput::Down));
        assert!(apply_input(&mut state, pane, LxInput::Enter));
        let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
        assert_eq!(btw.active.as_deref(), Some("qa-2"));
        assert!(btw.picking.is_none());
        assert!(state.effects.iter().any(|effect| matches!(
            effect,
            Effect::Btw(BtwEffect::LoadThread { qa_id, .. }) if qa_id == "qa-2"
        )));
    }

    #[test]
    fn empty_btw_composer_arrows_switch_tabs() {
        let (mut state, pane) = btw_pane();
        assert!(apply_input(&mut state, pane, LxInput::Left));
        assert_eq!(
            state
                .pane_anywhere(pane)
                .expect("pane")
                .lx
                .task
                .as_ref()
                .expect("task")
                .tab,
            TaskTab::Agents
        );
    }

    #[test]
    fn non_empty_btw_composer_arrows_move_the_cursor() {
        let (mut state, pane) = btw_pane();
        state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_composer_mut()
            .expect("composer")
            .insert_char('x');
        let _ = apply_input(&mut state, pane, LxInput::Left);
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.task.as_ref().expect("task").tab, TaskTab::Btw);
        assert_eq!(target.lx.btw.active_composer().expect("composer").cursor, 0);
    }

    #[test]
    fn btw_loading_frames_advance_while_running() {
        let (mut state, pane) = btw_pane();
        let before = {
            let thread = state
                .pane_mut_anywhere(pane)
                .expect("pane")
                .lx
                .btw
                .active_thread_mut()
                .expect("thread");
            thread.pending_submit = Some(("prompt".into(), 1));
            assert!(thread.is_loading());
            thread.chat.loading_frame
        };
        assert!(super::super::advance_lx_loading(&mut state));
        let frame = state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread()
            .expect("thread")
            .chat
            .loading_frame;
        assert_eq!(frame, before + 1);
    }

    #[test]
    fn backspace_removes_the_at_trigger_while_the_mention_panel_is_open() {
        let (mut state, pane) = btw_pane();
        state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_composer_mut()
            .expect("composer")
            .insert_char('@');
        assert!(
            state
                .pane_anywhere(pane)
                .expect("pane")
                .lx
                .btw
                .active_composer()
                .expect("composer")
                .mention_visible(),
            "输入 `@` 后面板应可见"
        );

        assert!(apply_input(&mut state, pane, LxInput::Backspace));
        let composer = state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_composer()
            .expect("composer");
        assert_eq!(composer.input, "", "退格必须能删掉触发用 `@`");
        assert!(!composer.mention_visible(), "删除后提及面板应关闭");
    }
}
