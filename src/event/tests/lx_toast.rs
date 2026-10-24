//! lx 页 toast 的**端到端**测试：反馈落在窗格内、随窗格宽度折行、能被点关闭。
//!
//! 与 `ui::main_content::lx::toast` 的纯几何测试互补：这里走完整的事件/渲染链路
//! （`AppState` → `ui::render` → 终端缓冲区），验证用户看到的那一屏。

use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use super::*;
use crate::app::update;
use crate::ui::text;

/// 把主区窗格切到 lx 视图（demo 状态默认是终端视图）。
fn lx_state() -> AppState {
    let mut state = AppState::demo();
    for workspace in &mut state.workspaces {
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    pane.view = crate::app::state::PaneView::Lx;
                }
            }
        }
    }
    state
}

/// 在焦点窗格里弹一条 lx 反馈。
fn hint(state: &mut AppState, level: crate::app::state::InputHintLevel, message: &str) -> PaneId {
    let pane = state.active_tab().layout.focus();
    update::show_lx_hint(state, pane, level, message);
    pane
}

/// 画一帧并取回缓冲区。
fn draw(state: &AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("backend");
    let config = Config::default();
    terminal
        .draw(|frame| crate::ui::render(frame, state, &config))
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// lx 反馈**只**写自己的窗格槽：全局 toast 槽保持为空。
///
/// 这是用户要求的核心约束：lx 页的 toast 都在 lx 里显示，不用全局的。
#[test]
fn lx_feedback_never_occupies_the_global_toast_slot() {
    let mut state = lx_state();
    let pane = hint(
        &mut state,
        crate::app::state::InputHintLevel::Error,
        text::LX_COMPACT_FAILED,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .is_some_and(|target| target.lx.toast.is_some()),
        "反馈必须落在该窗格的 lx 槽"
    );
    assert!(state.toast.is_none(), "全局 toast 槽必须保持为空");
}

/// 长错误文案在窄窗格里折行，且**整条盒都在窗格内容区内**（不横跨整个窗口）。
///
/// 这条测试对应截图里的问题：以前走全局 toast 时，窄窗格放不下就降级到屏幕顶部居中，
/// 盒子横跨整个窗口。
#[test]
fn compact_failure_stays_inside_the_pane() {
    let mut state = lx_state();
    let message = format!(
        "{} ({})",
        text::LX_COMPACT_FAILED,
        "summary request failed: cancelled"
    );
    let pane = hint(
        &mut state,
        crate::app::state::InputHintLevel::Error,
        &message,
    );
    let config = Config::default();
    let screen = Rect::new(0, 0, 140, 34);
    let view = crate::ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    );
    let rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let Some((_, pane_rect)) = rects.iter().find(|(id, _)| *id == pane) else {
        panic!("焦点窗格有矩形");
    };
    let inner = crate::layout::pane_inner_rect(*pane_rect);
    let target = state.pane_anywhere(pane).expect("pane");
    let toast = crate::ui::main_content::lx::toast_rect(inner, &target.lx).expect("toast 可见");
    assert!(
        toast.left() >= inner.left() && toast.right() <= inner.right(),
        "toast 盒 {toast:?} 必须整条落在窗格内容区 {inner:?} 内"
    );
    assert!(toast.bottom() <= inner.bottom(), "toast 不得盖住窗格底边框");
    // 长文案确实被折成了多行（而不是越界或被截成一行）。
    assert!(
        toast.height > 3,
        "长文案必须在窗格内折行（高度 {}）",
        toast.height
    );
}

/// 渲染一帧：窗格内的确出现了红框与提示文字，全局 toast 位置保持干净。
#[test]
fn lx_toast_is_painted_inside_the_pane() {
    let mut state = lx_state();
    let message = "Compact failed; conversation was left unchanged.";
    let pane = hint(
        &mut state,
        crate::app::state::InputHintLevel::Error,
        message,
    );
    let config = Config::default();
    let screen = Rect::new(0, 0, 120, 30);
    let view = crate::ui::layout::compute(
        screen,
        &config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    );
    let rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    let buffer = draw(&state, screen.width, screen.height);
    let inner = rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| crate::layout::pane_inner_rect(*rect))
        .expect("pane rect");
    let target = state.pane_anywhere(pane).expect("pane");
    let toast = crate::ui::main_content::lx::toast_rect(inner, &target.lx).expect("toast 可见");
    // 盒的左上角必须是边框（不是历史正文）。
    let corner = buffer[(toast.x, toast.y)].symbol();
    assert_eq!(corner, "╭", "toast 盒左上角应为圆角边框");
    // 顶部一行出现提示文字的前半段。
    let header: String = (toast.x..toast.right())
        .map(|x| buffer[(x, toast.y + 1)].symbol())
        .collect();
    assert!(
        header.contains("Compact failed"),
        "窗格内必须画出提示正文：{header:?}"
    );
    // 屏幕第 2 行（全局 toast 的默认降级位置）不得出现这条提示。
    let global_row: String = (0..screen.width).map(|x| buffer[(x, 1)].symbol()).collect();
    assert!(
        !global_row.contains("Compact failed"),
        "提示不得出现在全局 toast 位置：{global_row:?}"
    );
}

