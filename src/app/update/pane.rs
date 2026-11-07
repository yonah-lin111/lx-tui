//! 窗格与终端生命周期、PTY 喂入与 lx 视图交互。

use std::path::Path;

use ratatui::layout::Rect;

use crate::app::actions::LxInput;
use crate::app::state::{AppState, Lx, PaneKind, PaneView};
use crate::detect::PaneAgentSnapshot;
use crate::layout::{self, PaneId};

use super::selection::{WHEEL_LINES, clear_selection};

mod popups;
mod task_page;

#[cfg(test)]
pub use super::chat::accept_slash_command;
pub use popups::{
    apply_lx_mention_entries, hover_lx_mention, hover_lx_paste, hover_lx_refer, hover_lx_slash,
    lx_mention_root_for_pane, scroll_lx_mention, scroll_lx_refer, scroll_lx_slash,
    select_lx_mention, select_lx_paste, select_lx_refer, select_lx_slash, switch_lx_mention_tab,
    sync_all_lx_mention_roots, sync_lx_mention_root, take_lx_mention_scan_requests,
};
pub use task_page::{apply_task_paste, select_task_agent_filter};

/// 按方向滚动终端窗格回滚视口（负数向上、正数向下）；返回视口是否移动。
pub fn scroll_pane(state: &mut AppState, id: PaneId, direction: isize) -> bool {
    scroll_pane_lines(state, id, direction, WHEEL_LINES)
}

/// 按方向滚动终端窗格指定行数；返回视口是否移动。
pub(crate) fn scroll_pane_lines(
    state: &mut AppState,
    id: PaneId,
    direction: isize,
    lines: isize,
) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.kind != PaneKind::Terminal {
        return false;
    }
    // 仿真器的 delta 正数指向历史（视口上移），与方向的上下语义相反。
    pane.terminal
        .scroll_display(-(direction.signum() * lines) as i32)
}

/// 键盘或粘贴输入后把窗格视口吸回最新输出；返回视口是否移动。
pub fn reset_pane_scroll(state: &mut AppState, id: PaneId) -> bool {
    state
        .pane_mut_anywhere(id)
        .is_some_and(|pane| pane.kind == PaneKind::Terminal && pane.terminal.scroll_to_bottom())
}

/// 滚轮滚动 lx 输入框视口（一格放大为 `WHEEL_LINES` 行）；返回视口是否移动。
pub fn scroll_lx_input(state: &mut AppState, id: PaneId, direction: isize) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.view != PaneView::Lx {
        return false;
    }
    pane.lx.scroll_by(direction.saturating_mul(WHEEL_LINES))
}

/// 设置 lx 输入框滚动条视口偏移（滚动条点击/拖拽）；返回视口是否移动。
pub fn set_lx_scroll(state: &mut AppState, id: PaneId, offset: usize) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    let before = pane.lx.scroll();
    pane.lx.scroll_to(offset);
    pane.lx.scroll() != before
}

