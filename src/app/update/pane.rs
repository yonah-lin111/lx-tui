//! 窗格与终端生命周期、PTY 喂入与 lx 视图交互。

use std::path::Path;

use ratatui::layout::Rect;

use crate::app::actions::LxInput;
use crate::app::state::{AppState, Lx, PaneKind, PaneView};
use crate::detect::PaneAgentSnapshot;
use crate::layout::{self, PaneId};

use super::prompt::cancel_page_rename;
use super::selection::{WHEEL_LINES, clear_selection, clear_terminal_selection};
use super::todo::commit_todo_edit;

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

/// 点击终端窗格：焦点回到窗格，prompt 失焦并关闭块命令面板与页重命名输入。
pub fn focus_pane(state: &mut AppState, id: PaneId) {
    commit_todo_edit(state);
    state.todo_focused = false;
    state.prompt_focused = false;
    state.prompt.clear_panel();
    cancel_page_rename(state);
    state.active_tab_mut().layout.focus_pane(id);
}

/// 应用 lx 页输入命令到焦点窗格；非 lx 视图忽略。
///
/// 面板优先级：粘贴参照 > `@` 提及 > `/` 命令（model/resume/new） > md 块命令 >
/// 历史 prompt 切换；面板打开时其余按键按面板策略吞掉。
/// 提及面板打开时 `↑`/`↓` 移动高亮、`Tab`/`Enter` 采纳、`Shift+Enter` 进入目录、`Esc` 关闭。
/// 无面板时 `↑`/`↓` 先尝试历史切换（光标在文首/文末才接管），否则回落光标移动。
pub fn apply_lx_input(state: &mut AppState, command: LxInput) {
    let pane_id = state.active_tab().layout.focus();
    // 后台任务页是窗格内的第二个页面：整页接管按键，不回落到输入框。
    if state
        .pane_anywhere(pane_id)
        .is_some_and(|pane| pane.lx.task.is_some())
    {
        apply_task_page_input(state, pane_id, command);
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
    let Some(pane) = state.active_pane_mut() else {
        return;
    };
    if pane.view != PaneView::Lx {
        return;
    }
    let lx = &mut pane.lx;
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
    // `@` 提及面板次优先：仅接管导航、采纳、进入目录与关闭；其它编辑键继续走编辑并重算触发。
    let mention_visible = lx.mention_visible();
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
            LxInput::Escape => {
                lx.mention_escape();
                return;
            }
            // Shift+Enter 由下方 mention_visible 分支统一处理。
            LxInput::EnterFolder => {
                lx.mention_enter_folder();
                return;
            }
            _ => {}
        }
    }
    // 面板可见（含扫描中/无匹配）时 Shift+Enter 吞键：不进入目录也不换行。
    if mention_visible && command == LxInput::EnterFolder {
        return;
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
            // `/model`、`/resume` 二级面板：其余编辑键放行，编辑后重算过滤
            // （`^C` 与 `⇧↵` 已在上方各自接管）。
            command if edits_text(command) && command != LxInput::Clear => {}
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
        // 翻页只在后台任务面板里有对象；聊天页不消费。
        LxInput::PageUp | LxInput::PageDown => {}
        // 停止进程只在后台任务面板里有对象；聊天页不消费（Ctrl+D 原本也无动作）。
        LxInput::StopProcess => {}
        // 纯文本编辑（ask 面板的编辑态复用同一套逻辑）；未改写的命令落到下面专用分支。
        _ if apply_editor_edit(lx, command) => {
            after_editor_edit(state, pane_id, command);
        }
        // 无面板且 Native 空闲时 Enter 发送；流中不重复发送。
        LxInput::Enter => {
            super::chat::try_send(state, pane_id);
        }
        // 无面板：空 Shell 模式且无活动任务时 Esc 退出模式；否则走停止待命。
        LxInput::Escape => {
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
        refresh_slash_panel(state, pane_id);
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
    fn delete_to_line_start(&mut self);
    fn delete_to_line_end(&mut self);
    fn delete_word_backward(&mut self);
    fn delete_word_forward(&mut self);
    fn undo(&mut self);
    fn redo(&mut self);
    fn clear(&mut self);
}

impl TextEdit for Lx {
    fn insert_char(&mut self, ch: char) {
        Lx::insert_char(self, ch);
    }
    fn newline(&mut self) {
        Lx::newline(self);
    }
    fn backspace(&mut self) {
        Lx::backspace(self);
    }
    fn delete(&mut self) {
        Lx::delete(self);
    }
    fn move_left(&mut self) {
        Lx::move_left(self);
    }
    fn move_right(&mut self) {
        Lx::move_right(self);
    }
    fn move_up(&mut self) {
        Lx::move_up(self);
    }
    fn home(&mut self) {
        Lx::home(self);
    }
    fn end(&mut self) {
        Lx::end(self);
    }
    fn word_left(&mut self) {
        Lx::word_left(self);
    }
    fn word_right(&mut self) {
        Lx::word_right(self);
    }
    fn delete_to_line_start(&mut self) {
        Lx::delete_to_line_start(self);
    }
    fn delete_to_line_end(&mut self) {
        Lx::delete_to_line_end(self);
    }
    fn delete_word_backward(&mut self) {
        Lx::delete_word_backward(self);
    }
    fn delete_word_forward(&mut self) {
        Lx::delete_word_forward(self);
    }
    fn undo(&mut self) {
        Lx::undo(self);
    }
    fn redo(&mut self) {
        Lx::redo(self);
    }
    fn clear(&mut self) {
        Lx::clear(self);
    }
}

impl TextEdit for crate::app::state::AskEditor {
    fn insert_char(&mut self, ch: char) {
        crate::app::state::AskEditor::insert_char(self, ch);
    }
    fn newline(&mut self) {
        crate::app::state::AskEditor::newline(self);
    }
    fn backspace(&mut self) {
        crate::app::state::AskEditor::backspace(self);
    }
    fn delete(&mut self) {
        crate::app::state::AskEditor::delete(self);
    }
    fn move_left(&mut self) {
        crate::app::state::AskEditor::move_left(self);
    }
    fn move_right(&mut self) {
        crate::app::state::AskEditor::move_right(self);
    }
    fn move_up(&mut self) {
        crate::app::state::AskEditor::move_up(self);
    }
    fn home(&mut self) {
        crate::app::state::AskEditor::home(self);
    }
    fn end(&mut self) {
        crate::app::state::AskEditor::end(self);
    }
    fn word_left(&mut self) {
        crate::app::state::AskEditor::word_left(self);
    }
    fn word_right(&mut self) {
        crate::app::state::AskEditor::word_right(self);
    }
    fn delete_to_line_start(&mut self) {
        crate::app::state::AskEditor::delete_to_line_start(self);
    }
    fn delete_to_line_end(&mut self) {
        crate::app::state::AskEditor::delete_to_line_end(self);
    }
    fn delete_word_backward(&mut self) {
        crate::app::state::AskEditor::delete_word_backward(self);
    }
    fn delete_word_forward(&mut self) {
        crate::app::state::AskEditor::delete_word_forward(self);
    }
    fn undo(&mut self) {
        crate::app::state::AskEditor::undo(self);
    }
    fn redo(&mut self) {
        crate::app::state::AskEditor::redo(self);
    }
    fn clear(&mut self) {
        crate::app::state::AskEditor::clear(self);
    }
}

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
    )
}