/// 点 `[✕]` 关闭；点盒内其余位置不关闭；点盒外关闭。
#[test]
fn lx_toast_click_closes_it() {
    let mut state = lx_state();
    let pane = hint(
        &mut state,
        crate::app::state::InputHintLevel::Warning,
        "watch out",
    );
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let inner = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| crate::layout::pane_inner_rect(*rect))
        .expect("pane rect");
    let target = state.pane_anywhere(pane).expect("pane");
    let toast = crate::ui::main_content::lx::toast_rect(inner, &target.lx).expect("toast 可见");
    let mut sessions = std::collections::HashMap::new();

    // 点盒内正文：不关闭。
    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            toast.x + 2,
            toast.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .is_some_and(|target| target.lx.toast.is_some()),
        "点 toast 内部内容不关闭"
    );

    // 点右上角 [✕]：关闭。
    let close = crate::ui::widgets::modal::close_button_rect(toast).expect("close button");
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), close.x, close.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .is_some_and(|target| target.lx.toast.is_none()),
        "[✕] 关闭窗格内 toast"
    );

    // 重新弹一条，点盒外：关闭（且不吞掉这次点击）。
    hint(
        &mut state,
        crate::app::state::InputHintLevel::Warning,
        "watch out",
    );
    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 1,
            inner.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .is_some_and(|target| target.lx.toast.is_none()),
        "点 toast 之外关闭窗格内 toast"
    );
}

/// 悬停冻结消失计时：指针在盒上时 `tick` 不清除它。
#[test]
fn hovering_the_lx_toast_freezes_its_expiry() {
    let mut state = lx_state();
    let pane = hint(
        &mut state,
        crate::app::state::InputHintLevel::Warning,
        "watch out",
    );
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let inner = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| crate::layout::pane_inner_rect(*rect))
        .expect("pane rect");
    let target = state.pane_anywhere(pane).expect("pane");
    let toast = crate::ui::main_content::lx::toast_rect(inner, &target.lx).expect("toast 可见");
    let mut sessions = std::collections::HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, toast.x + 2, toast.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.toast.as_ref())
            .is_some_and(|toast| toast.hovered()),
        "悬停必须被记录"
    );
    // 悬停期间即使超过默认时长也不消失。
    let later = Instant::now() + Duration::from_secs(10);
    assert!(!update::tick(&mut state, later), "悬停时不安排变化");
    assert!(
        state
            .pane_anywhere(pane)
            .is_some_and(|target| target.lx.toast.is_some()),
        "悬停期间不过期"
    );
}

/// 每个窗格各自一条：两个 lx 窗格的反馈互不覆盖。
#[test]
fn each_pane_keeps_its_own_toast() {
    let mut state = lx_state();
    let first = state.active_tab().layout.focus();
    let second = state
        .active_tab_mut()
        .split_pane(first, ratatui::layout::Direction::Vertical)
        .expect("split");
    // 新窗格沿用源窗格视图（lx），反馈才有地方落。
    state.pane_mut_anywhere(second).expect("second pane").view = crate::app::state::PaneView::Lx;
    for pane in [first, second] {
        update::show_lx_hint(
            &mut state,
            pane,
            crate::app::state::InputHintLevel::Warning,
            format!("pane {pane:?}"),
        );
    }
    let first_message = state
        .pane_anywhere(first)
        .and_then(|target| target.lx.toast.as_ref())
        .map(|toast| toast.message.clone());
    let second_message = state
        .pane_anywhere(second)
        .and_then(|target| target.lx.toast.as_ref())
        .map(|toast| toast.message.clone());
    assert!(first_message.is_some(), "第一个窗格有自己的提示");
    assert!(second_message.is_some(), "第二个窗格有自己的提示");
    assert_ne!(first_message, second_message, "两条提示互不覆盖");
    assert!(state.toast.is_none(), "两条都在窗格内，全局槽为空");
}

/// `/compact` 失败：提示落在**该窗格**的 lx 槽里，全局槽保持为空。
///
/// 用户报的问题就是这个场景——截图里那条 `Compact failed; conversation was left
/// unchanged. (summary request failed: cancelled)` 横跨整个窗口，正是全局 toast
/// 在窄窗格放不下时的降级行为。
#[test]
fn compact_failure_lands_in_the_pane_slot() {
    let mut state = lx_state();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-compact".into());
        target.lx.compact.running = Some("op".into());
    }
    assert!(update::on_compact_failed(
        &mut state,
        pane,
        "summary request failed: cancelled"
    ));
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("失败提示必须可见");
    assert_eq!(toast.kind, crate::app::toast::ToastKind::Danger);
    assert!(
        toast.message.contains(text::LX_COMPACT_FAILED),
        "文案保真：{}",
        toast.message
    );
    assert!(state.toast.is_none(), "不得再占用全局 toast");
    // 执行态已清（不写检查点）。
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .compact
            .is_running()
    );
}

/// `compact` 空增量提示（warning 档）同样落在窗格内。
#[test]
fn compact_empty_notice_lands_in_the_pane_slot() {
    let mut state = lx_state();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-compact".into());
        target.lx.compact.running = Some("op".into());
    }
    assert!(update::on_compact_empty(&mut state, pane, "sess-compact"));
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("空增量提示必须可见");
    assert_eq!(toast.kind, crate::app::toast::ToastKind::Warning);
    assert!(state.toast.is_none(), "不得占用全局 toast");
}