/// 应用 lx 页输入命令到焦点窗格；非 lx 视图忽略。
///
/// 面板优先级：粘贴参照 > `@` 提及 > `/` 命令（model/resume/new） > md 块命令 >
/// 历史 prompt 切换；面板打开时其余按键按面板策略吞掉。
/// 提及面板打开时 `↑`/`↓` 移动高亮、`Tab`/`Enter` 采纳、`Shift+Enter` 进入目录、`Esc` 关闭。
/// 无面板时 `↑`/`↓` 先尝试历史切换（光标在文首/文末才接管），否则回落光标移动。
pub fn apply_lx_input(state: &mut AppState, command: LxInput) {
    if command == LxInput::Escape && super::escape_clear_selection(state) {
        return;
    }
    let pane_id = state.active_tab().layout.focus();
    // 主内容区或底部输入框存在非空选区（且非 /resume 历史面板）时，
    // Ctrl/Cmd+C 只负责复制选区（由事件层处理），不清空输入框草稿。
    if command == LxInput::Clear
        && state.pane_anywhere(pane_id).is_some_and(|pane| {
            !matches!(
                pane.lx.slash,
                Some(super::super::state::SlashPanel::History(_))
            ) && (pane.lx.text_selection_range().is_some()
                || pane.lx.preview_selection().is_some()
                || state
                    .lx_selection
                    .is_some_and(|sel| sel.pane() == pane_id && sel.range().is_some()))
        })
    {
        return;
    }
    // 后台任务页是窗格内的第二个页面：整页接管按键，不回落到输入框。
    if state
        .pane_anywhere(pane_id)
        .is_some_and(|pane| pane.lx.task.is_some())
    {
        task_page::apply_task_page_input(state, pane_id, command);
        return;
    }
    // 挂起的 ask 面板：整块接管按键（不回落到输入框，也不改草稿）。
    if state
        .pane_anywhere(pane_id)
        .is_some_and(|pane| pane.lx.ask.is_some())
    {
        super::chat::apply_ask_input(state, pane_id, command);
        return;
    }
    // 最新轮次撤销：执行中吞键；选中态接管 Enter/Esc/方向键；空闲空输入的双 Esc
    // 在这里待命/选中。动作面板由下方 `/` 面板路由接管。
    if matches!(
        super::chat::rewind::apply_input(state, pane_id, command),
        super::chat::rewind::Route::Consumed
    ) {
        return;
    }
    // 历史 prompt 切换：只读定出路由（要读全局历史 + 窗格编辑器，两个不可变借用），
    // 再单独应用，避免同时借用 `state` 的两个字段。
    match lx_history_route(state, pane_id, command) {
        LxHistoryRoute::Move(next) => {
            if let Some(pane) = state.pane_mut_anywhere(pane_id) {
                pane.lx.apply_history(next);
            }
            return;
        }
        // 浏览态 `Esc`：只退出浏览、不动文本，并吞掉该键（不改现有停止语义）。
        LxHistoryRoute::EndBrowse => {
            if let Some(pane) = state.pane_mut_anywhere(pane_id) {
                pane.lx.end_history_browse();
            }
            return;
        }
        // 编辑类命令：先退出浏览态，再按原语义继续处理。
        LxHistoryRoute::ClearBrowse => {
            if let Some(pane) = state.pane_mut_anywhere(pane_id) {
                pane.lx.end_history_browse();
            }
        }
        LxHistoryRoute::None => {}
    }
    let referenced_folders = state.referenced.folders.clone();
    let Some(pane) = state.active_pane_mut() else {
        return;
    };
    if pane.view != PaneView::Lx {
        return;
    }
    let lx = &mut pane.lx;
    lx.sync_referenced_folders(&referenced_folders);
    // 长文本粘贴面板优先：面板打开时仅响应上下移动、回车确认与 Esc 取消，其余按键吞掉。
    if lx.text_paste_panel().is_some() {
        match command {
            LxInput::Up => {
                lx.text_paste_move(-1);
            }
            LxInput::Down => {
                lx.text_paste_move(1);
            }
            LxInput::Enter => match lx
                .text_paste_panel()
                .and_then(|panel| panel.active_action())
            {
                Some(action) => {
                    lx.text_paste_confirm(action);
                }
                None => {
                    lx.text_paste_cancel();
                }
            },
            LxInput::Escape => {
                lx.text_paste_cancel();
            }
            _ => {}
        }
        return;
    }
    // 路径粘贴参照面板优先：与 md 块面板互斥；面板打开时其余按键按面板策略吞掉。
    if lx.paste_panel().is_some() {
        match command {
            LxInput::Up => {
                lx.paste_move(-1);
            }
            LxInput::Down => {
                lx.paste_move(1);
            }
            LxInput::Enter => match lx.paste_panel().and_then(|panel| panel.active_action()) {
                Some(action) => {
                    lx.paste_confirm(action);
                }
                None => {
                    lx.paste_cancel();
                }
            },
            LxInput::Escape => {
                lx.paste_cancel();
            }
            _ => {}
        }
        return;
    }
    // `@` 提及面板次优先：仅接管导航、左右切 Tab、采纳、进入目录与关闭；其它编辑键继续走编辑并重算触发。
    let mention_visible = lx.mention_visible();
    if mention_visible {
        match command {
            LxInput::Left => {
                lx.mention_switch_tab(-1);
                return;
            }
            LxInput::Right => {
                lx.mention_switch_tab(1);
                return;
            }
            LxInput::Escape => {
                lx.mention_escape();
                return;
            }
            LxInput::EnterFolder => {
                lx.mention_enter_folder();
                return;
            }
            _ => {}
        }
    }
    if lx.mention_open() {
        match command {
            LxInput::Up => {
                lx.mention_move(-1);
                return;
            }
            LxInput::Down => {
                lx.mention_move(1);
                return;
            }
            LxInput::Tab | LxInput::Enter => {
                lx.mention_confirm();
                return;
            }
            _ => {}
        }
    }
    // 快捷引用面板优先于斜杠面板：仅接管导航、采纳与关闭；其它编辑键继续走编辑并重算触发。
    if lx.refer_active() {
        match command {
            LxInput::Up => {
                lx.refer_move(-1);
                return;
            }
            LxInput::Down => {
                lx.refer_move(1);
                return;
            }
            LxInput::Tab | LxInput::Enter => {
                lx.refer_confirm();
                return;
            }
            LxInput::Escape => {
                lx.refer_escape();
                return;
            }
            _ => {}
        }
    }
    // `/` 命令面板：与 `@` 提及面板一致的交互——仅接管导航、采纳与关闭，
    // 其余编辑键**继续走下方编辑分支并重算面板**，因此可以删除 `/`（触发消失即关闭）
    // 或继续输入做模糊过滤。Model/Resume/History 二级面板仍吞掉编辑键（其内容不由草稿驱动）。
    if let Some(slash) = lx.slash.as_ref() {
        let move_panel = |lx: &mut super::super::state::Lx, delta: isize| {
            if let Some(panel) = lx.slash.as_mut() {
                panel.move_active(delta);
            }
        };
        let editable = matches!(slash, super::super::state::SlashPanel::Commands { .. });
        match command {
            LxInput::Up => {
                move_panel(lx, -1);
                return;
            }
            LxInput::Down => {
                move_panel(lx, 1);
                return;
            }
            LxInput::Enter | LxInput::Tab => {}
            LxInput::Escape => {
                match slash {
                    super::super::state::SlashPanel::Model(picker)
                        if matches!(
                            picker.stage,
                            super::super::state::ModelPickerStage::Variants { .. }
                        ) =>
                    {
                        // 第二级退回第一级：不清空输入框（它在「选中模型」时已被清空）。
                        // 按输入框现状重建过滤视图——清空后即全量，不留任何
                        // 用户看不见也清不掉的隐藏过滤状态。
                        let query = lx.input.clone();
                        if let Some(panel) = lx.slash.as_mut()
                            && let super::super::state::SlashPanel::Model(picker) = panel
                        {
                            let active = picker.window.active;
                            picker.stage = super::super::state::ModelPickerStage::Models;
                            // 先切回第一级，再按过滤词重算（`refilter` 只对 Models 生效）。
                            picker.refilter(&query);
                            let len = picker.filtered.len();
                            picker.window = super::super::state::PanelWindow::new(if len == 0 {
                                0
                            } else {
                                active.min(len - 1)
                            });
                        }
                    }
                    // 撤销面板 Esc：退回选中态（不清输入框、不取消目标）。
                    super::super::state::SlashPanel::Rewind(_) => {
                        lx.slash = None;
                        lx.rewind.panel = None;
                    }
                    // 真正关闭二级面板（`/model` 第一级、`/resume`、命令候选）：整框清空。
                    // 命令候选面板（`/`）不清：它的过滤词就是命令本身，由
                    // `accept_slash_command` 按行删除，`Esc` 关闭时草稿应原样保留。
                    super::super::state::SlashPanel::Commands { .. } => lx.slash = None,
                    _ => {
                        lx.slash = None;
                        lx.clear();
                    }
                }
                return;
            }
            // 命令候选面板：编辑键放行（继续输入过滤 / 删除 `/` 关闭）；二级面板同样放行，
            // 但过滤词改为输入框全文（见下方 `refresh_slash_filter`）。
            _ if editable => {}
            // `/resume` 历史面板：`Shift+Enter` 打开该会话的编辑模态（改名/删除）。
            // 必须排在「编辑键放行」之前：`EnterFolder` 也出现在 `edits_text` 里，
            // 放行会让它落进下方编辑分支而永远打不开模态。
            LxInput::EnterFolder
                if matches!(slash, super::super::state::SlashPanel::History(_)) =>
            {
                super::resume_edit::open_resume_session_edit(state, pane_id);
                return;
            }
            // `/resume` 历史面板：`Ctrl/Cmd+C` 复制高亮行的会话 id（不清空输入框）。
            LxInput::Clear if matches!(slash, super::super::state::SlashPanel::History(_)) => {
                super::resume_edit::copy_resume_session_id(state, pane_id);
                return;
            }
            // `/mcp` 面板：`Space` 原地切换当前高亮 MCP 启用/禁用（保持面板打开，不写入输入框）。
            LxInput::Char(' ') if matches!(slash, super::super::state::SlashPanel::Mcp(_)) => {
                super::chat::apply_slash_enter(state, pane_id);
                return;
            }
            // `/model`、`/resume`、`/mcp` 二级面板：其余编辑键与选区键放行，编辑后重算过滤
            // （`^C` 与 `⇧↵` 已在上方各自接管）。
            command
                if (edits_text(command) || is_select_command(command))
                    && command != LxInput::Clear => {}
            _ => return,
        }
        if command == LxInput::Enter || command == LxInput::Tab {
            super::chat::apply_slash_enter(state, pane_id);
            return;
        }
    }
    let panel_open = lx.panel.is_some();
    match command {
        LxInput::Up if panel_open => lx.panel_move(-1),
        LxInput::Down if panel_open => lx.panel_move(1),
        LxInput::Tab if panel_open => {
            lx.panel_accept();
        }
        LxInput::Enter if panel_open => {
            lx.panel_accept();
        }
        LxInput::Escape if panel_open => lx.panel_close(),
        // ↓ 落空：历史不接管、草稿为空、光标在末尾时进入后台任务页（对齐 opencode 的
        // `run() == false` 向下层派发）。有草稿时 ↓ 仍是光标/历史移动，不会突然换页。
        LxInput::Down if lx.input.trim().is_empty() && lx.cursor == lx.input.len() => {
            super::chat::open_task_page(state, pane_id);
        }
        LxInput::Down => lx.move_down(),
        // 剪切由事件层完成（需要系统剪贴板）；此处保留为无副作用占位。
        LxInput::Cut => {}
        // 翻页只在后台任务面板里有对象；聊天页不消费。
        LxInput::PageUp | LxInput::PageDown => {}
        // 停止进程只在后台任务面板里有对象；聊天页不消费（Ctrl+D 原本也无动作）。
        LxInput::StopProcess => {}
        command if is_select_command(command) => {
            if command == LxInput::SelectAll || !lx.has_text_selection() {
                clear_selection(state);
            }
            if let Some(target) = state.pane_mut_anywhere(pane_id) {
                apply_editor_edit(&mut target.lx, command);
            }
        }
        // 纯文本编辑（ask 面板的编辑态复用同一套逻辑）；未改写的命令落到下面专用分支。
        _ if apply_editor_edit(lx, command) => {
            after_editor_edit(state, pane_id, command);
        }
        // 无面板且 Native 空闲时 Enter 发送；流中不重复发送。
        LxInput::Enter => {
            super::chat::try_send(state, pane_id);
        }
        // 无面板：先清理展开芯片选区或收起全部展开芯片；再判空 Shell 退出或停止待命。
        LxInput::Escape => {
            if lx.clear_preview_selection() || lx.clear_expanded_pastes() {
                return;
            }
            let idle_shell = state.pane_anywhere(pane_id).is_some_and(|target| {
                target.lx.shell_mode()
                    // 与提交/排队的判空口径一致：只有空格也算空命令。
                    && target.lx.input.trim().is_empty()
                    && !target.lx.chat.request.is_busy()
                    && !target.lx.chat.has_running_shell()
            });
            if idle_shell {
                if let Some(target) = state.pane_mut_anywhere(pane_id) {
                    target.lx.exit_shell();
                }
            } else {
                super::chat::escape_stop(state, pane_id);
            }
        }
        // Ctrl+T：切换最近 thinking 块。
        LxInput::ToggleThinking => {
            super::chat::toggle_latest_thinking(state, pane_id);
        }
        // Ctrl+O：打开后台任务面板并定位到当前（或最近一张）进程卡片；
        // Alt+↑/↓ 切换键盘选择。三者都只改窗格内的展示状态，不转发给后台 shell。
        LxInput::ToggleToolDetail => {
            super::chat::open_task_for_selected_tool(state, pane_id);
        }
        LxInput::SelectPrevTool => {
            super::chat::select_adjacent_tool(state, pane_id, false);
        }
        LxInput::SelectNextTool => {
            super::chat::select_adjacent_tool(state, pane_id, true);
        }
        // 忙时 Tab 把 shell 草稿排队；空闲时保留输入（不执行）。
        LxInput::Tab => {
            super::chat::queue_shell(state, pane_id);
        }
        // 上面带守卫的编辑分支已覆盖全部编辑命令；这里只兜住剩余无动作的键。
        _ => {}
    }
}