/// 编辑后刷新 `/` 命令候选面板：行首命令位置输入 `/` 时打开/过滤。
fn refresh_slash_panel(state: &mut AppState, pane: PaneId) {
    // Shell 模式：`/` 是命令字面值，不打开任何内置面板。
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.shell_mode())
    {
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.slash = None;
        }
        return;
    }
    // 二级面板（`/model`、`/resume`）的过滤词是**输入框全文**：任何编辑后都重算一次。
    refresh_slash_filter(state, pane);
    let query = state.pane_anywhere(pane).and_then(|target| {
        super::super::state::lx::slash::slash_trigger(&target.lx.input, target.lx.cursor)
    });
    let Some(query) = query else {
        // 离开命令位置时关闭命令候选面板（model/resume 面板不受影响）。
        if let Some(target) = state.pane_mut_anywhere(pane)
            && matches!(
                target.lx.slash,
                Some(super::super::state::SlashPanel::Commands { .. })
            )
        {
            target.lx.slash = None;
        }
        return;
    };
    // `/` 前缀或更具体前缀：仅命令候选面板实时过滤。
    let matching = super::super::state::lx::slash::SlashPanel::commands_matching(&query);
    if let Some(target) = state.pane_mut_anywhere(pane) {
        if matching.is_empty() {
            if matches!(
                target.lx.slash,
                Some(super::super::state::SlashPanel::Commands { .. })
            ) {
                target.lx.slash = None;
            }
            return;
        }
        // 高亮保持或钳到范围；同一起点时保留窗口（含滚轮视口）。
        let mut window = match &target.lx.slash {
            Some(super::super::state::SlashPanel::Commands { window, .. }) => *window,
            _ => super::super::state::lx::slash::PanelWindow::default(),
        };
        window.clamp(matching.len());
        target.lx.slash = Some(super::super::state::SlashPanel::Commands {
            items: matching,
            window,
        });
    }
}

