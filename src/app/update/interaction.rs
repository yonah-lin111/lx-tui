//! 界面交互：分割线拖拽、悬停高亮、toast/tooltip 提示与 tick 定时器。

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
use crate::app::tooltip::{Tooltip, TooltipTarget};
use crate::layout::{BoundaryHit, PaneId};
use crate::ui::text;

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

/// 复制 / 剪切反馈：统一落在屏幕顶部居中，并报告本次剪贴板文本的字符数。
///
/// 复制反馈不跟随 lx/prompt/窗格锚点，避免窄容器把正文截成半截；`confirmed`
/// 只影响成功文案（剪切走原生写入确认，普通复制仍是尽力而为）。
pub fn show_copy_result(state: &mut AppState, chars: usize, confirmed: bool, success: bool) {
    let (kind, message) = match (success, confirmed) {
        (true, true) => (ToastKind::Default, text::copied_message(chars)),
        (true, false) => (ToastKind::Default, text::copy_sent_message(chars)),
        (false, _) => (ToastKind::Danger, text::copy_failed_message(chars)),
    };
    show_toast(
        state,
        Toast::new(kind, message, None, Instant::now()).with_title(text::TOAST_CLIPBOARD_TITLE),
    );
}

/// 在指定窗格**自己的** toast 槽展示反馈（单条替换并重置倒计时）。
///
/// lx 页的反馈必须停在触发它的窗格里（见 `AppState.toast` 与 `Lx.toast` 的注释）：
/// 全局槽只有一个，且锚点容器放不下时会降级到屏幕顶部居中、横跨整个窗口。
/// 窗格不存在时静默丢弃——没有可落地的页面，就不该凭空造一条全局提示。
pub fn show_lx_toast(state: &mut AppState, pane: PaneId, toast: Toast) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.toast = Some(toast);
    }
}

/// 关闭当前全局 toast；返回是否确实清除。
pub fn dismiss_toast(state: &mut AppState) -> bool {
    state.toast.take().is_some()
}

/// 锚点可选的反馈分派：有锚点且锚点窗格是 lx 页 → 落该窗格自己的槽；
/// 其余情况（无锚点、窗格已不存在、锚点是终端视图）保持全局 toast。
pub fn show_toast_at(state: &mut AppState, anchor: Option<PaneId>, toast: Toast) {
    match anchor {
        Some(pane)
            if state
                .pane_anywhere(pane)
                .is_some_and(|target| target.view == crate::app::state::PaneView::Lx) =>
        {
            show_lx_toast(state, pane, toast);
        }
        _ => show_toast(state, toast),
    }
}

/// 把反馈送到所有绑定该会话的 lx 窗格（会话级效应没有单一窗格可比锚定）。
///
/// 返回是否确实投递到了至少一个窗格；没有 lx 窗格绑定时调用方应回退到全局 toast。
pub fn show_lx_toast_for_session(state: &mut AppState, session_id: &str, toast: Toast) -> bool {
    let panes: Vec<PaneId> = state
        .all_pane_ids()
        .into_iter()
        .filter(|pane| {
            state.pane_anywhere(*pane).is_some_and(|target| {
                target.view == crate::app::state::PaneView::Lx
                    && target.lx.chat.session_id.as_deref() == Some(session_id)
            })
        })
        .collect();
    if panes.is_empty() {
        return false;
    }
    for pane in panes {
        show_lx_toast(state, pane, toast.clone());
    }
    true
}

/// 关闭指定窗格的 toast；返回是否确实清除。
pub fn dismiss_lx_toast(state: &mut AppState, pane: PaneId) -> bool {
    state
        .pane_mut_anywhere(pane)
        .is_some_and(|target| target.lx.toast.take().is_some())
}

/// 更新指定窗格 toast 的悬停状态；返回是否发生变化。
pub fn set_lx_toast_hover(state: &mut AppState, pane: PaneId, hovered: bool) -> bool {
    let Some(toast) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.toast.as_mut())
    else {
        return false;
    };
    if toast.hovered() == hovered {
        return false;
    }
    toast.set_hovered(hovered);
    true
}

/// 清除已过期的 lx 窗格 toast；返回是否发生变化。
fn expire_lx_toasts(state: &mut AppState, now: Instant) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        if let Some(target) = state.pane_mut_anywhere(pane)
            && target
                .lx
                .toast
                .as_ref()
                .is_some_and(|toast| toast.is_expired(now))
        {
            target.lx.toast = None;
            changed = true;
        }
    }
    changed
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
    // 可见屏幕选区存在时暂停所有 toast 过期：用户正在读/选的内容不得中途消失。
    let paused = state
        .screen_selection
        .as_ref()
        .is_some_and(|selection| selection.visible());
    let expired = !paused
        && state
            .toast
            .as_ref()
            .is_some_and(|toast| toast.is_expired(now));
    if expired {
        state.toast = None;
    }
    // 窗格内的 lx 反馈与全局 toast 同一套到期语义（hover 期间不过期）。
    let lx_expired = !paused && expire_lx_toasts(state, now);
    let revealed = state
        .tooltip
        .as_mut()
        .is_some_and(|tooltip| tooltip.reveal(now));
    let saved = tick_prompt_autosave(state, now);
    let scrolled = tick_selection_autoscroll(state, now);
    // `/resume` 面板停留期间刷新"现在"，相对时间随之自然推进。
    let retimed = tick_slash_panel_clock(state);
    expired || lx_expired || revealed || saved || scrolled || retimed
}

/// 刷新打开中的 `/resume` 面板时间戳；返回是否有面板需要重绘。
///
/// 只改面板自身的时间快照，不触碰列表内容与窗口状态——渲染层据此得到新的相对时间。
fn tick_slash_panel_clock(state: &mut AppState) -> bool {
    let now_ms = crate::db::now_ms();
    let ids = state.all_pane_ids();
    let mut changed = false;
    for id in ids {
        let Some(pane) = state.pane_mut_anywhere(id) else {
            continue;
        };
        let Some(crate::app::state::SlashPanel::History(panel)) = pane.lx.slash.as_mut() else {
            continue;
        };
        // 同一毫秒内不重复标记变化，避免空转重绘。
        if panel.now_ms != now_ms {
            panel.now_ms = now_ms;
            changed = true;
        }
    }
    changed
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
    let paused = state
        .screen_selection
        .as_ref()
        .is_some_and(|selection| selection.visible());
    let toast = (!paused)
        .then(|| state.toast.as_ref().and_then(Toast::next_deadline))
        .flatten();
    // 窗格内的 lx 反馈同样需要唤醒：否则没人 tick，提示会一直挂在屏幕上。
    let lx_toast = (!paused)
        .then(|| {
            state
                .all_pane_ids()
                .into_iter()
                .filter_map(|pane| state.pane_anywhere(pane))
                .filter_map(|target| target.lx.toast.as_ref())
                .filter_map(Toast::next_deadline)
                .min()
        })
        .flatten();
    let tooltip = state.tooltip.as_ref().and_then(Tooltip::next_deadline);
    let autoscroll = state
        .selection_autoscroll
        .map(|autoscroll| autoscroll.next_at);
    let autosave = prompt_item_editing(state)
        .then(|| state.prompt.autosave_deadline(PROMPT_AUTOSAVE_DELAY))
        .flatten();
    [toast, lx_toast, tooltip, autoscroll, autosave]
        .into_iter()
        .flatten()
        .min()
}