/// 编辑后的副作用：改写草稿时重算 `/` 命令候选面板。
///
/// 纯导航（`Left`/`Right`/`Home`/`End`/词移动）与换行都不触发——现行为如此。
fn after_editor_edit(state: &mut AppState, pane_id: PaneId, command: LxInput) {
    let rewrote =
        edits_text(command) && !matches!(command, LxInput::Newline | LxInput::EnterFolder);
    if rewrote {
        super::chat::refresh_slash_panel(state, pane_id);
    }
}

/// 纯文本编辑命令的接收者：lx 输入框与 ask 手写缓冲共用**同一份**编辑映射。
///
/// 抽成 trait 而不是给 `AskEditor` 加 `DerefMut`：手写缓冲只允许编辑，不允许外面
/// 顺手写 `chat`/`rewind` 等字段（design §4.5）。
pub(crate) trait TextEdit {
    fn insert_char(&mut self, ch: char);
    fn newline(&mut self);
    fn backspace(&mut self);
    fn delete(&mut self);
    fn move_left(&mut self);
    fn move_right(&mut self);
    fn move_up(&mut self);
    fn home(&mut self);
    fn end(&mut self);
    fn word_left(&mut self);
    fn word_right(&mut self);
    fn select_left(&mut self);
    fn select_right(&mut self);
    fn select_word_left(&mut self);
    fn select_word_right(&mut self);
    fn select_home(&mut self);
    fn select_end(&mut self);
    fn select_up(&mut self);
    fn select_down(&mut self);
    fn select_all(&mut self);
    fn doc_start(&mut self);
    fn doc_end(&mut self);
    fn select_doc_start(&mut self);
    fn select_doc_end(&mut self);
    fn delete_to_line_start(&mut self);
    fn delete_to_line_end(&mut self);
    fn delete_word_backward(&mut self);
    fn delete_word_forward(&mut self);
    fn undo(&mut self);
    fn redo(&mut self);
    fn clear(&mut self);
}

