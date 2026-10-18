//! 界面交互：分割线拖拽、悬停高亮、toast/tooltip 提示与 tick 定时器。

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::app::toast::Toast;
use crate::app::tooltip::{Tooltip, TooltipTarget};
use crate::layout::BoundaryHit;

use super::editor::prompt_item_editing;
use super::prompt::commit_editing_prompt;
use super::selection::{clear_selection, tick_selection_autoscroll};

/// prompt 自动保存静置时长：条目编辑停止输入约 800ms 后提交落库。
const PROMPT_AUTOSAVE_DELAY: Duration = Duration::from_millis(800);

/// 交换主区窗格与 prompt 栏的左右位置：翻转所在侧、两栏都恢复展开，
/// 宽度按位置模型重置——prompt 落在左列取 `available - side_width`，落回右列取 `side_width`。
pub fn swap_prompt_side(state: &mut AppState, available: u16, side_width: u16) {
    state.prompt_left = !state.prompt_left;
    state.prompt_collapsed = false;
    state.panes_collapsed = false;
    state.prompt_width = if state.prompt_left {
        available.saturating_sub(side_width)
    } else {
        side_width
    };
}

/// 在 prompt 右栏分割线上开始拖拽；清除已有选区。
pub fn begin_prompt_resize(state: &mut AppState) {
    clear_selection(state);
    state.resizing_prompt = true;
}

/// 拖拽中更新右栏宽度；未处于拖拽时忽略。
pub fn drag_prompt(state: &mut AppState, width: u16) {
    if state.resizing_prompt {
        state.prompt_width = width;
    }
}

/// 结束右栏拖拽。
pub fn end_prompt_resize(state: &mut AppState) {
    state.resizing_prompt = false;
}

/// 在侧栏分割线上开始拖拽；清除已有选区。
pub fn begin_sidebar_resize(state: &mut AppState) {
    clear_selection(state);
    state.resizing_sidebar = true;
}

/// 拖拽中更新侧栏宽度；未处于拖拽时忽略。
pub fn drag_sidebar(state: &mut AppState, width: u16) {
    if state.resizing_sidebar {
        state.sidebar_width = width;
    }
}

/// 结束侧栏拖拽。
pub fn end_sidebar_resize(state: &mut AppState) {
    state.resizing_sidebar = false;
}

/// 在主区窗格分割线上开始拖拽；清除已有选区。
pub fn begin_pane_resize(state: &mut AppState, hit: BoundaryHit) {
    clear_selection(state);
    state.resizing_pane = Some(hit);
}

/// 拖拽中把分割线移到 `pos`（水平分割为屏幕列，垂直分割为屏幕行）；未处于该拖拽或调整失败时返回 false。
pub fn drag_pane_boundary(
    state: &mut AppState,
    hit: BoundaryHit,
    area: Rect,
    pos: u16,
    min_width: u16,
    min_height: u16,
) -> bool {
    if state.resizing_pane != Some(hit) {
        return false;
    }
    state
        .active_tab_mut()
        .layout
        .resize_boundary(hit, area, pos, min_width, min_height)
}

/// 结束主区窗格拖拽。
pub fn end_pane_resize(state: &mut AppState) {
    state.resizing_pane = None;
}

/// 更新主区窗格分割线悬停状态；返回是否发生变化。
pub fn set_pane_hover(state: &mut AppState, hover: Option<BoundaryHit>) -> bool {
    if state.pane_hover == hover {
        return false;
    }
    state.pane_hover = hover;
    true
}

/// 更新侧栏分割线悬停状态；返回是否发生变化。
pub fn set_sidebar_hover(state: &mut AppState, hover: bool) -> bool {
    if state.sidebar_hover == hover {
        return false;
    }
    state.sidebar_hover = hover;
    true
}

/// 更新右栏分割线悬停状态；返回是否发生变化。
pub fn set_prompt_hover(state: &mut AppState, hover: bool) -> bool {
    if state.prompt_hover == hover {
        return false;
    }
    state.prompt_hover = hover;
    true
}

/// 更新侧栏工作区列表悬停项；返回是否发生变化。
pub fn set_workspace_hover(state: &mut AppState, hover: Option<usize>) -> bool {
    if state.workspace_hover == hover {
        return false;
    }
    state.workspace_hover = hover;
    true
}

/// 更新标签栏悬停标签；返回是否发生变化。
pub fn set_tab_hover(state: &mut AppState, hover: Option<usize>) -> bool {
    if state.tab_hover == hover {
        return false;
    }
    state.tab_hover = hover;
    true
}