/// 按输入框全文重算二级面板（`/model`、`/resume`）的过滤视图。
///
/// 只在相应面板打开时生效；`CommandPanelView` 候选面板由 `refresh_slash_panel`
/// 自己按行首 `/...` 过滤，不走这里。过滤词是**输入框全文**（含其它行草稿），
/// 与用户在编辑区看到的内容一致，不做任何暂存/恢复。
fn refresh_slash_filter(state: &mut AppState, pane: PaneId) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    let query = target.lx.input.clone();
    match target.lx.slash.as_mut() {
        Some(super::super::state::SlashPanel::History(panel)) => panel.refilter(&query),
        Some(super::super::state::SlashPanel::Model(picker)) => picker.refilter(&query),
        _ => {}
    }
}

/// 采纳 `/` 命令候选：消费整条命令文本（不写入草稿或 DB）。
pub fn accept_slash_command(state: &mut AppState, pane: PaneId) -> bool {
    // 只读取出候选与输入，避免在调用 update 函数时仍持有窗格可变借用。
    let Some((command, input, cursor)) = state.pane_anywhere(pane).and_then(|target| {
        let Some(super::super::state::SlashPanel::Commands { items, window }) =
            target.lx.slash.clone()
        else {
            return None;
        };
        items
            .get(window.active)
            .copied()
            .map(|command| (command, target.lx.input.clone(), target.lx.cursor))
    }) else {
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.slash = None;
        }
        return false;
    };
    // 撤销命令必须独立占满输入：夹带正文/多行/参数/光标前后残留时拒绝并保留草稿。
    if command == super::super::state::SlashCommand::Rewind {
        let trimmed = input.trim();
        let line_start = input[..cursor].rfind('\n').map_or(0, |offset| offset + 1);
        let line = input[line_start..].trim();
        let standalone = trimmed == line
            && input[cursor..].trim().is_empty()
            && !line.contains('\n')
            && !line.contains('\r')
            && !line.contains(char::is_whitespace)
            && line.starts_with('/');
        if !standalone {
            super::chat::show_lx_hint(
                state,
                pane,
                super::super::state::InputHintLevel::Warning,
                crate::ui::text::LX_REWIND_FAILED,
            );
            return true;
        }
        // 命令文本整条消费（不进入历史/撤销栈）。
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.input.clear();
            target.lx.cursor = 0;
            target.lx.slash = None;
        }
        super::chat::apply_slash_command(state, pane, command);
        return true;
    }
    let line_start = input[..cursor].rfind('\n').map_or(0, |offset| offset + 1);
    // 消费整条命令：删除该行命令文本。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.input.replace_range(line_start..cursor, "");
        target.lx.cursor = line_start;
        target.lx.slash = None;
    }
    super::chat::apply_slash_command(state, pane, command);
    true
}

/// 切换 lx 页选中入口（仅鼠标点击触发）；切换入口会取消在途粘贴面板。
pub fn set_lx_entry(state: &mut AppState, id: PaneId, entry: crate::app::state::LxEntry) {
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.paste_cancel();
        pane.lx.entry = entry;
    }
}

/// 单一窗格的有效 `@` 扫描根：`pane.cwd` 优先，回退所属工作区 cwd；两者皆无时 None。
///
/// 不使用当前进程 cwd、右侧 prompt 根或其它 pane 的 cwd。
pub fn lx_mention_root_for_pane(state: &AppState, id: PaneId) -> Option<std::path::PathBuf> {
    let pane = state.pane_anywhere(id)?;
    pane.cwd
        .clone()
        .or_else(|| state.workspace_cwd_for_pane(id))
}