macro_rules! impl_text_edit {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl TextEdit for $ty {
                fn insert_char(&mut self, ch: char) { <$ty>::insert_char(self, ch); }
                fn newline(&mut self) { <$ty>::newline(self); }
                fn backspace(&mut self) { <$ty>::backspace(self); }
                fn delete(&mut self) { <$ty>::delete(self); }
                fn move_left(&mut self) { <$ty>::move_left(self); }
                fn move_right(&mut self) { <$ty>::move_right(self); }
                fn move_up(&mut self) { <$ty>::move_up(self); }
                fn home(&mut self) { <$ty>::home(self); }
                fn end(&mut self) { <$ty>::end(self); }
                fn word_left(&mut self) { <$ty>::word_left(self); }
                fn word_right(&mut self) { <$ty>::word_right(self); }
                fn select_left(&mut self) { <$ty>::select_left(self); }
                fn select_right(&mut self) { <$ty>::select_right(self); }
                fn select_word_left(&mut self) { <$ty>::select_word_left(self); }
                fn select_word_right(&mut self) { <$ty>::select_word_right(self); }
                fn select_home(&mut self) { <$ty>::select_home(self); }
                fn select_end(&mut self) { <$ty>::select_end(self); }
                fn select_up(&mut self) { <$ty>::select_up(self); }
                fn select_down(&mut self) { <$ty>::select_down(self); }
                fn select_all(&mut self) { <$ty>::select_all(self); }
                fn doc_start(&mut self) { <$ty>::doc_start(self); }
                fn doc_end(&mut self) { <$ty>::doc_end(self); }
                fn select_doc_start(&mut self) { <$ty>::select_doc_start(self); }
                fn select_doc_end(&mut self) { <$ty>::select_doc_end(self); }
                fn delete_to_line_start(&mut self) { <$ty>::delete_to_line_start(self); }
                fn delete_to_line_end(&mut self) { <$ty>::delete_to_line_end(self); }
                fn delete_word_backward(&mut self) { <$ty>::delete_word_backward(self); }
                fn delete_word_forward(&mut self) { <$ty>::delete_word_forward(self); }
                fn undo(&mut self) { <$ty>::undo(self); }
                fn redo(&mut self) { <$ty>::redo(self); }
                fn clear(&mut self) { <$ty>::clear(self); }
            }
        )+
    };
}

