//! 详情输入框（Agents / BTW composer）的 `/` 命令路由。
//!
//! 主输入框的面板逻辑留在 `chat/slash.rs`；这里只处理 composer 的上下文命令：
//! 候选刷新、面板按键、采纳与命令执行分派。所有读写都落到 `Lx::active_editor()`。

use crate::app::actions::LxInput;
use crate::app::state::lx::slash::composer::{self, DetailContext};
use crate::app::state::{AppState, InputHintLevel, Lx, SlashEntry, SlashPanel};
use crate::app::update::{show_lx_hint, show_lx_toast};
use crate::tiling::PaneId;
use crate::ui::text;

/// 当前窗格是否停在某个详情 composer 上。
pub(crate) fn context(lx: &Lx) -> Option<DetailContext> {
    if lx.agents_editing() {
        Some(DetailContext::Agent)
    } else if lx.btw_editing() {
        Some(DetailContext::Btw)
    } else {
        None
    }
}

/// 窗格级的 composer 上下文判定。
pub(crate) fn context_of(state: &AppState, pane: PaneId) -> Option<DetailContext> {
    state
        .pane_anywhere(pane)
        .and_then(|target| context(&target.lx))
}

/// 编辑后重算 composer 的 `/` 命令候选（行首 `/` 触发、模糊过滤）。
pub(crate) fn refresh_slash_panel(state: &mut AppState, pane: PaneId) {
    let Some((context, query)) = state.pane_anywhere(pane).and_then(|target| {
        let context = context(&target.lx)?;
        let editor = target.lx.active_editor();
        let masked = crate::app::paste::mask_pasted_text(&editor.input);
        crate::app::state::lx::slash::slash_trigger(&masked, editor.cursor)
            .map(|query| (context, query))
    }) else {
        // 离开命令位置：只关命令候选，不动撤销确认面板。
        close_command_candidates(state, pane);
        return;
    };
    let matching = composer::entries(context, &query);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    target.lx.with_active_editor(|editor| {
        // 撤销确认是模态面板，不被命令候选覆盖。
        if matches!(editor.slash, Some(SlashPanel::Rewind(_))) {
            return;
        }
        if matching.is_empty() {
            if matches!(editor.slash, Some(SlashPanel::Commands { .. })) {
                editor.slash = None;
            }
            return;
        }
        // 过滤词变化时重置高亮至首项；未变时保留原窗口。
        let query_changed = match &editor.slash {
            Some(SlashPanel::Commands {
                query: previous, ..
            }) => previous != &query,
            _ => true,
        };
        let mut window = match &editor.slash {
            Some(SlashPanel::Commands { window, .. }) if !query_changed => *window,
            _ => crate::app::state::lx::slash::PanelWindow::default(),
        };
        window.clamp(matching.len());
        editor.slash = Some(SlashPanel::Commands {
            items: matching,
            window,
            query,
        });
    });
}

/// 只关闭命令候选；保留撤销确认等模态面板。
pub(crate) fn close_command_candidates(state: &mut AppState, pane: PaneId) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    target.lx.with_active_editor(|editor| {
        if matches!(editor.slash, Some(SlashPanel::Commands { .. })) {
            editor.slash = None;
        }
    });
}

/// 关闭 composer 上任意 `/` 面板。
pub(crate) fn close(state: &mut AppState, pane: PaneId) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    target.lx.with_active_editor(|editor| editor.slash = None);
}

/// 切换 tab / 打开其它页面时清掉所有详情 composer 的 `/` 面板。
pub(crate) fn close_all(state: &mut AppState, pane: PaneId) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    target.lx.slash = None;
    for composer in target.lx.agents.composers.values_mut() {
        composer.editor_mut().slash = None;
    }
    if let Some(composer) = target.lx.btw.active_composer_mut() {
        composer.slash = None;
    }
}