/// 为指定窗格同步 `@` 提及扫描根；根变化返回 true（供置脏）。
pub fn sync_lx_mention_root(state: &mut AppState, id: PaneId) -> bool {
    let root = lx_mention_root_for_pane(state, id);
    state
        .pane_mut_anywhere(id)
        .is_some_and(|pane| pane.lx.set_mention_root(root))
}

/// 为全部 lx 视图窗格同步 `@` 提及扫描根；任一根变化返回 true。
pub fn sync_all_lx_mention_roots(state: &mut AppState) -> bool {
    let mut changed = false;
    for id in state.all_pane_ids() {
        if state
            .pane_anywhere(id)
            .is_some_and(|pane| pane.view == PaneView::Lx)
        {
            changed |= sync_lx_mention_root(state, id);
        }
    }
    changed
}

/// 取走当前可见窗格待发起的提及扫描请求（窗格、代号、根）；仅事件层调用。
///
/// 只对当前标签（可见）中处于 lx 视图、有有效 `@` 触发器且无在途/缓存的窗格发起；
/// 不可见窗格的根仍会被同步，恢复可见时再发起扫描。
pub fn take_lx_mention_scan_requests(
    state: &mut AppState,
) -> Vec<(PaneId, u64, std::path::PathBuf)> {
    let visible = state.active_tab().layout.pane_ids();
    let mut requests = Vec::new();
    for id in visible {
        let Some(pane) = state.pane_mut_anywhere(id) else {
            continue;
        };
        if pane.view != PaneView::Lx {
            continue;
        }
        if let Some((generation, root)) = pane.lx.take_mention_scan_request() {
            requests.push((id, generation, root));
        }
    }
    requests
}

/// 写入指定窗格的提及扫描结果；窗格已删除或结果过期时丢弃。
pub fn apply_lx_mention_entries(
    state: &mut AppState,
    id: PaneId,
    generation: u64,
    root: &std::path::Path,
    entries: Vec<crate::app::markdown::MentionEntry>,
) {
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.apply_mention_entries(generation, root, entries);
    }
}

/// 鼠标点选 `@` 提及候选：设置高亮并按 `Enter` 规则采纳；返回是否消费。
pub fn select_lx_mention(state: &mut AppState, id: PaneId, index: usize) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if !pane.lx.mention_set_active(index) && !pane.lx.mention_open() {
        return false;
    }
    pane.lx.mention_confirm()
}

/// 滚轮在 `@` 提及面板上滚动候选视口；`base` 为当前窗口起点，高亮不动。
pub fn scroll_lx_mention(state: &mut AppState, id: PaneId, delta: isize, base: usize) -> bool {
    state
        .pane_mut_anywhere(id)
        .is_some_and(|pane| pane.lx.mention_scroll(delta, base))
}

/// 鼠标悬停 `/` 命令面板条目：更新高亮（窗口不滚动）；返回是否变化。
pub fn hover_lx_slash(state: &mut AppState, id: PaneId, index: usize) -> bool {
    state
        .pane_mut_anywhere(id)
        .is_some_and(|pane| pane.lx.slash.as_mut().is_some_and(|p| p.set_active(index)))
}

/// 鼠标点选 `/` 命令面板条目：按该条目的 Enter 语义采纳；返回是否消费。
pub fn select_lx_slash(state: &mut AppState, id: PaneId, index: usize) -> bool {
    if let Some(pane) = state.pane_mut_anywhere(id)
        && let Some(panel) = pane.lx.slash.as_mut()
    {
        panel.set_active(index);
    }
    super::chat::apply_slash_enter(state, id)
}