/// 更新提示词列表悬停行；返回是否发生变化。
pub fn set_prompt_tree_hover(state: &mut AppState, hover: Option<usize>) -> bool {
    if state.prompt_tree_hover == hover {
        return false;
    }
    state.prompt_tree_hover = hover;
    true
}

/// 展示 toast；单条替换并重置倒计时。
pub fn show_toast(state: &mut AppState, toast: Toast) {
    state.toast = Some(toast);
}

/// 关闭当前 toast。
pub fn dismiss_toast(state: &mut AppState) {
    state.toast = None;
}

/// 更新 toast 悬停状态；返回是否发生变化。
pub fn set_toast_hover(state: &mut AppState, hovered: bool) -> bool {
    let Some(toast) = state.toast.as_mut() else {
        return false;
    };
    if toast.hovered() == hovered {
        return false;
    }
    toast.set_hovered(hovered);
    true
}

/// 更新 tooltip 悬停目标：目标不变保持计时，变化则整体替换并重新计时；返回是否变化。
pub fn set_tooltip_target(
    state: &mut AppState,
    target: Option<TooltipTarget>,
    now: Instant,
) -> bool {
    match (state.tooltip.as_ref().map(|tooltip| tooltip.target), target) {
        (None, None) => false,
        (Some(current), Some(target)) if current == target => false,
        (_, target) => {
            state.tooltip = target.map(|target| Tooltip::new(target, now));
            true
        }
    }
}

/// 立即清除 tooltip（左键按下、拖拽或浮层接管）；返回是否发生变化。
pub fn clear_tooltip(state: &mut AppState) -> bool {
    state.tooltip.take().is_some()
}

/// 清除已过期的 toast、提升到时的 tooltip、推进拖选边缘自动滚动；返回是否发生变化。
pub fn tick(state: &mut AppState, now: Instant) -> bool {
    let expired = state
        .toast
        .as_ref()
        .is_some_and(|toast| toast.is_expired(now));
    if expired {
        state.toast = None;
    }
    // 输入框上方的临时提示（错误红 / 默认黄）：到点清除，那一行随之消失。
    let hint_expired = clear_expired_lx_hints(state, now);

    let revealed = state
        .tooltip
        .as_mut()
        .is_some_and(|tooltip| tooltip.reveal(now));
    let saved = tick_prompt_autosave(state, now);
    let scrolled = tick_selection_autoscroll(state, now);
    expired || hint_expired || revealed || saved || scrolled
}

/// 自动保存轮询：条目编辑视图下缓冲静置达标后提交落库并回到已保存态；
/// 临时提示词与树视图不参与；粘贴提案未收束时占位文本不得落库；返回是否触发保存。
fn tick_prompt_autosave(state: &mut AppState, now: Instant) -> bool {
    if !prompt_item_editing(state)
        || state.prompt.paste_panel().is_some()
        || !state.prompt.autosave_due(now, PROMPT_AUTOSAVE_DELAY)
    {
        return false;
    }
    // 不预先标记已保存：保存成功回执由服务按修订号确认；失败保持脏状态重试。
    commit_editing_prompt(state);
    true
}

/// 最近一次定时到期时间（toast 消失、tooltip 显示、自动滚动或 prompt 自动保存）；
/// 事件循环据此安排唤醒。
pub fn next_deadline(state: &AppState) -> Option<Instant> {
    let toast = state.toast.as_ref().and_then(Toast::next_deadline);
    let hint = lx_hint_deadline(state);
    let tooltip = state.tooltip.as_ref().and_then(Tooltip::next_deadline);
    let autoscroll = state
        .selection_autoscroll
        .map(|autoscroll| autoscroll.next_at);
    let autosave = prompt_item_editing(state)
        .then(|| state.prompt.autosave_deadline(PROMPT_AUTOSAVE_DELAY))
        .flatten();
    [toast, hint, tooltip, autoscroll, autosave]
        .into_iter()
        .flatten()
        .min()
}

/// 清除已到期的输入框上方提示；返回是否发生变化（逐窗格）。
fn clear_expired_lx_hints(state: &mut AppState, now: Instant) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        if let Some(target) = state.pane_mut_anywhere(pane)
            && target
                .lx
                .chat
                .hint
                .as_ref()
                .is_some_and(|hint| hint.is_expired(now))
        {
            target.lx.chat.hint = None;
            changed = true;
        }
    }
    changed
}

/// 最近一个输入框上方提示的到期时刻（事件循环据此安排唤醒）。
fn lx_hint_deadline(state: &AppState) -> Option<Instant> {
    state
        .all_pane_ids()
        .into_iter()
        .filter_map(|pane| state.pane_anywhere(pane))
        .filter_map(|target| target.lx.chat.hint.as_ref())
        .map(|hint| hint.deadline)
        .min()
}
