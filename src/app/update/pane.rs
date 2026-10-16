//! 窗格与终端生命周期、PTY 喂入与 lx 视图交互。

use std::path::Path;

use ratatui::layout::Rect;

use crate::app::actions::LxInput;
use crate::app::state::{AppState, PaneKind, PaneView};
use crate::detect::PaneAgentSnapshot;
use crate::layout::{self, PaneId};

use super::prompt::cancel_page_rename;
use super::selection::{WHEEL_LINES, clear_selection, clear_terminal_selection};

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
    state.prompt_focused = false;
    state.prompt.clear_panel();
    cancel_page_rename(state);
    state.active_tab_mut().layout.focus_pane(id);
}

/// 应用 lx 页输入命令到焦点窗格；非 lx 视图忽略。
///
/// 面板优先级：粘贴参照 > `@` 提及 > `/` 命令（model/resume/history） > md 块命令；
/// 面板打开时其余按键按面板策略吞掉。
/// 提及面板打开时 `↑`/`↓` 移动高亮、`Tab`/`Enter` 采纳、`Shift+Enter` 进入目录、`Esc` 关闭。
pub fn apply_lx_input(state: &mut AppState, command: LxInput) {
    let pane_id = state.active_tab().layout.focus();
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
                        if let Some(panel) = lx.slash.as_mut()
                            && let super::super::state::SlashPanel::Model(picker) = panel
                        {
                            picker.stage = super::super::state::ModelPickerStage::Models;
                            picker.window =
                                super::super::state::PanelWindow::new(picker.window.active);
                        }
                    }
                    _ => lx.slash = None,
                }
                return;
            }
            // 命令候选面板：编辑键放行（继续输入过滤 / 删除 `/` 关闭）；二级面板吞掉。
            _ if editable => {}
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
        LxInput::Char(ch) => {
            lx.insert_char(ch);
            refresh_slash_panel(state, pane_id);
        }
        // 提及面板未打开时 Shift+Enter 回落为换行。
        LxInput::Newline | LxInput::EnterFolder => lx.newline(),
        LxInput::Backspace => {
            lx.backspace();
            refresh_slash_panel(state, pane_id);
        }
        LxInput::Delete => {
            lx.delete();
            refresh_slash_panel(state, pane_id);
        }
        LxInput::Left => lx.move_left(),
        LxInput::Right => lx.move_right(),
        LxInput::Up => lx.move_up(),
        LxInput::Down => lx.move_down(),
        LxInput::Home | LxInput::LineStart => lx.home(),
        LxInput::End | LxInput::LineEnd => lx.end(),
        LxInput::WordLeft => lx.word_left(),
        LxInput::WordRight => lx.word_right(),
        LxInput::DeleteToLineStart => {
            lx.delete_to_line_start();
            refresh_slash_panel(state, pane_id);
        }
        LxInput::DeleteToLineEnd => {
            lx.delete_to_line_end();
            refresh_slash_panel(state, pane_id);
        }
        LxInput::DeleteWordBackward => {
            lx.delete_word_backward();
            refresh_slash_panel(state, pane_id);
        }
        LxInput::DeleteWordForward => {
            lx.delete_word_forward();
            refresh_slash_panel(state, pane_id);
        }
        LxInput::Undo => {
            lx.undo();
            refresh_slash_panel(state, pane_id);
        }
        LxInput::Redo => {
            lx.redo();
            refresh_slash_panel(state, pane_id);
        }
        LxInput::Clear => {
            lx.clear();
            refresh_slash_panel(state, pane_id);
        }
        // 无面板且 Native 空闲时 Enter 发送；流中不重复发送。
        LxInput::Enter => {
            super::chat::try_send(state, pane_id);
        }
        // 无面板：Esc 在流中停止生成，否则吞掉。
        LxInput::Escape => {
            super::chat::stop_stream(state, pane_id);
        }
        // Ctrl+T：切换最近 thinking 块。
        LxInput::ToggleThinking => {
            super::chat::toggle_latest_thinking(state, pane_id);
        }
        LxInput::Tab => {}
    }
}

/// 编辑后刷新 `/` 命令候选面板：行首命令位置输入 `/` 时打开/过滤。
fn refresh_slash_panel(state: &mut AppState, pane: PaneId) {
    let query = state.pane_anywhere(pane).and_then(|target| {
        super::super::state::lx::slash::slash_trigger(&target.lx.input, target.lx.cursor)
    });
    let Some(query) = query else {
        // 离开命令位置时关闭命令候选面板（model/resume/history 面板不受影响）。
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

/// 采纳 `/` 命令候选：消费整条命令文本（不写入草稿或 DB）。
pub fn accept_slash_command(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 只在命令候选面板时采纳高亮命令。
    let Some(super::super::state::SlashPanel::Commands { items, window }) = target.lx.slash.clone()
    else {
        return false;
    };
    let Some(command) = items.get(window.active).copied() else {
        target.lx.slash = None;
        return false;
    };
    // 命令文本 = 当前行首 `/...`（含光标前的完整命令）。
    let input = target.lx.input.clone();
    let cursor = target.lx.cursor;
    let line_start = input[..cursor].rfind('\n').map_or(0, |offset| offset + 1);
    // 消费整条命令：删除该行命令文本。
    target.lx.input.replace_range(line_start..cursor, "");
    target.lx.cursor = line_start;
    target.lx.slash = None;
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
