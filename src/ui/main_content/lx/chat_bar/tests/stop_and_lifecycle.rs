//! Esc 待命组合与生命周期（A19–A21）：停止反馈优先、结束后消失、回到底部共存。

use super::*;

// ===== 停止待命与生命周期（A19–A21）=====

/// A19：Esc 待命替换 todo/阶段，空间充足时统计保留。
#[test]
fn esc_armed_replaces_label_and_keeps_stats() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("修复输入框布局", PlanStepView::InProgress)]));
    with_stats(&mut lx);
    lx.chat.stop_armed_at = Some(std::time::Instant::now());
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(row.contains(text::LX_CHAT_STOP_PHRASES[0]), "{row:?}");
    assert!(
        !row.contains("修复输入框布局"),
        "停止提示覆盖 todo：{row:?}"
    );
    assert!(row.contains(STATS), "待命期间统计仍保留：{row:?}");
}

/// A19 续：极窄时停止反馈优先（统计在剩余列裁切，仍不越界）。
#[test]
fn esc_armed_keeps_stop_hint_when_stats_cannot_fit() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    with_stats(&mut lx);
    lx.chat.stop_armed_at = Some(std::time::Instant::now());
    let shortest = text::LX_CHAT_STOP_PHRASES[text::LX_CHAT_STOP_PHRASES.len() - 1];
    let shortest_w = u16::try_from(shortest.width()).unwrap_or(0);
    let width = shortest_w + 2;
    let (grid, bar) = draw_bar(&lx, width);
    let row = row_text(&grid, bar);
    assert!(row.contains(shortest), "停止提示优先：{row:?}");
    assert!(!row.contains(STATS), "统计按剩余列裁切：{row:?}");
    assert!(row.width() <= usize::from(width), "不得越界：{row:?}");
}

/// A19 续：待命超时（清除待命）后恢复当前 todo。
#[test]
fn clearing_stop_armed_restores_the_todo() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("修复输入框布局", PlanStepView::InProgress)]));
    lx.chat.stop_armed_at = Some(std::time::Instant::now());
    assert!(row_text(&draw_bar(&lx, 60).0, draw_bar(&lx, 60).1).contains("Press esc again"));
    lx.chat.stop_armed_at = None;
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(row.contains("修复输入框布局"), "{row:?}");
    assert!(!row.contains("Press esc again"), "{row:?}");
}

/// A20：请求结束后 loading 依据既有可见性规则消失。
#[test]
fn ended_turn_hides_the_loading_row() {
    let mut lx = streaming_lx("a1", vec![reasoning_step(0, true)]);
    assert!(visible(&lx), "流中可见");
    lx.chat.status.streaming = false;
    lx.chat.request = RequestState::Idle;
    assert!(!visible(&lx), "空闲且跟随底部：整行消失");
    // 面板打开时同样不显示（沿用原生命周期，不新增加载入口）。
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.task = Some(crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Processes,
        ..Default::default()
    });
    assert!(!visible(&lx), "后台任务面板打开时不画聊天栏");
}

/// A21：上滚未跟随底部时，回到底部控件与 loading 共存且不互相覆盖。
#[test]
fn jump_bottom_coexists_with_the_dynamic_label() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("修复输入框布局", PlanStepView::InProgress)]));
    lx.chat.follow_bottom = false;
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(row.contains("修复输入框布局"), "{row:?}");
    assert!(row.contains(text::LX_JUMP_BOTTOM), "{row:?}");
    assert!(row.contains(text::LX_JUMP_BOTTOM_HINT), "{row:?}");
}