impl_text_edit!(Lx, crate::app::state::AskEditor);

/// 纯文本编辑分派（不含面板/发送/停止语义）；返回是否改写了文本。
///
/// `Down` 特意不在此列：无面板且草稿为空时它要落到「打开后台任务页」，
/// 有草稿时才下调光标——那段路由留在调用点。
pub(crate) fn apply_editor_edit<T: TextEdit>(target: &mut T, command: LxInput) -> bool {
    match command {
        LxInput::Char(ch) => target.insert_char(ch),
        // 提及面板未打开时 Shift+Enter 回落为换行。
        LxInput::Newline | LxInput::EnterFolder => target.newline(),
        LxInput::Backspace => target.backspace(),
        LxInput::Delete => target.delete(),
        LxInput::Left => target.move_left(),
        LxInput::Right => target.move_right(),
        LxInput::Up => target.move_up(),
        LxInput::Home | LxInput::LineStart => target.home(),
        LxInput::End | LxInput::LineEnd => target.end(),
        LxInput::WordLeft => target.word_left(),
        LxInput::WordRight => target.word_right(),
        LxInput::SelectLeft => target.select_left(),
        LxInput::SelectRight => target.select_right(),
        LxInput::SelectWordLeft => target.select_word_left(),
        LxInput::SelectWordRight => target.select_word_right(),
        LxInput::SelectHome => target.select_home(),
        LxInput::SelectEnd => target.select_end(),
        LxInput::SelectUp => target.select_up(),
        LxInput::SelectDown => target.select_down(),
        LxInput::SelectAll => target.select_all(),
        LxInput::DocStart => target.doc_start(),
        LxInput::DocEnd => target.doc_end(),
        LxInput::SelectDocStart => target.select_doc_start(),
        LxInput::SelectDocEnd => target.select_doc_end(),
        LxInput::DeleteToLineStart => target.delete_to_line_start(),
        LxInput::DeleteToLineEnd => target.delete_to_line_end(),
        LxInput::DeleteWordBackward => target.delete_word_backward(),
        LxInput::DeleteWordForward => target.delete_word_forward(),
        LxInput::Undo => target.undo(),
        LxInput::Redo => target.redo(),
        LxInput::Clear => target.clear(),
        _ => return false,
    }
    true
}