/// 滚轮在 `/` 命令面板上滚动窗口；`base` 为当前窗口起点，高亮不动。
pub fn scroll_lx_slash(state: &mut AppState, id: PaneId, delta: isize, base: usize) -> bool {
    state.pane_mut_anywhere(id).is_some_and(|pane| {
        pane.lx
            .slash
            .as_mut()
            .is_some_and(|p| p.scroll_viewport(delta, base))
    })
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
        pane.lx.paste_cancel();
        // 隐藏的 lx 页不得保留可点击的提及面板。
        pane.lx.mention_escape();
        pane.view
    }) else {
        return false;
    };
    if view == PaneView::Lx {
        if state.terminal_selection == Some(id) {
            clear_terminal_selection(state);
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
            let (cols, rows) = layout::prompt_inner_size(*rect);
            if state.prompt.size() != (cols, rows) {
                clear_selection(state);
            }
            state.prompt.resize(cols, rows);
        } else if let Some(pane) = state.pane_mut_anywhere(*id) {
            let (cols, rows) = layout::pane_inner_size(*rect);
            pane.terminal.resize(cols, rows);
            let (lx_cols, lx_rows) = layout::lx_input_size(*rect);
            pane.lx.resize(lx_cols, lx_rows);
            if let Some(ask) = pane.lx.ask.as_mut() {
                let (width, rows) = layout::lx_ask_editor_size(*rect);
                ask.resize_editor(width, rows);
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
    }
    focus_pane(state, pane);
    clear_selection(state);
    true
}

/// 把终端窗格视口移动到距内容顶部 `offset` 行（0 为最旧一屏）；返回视口是否移动。
pub fn set_terminal_scroll(state: &mut AppState, pane: PaneId, offset: usize) -> bool {
    state.pane_mut_anywhere(pane).is_some_and(|target| {
        target.kind == PaneKind::Terminal && target.terminal.scroll_to_content_offset(offset)
    })
}

/// 后台任务页的按键：`↑/↓` 移动、`^d` 停止高亮进程、`←/→` 换 tab、`Esc` 返回 lx 页。
///
/// 页面整页接管输入（不回落到草稿编辑）：它是一次独立浏览，退出来后草稿与光标
/// 仍是进入前的样子。不可用的键直接吞掉，避免"按下去没反应但草稿里多了字符"。
/// 后台任务面板里 `PageUp`/`PageDown` 对**输出区**的翻页量；其余键返回 None。
///
/// `↑`/`↓` 不在此列——它们归面板列表的高亮（见 [`apply_task_page_input`]）。
/// 面板接管历史区后上方已经没有可见的消息列表，因此翻页键改翻输出区
/// （决策 11：让它继续有意义，而不是去翻一块看不见的内容）。
fn task_output_delta(command: LxInput) -> Option<isize> {
    match command {
        LxInput::PageUp => Some(-10),
        LxInput::PageDown => Some(10),
        _ => None,
    }
}

fn apply_task_page_input(state: &mut AppState, pane: PaneId, command: LxInput) {
    // `PageUp`/`PageDown`：翻输出区（`↑`/`↓` 已经归任务列表高亮）。
    if let Some(delta) = task_output_delta(command) {
        if let Some(page) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.task.as_mut())
        {
            page.scroll_output(delta);
        }
        return;
    }
    match command {
        // `esc` 无条件退出：列表很长时不必连按 ↑ 到顶。
        LxInput::Escape => {
            super::chat::close_task_page(state, pane);
        }
        // `↑`：列表非空且不在顶部 → 上移高亮；已在顶部或列表为空 → 回到输入框。
        //
        // 「回到输入框」就是把面板关掉（`pane.lx.task = None`）：面板占的是输入框的
        // 矩形，两者不可能同时可见，所以没有"切焦点"的中间态；草稿、光标、历史滚动
        // 本来就没被销毁，关掉即原样恢复。
        LxInput::Up => {
            let moved = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
                .is_some_and(|page| page.move_active(-1));
            if !moved {
                super::chat::close_task_page(state, pane);
            }
        }
        LxInput::Down => {
            if let Some(page) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
            {
                page.move_active(1);
            }
        }
        // ←/→ 换 tab（对齐 opencode composer 的左右切 tab）。
        LxInput::Left => {
            if let Some(page) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
            {
                page.cycle_tab(-1);
            }
        }
        LxInput::Right => {
            if let Some(page) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.task.as_mut())
            {
                page.cycle_tab(1);
            }
        }
        // `^d` 停止高亮进程（对齐 opencode `composer.shell.kill`）。
        //
        // 只认 `StopProcess`（仅 Ctrl+D 映射）：裸 `Delete` 键在编辑器语义里是
        // 「向后删一个字符」，面板里把它当停止用会让用户「想删一行」时
        // SIGTERM 掉一个正在跑的服务，且不可撤销。
        LxInput::StopProcess => {
            super::chat::stop_selected_process(state, pane);
        }
        // 其余键不属于面板：吞掉，不落到草稿。
        _ => {}
    }
}