/// 消费 composer 的 `/` 面板按键；面板未打开返回 false 交由编辑逻辑。
pub(crate) fn apply_slash_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(panel) = state.pane_anywhere(pane).and_then(|target| {
        context(&target.lx)?;
        target.lx.active_editor().slash.clone()
    }) else {
        return false;
    };
    match command {
        LxInput::Up | LxInput::Down => {
            let delta = if command == LxInput::Up { -1 } else { 1 };
            let _ = state.pane_mut_anywhere(pane).is_some_and(|target| {
                target.lx.with_active_editor(|editor| {
                    editor
                        .slash
                        .as_mut()
                        .is_some_and(|panel| panel.move_active(delta))
                })
            });
            true
        }
        LxInput::Tab | LxInput::Enter => {
            apply_enter(state, pane);
            true
        }
        LxInput::Escape => {
            close(state, pane);
            true
        }
        // 撤销确认是模态面板：其余编辑键不落到 composer，避免面板与草稿错位。
        _ if matches!(panel, SlashPanel::Rewind(_)) => true,
        _ => false,
    }
}

/// 面板 Enter：采纳高亮命令（`apply_slash_enter` 的 composer 分支）。
pub(crate) fn apply_enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(panel) = state.pane_anywhere(pane).and_then(|target| {
        context(&target.lx)?;
        target.lx.active_editor().slash.clone()
    }) else {
        return false;
    };
    match panel {
        SlashPanel::Commands { .. } => accept(state, pane),
        SlashPanel::Rewind(_) => super::rewind::apply_enter(state, pane),
        _ => false,
    }
}

/// 采纳高亮候选：带参命令回显前缀，其余命令保留文本等待 Enter 执行。
fn accept(state: &mut AppState, pane: PaneId) -> bool {
    let Some((entry, input, cursor)) = state.pane_anywhere(pane).and_then(|target| {
        context(&target.lx)?;
        let editor = target.lx.active_editor();
        let Some(SlashPanel::Commands { items, window, .. }) = editor.slash.clone() else {
            return None;
        };
        items
            .get(window.active)
            .cloned()
            .map(|entry| (entry, editor.input.clone(), editor.cursor))
    }) else {
        close_command_candidates(state, pane);
        return false;
    };
    let SlashEntry::Detail(command) = entry else {
        return false;
    };
    let Some(prefix) = command.echo_prefix() else {
        close_command_candidates(state, pane);
        return true;
    };
    if !standalone_line(&input, cursor) {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::detail::LX_DETAIL_COMMAND_STANDALONE,
        );
        return true;
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.with_active_editor(|editor| {
            editor.replace_input(prefix);
            editor.slash = None;
        });
    }
    true
}

/// 草稿必须整段就是当前命令行：无前后残留、单行、命令词无空白。
fn standalone_line(input: &str, cursor: usize) -> bool {
    let trimmed = input.trim();
    let line_start = input[..cursor].rfind('\n').map_or(0, |offset| offset + 1);
    let line = input[line_start..].trim();
    trimmed == line
        && input[cursor..].trim().is_empty()
        && !line.contains(['\n', '\r'])
        && !line.contains(char::is_whitespace)
        && line.starts_with('/')
}

/// 提交前拦截：整段详情命令执行、未知 `/` 命令提示；都不是则返回 false。
pub(crate) fn try_submit(state: &mut AppState, pane: PaneId) -> bool {
    let Some(context) = context_of(state, pane) else {
        return false;
    };
    let Some(draft) = state
        .pane_anywhere(pane)
        .map(|target| target.lx.active_editor().input.clone())
    else {
        return false;
    };
    if composer::parse(&draft, context).is_some() {
        clear(state, pane);
        show_lx_toast(
            state,
            pane,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Warning,
                text::detail::LX_DETAIL_COMMAND_UNAVAILABLE.to_string(),
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return true;
    }
    if let Some(command) = super::slash::unknown_slash_command(&draft) {
        clear(state, pane);
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::chat::unknown_command(&command),
        );
        return true;
    }
    false
}

/// 清空当前 composer 草稿并关闭 `/` 面板。
pub(crate) fn clear(state: &mut AppState, pane: PaneId) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.with_active_editor(|editor| {
            editor.clear();
            editor.slash = None;
        });
    }
}