/// 是否为键盘扩展/全选选区命令。
pub(crate) fn is_select_command(command: LxInput) -> bool {
    matches!(
        command,
        LxInput::SelectLeft
            | LxInput::SelectRight
            | LxInput::SelectWordLeft
            | LxInput::SelectWordRight
            | LxInput::SelectHome
            | LxInput::SelectEnd
            | LxInput::SelectUp
            | LxInput::SelectDown
            | LxInput::SelectAll
            | LxInput::SelectDocStart
            | LxInput::SelectDocEnd
    )
}

/// 历史 prompt 与当前按键的路由结果。
enum LxHistoryRoute {
    /// 由历史接管：应用这次切换。
    Move(crate::app::state::lx::HistoryMove),
    /// 浏览态 `Esc`：退出浏览并吞掉该键。
    EndBrowse,
    /// 编辑类命令：先退出浏览，再按原语义继续。
    ClearBrowse,
    /// 与历史无关；按原语义继续。
    None,
}

/// 只读判定历史路由：面板优先级高于历史，且 `↑`/`↓` 只在光标位于文首/文末时接管。
///
/// 面板（粘贴参照 / `@` 提及 / `/` 命令 / md 块）任一打开时方向键归面板，历史一律不接管，
/// 因此这里先把"面板占用"和"历史接管"分开判断，避免面板抢键失败后又被历史接走。
fn lx_history_route(state: &AppState, pane: PaneId, command: LxInput) -> LxHistoryRoute {
    let Some(pane) = state.pane_anywhere(pane) else {
        return LxHistoryRoute::None;
    };
    if pane.view != PaneView::Lx {
        return LxHistoryRoute::None;
    }
    let lx = &pane.lx;
    // 与 `apply_lx_input` 完全一致的面板占用判定：任何面板可见都不让历史接管方向键。
    let panel_busy = lx.paste_panel().is_some()
        || lx.text_paste_panel().is_some()
        || lx.refer_active()
        || lx.mention_open()
        || lx.mention_visible()
        || lx.slash.is_some()
        || lx.panel.is_some();
    if panel_busy {
        return LxHistoryRoute::None;
    }
    match command {
        LxInput::Up => match lx.pending_history_move(state.prompt_history.entries(), -1) {
            Some(next) => LxHistoryRoute::Move(next),
            None => LxHistoryRoute::None,
        },
        LxInput::Down => match lx.pending_history_move(state.prompt_history.entries(), 1) {
            Some(next) => LxHistoryRoute::Move(next),
            None => LxHistoryRoute::None,
        },
        LxInput::Escape if lx.history_browsing() => LxHistoryRoute::EndBrowse,
        // 其余编辑类命令离开浏览态；导航类（Left/Right/Home/End/词移动）只移动光标，
        // 不改变文本，因此保留浏览态（继续 `↑`/`↓` 仍沿同一条历史前后翻）。
        command if edits_text(command) => LxHistoryRoute::ClearBrowse,
        _ => LxHistoryRoute::None,
    }
}

/// 该命令是否会改写输入框文本；改写即视为"离开浏览态"。
///
/// 纯光标导航（`Left`/`Right`/`Home`/`End`/词移动）、`Tab`（无面板时无动作）与
/// `Enter`（发送）都不改写文本，因此不在此列：浏览游标保留。
fn edits_text(command: LxInput) -> bool {
    matches!(
        command,
        LxInput::Char(_)
            | LxInput::Newline
            | LxInput::EnterFolder
            | LxInput::Backspace
            | LxInput::Delete
            | LxInput::DeleteToLineStart
            | LxInput::DeleteToLineEnd
            | LxInput::DeleteWordBackward
            | LxInput::DeleteWordForward
            | LxInput::Undo
            | LxInput::Redo
            | LxInput::Clear
            | LxInput::Cut
    )
}

/// 切换 lx 页选中入口（仅鼠标点击触发）；切换入口会取消在途粘贴面板并隔离跨模式会话。
pub fn set_lx_entry(state: &mut AppState, id: PaneId, entry: crate::app::state::LxEntry) {
    let crossing_ai_web = state.pane_anywhere(id).is_some_and(|pane| {
        pane.lx.entry != entry
            && (pane.lx.entry == crate::app::state::LxEntry::AiWeb
                || entry == crate::app::state::LxEntry::AiWeb)
    });
    if crossing_ai_web
        && state
            .pane_anywhere(id)
            .is_some_and(|pane| pane.lx.chat.request.is_busy())
    {
        let _ = super::chat::stop_stream(state, id);
    }
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.text_paste_cancel();
        pane.lx.paste_cancel();
        if pane.lx.entry != entry {
            pane.lx.slash = None;
        }
        if crossing_ai_web {
            let incompatible = pane
                .lx
                .chat
                .session_id
                .as_deref()
                .is_some_and(|session_id| {
                    let is_ai = super::chat::is_ai_web_session(session_id);
                    if entry == crate::app::state::LxEntry::AiWeb {
                        !is_ai
                    } else {
                        is_ai
                    }
                });
            if incompatible {
                pane.lx.chat.reset_session_view();
            }
            pane.lx.clear_rewind();
            pane.lx.ai_web_edit_user_index = None;
            pane.lx.ai_web_edit_cloud_files.clear();
            pane.lx.chat.loading_session = false;
        }
        pane.lx.entry = entry;
        if entry == crate::app::state::LxEntry::AiWeb {
            pane.lx
                .mention_force_tab(crate::app::prompt::MentionTab::Files);
            pane.lx.exit_shell_mode();
            pane.lx.refresh_refer_panel();
        }
    }
    if crossing_ai_web {
        super::selection::clear_lx_selection_for(state, id);
    }
    super::chat::refresh_lx_status(state, id);
}

/// 切换 AI Web 目标站点；切换站点时停止在途流、清空跨站点残留会话与编辑态，并更新底边框状态行。
pub fn set_ai_web_site(state: &mut AppState, id: PaneId, site: crate::app::state::AiWebSite) {
    let changed = state
        .pane_anywhere(id)
        .is_some_and(|pane| pane.lx.ai_web_site != site);
    if !changed {
        return;
    }
    if state
        .pane_anywhere(id)
        .is_some_and(|pane| pane.lx.chat.request.is_busy())
    {
        let _ = super::chat::stop_stream(state, id);
    }
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.ai_web_site = site;
        pane.lx.chat.reset_session_view();
        pane.lx.clear_rewind();
        pane.lx.ai_web_edit_user_index = None;
        pane.lx.ai_web_edit_cloud_files.clear();
        pane.lx.chat.loading_session = false;
        pane.lx.slash = None;
    }
    super::selection::clear_lx_selection_for(state, id);
    super::chat::refresh_lx_status(state, id);
}

/// 切换窗格主内容视图（lx 页 ↔ 终端）；返回是否变化。
///
/// 切到 lx 时清掉该窗格上的终端选区与滚动条拖拽：隐藏终端不接受交互。
pub fn toggle_pane_view(state: &mut AppState, id: PaneId) -> bool {
    let Some(view) = state.active_tab_mut().pane_mut(id).map(|pane| {
        pane.view = match pane.view {
            PaneView::Lx => PaneView::Terminal,
            PaneView::Terminal => PaneView::Lx,
        };
        // 切换视图时取消在途粘贴面板：隐藏的 lx 页不再持有悬空面板。
        pane.lx.text_paste_cancel();
        pane.lx.paste_cancel();
        // 隐藏的 lx 页不得保留可点击的提及面板或 `/mcp` 面板。
        pane.lx.mention_escape();
        if matches!(pane.lx.slash, Some(super::super::state::SlashPanel::Mcp(_))) {
            pane.lx.slash = None;
            pane.lx.clear();
        }
        // 视图切换时清理旧视图残留的窗格内 toast，避免冻结态残留。
        pane.lx.toast = None;
        pane.view
    }) else {
        return false;
    };
    if state.toast.as_ref().and_then(|toast| toast.anchor) == Some(id) {
        state.toast = None;
    }
    super::selection::clear_lx_selection_for(state, id);
    if view == PaneView::Lx {
        if state.terminal_selection == Some(id) {
            state.terminal_selection = None;
        }
        if let Some(pane) = state.pane_mut_anywhere(id) {
            pane.terminal.clear_selection();
        }
        if state
            .terminal_scroll_drag
            .is_some_and(|(pane, _)| pane == id)
        {
            state.terminal_scroll_drag = None;
        }
        // 恢复 lx 视图：按当前根与输入重新评估提及面板（在途结果已写入缓存）。
        sync_lx_mention_root(state, id);
        if let Some(pane) = state.pane_mut_anywhere(id) {
            pane.lx.refresh_panel();
        }
    }
    true
}

/// 按几何同步各窗格终端与 prompt 编辑器的尺寸。
///
/// prompt 文本区固定预留滚动条槽；尺寸变化会让视口选区坐标失效，此时清除选区。
pub fn resize_panes(state: &mut AppState, pane_rects: &[(PaneId, Rect)]) {
    for (id, rect) in pane_rects {
        if *id == state.prompt.id() {
            let (recent_open, recent_count) = {
                let panel = &state.prompt_owner().prompt_panel;
                (
                    panel.recent_open,
                    crate::app::prompt::recent_page_count(panel),
                )
            };
            let split = layout::prompt_body_split(*rect, recent_open, recent_count);
            if recent_open && split.recent_divider.is_none() {
                super::close_prompt_recent(state);
            }
            let (cols, rows) = (split.main.width.max(1), split.main.height.max(1));
            if state.prompt.size() != (cols, rows) {
                clear_selection(state);
            }
            state.prompt.resize(cols, rows);
            let workspace = state.prompt_workspace();
            if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
                workspace_state.prompt_panel.recent_visible = split.visible_entries;
            }
            state.prompt_resize_pending = false;
        } else if let Some(pane) = state.pane_mut_anywhere(*id) {
            let (cols, rows) = layout::pane_inner_size(*rect);
            let old_size = pane.terminal.size();
            pane.terminal.resize(cols, rows);
            let (lx_cols, lx_rows) = layout::lx_input_size(*rect);
            pane.lx.resize(lx_cols, lx_rows);
            if let Some(ask) = pane.lx.ask.as_mut() {
                let (width, rows) = layout::lx_ask_editor_size(*rect);
                ask.resize_editor(width, rows);
            }
            if (old_size.cols, old_size.rows) != (cols, rows) {
                for run in &mut pane.lx.agents.runs {
                    run.rows_dirty = true;
                }
                super::selection::clear_lx_selection_for(state, *id);
                if state.terminal_selection == Some(*id) {
                    state.terminal_selection = None;
                    state.selection_autoscroll = None;
                }
            }
        }
    }
}

/// 喂入窗格输出；返回需要写回 PTY 的响应。
pub fn feed_pane(state: &mut AppState, id: PaneId, bytes: &[u8]) -> Vec<u8> {
    state
        .pane_mut_anywhere(id)
        .map(|pane| pane.terminal.feed(bytes))
        .unwrap_or_default()
}

/// 标记窗格进程已退出；保留最后一屏。
pub fn mark_pane_exited(state: &mut AppState, id: PaneId) {
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.exited = true;
    }
}

/// 更新窗格 shell cwd（路径与标题标签）；任一变化返回 true（供置脏重绘）。
pub fn update_pane_cwd(state: &mut AppState, id: PaneId, cwd: &Path, label: String) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.cwd.as_deref() == Some(cwd) && pane.cwd_label.as_deref() == Some(label.as_str()) {
        return false;
    }
    pane.cwd = Some(cwd.to_path_buf());
    pane.cwd_label = Some(label);
    true
}

/// 更新窗格 Agent 快照（None 移除条目）；有变化返回 true（供置脏重绘）。
pub fn update_pane_agent(
    state: &mut AppState,
    id: PaneId,
    snapshot: Option<PaneAgentSnapshot>,
) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.agent == snapshot {
        return false;
    }
    pane.agent = snapshot;
    true
}

/// 点击 Agents 条目：切到所属标签、聚焦窗格、lx 视图切回终端、收回 prompt 焦点；返回是否成功。
pub fn focus_agent_pane(state: &mut AppState, pane: PaneId) -> bool {
    let Some(tab_index) = state
        .active_workspace()
        .tabs
        .iter()
        .position(|tab| tab.pane(pane).is_some())
    else {
        return false;
    };
    state.active_workspace_mut().active_tab = tab_index;
    if let Some(target) = state.active_tab_mut().pane_mut(pane)
        && target.view == PaneView::Lx
    {
        target.view = PaneView::Terminal;
        target.lx.toast = None;
    }
    super::focus::focus_pane(state, pane);
    clear_selection(state);
    true
}

/// 把终端窗格视口移动到距内容顶部 `offset` 行（0 为最旧一屏）；返回视口是否移动。
pub fn set_terminal_scroll(state: &mut AppState, pane: PaneId, offset: usize) -> bool {
    state.pane_mut_anywhere(pane).is_some_and(|target| {
        target.kind == PaneKind::Terminal && target.terminal.scroll_to_content_offset(offset)
    })
}
